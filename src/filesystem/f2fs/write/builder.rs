// F2FS 镜像构建器
use crate::filesystem::f2fs::consts::*;
//
// 提供完整的 F2FS 镜像构建能力.

use crate::filesystem::f2fs::types::*;
use crate::filesystem::f2fs::write::{
    CheckpointBuilder, CursegInfo, DentryBlockBuilder, DentryInfo, DirectNodeBuilder, FsConfig,
    IndirectNodeBuilder, InodeBuilder, NatManager, SegmentAllocator, SelinuxContexts, SitManager,
    SsaManager, SuperblockBuilder,
};
use crate::filesystem::f2fs::{F2fsError, Result};
use crate::utils::symlink::read_symlink_info;
use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{BufWriter, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

// 目录信息 (用于延迟写入 inode)
struct DirInfo {
    path: PathBuf,
    fs_path: String,
    ino: u32,
    blkaddr: u32,
}

// F2FS 镜像构建器
pub struct F2fsBuilder {
    config: F2fsBuilderConfig,
    writer: BufWriter<File>,

    // 元数据管理器
    superblock: SuperblockBuilder,
    sit: SitManager,
    nat: NatManager,
    ssa: SsaManager,
    segment_alloc: SegmentAllocator,

    // 当前状态
    cp_ver: u64,
    timestamp: u64,

    // inode 映射 (path -> ino)
    inode_map: HashMap<String, u32>,

    // SELinux 上下文
    selinux_contexts: Option<SelinuxContexts>,

    // 文件系统配置
    fs_config: Option<FsConfig>,
}

impl F2fsBuilder {
    // 创建新的构建器
    pub fn new(config: F2fsBuilderConfig) -> Result<Self> {
        // 创建输出文件
        let file = File::create(&config.output_path)?;
        let writer = BufWriter::new(file);

        // 获取时间戳
        let timestamp = config.timestamp.unwrap_or_else(|| {
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0)
        });

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

        // 创建 superblock 构建器并计算布局
        let mut superblock = SuperblockBuilder::new(config.image_size)
            .with_features(config.features.clone())
            .with_label(&config.volume_label);
        superblock.calculate_layout()?;

        let layout = superblock
            .layout()
            .ok_or_else(|| F2fsError::InvalidData("layout calculation failed".into()))?;

        // 初始化各管理器
        let sit = SitManager::new(
            layout.segment_count_main,
            layout.sit_blkaddr,
            layout.main_blkaddr,
        );
        let nat = NatManager::new(layout.nat_blkaddr, layout.segment_count_nat);
        let ssa = SsaManager::new(
            layout.segment_count_main,
            layout.ssa_blkaddr,
            layout.main_blkaddr,
        );
        let segment_alloc = SegmentAllocator::new(layout.main_blkaddr, layout.segment_count_main);

        Ok(F2fsBuilder {
            config,
            writer,
            superblock,
            sit,
            nat,
            ssa,
            segment_alloc,
            cp_ver: 1,
            timestamp,
            inode_map: HashMap::new(),
            selinux_contexts,
            fs_config,
        })
    }

    // 构建镜像
    pub fn build(&mut self) -> Result<()> {
        // 初始化镜像文件
        self.writer.get_ref().set_len(self.config.image_size)?;

        // 创建根目录并加载内容
        let root_blkaddr = self.create_root_dir()?;
        let source_dir = self.config.source_dir.clone();
        let mount_point = self.config.mount_point.clone();

        let (root_data_addrs, subdir_count) = if source_dir.exists() {
            self.load_directory(&source_dir, F2FS_ROOT_INO, F2FS_ROOT_INO, &mount_point)?
        } else {
            // 即使没有源目录, 也要创建包含 "." 与 ".." 的根目录数据块
            let mut dentry_block = DentryBlockBuilder::new();
            dentry_block.add_entry(DentryInfo::new(b".", F2FS_ROOT_INO, FileType::Dir));
            dentry_block.add_entry(DentryInfo::new(b"..", F2FS_ROOT_INO, FileType::Dir));

            let data_blkaddr = self.segment_alloc.alloc_data_block(SegType::HotData)?;
            self.sit
                .mark_block_used(data_blkaddr, CURSEG_HOT_DATA as u16)?;
            self.ssa.set_data_summary(data_blkaddr, F2FS_ROOT_INO, 0)?;

            let dentry_data = dentry_block.build()?;
            self.write_block(data_blkaddr, &dentry_data)?;

            (vec![data_blkaddr], 0)
        };

        // 写入根目录 inode
        self.write_dir_inode(
            F2FS_ROOT_INO,
            root_blkaddr,
            F2FS_ROOT_INO,
            b"/",
            1,
            &root_data_addrs,
            subdir_count,
            &mount_point,
        )?;

        // 写入最终元数据
        self.finalize()
    }

    // 创建根目录 (仅分配 inode 块)
    fn create_root_dir(&mut self) -> Result<u32> {
        let root_blkaddr = self.segment_alloc.alloc_node_block(SegType::HotNode)?;
        self.nat.init_reserved_inodes(root_blkaddr);
        self.sit
            .mark_block_used(root_blkaddr, CURSEG_HOT_NODE as u16)?;
        self.ssa.set_node_summary(root_blkaddr, F2FS_ROOT_INO)?;
        self.inode_map
            .insert(self.config.mount_point.clone(), F2FS_ROOT_INO);
        Ok(root_blkaddr)
    }

    // 写入目录 inode
    #[allow(clippy::too_many_arguments)]
    fn write_dir_inode(
        &mut self,
        ino: u32,
        blkaddr: u32,
        pino: u32,
        name: &[u8],
        depth: u32,
        data_addrs: &[u32],
        child_count: u32,
        fs_path: &str,
    ) -> Result<()> {
        // 获取 uid/gid/mode
        let (uid, gid, mode) = if let Some(ref cfg) = self.fs_config {
            cfg.get_attrs(fs_path, true)
        } else {
            (self.config.root_uid, self.config.root_gid, 0o755)
        };

        let mut inode = InodeBuilder::new_dir(mode as u16, uid, gid)
            .with_timestamp(self.timestamp)
            .with_pino(pino)
            .with_name(name)
            .with_depth(depth)
            .with_links(2 + child_count)
            .with_size(F2FS_BLKSIZE as u64 * data_addrs.len() as u64)
            .with_blocks((1 + data_addrs.len()) as u64)
            .with_addrs(data_addrs.to_vec());

        // 设置 SELinux 上下文
        if let Some(ref mut ctx) = self.selinux_contexts
            && let Some(context) = ctx.lookup(fs_path)
        {
            inode = inode.with_selinux_context(&context);
        }

        self.write_block(blkaddr, &inode.build(ino, ino, self.cp_ver)?)
    }

    // 加载目录内容
    fn load_directory(
        &mut self,
        source_path: &Path,
        parent_ino: u32,
        parent_pino: u32,
        parent_fs_path: &str,
    ) -> Result<(Vec<u32>, u32)> {
        let mut entries: Vec<_> = fs::read_dir(source_path)?.filter_map(|e| e.ok()).collect();
        entries.sort_by(|a, b| {
            let a_name = a.file_name();
            let b_name = b.file_name();
            let a_name_str = a_name.to_string_lossy();
            let b_name_str = b_name.to_string_lossy();
            let a_fs_path = if parent_fs_path == "/" {
                format!("/{}", a_name_str)
            } else {
                format!("{}/{}", parent_fs_path, a_name_str)
            };
            let b_fs_path = if parent_fs_path == "/" {
                format!("/{}", b_name_str)
            } else {
                format!("{}/{}", parent_fs_path, b_name_str)
            };
            let a_order = self
                .fs_config
                .as_ref()
                .and_then(|cfg| cfg.order_of(&a_fs_path));
            let b_order = self
                .fs_config
                .as_ref()
                .and_then(|cfg| cfg.order_of(&b_fs_path));

            match (a_order, b_order) {
                (Some(x), Some(y)) => x
                    .cmp(&y)
                    .then_with(|| a_name.as_encoded_bytes().cmp(b_name.as_encoded_bytes())),
                (Some(_), None) => std::cmp::Ordering::Less,
                (None, Some(_)) => std::cmp::Ordering::Greater,
                (None, None) => a_name.as_encoded_bytes().cmp(b_name.as_encoded_bytes()),
            }
        });

        // 支持多个 dentry 块
        let mut dentry_blocks: Vec<DentryBlockBuilder> = vec![DentryBlockBuilder::new()];
        dentry_blocks[0].add_entry(DentryInfo::new(b".", parent_ino, FileType::Dir));
        dentry_blocks[0].add_entry(DentryInfo::new(b"..", parent_pino, FileType::Dir));

        let mut subdirs: Vec<DirInfo> = Vec::new();
        let mut subdir_count = 0u32;

        for entry in &entries {
            let name_bytes = entry.file_name();
            let name = name_bytes.as_encoded_bytes();
            let metadata = entry.metadata()?;

            // 计算文件系统路径
            let name_str = String::from_utf8_lossy(name);
            let fs_path = if parent_fs_path == "/" {
                format!("/{}", name_str)
            } else {
                format!("{}/{}", parent_fs_path, name_str)
            };

            let (ino, file_type) = if metadata.is_dir() {
                subdir_count += 1;
                let (ino, blkaddr) = self.alloc_dir_inode()?;
                subdirs.push(DirInfo {
                    path: entry.path(),
                    fs_path: fs_path.clone(),
                    ino,
                    blkaddr,
                });
                self.inode_map
                    .insert(entry.path().to_string_lossy().to_string(), ino);
                (ino, FileType::Dir)
            } else {
                // 检测符号链接 (支持 Windows !<symlink> 格式)
                let symlink_info = read_symlink_info(&entry.path())
                    .map_err(|e| F2fsError::Io(std::io::Error::other(e.to_string())))?;

                let file_type = if symlink_info.is_symlink {
                    FileType::Symlink
                } else {
                    FileType::RegFile
                };
                let ino = self.create_file_inode(
                    &entry.path(),
                    parent_ino,
                    &metadata,
                    &fs_path,
                    symlink_info.target.as_deref(),
                )?;
                (ino, file_type)
            };

            // 先尝试加入当前块, 满了则新建一个块
            let dentry = DentryInfo::new(name, ino, file_type);
            let added = dentry_blocks
                .last_mut()
                .is_some_and(|current_block| current_block.add_entry(dentry.clone()));
            if !added {
                let mut new_block = DentryBlockBuilder::new();
                new_block.add_entry(dentry);
                dentry_blocks.push(new_block);
            }
        }

        // 写入所有目录数据块
        let mut data_addrs = Vec::new();
        for builder in &dentry_blocks {
            if !builder.is_empty() {
                let blkaddr = self.segment_alloc.alloc_data_block(SegType::HotData)?;
                self.sit.mark_block_used(blkaddr, CURSEG_HOT_DATA as u16)?;
                self.ssa
                    .set_data_summary(blkaddr, parent_ino, data_addrs.len() as u16)?;
                self.write_block(blkaddr, &builder.build()?)?;
                data_addrs.push(blkaddr);
            }
        }

        // 递归处理子目录
        for dir in subdirs {
            let name = dir
                .path
                .file_name()
                .map(|n| n.as_encoded_bytes().to_vec())
                .unwrap_or_default();
            let (sub_addrs, sub_count) =
                self.load_directory(&dir.path, dir.ino, parent_ino, &dir.fs_path)?;
            self.write_dir_inode(
                dir.ino,
                dir.blkaddr,
                parent_ino,
                &name,
                2,
                &sub_addrs,
                sub_count,
                &dir.fs_path,
            )?;
        }

        Ok((data_addrs, subdir_count))
    }

    // 分配目录 inode 块
    fn alloc_dir_inode(&mut self) -> Result<(u32, u32)> {
        let nid = self.nat.alloc_nid();
        let blkaddr = self.segment_alloc.alloc_node_block(SegType::HotNode)?;
        self.nat.set_entry(nid, blkaddr, nid.0);
        self.sit.mark_block_used(blkaddr, CURSEG_HOT_NODE as u16)?;
        self.ssa.set_node_summary(blkaddr, nid.0)?;
        Ok((nid.0, blkaddr))
    }

    // 创建文件 inode
    fn create_file_inode(
        &mut self,
        path: &Path,
        parent_ino: u32,
        metadata: &fs::Metadata,
        fs_path: &str,
        symlink_target: Option<&str>,
    ) -> Result<u32> {
        let nid = self.nat.alloc_nid();
        let ino = nid.0;
        let blkaddr = self.segment_alloc.alloc_node_block(SegType::WarmNode)?;
        self.nat.set_entry(nid, blkaddr, ino);
        self.sit.mark_block_used(blkaddr, CURSEG_WARM_NODE as u16)?;
        self.ssa.set_node_summary(blkaddr, nid.0)?;

        let file_name = path
            .file_name()
            .map(|n| n.as_encoded_bytes().to_vec())
            .unwrap_or_default();

        // 获取 uid/gid/mode
        let (uid, gid, mode) = if let Some(ref cfg) = self.fs_config {
            cfg.get_attrs(fs_path, false)
        } else {
            (self.config.root_uid, self.config.root_gid, 0o644)
        };

        // 创建 inode
        let mut inode = if let Some(target) = symlink_target {
            // 符号链接
            InodeBuilder::new_symlink(uid, gid)
                .with_mode(S_IFLNK | ((mode as u16) & 0o7777))
                .with_symlink_target(target)
        } else {
            // 普通文件
            let file_size = metadata.len();

            // 写入文件数据块 (仅普通文件)
            let (direct_addrs, nids) = if metadata.is_file() && file_size > 0 {
                let all_addrs = self.write_file_data(path)?;
                self.organize_file_addrs(ino, all_addrs)?
            } else {
                (vec![], [0; 5])
            };

            InodeBuilder::new_file(mode as u16, uid, gid)
                .with_size(file_size)
                // i_blocks 包含 inode 块自身与数据块数量
                .with_blocks(file_size.div_ceil(F2FS_BLKSIZE as u64) + 1)
                .with_addrs(direct_addrs)
                .with_nids(nids)
        }
        .with_timestamp(self.timestamp)
        .with_pino(parent_ino)
        .with_name(&file_name);

        // 设置 SELinux 上下文
        if let Some(ref mut ctx) = self.selinux_contexts
            && let Some(context) = ctx.lookup(fs_path)
        {
            inode = inode.with_selinux_context(&context);
        }

        self.write_block(blkaddr, &inode.build(ino, ino, self.cp_ver)?)?;
        self.inode_map
            .insert(path.to_string_lossy().to_string(), ino);
        Ok(ino)
    }

    // 写入文件数据块
    fn write_file_data(&mut self, path: &Path) -> Result<Vec<u32>> {
        let data = fs::read(path)?;
        let mut all_addrs = Vec::new();

        // 分配所有数据块
        for chunk in data.chunks(F2FS_BLKSIZE) {
            let blkaddr = self.segment_alloc.alloc_data_block(SegType::WarmData)?;
            self.sit.mark_block_used(blkaddr, CURSEG_WARM_DATA as u16)?;
            // SSA 记录在 organize_file_addrs 中设置, 因为此处还需要已知 ino

            let mut block = vec![0u8; F2FS_BLKSIZE];
            block[..chunk.len()].copy_from_slice(chunk);
            self.write_block(blkaddr, &block)?;
            all_addrs.push(blkaddr);
        }

        Ok(all_addrs)
    }

    // 组织文件地址 (处理直接地址与间接地址)
    fn organize_file_addrs(
        &mut self,
        ino: u32,
        all_addrs: Vec<u32>,
    ) -> Result<(Vec<u32>, [u32; 5])> {
        const ADDRS_PER_INODE: usize = 864; // 存在 extra_attr 与 inline_xattr 时
        const ADDRS_PER_BLOCK: usize = 1018;
        const NIDS_PER_BLOCK: usize = 1018;

        let mut direct_addrs = Vec::new();
        let mut nids = [0u32; 5];

        // 为所有数据块设置 SSA 记录
        for (idx, &blkaddr) in all_addrs.iter().enumerate() {
            self.ssa.set_data_summary(blkaddr, ino, idx as u16)?;
        }

        // 1. 直接地址 (存放在 inode 内)
        let direct_count = all_addrs.len().min(ADDRS_PER_INODE);
        direct_addrs.extend_from_slice(&all_addrs[..direct_count]);

        if all_addrs.len() <= ADDRS_PER_INODE {
            return Ok((direct_addrs, nids));
        }

        // 2. 第一个 direct node (nids[0])
        let mut remaining = &all_addrs[direct_count..];
        if !remaining.is_empty() {
            let count = remaining.len().min(ADDRS_PER_BLOCK);
            let nid = self.nat.alloc_nid();
            let blkaddr = self.segment_alloc.alloc_node_block(SegType::WarmNode)?;
            self.nat.set_entry(nid, blkaddr, ino);
            self.sit.mark_block_used(blkaddr, CURSEG_WARM_NODE as u16)?;
            self.ssa.set_node_summary(blkaddr, nid.0)?;

            let direct_node = DirectNodeBuilder::new()
                .with_addrs(remaining[..count].to_vec())
                .build(nid.0, ino, self.cp_ver);
            self.write_block(blkaddr, &direct_node)?;
            nids[0] = nid.0;

            remaining = &remaining[count..];
        }

        // 3. 第二个 direct node (nids[1])
        if !remaining.is_empty() {
            let count = remaining.len().min(ADDRS_PER_BLOCK);
            let nid = self.nat.alloc_nid();
            let blkaddr = self.segment_alloc.alloc_node_block(SegType::WarmNode)?;
            self.nat.set_entry(nid, blkaddr, ino);
            self.sit.mark_block_used(blkaddr, CURSEG_WARM_NODE as u16)?;
            self.ssa.set_node_summary(blkaddr, nid.0)?;

            let direct_node = DirectNodeBuilder::new()
                .with_addrs(remaining[..count].to_vec())
                .build(nid.0, ino, self.cp_ver);
            self.write_block(blkaddr, &direct_node)?;
            nids[1] = nid.0;

            remaining = &remaining[count..];
        }

        // 4. 第一个二级间接 node (nids[2])
        if !remaining.is_empty() {
            nids[2] = self.alloc_double_indirect_node(ino, remaining, ADDRS_PER_BLOCK)?;
            let consumed = remaining.len().min(NIDS_PER_BLOCK * ADDRS_PER_BLOCK);
            remaining = &remaining[consumed..];
        }

        // 5. 第二个二级间接 node (nids[3])
        if !remaining.is_empty() {
            nids[3] = self.alloc_double_indirect_node(ino, remaining, ADDRS_PER_BLOCK)?;
            let consumed = remaining.len().min(NIDS_PER_BLOCK * ADDRS_PER_BLOCK);
            remaining = &remaining[consumed..];
        }

        // 6. 三级间接 node (nids[4]) - 按需使用
        if !remaining.is_empty() {
            log::warn!(
                "file requires triple indirect node ({} blocks remaining), not implemented",
                remaining.len()
            );
        }

        Ok((direct_addrs, nids))
    }

    // 分配二级间接 node
    fn alloc_double_indirect_node(
        &mut self,
        ino: u32,
        addrs: &[u32],
        addrs_per_block: usize,
    ) -> Result<u32> {
        const NIDS_PER_BLOCK: usize = 1018;

        // 分配二级间接 node
        let double_indirect_nid = self.nat.alloc_nid();
        let double_indirect_blkaddr = self.segment_alloc.alloc_node_block(SegType::WarmNode)?;
        self.nat
            .set_entry(double_indirect_nid, double_indirect_blkaddr, ino);
        self.sit
            .mark_block_used(double_indirect_blkaddr, CURSEG_WARM_NODE as u16)?;
        self.ssa
            .set_node_summary(double_indirect_blkaddr, double_indirect_nid.0)?;

        // 创建 indirect node 构建器
        let mut indirect_builder = IndirectNodeBuilder::new();

        // 为每个 direct node 分配地址
        let mut offset = 0;
        while offset < addrs.len() && indirect_builder.len() < NIDS_PER_BLOCK {
            let chunk_size = (addrs.len() - offset).min(addrs_per_block);
            let chunk = &addrs[offset..offset + chunk_size];

            // 分配 direct node
            let direct_nid = self.nat.alloc_nid();
            let direct_blkaddr = self.segment_alloc.alloc_node_block(SegType::WarmNode)?;
            self.nat.set_entry(direct_nid, direct_blkaddr, ino);
            self.sit
                .mark_block_used(direct_blkaddr, CURSEG_WARM_NODE as u16)?;
            self.ssa.set_node_summary(direct_blkaddr, direct_nid.0)?;

            // 写入 direct node
            let direct_node = DirectNodeBuilder::new().with_addrs(chunk.to_vec()).build(
                direct_nid.0,
                ino,
                self.cp_ver,
            );
            self.write_block(direct_blkaddr, &direct_node)?;

            // 加入 indirect node
            indirect_builder.add_nid(direct_nid.0);

            offset += chunk_size;
        }

        // 写入二级间接 node
        let double_indirect_node = indirect_builder.build(double_indirect_nid.0, ino, self.cp_ver);
        self.write_block(double_indirect_blkaddr, &double_indirect_node)?;

        Ok(double_indirect_nid.0)
    }

    // 收尾构建
    fn finalize(&mut self) -> Result<()> {
        let layout = self
            .superblock
            .layout()
            .ok_or_else(|| F2fsError::InvalidData("layout not calculated".into()))?
            .clone();

        // 写入 superblock
        let sb_data = self.superblock.build()?;
        self.writer.seek(SeekFrom::Start(F2FS_SUPER_OFFSET))?;
        self.writer.write_all(&sb_data)?;
        self.writer
            .seek(SeekFrom::Start(F2FS_SUPER_OFFSET + F2FS_BLKSIZE as u64))?;
        self.writer.write_all(&sb_data)?;

        // CP pack 结构: cp_header(1) + data_sum(1) + node_sum(3) + cp_footer(1) = 6 块
        let cp_pack_blocks = 6u32;

        // 计算保留 segment 与超额预留 segment
        let ovp_segment_count = (layout.segment_count_main as f64 * 0.05) as u32;
        let ovp_segment_count = ovp_segment_count.max(2); // 至少 2 个
        let rsvd_segment_count = ovp_segment_count.max(2); // 至少 2 个

        // 计算用户块数量 (main 区域块数 - 预留块数)
        let user_block_count = (layout.segment_count_main - ovp_segment_count) as u64
            * DEFAULT_BLOCKS_PER_SEGMENT as u64;

        // 计算 bitmap 大小
        // sit_ver_bitmap_bytesize = (segment_count_sit / 2) * blocks_per_seg / 8
        // nat_ver_bitmap_bytesize = (segment_count_nat / 2) * blocks_per_seg / 8
        let sit_bitmap_size = ((layout.segment_count_sit / 2) * DEFAULT_BLOCKS_PER_SEGMENT) / 8;
        let nat_bitmap_size = ((layout.segment_count_nat / 2) * DEFAULT_BLOCKS_PER_SEGMENT) / 8;

        // 生成正确尺寸的 bitmap
        let sit_bitmap = vec![0u8; sit_bitmap_size as usize];
        let nat_bitmap = vec![0u8; nat_bitmap_size as usize];

        // 获取当前 segment 分配信息
        let curseg_info = self.segment_alloc.get_curseg_info();

        // 为所有 curseg 设置正确的 SIT 类型
        // 即使 segment 中没有已分配的块也需要设置类型
        self.sit
            .set_seg_type(curseg_info.node_segno[0], CURSEG_HOT_NODE as u16)?;
        self.sit
            .set_seg_type(curseg_info.node_segno[1], CURSEG_WARM_NODE as u16)?;
        self.sit
            .set_seg_type(curseg_info.node_segno[2], CURSEG_COLD_NODE as u16)?;
        self.sit
            .set_seg_type(curseg_info.data_segno[0], CURSEG_HOT_DATA as u16)?;
        self.sit
            .set_seg_type(curseg_info.data_segno[1], CURSEG_WARM_DATA as u16)?;
        self.sit
            .set_seg_type(curseg_info.data_segno[2], CURSEG_COLD_DATA as u16)?;

        // 构建 checkpoint
        let mut checkpoint = CheckpointBuilder::new()
            .with_version(self.cp_ver)
            .with_user_block_count(user_block_count)
            .with_valid_block_count(self.segment_alloc.allocated_blocks())
            .with_free_segment_count(self.segment_alloc.free_segments())
            .with_rsvd_segment_count(rsvd_segment_count)
            .with_overprov_segment_count(ovp_segment_count)
            .with_next_free_nid(self.nat.next_free_nid())
            // valid_node_count: 实际的 node 块数量 (不含 node_ino 与 meta_ino)
            // NAT 中存在 node_ino(1), meta_ino(2), root_ino(3) 以及其他 inode
            // node_ino 与 meta_ino 的 block_addr=1 为特殊标记, 不计入统计
            .with_valid_node_count(self.nat.entry_count() as u32 - 2)
            // valid_inode_count: 实际的 inode 数量 (目录与文件)
            // inode_map 已包含根目录, 因此直接使用 inode_map.len()
            .with_valid_inode_count(self.inode_map.len() as u32)
            .with_sit_bitmap(sit_bitmap)
            .with_nat_bitmap(nat_bitmap)
            .with_cp_pack_total_block_count(cp_pack_blocks)
            // 使用 CP_UMOUNT_FLAG | CP_COMPACT_SUM_FLAG
            // CP_COMPACT_SUM_FLAG 表示 DATA summary 采用 compact 格式
            .with_flags(CP_UMOUNT_FLAG | CP_COMPACT_SUM_FLAG);

        // 设置当前 segment 信息
        checkpoint.set_cur_node_seg(0, curseg_info.node_segno[0], curseg_info.node_blkoff[0]);
        checkpoint.set_cur_node_seg(1, curseg_info.node_segno[1], curseg_info.node_blkoff[1]);
        checkpoint.set_cur_node_seg(2, curseg_info.node_segno[2], curseg_info.node_blkoff[2]);
        // 未使用的 segment 置为 0xFFFFFFFF
        for i in 3..8 {
            checkpoint.set_cur_node_seg(i, 0xFFFFFFFF, 0);
        }

        checkpoint.set_cur_data_seg(0, curseg_info.data_segno[0], curseg_info.data_blkoff[0]);
        checkpoint.set_cur_data_seg(1, curseg_info.data_segno[1], curseg_info.data_blkoff[1]);
        checkpoint.set_cur_data_seg(2, curseg_info.data_segno[2], curseg_info.data_blkoff[2]);
        // 未使用的 segment 置为 0xFFFFFFFF
        for i in 3..8 {
            checkpoint.set_cur_data_seg(i, 0xFFFFFFFF, 0);
        }

        let cp_data = checkpoint.build()?;

        // 获取当前 segment 的 SSA 数据, 供 checkpoint pack 使用
        let hot_node_segno = curseg_info.node_segno[0] as usize;
        let warm_node_segno = curseg_info.node_segno[1] as usize;
        let cold_node_segno = curseg_info.node_segno[2] as usize;
        let hot_data_segno = curseg_info.data_segno[0] as usize;
        let warm_data_segno = curseg_info.data_segno[1] as usize;
        let cold_data_segno = curseg_info.data_segno[2] as usize;

        // 构造 compact 格式的 DATA summary 块
        // 结构: nat_journal(SUM_JOURNAL_SIZE) + sit_journal(SUM_JOURNAL_SIZE) + data summaries
        let compact_sum = self.build_compact_data_summary(
            &curseg_info,
            hot_data_segno,
            warm_data_segno,
            cold_data_segno,
        )?;

        // 构建 NODE summary 块 (普通格式)
        let node_sum_hot = self.ssa.build_curseg_summary(hot_node_segno, true)?;
        let node_sum_warm = self.ssa.build_curseg_summary(warm_node_segno, true)?;
        let node_sum_cold = self.ssa.build_curseg_summary(cold_node_segno, true)?;

        // 写入第一个 checkpoint pack
        let cp_offset = layout.cp_blkaddr as u64 * F2FS_BLKSIZE as u64;
        self.writer.seek(SeekFrom::Start(cp_offset))?;
        self.writer.write_all(&cp_data)?; // 块 0: CP header

        self.writer.write_all(&compact_sum)?; // 块 1: compact data summary
        self.writer.write_all(&node_sum_hot)?; // 块 2: hot node summary
        self.writer.write_all(&node_sum_warm)?; // 块 3: warm node summary
        self.writer.write_all(&node_sum_cold)?; // 块 4: cold node summary
        self.writer.write_all(&cp_data)?; // 块 5: CP footer

        // 写入第二个 checkpoint pack (位于下一个 segment)
        let cp2_offset =
            (layout.cp_blkaddr + DEFAULT_BLOCKS_PER_SEGMENT) as u64 * F2FS_BLKSIZE as u64;
        self.writer.seek(SeekFrom::Start(cp2_offset))?;
        self.writer.write_all(&cp_data)?; // 块 0: CP header
        self.writer.write_all(&compact_sum)?; // 块 1: compact data summary
        self.writer.write_all(&node_sum_hot)?; // 块 2: hot node summary
        self.writer.write_all(&node_sum_warm)?; // 块 3: warm node summary
        self.writer.write_all(&node_sum_cold)?; // 块 4: cold node summary
        self.writer.write_all(&cp_data)?; // 块 5: CP footer

        // SIT 区域保持为空, 数据存放在 checkpoint 的 SIT journal 中

        // 写入 NAT 区域
        let nat_data = self.nat.to_bytes();
        let nat_offset = layout.nat_blkaddr as u64 * F2FS_BLKSIZE as u64;
        self.writer.seek(SeekFrom::Start(nat_offset))?;
        self.writer.write_all(&nat_data)?;

        // 写入第二份 NAT
        let nat_blocks_per_copy = (layout.segment_count_nat / 2) * DEFAULT_BLOCKS_PER_SEGMENT;
        let nat_copy_size = nat_blocks_per_copy as usize * F2FS_BLKSIZE;
        if nat_data.len() > nat_copy_size {
            return Err(F2fsError::InvalidData(format!(
                "NAT data exceeds single copy capacity: {} > {}",
                nat_data.len(),
                nat_copy_size
            )));
        }
        let nat_offset_2 = (layout.nat_blkaddr + nat_blocks_per_copy) as u64 * F2FS_BLKSIZE as u64;
        self.writer.seek(SeekFrom::Start(nat_offset_2))?;
        self.writer.write_all(&nat_data)?;

        // SSA 区域保持为空, 数据存放在 checkpoint 的 summary 块中

        self.writer.flush()?;
        Ok(())
    }

    // 构造 compact 格式的 DATA summary 块
    // 结构: n_nats + n_sits + NAT 条目 + SIT 条目 + DATA summaries + footer
    fn build_compact_data_summary(
        &self,
        curseg_info: &CursegInfo,
        hot_data_segno: usize,
        warm_data_segno: usize,
        cold_data_segno: usize,
    ) -> Result<Vec<u8>> {
        let mut buf = vec![0u8; F2FS_BLKSIZE];
        let mut offset = 0usize;

        // 1. NAT journal (507 字节)
        buf[offset..offset + 2].copy_from_slice(&0u16.to_le_bytes());
        offset = 507; // 跳过整个 NAT journal 空间

        // 2. SIT journal (507 字节)
        let n_sits: u16 = 6;
        buf[offset..offset + 2].copy_from_slice(&n_sits.to_le_bytes());
        offset += 2;

        for i in 0..6 {
            let (segno, seg_type) = if i < 3 {
                (curseg_info.data_segno[i], i as u16)
            } else {
                (curseg_info.node_segno[i - 3], i as u16)
            };

            let valid_blocks = if i < 3 {
                curseg_info.data_blkoff[i]
            } else {
                curseg_info.node_blkoff[i - 3]
            };

            // segno (4 字节)
            buf[offset..offset + 4].copy_from_slice(&segno.to_le_bytes());
            offset += 4;

            // vblocks (2 字节)
            let vblocks = valid_blocks | (seg_type << SIT_VBLOCKS_SHIFT);
            buf[offset..offset + 2].copy_from_slice(&vblocks.to_le_bytes());
            offset += 2;

            // valid_map (64 字节)
            if let Some(sit_entry) = self.sit.get_entry(segno) {
                buf[offset..offset + 64].copy_from_slice(&sit_entry.valid_map);
            }
            offset += 64;

            // mtime (8 字节)
            offset += 8;
        }

        // 将 SIT journal 补齐到 507 字节
        offset = 1014; // NAT journal (507) + SIT journal (507)

        // 3. DATA summary 三类: hot, warm, cold
        let data_segnos = [hot_data_segno, warm_data_segno, cold_data_segno];
        let data_blkoffs = [
            curseg_info.data_blkoff[0] as usize,
            curseg_info.data_blkoff[1] as usize,
            curseg_info.data_blkoff[2] as usize,
        ];

        for (seg_idx, &segno) in data_segnos.iter().enumerate() {
            let blk_off = data_blkoffs[seg_idx];

            for j in 0..blk_off {
                if offset + SUMMARY_SIZE > F2FS_BLKSIZE - SUM_FOOTER_SIZE {
                    break;
                }

                // 从 SSA 管理器获取 summary 条目
                if let Some(entry) = self.ssa.get_summary_entry(segno, j) {
                    buf[offset..offset + 4].copy_from_slice(&entry.nid.to_le_bytes());
                    buf[offset + 4] = entry.version;
                    buf[offset + 5..offset + 7].copy_from_slice(&entry.ofs_in_node.to_le_bytes());
                }
                offset += SUMMARY_SIZE;
            }
        }

        Ok(buf)
    }

    // 写入块
    fn write_block(&mut self, blkaddr: u32, data: &[u8]) -> Result<()> {
        self.writer
            .seek(SeekFrom::Start(blkaddr as u64 * F2FS_BLKSIZE as u64))?;
        self.writer.write_all(data)?;
        Ok(())
    }
}

