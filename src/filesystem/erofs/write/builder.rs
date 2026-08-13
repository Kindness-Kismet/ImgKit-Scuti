// EROFS 镜像构建器
use crate::filesystem::erofs::consts::*;
//
// 提供完整的 EROFS 镜像构建功能.

use crate::compression::Compressor;
use crate::filesystem::erofs::write::compress::{
    PhysicalCluster, build_compress_metadata, compress_file_data, create_compressor,
    get_algorithm_type,
};
use crate::filesystem::erofs::write::{
    ErofsConfig, FsConfig, InodeBuilder, SelinuxContexts, SuperblockBuilder,
};
use crate::filesystem::erofs::{ErofsError, Result};
use crate::utils::symlink::read_symlink_info;
use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::{BufWriter, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

// 文件信息
#[derive(Debug)]
struct FileInfo {
    path: PathBuf,
    fs_path: String,
    is_dir: bool,
    is_symlink: bool,
    symlink_target: Option<String>,
    size: u64,
    mode: u16,
    uid: u32,
    gid: u32,
    mtime: u64,
    children: Vec<String>,
    // 压缩相关
    file_data: Option<Vec<u8>>,                      // 文件原始数据
    physical_clusters: Option<Vec<PhysicalCluster>>, // 物理簇列表
    compress_meta_size: usize,                       // 压缩元数据大小 (map header + 索引)
    use_compression: bool,                           // 是否使用压缩
    // xattr 相关
    xattr_size: usize, // xattr 数据大小 (含 ibody header)
}

// EROFS 镜像构建器
pub struct ErofsBuilder {
    config: ErofsConfig,
    writer: BufWriter<File>,

    // superblock 构建器
    superblock: SuperblockBuilder,

    // 时间戳
    timestamp: u64,

    // 块大小
    block_size: u32,

    // 元数据起始块
    meta_blkaddr: u32,

    // 文件信息映射
    files: BTreeMap<String, FileInfo>,

    // NID 映射
    nid_map: BTreeMap<String, u64>,

    // SELinux 上下文
    selinux_contexts: Option<SelinuxContexts>,

    // 文件系统配置
    fs_config: Option<FsConfig>,

    // 压缩器
    compressor: Option<Box<dyn Compressor>>,
}

impl ErofsBuilder {
    pub fn new(config: ErofsConfig) -> Result<Self> {
        let file = File::create(&config.output_path)?;
        let writer = BufWriter::new(file);

        let timestamp = config.timestamp.unwrap_or_else(|| {
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0)
        });

        let block_size = config.block_size;

        // 加载 SELinux 上下文
        let selinux_contexts = config
            .file_contexts
            .as_ref()
            .and_then(|path| SelinuxContexts::from_file(path).ok());

        // 加载文件系统配置
        let fs_config = config
            .fs_config
            .as_ref()
            .and_then(|path| FsConfig::from_file(path).ok());

        // 创建 superblock 构建器
        let mut superblock = SuperblockBuilder::new(block_size)
            .with_volume_name(&config.volume_label)
            .with_build_time(timestamp)
            .with_feature_compat(EROFS_FEATURE_COMPAT_SB_CHKSUM | EROFS_FEATURE_COMPAT_MTIME);

        if let Some(uuid) = config.uuid {
            superblock = superblock.with_uuid(uuid);
        } else {
            // 生成随机 UUID
            let mut uuid = [0u8; 16];
            for (i, byte) in uuid.iter_mut().enumerate() {
                *byte = ((timestamp >> (i * 4)) & 0xFF) as u8 ^ (i as u8 * 17);
            }
            uuid[6] = (uuid[6] & 0x0F) | 0x40; // 版本 4
            uuid[8] = (uuid[8] & 0x3F) | 0x80; // 变体
            superblock = superblock.with_uuid(uuid);
        }

        // 元数据紧随 superblock 之后
        // meta_blkaddr 置为 0 (表示元数据从 block 0 开始, 即 EROFS_SUPER_OFFSET 之后)
        let meta_blkaddr = 0;

        // 创建压缩器
        let compressor = if let Some(ref algorithm) = config.compress_algorithm {
            let algorithm_type = get_algorithm_type(algorithm)?;
            superblock = superblock
                .with_compression(algorithm_type)
                .with_feature_incompat(EROFS_FEATURE_INCOMPAT_ZERO_PADDING);

            // 非 LZ4 算法需要设置 COMPR_CFGS 标志
            if algorithm_type != Z_EROFS_COMPRESSION_LZ4 {
                superblock.add_feature_incompat(EROFS_FEATURE_INCOMPAT_COMPR_CFGS);
            }

            Some(create_compressor(algorithm, config.compress_level)?)
        } else {
            None
        };

        Ok(ErofsBuilder {
            config,
            writer,
            superblock,
            timestamp,
            block_size,
            meta_blkaddr,
            files: BTreeMap::new(),
            nid_map: BTreeMap::new(),
            selinux_contexts,
            fs_config,
            compressor,
        })
    }

    // 将数据写入指定位置
    fn write_at(&mut self, offset: u64, data: &[u8]) -> Result<()> {
        self.writer.seek(SeekFrom::Start(offset))?;
        self.writer.write_all(data)?;
        Ok(())
    }

    // 计算 SELinux xattr 数据大小
    // xattr ibody header (12 字节) + 条目头 (4 字节) + name ("selinux" = 7 字节) + value
    fn calc_xattr_size(&self, fs_path: &str) -> usize {
        if let Some(ref ctx) = self.selinux_contexts
            && let Some(context) = ctx.lookup_without_mut(fs_path)
        {
            // xattr ibody header: 12 字节
            // 条目: 4 字节头 + 7 字节 name + value 长度, 按 4 字节对齐
            let entry_size = 4 + 7 + context.len();
            let aligned_entry_size = (entry_size + 3) & !3;
            return 12 + aligned_entry_size;
        }
        0
    }

    // 扫描源目录
    fn scan_directory(&mut self, source_path: &Path, fs_path: &str) -> Result<()> {
        let metadata = fs::symlink_metadata(source_path)?;

        // 获取 uid/gid/mode
        let (uid, gid, mode) = if let Some(ref cfg) = self.fs_config {
            cfg.get_attrs(fs_path, metadata.is_dir())
        } else {
            (
                self.config.root_uid,
                self.config.root_gid,
                if metadata.is_dir() { 0o755 } else { 0o644 },
            )
        };

        let mtime = metadata
            .modified()
            .ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map(|d| d.as_secs())
            .unwrap_or(self.timestamp);

        if metadata.is_dir() {
            let mut children = Vec::new();

            for entry in fs::read_dir(source_path)? {
                let entry = entry?;
                let name = entry.file_name();
                let name_str = name.to_string_lossy().to_string();
                let child_path = entry.path();
                let child_fs_path = if fs_path == "/" {
                    format!("/{}", name_str)
                } else {
                    format!("{}/{}", fs_path, name_str)
                };

                children.push(name_str);
                self.scan_directory(&child_path, &child_fs_path)?;
            }

            // 计算 xattr 大小
            let xattr_size = self.calc_xattr_size(fs_path);

            self.files.insert(
                fs_path.to_string(),
                FileInfo {
                    path: source_path.to_path_buf(),
                    fs_path: fs_path.to_string(),
                    is_dir: true,
                    is_symlink: false,
                    symlink_target: None,
                    size: 0,
                    mode: mode as u16,
                    uid,
                    gid,
                    mtime,
                    children,
                    file_data: None,
                    physical_clusters: None,
                    compress_meta_size: 0,
                    use_compression: false,
                    xattr_size,
                },
            );
        } else {
            // 检测符号链接
            let symlink_info = read_symlink_info(source_path)
                .map_err(|e| ErofsError::Io(std::io::Error::other(e.to_string())))?;

            // 计算 xattr 大小 (需在构建压缩元数据之前完成)
            let xattr_size = self.calc_xattr_size(fs_path);

            // 读取文件数据并按需压缩
            let (file_data, physical_clusters, compress_meta_size, use_compression) =
                if !symlink_info.is_symlink {
                    let data = fs::read(source_path)?;
                    let compressor = if data.is_empty() {
                        None
                    } else {
                        self.compressor.as_ref()
                    };

                    if let Some(compressor) = compressor {
                        let pclusters =
                            compress_file_data(&data, self.block_size, compressor.as_ref())?;

                        // 检查是否存在实际使用压缩的物理簇
                        let has_compressed = pclusters
                            .iter()
                            .any(|pc| pc.logical_clusters.iter().any(|lc| lc.is_compressed));

                        if has_compressed {
                            // 获取压缩算法类型
                            let algorithm = get_algorithm_type(
                                self.config
                                    .compress_algorithm
                                    .as_ref()
                                    .unwrap_or(&"lz4".to_string()),
                            )?;

                            // 构建压缩元数据以获得准确大小
                            // 使用占位 start_blkaddr (1) 以避免 u32 下溢
                            let (header, indexes) = build_compress_metadata(
                                data.len() as u64,
                                self.block_size,
                                algorithm,
                                &pclusters,
                                1, // 占位值, 避免 0 - 1 下溢
                                xattr_size,
                            )?;

                            let meta_size = header.len() + indexes.len();

                            (Some(data), Some(pclusters), meta_size, true)
                        } else {
                            (Some(data), Some(pclusters), 0, false)
                        }
                    } else {
                        (Some(data), None, 0, false)
                    }
                } else {
                    (None, None, 0, false)
                };

            self.files.insert(
                fs_path.to_string(),
                FileInfo {
                    path: source_path.to_path_buf(),
                    fs_path: fs_path.to_string(),
                    is_dir: false,
                    is_symlink: symlink_info.is_symlink,
                    symlink_target: symlink_info.target,
                    size: if symlink_info.is_symlink {
                        0
                    } else {
                        metadata.len()
                    },
                    mode: mode as u16,
                    uid,
                    gid,
                    mtime,
                    children: Vec::new(),
                    file_data,
                    physical_clusters,
                    compress_meta_size,
                    use_compression,
                    xattr_size,
                },
            );
        }

        Ok(())
    }

    // 计算目录项数据大小 (按块切分)
    // EROFS 目录数据按 block 组织, 每个 block 的 nameoff 是相对该 block 起始处的偏移.
    fn calc_dentry_size(&self, children: &[String]) -> usize {
        let block_size = self.block_size as usize;

        // EROFS 要求目录项按名称字典序排序
        let mut sorted_children: Vec<&String> = children.iter().collect();
        sorted_children.sort();

        // 收集所有目录项 (. 和 .. 以及子项)
        let mut entries: Vec<&[u8]> = Vec::new();
        entries.push(b".");
        entries.push(b"..");
        for child_name in &sorted_children {
            entries.push(child_name.as_bytes());
        }

        // 按块计算总大小
        let mut total_size = 0;
        let mut entry_idx = 0;

        while entry_idx < entries.len() {
            // 计算当前 block 能容纳多少个目录项
            let mut block_used = 0;
            let mut block_entries = 0;

            while entry_idx + block_entries < entries.len() {
                let name = entries[entry_idx + block_entries];
                let entry_size = 12 + name.len(); // dirent (12) + name

                if block_used + entry_size > block_size {
                    break;
                }
                block_used += entry_size;
                block_entries += 1;
            }

            // 若一个目录项都放不下, 说明名称过长, 必须强制放入.
            if block_entries == 0 {
                let name = entries[entry_idx];
                block_used = 12 + name.len();
                block_entries = 1;
            }

            entry_idx += block_entries;

            // 当前 block 的大小 (可能不足一个完整 block)
            let remaining_entries = entries.len() - entry_idx;
            if remaining_entries == 0 {
                // 最后一个 block, 无需填充到 block 边界
                total_size += block_used;
            } else {
                // 非最后一个 block, 填充到 block 边界
                total_size += block_size;
            }
        }

        total_size
    }

    // 构建目录项数据 (按块切分)
    fn build_dentries(&self, children: &[String], parent_fs_path: &str) -> Vec<u8> {
        let block_size = self.block_size as usize;
        let mut buf = Vec::new();

        // EROFS 要求目录项按名称字典序排序
        let mut sorted_children: Vec<&String> = children.iter().collect();
        sorted_children.sort();

        // 收集所有目录项
        let mut entries: Vec<(u64, u8, Vec<u8>)> = Vec::new();

        // . 条目 (当前目录)
        let self_nid = self.nid_map.get(parent_fs_path).copied().unwrap_or(0);
        entries.push((self_nid, EROFS_FT_DIR, b".".to_vec()));

        // .. 条目 (父目录)
        let parent_path = if parent_fs_path == "/" {
            "/".to_string()
        } else {
            let parts: Vec<&str> = parent_fs_path.rsplitn(2, '/').collect();
            if parts.len() > 1 && !parts[1].is_empty() {
                parts[1].to_string()
            } else {
                "/".to_string()
            }
        };
        let parent_nid = self.nid_map.get(&parent_path).copied().unwrap_or(self_nid);
        entries.push((parent_nid, EROFS_FT_DIR, b"..".to_vec()));

        // 子项 (已排序)
        for child_name in &sorted_children {
            let child_fs_path = if parent_fs_path == "/" {
                format!("/{}", child_name)
            } else {
                format!("{}/{}", parent_fs_path, child_name)
            };
            if let Some(nid) = self.nid_map.get(&child_fs_path) {
                let file_info = self.files.get(&child_fs_path);
                let file_type = if let Some(info) = file_info {
                    if info.is_dir {
                        EROFS_FT_DIR
                    } else if info.is_symlink {
                        EROFS_FT_SYMLINK
                    } else {
                        EROFS_FT_REG_FILE
                    }
                } else {
                    EROFS_FT_REG_FILE
                };
                entries.push((*nid, file_type, child_name.as_bytes().to_vec()));
            }
        }

        // 按块写入目录数据
        let mut entry_idx = 0;

        while entry_idx < entries.len() {
            // 计算当前 block 能容纳多少个目录项
            let mut block_entries = 0;
            let mut block_used = 0;

            while entry_idx + block_entries < entries.len() {
                let (_, _, ref name) = entries[entry_idx + block_entries];
                let entry_size = 12 + name.len();

                if block_used + entry_size > block_size {
                    break;
                }
                block_used += entry_size;
                block_entries += 1;
            }

            // 若一个目录项都放不下, 强制放入一个
            if block_entries == 0 {
                block_entries = 1;
            }

            // 写入当前 block 的目录项
            // nameoff 从 dirent 结构之后开始 (相对该 block 起始处)
            let mut name_offset = 12 * block_entries;
            let block_start = buf.len();

            for i in 0..block_entries {
                let (nid, file_type, ref name) = entries[entry_idx + i];

                // nid (8 字节)
                buf.extend_from_slice(&nid.to_le_bytes());
                // nameoff (2 字节) - 相对该 block 起始处的偏移
                buf.extend_from_slice(&(name_offset as u16).to_le_bytes());
                // file_type (1 字节)
                buf.push(file_type);
                // reserved (1 字节)
                buf.push(0);

                name_offset += name.len();
            }

            // 写入名称
            for i in 0..block_entries {
                let (_, _, ref name) = entries[entry_idx + i];
                buf.extend_from_slice(name);
            }

            // 填充到 block 边界 (若非最后一个 block)
            let remaining_entries = entries.len() - entry_idx - block_entries;
            if remaining_entries > 0 {
                let current_block_size = buf.len() - block_start;
                let padding = block_size - current_block_size;
                buf.resize(buf.len() + padding, 0);
            }

            entry_idx += block_entries;
        }

        buf
    }

    // 构建镜像
    pub fn build(&mut self) -> Result<()> {
        let source_dir = self.config.source_dir.clone();
        let mount_point = self.config.mount_point.clone();

        // 扫描源目录
        if source_dir.exists() {
            self.scan_directory(&source_dir, &mount_point)?;
        }

        // 第一遍: 计算每个 inode 的大小与偏移, 并分配 NID
        // NID = inode 绝对偏移 / 32 (相对镜像起始处)
        // meta_blkaddr = 0 时, 元数据从 EROFS_SUPER_OFFSET 开始
        // meta_blkaddr > 0 时, 元数据从 meta_blkaddr * block_size 开始
        // 注意: superblock 占用 128 字节, 范围为 0x400 到 0x47F
        // 若存在压缩配置数据, 则紧随 superblock 之后
        // 根 inode 位于 superblock 与压缩配置数据之后 (32 字节对齐)
        let compr_cfgs_size = self.superblock.compr_cfgs_size();
        let meta_base = if self.meta_blkaddr == 0 {
            // superblock 之后接压缩配置数据, 再按 32 字节对齐
            let base = EROFS_SUPER_OFFSET + 128 + compr_cfgs_size as u64;
            // 对齐到 32 字节
            base.div_ceil(32) * 32
        } else {
            self.meta_blkaddr as u64 * self.block_size as u64
        };
        let mut current_offset: u64 = 0;

        let paths: Vec<String> = self.files.keys().cloned().collect();
        for path in &paths {
            // 计算 inode 大小
            let info = self
                .files
                .get(path)
                .ok_or_else(|| ErofsError::Io(std::io::Error::other("file info not found")))?;

            // 基础 inode 大小 + xattr 大小
            let base_inode_size = if info.is_dir {
                let dentry_size = self.calc_dentry_size(&info.children);
                // 目录数据: 仅将不足一个 block 的尾部内联
                // 但 inline 数据不得导致 inode 跨越 block 边界
                let max_inline = self.block_size as usize - 32 - info.xattr_size;
                let tail_size = dentry_size % self.block_size as usize;
                let actual_inline = if tail_size > max_inline { 0 } else { tail_size };
                32 + actual_inline as u64 // compact inode (32 字节) + inline 数据
            } else if info.is_symlink {
                let target_len = info.symlink_target.as_ref().map(|t| t.len()).unwrap_or(0);
                32 + target_len as u64 // compact inode (32 字节)
            } else if info.use_compression {
                // 压缩文件: inode + 压缩元数据 (map header + 索引)
                32 + info.compress_meta_size as u64
            } else {
                // 普通文件: 超过一个 block 的部分存入外部数据块, 不足一个 block 的部分内联.
                // 但 inline 数据不得导致 inode 跨越 block 边界
                let max_inline = self.block_size as u64 - 32 - info.xattr_size as u64;
                let tail_size = info.size % self.block_size as u64;
                let actual_inline = if tail_size > max_inline { 0 } else { tail_size };
                32 + actual_inline // compact inode (32 字节) + inline 数据
            };

            // 加上 xattr 大小
            let mut inode_size = base_inode_size + info.xattr_size as u64;

            // 若存在 xattr 且带有压缩元数据, 需要考虑 8 字节对齐填充
            if info.xattr_size > 0 && info.use_compression {
                let before_compress = 32 + info.xattr_size as u64;
                let aligned = (before_compress + 7) & !7;
                let padding = aligned - before_compress;
                inode_size += padding;
            }

            let aligned_inode_size = inode_size.div_ceil(32) * 32;

            // 检查 inode 是否会跨越 block 边界
            let mut absolute_offset = meta_base + current_offset;
            let block_offset = absolute_offset % self.block_size as u64;

            if block_offset + aligned_inode_size > self.block_size as u64 {
                // 会跨越 block 边界, 需填充到下一个 block 起始处
                let padding = self.block_size as u64 - block_offset;
                current_offset += padding;
                absolute_offset = meta_base + current_offset;
            }

            // 计算 NID (基于相对镜像起始处的绝对偏移)
            let nid = absolute_offset / 32;
            self.nid_map.insert(path.clone(), nid);

            // 对齐到 32 字节
            current_offset += aligned_inode_size;
        }

        let meta_size = current_offset;

        // 计算数据块的起始位置
        let meta_blocks = meta_size.div_ceil(self.block_size as u64) as u32;
        // 预留 1 个保护块, 避免元数据估算的边界误差导致与数据区重叠
        let data_blkaddr = self.meta_blkaddr + meta_blocks + 1;

        // 设置 superblock
        self.superblock.set_meta_blkaddr(self.meta_blkaddr);
        self.superblock
            .set_root_nid(self.nid_map.get(&mount_point).copied().unwrap_or(0));
        self.superblock.set_inos(self.files.len() as u64);

        // 第二遍: 构建并写入数据
        let mut inode_entries: Vec<(u64, Vec<u8>)> = Vec::new();
        let mut data_entries: Vec<(u64, Vec<u8>)> = Vec::new();
        let mut data_offset = data_blkaddr as u64 * self.block_size as u64;
        let mut next_ino: u32 = 1;

        // 收集文件信息副本
        let file_entries: Vec<(String, FileInfo)> = self
            .files
            .iter()
            .map(|(k, v)| {
                (
                    k.clone(),
                    FileInfo {
                        path: v.path.clone(),
                        fs_path: v.fs_path.clone(),
                        is_dir: v.is_dir,
                        is_symlink: v.is_symlink,
                        symlink_target: v.symlink_target.clone(),
                        size: v.size,
                        mode: v.mode,
                        uid: v.uid,
                        gid: v.gid,
                        mtime: v.mtime,
                        children: v.children.clone(),
                        file_data: v.file_data.clone(),
                        physical_clusters: v.physical_clusters.clone(),
                        compress_meta_size: v.compress_meta_size,
                        use_compression: v.use_compression,
                        xattr_size: v.xattr_size,
                    },
                )
            })
            .collect();

        for (path, info) in &file_entries {
            let nid = self.nid_map.get(path).copied().unwrap_or(0);
            let inode_offset = nid * 32;

            // 获取 SELinux 上下文
            let selinux_context = if let Some(ref mut ctx) = self.selinux_contexts {
                ctx.lookup(path)
            } else {
                None
            };

            let inode_data = if info.is_dir {
                // 构建目录 inode
                let dentry_data = self.build_dentries(&info.children, &info.fs_path);
                // nlink = 2 (. 与 ..), 不计入子目录
                let mut inode = InodeBuilder::new_dir(info.mode, info.uid, info.gid)
                    .with_mtime(info.mtime)
                    .with_nlink(2)
                    .with_ino(next_ino)
                    .with_extended(false)
                    .with_size(dentry_data.len() as u64);

                // 目录数据处理: 与文件类似, 超过一个 block 的部分存入外部数据块
                // inline 数据不得导致 inode 跨越 block 边界
                let max_inline = self.block_size as usize - 32 - info.xattr_size;
                let tail_size = dentry_data.len() % self.block_size as usize;

                // 若尾部数据过大, 则不使用 inline, 全部数据存入外部块
                let (nblocks, actual_inline) = if tail_size > max_inline {
                    // 全部数据存入外部块 (向上取整)
                    (dentry_data.len().div_ceil(self.block_size as usize), 0)
                } else {
                    (dentry_data.len() / self.block_size as usize, tail_size)
                };

                if nblocks > 0 {
                    // 存在需要写入外部数据块的数据
                    let data_blk = (data_offset / self.block_size as u64) as u32;
                    // 存放完整的块数据 (可能需要填充到 block 边界)
                    let external_size = nblocks * self.block_size as usize;
                    let mut block_data =
                        dentry_data[..external_size.min(dentry_data.len())].to_vec();
                    // 数据不足时补零
                    if block_data.len() < external_size {
                        block_data.resize(external_size, 0);
                    }

                    // 记录数据块
                    data_entries.push((data_offset, block_data));
                    data_offset += external_size as u64;

                    // 设置 raw_blkaddr
                    inode = inode.with_raw_blkaddr(data_blk);
                }

                if actual_inline > 0 {
                    // 存在 inline 数据
                    let inline_data = dentry_data[nblocks * self.block_size as usize..].to_vec();
                    inode = inode.with_tail_inline_data(inline_data);
                    if nblocks == 0 {
                        // 仅有 inline 数据, 将 raw_blkaddr 设为 0xffffffff
                        inode = inode.with_raw_blkaddr(0xffffffff);
                    }
                } else if nblocks > 0 {
                    // 无 inline 数据但存在外部块, 使用 PLAIN 布局
                    inode = inode.with_data_layout(EROFS_INODE_FLAT_PLAIN);
                }

                next_ino += 1;

                if let Some(ref ctx) = selinux_context {
                    inode = inode.with_selinux_context(ctx);
                }

                inode.build()?
            } else if info.is_symlink {
                // 构建符号链接 inode
                let target = info
                    .symlink_target
                    .as_ref()
                    .map(|t| t.as_bytes().to_vec())
                    .unwrap_or_default();
                let mut inode = InodeBuilder::new_symlink(info.uid, info.gid)
                    .with_mtime(info.mtime)
                    .with_ino(next_ino)
                    .with_extended(false)
                    .with_inline_data(target);

                next_ino += 1;

                if let Some(ref ctx) = selinux_context {
                    inode = inode.with_selinux_context(ctx);
                }

                inode.build()?
            } else {
                // 文件处理
                let file_data = info
                    .file_data
                    .as_ref()
                    .ok_or_else(|| ErofsError::Io(std::io::Error::other("file data not found")))?;

                let mut inode = InodeBuilder::new_file(info.mode, info.uid, info.gid)
                    .with_mtime(info.mtime)
                    .with_ino(next_ino)
                    .with_size(file_data.len() as u64)
                    .with_extended(false);

                if info.use_compression {
                    // 使用压缩布局
                    let physical_clusters = info
                        .physical_clusters
                        .as_ref()
                        .ok_or_else(|| ErofsError::Io(std::io::Error::other("no pcluster data")))?;

                    // 获取压缩算法类型
                    let algorithm = get_algorithm_type(
                        self.config
                            .compress_algorithm
                            .as_ref()
                            .unwrap_or(&"lz4".to_string()),
                    )?;

                    // 写入压缩数据块并记录起始块地址
                    let data_blk = (data_offset / self.block_size as u64) as u32;

                    // 压缩数据连续存放.
                    // 仅压缩簇使用 ZERO_PADDING 前置补零, PLAIN 簇保持数据在前, 尾部补零.
                    let mut all_compressed_data = Vec::new();
                    for pcluster in physical_clusters {
                        // 计算对齐后的大小
                        let plen = pcluster.compressed_size.div_ceil(self.block_size as usize)
                            * self.block_size as usize;
                        let is_compressed = pcluster
                            .logical_clusters
                            .first()
                            .map(|lc| lc.is_compressed)
                            .unwrap_or(false);

                        if is_compressed {
                            // ZERO_PADDING: 压缩流置于 block 尾部, 前部补零.
                            let padding = plen - pcluster.compressed_size;
                            all_compressed_data.resize(all_compressed_data.len() + padding, 0);
                            all_compressed_data.extend_from_slice(&pcluster.compressed_data);
                        } else {
                            // PLAIN: 原始数据从 block 起始处写入, 尾部补零.
                            all_compressed_data.extend_from_slice(&pcluster.compressed_data);
                            if pcluster.compressed_size < plen {
                                all_compressed_data.resize(
                                    all_compressed_data.len() + (plen - pcluster.compressed_size),
                                    0,
                                );
                            }
                        }
                    }

                    // 一次性写入全部压缩数据
                    if !all_compressed_data.is_empty() {
                        data_entries.push((data_offset, all_compressed_data.clone()));
                        data_offset += all_compressed_data.len() as u64;
                    }

                    // 使用正确的 start_blkaddr 重建压缩元数据
                    let (header, indexes) = build_compress_metadata(
                        file_data.len() as u64,
                        self.block_size,
                        algorithm,
                        physical_clusters,
                        data_blk,
                        info.xattr_size,
                    )?;

                    inode = inode
                        .with_data_layout(EROFS_INODE_COMPRESSED_COMPACT)
                        .with_compress_header(header)
                        .with_compress_indexes(indexes)
                        .with_raw_blkaddr(data_blk);
                } else {
                    // 不压缩: 仅采用纯外部块或纯 inline, 避免混合尾部布局导致不一致
                    let max_inline = self.block_size as usize - 32 - info.xattr_size;
                    let (nblocks, actual_inline) = if file_data.len() <= max_inline {
                        (0usize, file_data.len())
                    } else {
                        (file_data.len().div_ceil(self.block_size as usize), 0usize)
                    };

                    if nblocks > 0 {
                        // 存在需要写入外部数据块的数据
                        let data_blk = (data_offset / self.block_size as u64) as u32;
                        // 存放数据 (可能需要填充到 block 边界)
                        let external_size = nblocks * self.block_size as usize;
                        let mut block_data = file_data.to_vec();

                        // 填充到 block 边界
                        if block_data.len() < external_size {
                            block_data.resize(external_size, 0);
                        }

                        // 记录数据块
                        data_entries.push((data_offset, block_data));
                        data_offset += external_size as u64;

                        // 设置 raw_blkaddr
                        inode = inode.with_raw_blkaddr(data_blk);
                    }

                    if actual_inline > 0 {
                        // 小文件内联
                        inode = inode.with_tail_inline_data(file_data.clone());
                        if nblocks == 0 {
                            // 仅有 inline 数据, 将 raw_blkaddr 设为 0xffffffff
                            inode = inode.with_raw_blkaddr(0xffffffff);
                        }
                    } else if nblocks > 0 {
                        // 无 inline 数据但存在外部块, 使用 PLAIN 布局
                        inode = inode.with_data_layout(EROFS_INODE_FLAT_PLAIN);
                    }
                }

                next_ino += 1;

                if let Some(ref ctx) = selinux_context {
                    inode = inode.with_selinux_context(ctx);
                }

                inode.build()?
            };

            // 记录 inode 数据
            inode_entries.push((inode_offset, inode_data.clone()));
        }

        // 写入所有数据块
        for (offset, data) in data_entries {
            self.write_at(offset, &data)?;
        }

        // 写入所有 inode
        for (offset, data) in &inode_entries {
            self.write_at(*offset, data)?;
        }

        // 计算总块数
        let total_blocks = data_offset.div_ceil(self.block_size as u64) as u32;
        self.superblock.set_blocks(total_blocks);

        // 构建首个 block 的数据 (从 superblock 起始处开始, 用于 CRC 计算)
        // 块数据 = superblock (128 字节) + 压缩配置数据 + inode 元数据
        let block_data_len = self.block_size as usize - EROFS_SUPER_OFFSET as usize;
        let mut block_data = vec![0u8; block_data_len];

        // 若存在压缩配置数据, 先填入块缓冲区
        let compr_cfgs_data =
            if self.superblock.feature_incompat() & EROFS_FEATURE_INCOMPAT_COMPR_CFGS != 0 {
                self.superblock.build_compr_cfgs()
            } else {
                Vec::new()
            };
        if !compr_cfgs_data.is_empty() {
            let cfgs_offset = EROFS_SUPER_BLOCK_SIZE;
            let copy_len = compr_cfgs_data.len().min(block_data_len - cfgs_offset);
            block_data[cfgs_offset..cfgs_offset + copy_len]
                .copy_from_slice(&compr_cfgs_data[..copy_len]);
        }

        // 将 inode 数据填入块缓冲区 (偏移相对于 EROFS_SUPER_OFFSET)
        for (offset, data) in &inode_entries {
            // offset 为绝对偏移, 需转换为相对 EROFS_SUPER_OFFSET 的偏移
            let rel_offset = *offset as usize - EROFS_SUPER_OFFSET as usize;
            // 仅复制首个 block 范围内的部分 (用于 CRC 计算)
            if rel_offset >= EROFS_SUPER_BLOCK_SIZE && rel_offset < block_data_len {
                let copy_len = data.len().min(block_data_len - rel_offset);
                block_data[rel_offset..rel_offset + copy_len].copy_from_slice(&data[..copy_len]);
            }
        }

        // 写入 superblock (仅写入 128 字节, 含正确的校验和)
        let sb_data = self.superblock.build_with_checksum(&block_data)?;
        self.write_at(EROFS_SUPER_OFFSET, &sb_data)?;

        // 写入压缩配置数据 (紧随 superblock 之后)
        if !compr_cfgs_data.is_empty() {
            let cfgs_offset = EROFS_SUPER_OFFSET + EROFS_SUPER_BLOCK_SIZE as u64;
            self.write_at(cfgs_offset, &compr_cfgs_data)?;
        }

        // 设置文件大小
        self.writer
            .get_ref()
            .set_len(total_blocks as u64 * self.block_size as u64)?;
        self.writer.flush()?;

        Ok(())
    }
}

// 便捷函数
pub fn build_erofs_image(config: ErofsConfig) -> Result<()> {
    let mut builder = ErofsBuilder::new(config)?;
    builder.build()
}