// 简化的构建函数
pub fn build_f2fs_image(
    source_dir: &Path,
    output_path: &Path,
    image_size: u64,
    mount_point: &str,
) -> Result<()> {
    let config = F2fsBuilderConfig {
        source_dir: source_dir.to_path_buf(),
        output_path: output_path.to_path_buf(),
        image_size,
        mount_point: mount_point.to_string(),
        features: F2fsFeatures::default(),
        ..Default::default()
    };

    let mut builder = F2fsBuilder::new(config)?;
    builder.build()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;
    use std::fs;
    use std::sync::atomic::{AtomicU32, Ordering};

    static TEST_COUNTER: AtomicU32 = AtomicU32::new(0);

    fn create_temp_dir() -> std::path::PathBuf {
        let counter = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
        let temp_dir = env::temp_dir().join(format!(
            "f2fs_test_{}_{}_{}",
            std::process::id(),
            counter,
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = fs::remove_dir_all(&temp_dir);
        fs::create_dir_all(&temp_dir).unwrap();
        temp_dir
    }

    fn cleanup_temp_dir(path: &Path) {
        let _ = fs::remove_dir_all(path);
    }

    #[test]
    fn test_f2fs_builder_new() {
        let temp_dir = create_temp_dir();
        let output_path = temp_dir.join("test.img");

        let config = F2fsBuilderConfig {
            source_dir: temp_dir.clone(),
            output_path: output_path.clone(),
            image_size: 100 * 1024 * 1024, // 100MB
            mount_point: "/".to_string(),
            features: F2fsFeatures::default(),
            ..Default::default()
        };

        let builder = F2fsBuilder::new(config);
        assert!(builder.is_ok());

        cleanup_temp_dir(&temp_dir);
    }

    #[test]
    fn test_f2fs_builder_build_empty() {
        let temp_dir = create_temp_dir();
        let source_dir = temp_dir.join("source");
        fs::create_dir_all(&source_dir).unwrap();
        let output_path = temp_dir.join("test.img");

        let config = F2fsBuilderConfig {
            source_dir,
            output_path: output_path.clone(),
            image_size: 100 * 1024 * 1024,
            mount_point: "/".to_string(),
            features: F2fsFeatures::default(),
            ..Default::default()
        };

        let mut builder = F2fsBuilder::new(config).unwrap();
        let result = builder.build();
        assert!(result.is_ok(), "Build failed: {:?}", result.err());

        // 校验文件已创建
        assert!(output_path.exists());
        let metadata = fs::metadata(&output_path).unwrap();
        assert_eq!(metadata.len(), 100 * 1024 * 1024);

        cleanup_temp_dir(&temp_dir);
    }
}
