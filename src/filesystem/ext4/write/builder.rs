// EXT4 镜像构建器

use crate::filesystem::ext4::Result;
use crate::filesystem::ext4::error::Ext4Error;
use crate::filesystem::ext4::types::*;
use crate::filesystem::ext4::write::directory::file_type;
use crate::filesystem::ext4::write::*;
use crate::filesystem::f2fs::write::{FsConfig, SelinuxContexts};
use crate::utils::symlink::read_symlink_info;
use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{BufWriter, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use zerocopy::TryFromBytes;

// 构建器配置
pub struct Ext4BuilderConfig {
    pub source_dir: PathBuf,
    pub output_path: PathBuf,
    pub image_size: u64,
    pub volume_label: String,
    pub mount_point: String,
    pub root_uid: u32,
    pub root_gid: u32,
    pub file_contexts: Option<PathBuf>,
    pub fs_config: Option<PathBuf>,
    pub timestamp: Option<u64>,
}

impl Default for Ext4BuilderConfig {
    fn default() -> Self {
        Ext4BuilderConfig {
            source_dir: PathBuf::new(),
            output_path: PathBuf::new(),
            image_size: 100 * 1024 * 1024, // 100MB
            volume_label: String::new(),
            mount_point: "/".to_string(),
            root_uid: 0,
            root_gid: 0,
            file_contexts: None,
            fs_config: None,
            timestamp: None,
        }
    }
}

// EXT4 镜像构建器
pub struct Ext4Builder {
    config: Ext4BuilderConfig,
    writer: BufWriter<File>,
    sb_builder: SuperblockBuilder,
    block_alloc: BlockAllocator,
    inode_alloc: InodeAllocator,
    inode_map: HashMap<String, u32>,
    #[allow(dead_code)]
    selinux_contexts: Option<SelinuxContexts>,
    #[allow(dead_code)]
    fs_config: Option<FsConfig>,
    #[allow(dead_code)]
    timestamp: u32,
    dir_count: u32,
}

impl Ext4Builder {
    // 创建新的构建器
    pub fn new(config: Ext4BuilderConfig) -> Result<Self> {
        let file = File::create(&config.output_path)?;
        let writer = BufWriter::new(file);

        let sb_builder = SuperblockBuilder::new(config.image_size).with_label(&config.volume_label);

        let block_alloc =
            BlockAllocator::new(sb_builder.blocks_count(), sb_builder.blocks_per_group());

        let inode_alloc =
            InodeAllocator::new(sb_builder.inodes_count(), sb_builder.inodes_per_group());

        // 加载 SELinux 安全上下文
        let selinux_contexts = config
            .file_contexts
            .as_ref()
            .and_then(|path| SelinuxContexts::from_file(path).ok());

        // 加载文件系统配置
        let fs_config = config
            .fs_config
            .as_ref()
            .and_then(|path| FsConfig::from_file(path).ok());

        let timestamp = config.timestamp.unwrap_or_else(|| {
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                // 系统时钟早于 epoch 时退化为 0, 不影响镜像有效性
                .unwrap_or_default()
                .as_secs()
        }) as u32;

        Ok(Ext4Builder {
            config,
            writer,
            sb_builder,
            block_alloc,
            inode_alloc,
            inode_map: HashMap::new(),
            selinux_contexts,
            fs_config,
            timestamp,
            dir_count: 0,
        })
    }

    // 构建镜像
    pub fn build(&mut self) -> Result<()> {
        // 初始化镜像文件
        self.writer.get_ref().set_len(self.config.image_size)?;

        // 预留元数据块
        self.reserve_metadata_blocks()?;

        // 创建根目录
        let root_ino = self.create_root_dir()?;

        // 加载源目录内容
        let source_dir = self.config.source_dir.clone();
        if source_dir.exists() {
            self.load_directory(&source_dir, root_ino, root_ino)?;
        }

        // 设置实际空闲数量
        self.sb_builder
            .set_free_blocks_count(self.block_alloc.free_count());
        self.sb_builder
            .set_free_inodes_count(self.inode_alloc.free_count());

        // 写入元数据
        self.write_metadata()?;

        self.writer.flush()?;
        Ok(())
    }

    // 预留元数据块
    fn reserve_metadata_blocks(&mut self) -> Result<()> {
        let group_count = self.sb_builder.group_count();
        let block_size = self.sb_builder.block_size();
        let blocks_per_group = self.sb_builder.blocks_per_group();

        for group_idx in 0..group_count {
            let group_start = group_idx as u64 * blocks_per_group as u64;

            // superblock (部分 block group 存在备份)
            if group_idx == 0 || self.has_super_backup(group_idx) {
                self.block_alloc
                    .reserve_metadata_blocks(group_idx, &[group_start]);
            }

            // group descriptor 表
            let gdt_blocks =
                (group_count as u64 * EXT2_MIN_DESC_SIZE_64BIT as u64).div_ceil(block_size as u64);
            let gdt_start = group_start + 1;
            for i in 0..gdt_blocks {
                self.block_alloc
                    .reserve_metadata_blocks(group_idx, &[gdt_start + i]);
            }

            // block bitmap 位置
            let block_bitmap = gdt_start + gdt_blocks;
            self.block_alloc
                .reserve_metadata_blocks(group_idx, &[block_bitmap]);

            // inode bitmap 位置
            let inode_bitmap = block_bitmap + 1;
            self.block_alloc
                .reserve_metadata_blocks(group_idx, &[inode_bitmap]);

            // inode table 位置
            let inode_table_start = inode_bitmap + 1;
            let inode_table_blocks = (self.sb_builder.inodes_per_group() as u64
                * self.sb_builder.inode_size() as u64)
                .div_ceil(block_size as u64);
            for i in 0..inode_table_blocks {
                self.block_alloc
                    .reserve_metadata_blocks(group_idx, &[inode_table_start + i]);
            }
        }

        Ok(())
    }

    // 检查 block group 是否存在 superblock 备份
    fn has_super_backup(&self, group_idx: u32) -> bool {
        if group_idx == 0 {
            return true;
        }
        // superblock 备份位于 3, 5, 7 的幂次 block group 中
        for base in [3, 5, 7] {
            let mut power = base;
            while power <= group_idx {
                if power == group_idx {
                    return true;
                }
                power *= base;
            }
        }
        false
    }

    // 创建根目录
    fn create_root_dir(&mut self) -> Result<u32> {
        let root_ino = self.inode_alloc.alloc_root_inode();
        self.inode_map
            .insert(self.config.mount_point.clone(), root_ino);
        Ok(root_ino)
    }

    // 加载目录内容
    fn load_directory(&mut self, path: &Path, current_ino: u32, parent_ino: u32) -> Result<()> {
        let entries: Vec<_> = fs::read_dir(path)?.collect::<std::io::Result<Vec<_>>>()?;

        // 创建目录构建器
        let mut dir_builder = DirectoryBuilder::new(self.sb_builder.block_size());

        // 添加 . 与 .. 两个 dir entry
        dir_builder.add_entry(current_ino, b".", file_type::DIR);
        dir_builder.add_entry(parent_ino, b"..", file_type::DIR);

        // 处理所有条目
        let mut dir_count = 0;

        for entry in &entries {
            let name = entry.file_name();
            let name_bytes = name.as_encoded_bytes();
            let metadata = entry.metadata()?;

            // 优先检测符号链接 (支持 Windows 的 !<symlink> 格式)
            let symlink_info = read_symlink_info(&entry.path())
                .map_err(|e| std::io::Error::other(e.to_string()))?;

            if metadata.is_dir() {
                let ino = self
                    .inode_alloc
                    .alloc_inode()
                    .ok_or_else(|| std::io::Error::other("No more inodes"))?;

                dir_builder.add_entry(ino, name_bytes, file_type::DIR);
                dir_count += 1;

                // 递归处理子目录
                self.load_directory(&entry.path(), ino, current_ino)?;
            } else if symlink_info.is_symlink {
                // 符号链接 (包括 Windows 的 !<symlink> 格式)
                let ino = self
                    .inode_alloc
                    .alloc_inode()
                    .ok_or_else(|| std::io::Error::other("No more inodes"))?;

                dir_builder.add_entry(ino, name_bytes, file_type::LNK);

                // 创建符号链接 inode
                self.create_symlink_inode(ino, &symlink_info.target.unwrap_or_default())?;
            } else if metadata.is_file() {
                let ino = self
                    .inode_alloc
                    .alloc_inode()
                    .ok_or_else(|| std::io::Error::other("No more inodes"))?;

                dir_builder.add_entry(ino, name_bytes, file_type::REG);

                // 创建文件 inode
                self.create_file_inode(ino, &entry.path(), &metadata)?;
            }
        }

        // 写入目录数据块
        let dir_blocks = dir_builder.build()?;

        // 分配块并记录块地址
        let mut block_addrs = Vec::new();
        for block_data in dir_blocks.iter() {
            if let Some(block) = self.block_alloc.alloc_block() {
                self.write_data_block(block, block_data)?;
                block_addrs.push(block);
            } else {
                return Err(std::io::Error::other("No more blocks").into());
            }
        }

        // 创建 extent 并写入 inode
        let extents = ExtentBuilder::from_blocks(&block_addrs);
        if extents.len() > 4 {
            log::error!(
                "{} too many extents: {}, at most 4 are supported",
                path.display(),
                extents.len()
            );
        }

        let block_size = self.sb_builder.block_size();
        let blocks_512 = (dir_blocks.len() as u32) * (block_size / 512);
        let dir_size = dir_blocks.len() * block_size as usize;

        let builder = InodeBuilder::new_dir(0o755, self.config.root_uid, self.config.root_gid)
            .with_size(dir_size as u64)
            .with_blocks(blocks_512)
            .with_links(2 + dir_count as u16)
            .with_extents(&extents);

        let inode_data = builder.build(self.sb_builder.inode_size())?;
        self.write_inode(current_ino, &inode_data)?;
        self.dir_count += 1;

        Ok(())
    }

    // 创建文件 inode
    fn create_file_inode(&mut self, ino: u32, path: &Path, metadata: &fs::Metadata) -> Result<()> {
        let file_size = metadata.len();
        let file_data = fs::read(path)?;

        // 分配数据块
        let block_size = self.sb_builder.block_size() as usize;
        let block_count = (file_size as usize).div_ceil(block_size);

        let mut blocks = Vec::new();
        for chunk in file_data.chunks(block_size) {
            if let Some(block) = self.block_alloc.alloc_block() {
                self.write_data_block(block, chunk)?;
                blocks.push(block);
            } else {
                return Err(std::io::Error::other("No more blocks").into());
            }
        }

        // 创建 extent
        let extents = ExtentBuilder::from_blocks(&blocks);

        // 创建 inode
        let builder = InodeBuilder::new_file(0o644, self.config.root_uid, self.config.root_gid)
            .with_size(file_size)
            .with_blocks((block_count * (block_size / 512)) as u32)
            .with_extents(&extents);

        let inode_data = builder.build(self.sb_builder.inode_size())?;
        self.write_inode(ino, &inode_data)?;

        Ok(())
    }

    // 创建符号链接 inode
    fn create_symlink_inode(&mut self, ino: u32, target: &str) -> Result<()> {
        let target_bytes = target.as_bytes();

        let builder = if target_bytes.len() <= 60 {
            // 快速符号链接: 目标路径存放在 i_block 中
            InodeBuilder::new_symlink(self.config.root_uid, self.config.root_gid)
                .with_symlink_target(target)
        } else {
            // 慢速符号链接: 目标路径存放在数据块中
            let block_size = self.sb_builder.block_size() as usize;
            let mut block_data = vec![0u8; block_size];
            block_data[..target_bytes.len()].copy_from_slice(target_bytes);

            // 分配数据块
            let block = self
                .block_alloc
                .alloc_block()
                .ok_or_else(|| std::io::Error::other("No more blocks"))?;
            self.write_data_block(block, &block_data)?;

            // 创建 extent
            let extents = ExtentBuilder::from_blocks(&[block]);

            InodeBuilder::new_symlink(self.config.root_uid, self.config.root_gid)
                .with_size(target_bytes.len() as u64)
                .with_blocks((block_size / 512) as u32)
                .with_extents(&extents)
                .with_extent_flag()
        };

        let inode_data = builder.build(self.sb_builder.inode_size())?;
        self.write_inode(ino, &inode_data)?;

        Ok(())
    }

    // 写入 inode
    fn write_inode(&mut self, ino: u32, data: &[u8]) -> Result<()> {
        let group_idx = self.inode_alloc.inode_group(ino);
        let inode_idx = self.inode_alloc.inode_index_in_group(ino);

        // 计算 inode table 的位置
        let blocks_per_group = self.sb_builder.blocks_per_group();
        let block_size = self.sb_builder.block_size();
        let group_start = group_idx as u64 * blocks_per_group as u64;

        // 跳过 superblock, GDT 与 bitmap
        let gdt_blocks = (self.sb_builder.group_count() as u64 * EXT2_MIN_DESC_SIZE_64BIT as u64)
            .div_ceil(block_size as u64);
        let inode_table_start = group_start + 1 + gdt_blocks + 2; // +2 对应两个 bitmap

        let inode_offset = inode_table_start * block_size as u64
            + inode_idx as u64 * self.sb_builder.inode_size() as u64;

        self.writer.seek(SeekFrom::Start(inode_offset))?;
        self.writer.write_all(data)?;

        Ok(())
    }

    // 写入数据块
    fn write_data_block(&mut self, block: u64, data: &[u8]) -> Result<()> {
        let block_size = self.sb_builder.block_size() as usize;
        let offset = block * block_size as u64;

        self.writer.seek(SeekFrom::Start(offset))?;

        if data.len() < block_size {
            let mut padded = vec![0u8; block_size];
            padded[..data.len()].copy_from_slice(data);
            self.writer.write_all(&padded)?;
        } else {
            self.writer.write_all(&data[..block_size])?;
        }

        Ok(())
    }

    // 写入元数据
    fn write_metadata(&mut self) -> Result<()> {
        // 写入 superblock
        let sb = self.sb_builder.build()?;
        let sb_bytes: &[u8] = zerocopy::IntoBytes::as_bytes(&sb);

        self.writer.seek(SeekFrom::Start(EXT4_SUPERBLOCK_OFFSET))?;
        self.writer.write_all(sb_bytes)?;

        // 写入 group descriptor
        self.write_group_descriptors()?;

        // 写入 bitmap
        self.write_bitmaps()?;

        Ok(())
    }

    // 写入 group descriptor
    fn write_group_descriptors(&mut self) -> Result<()> {
        let group_count = self.sb_builder.group_count();
        let block_size = self.sb_builder.block_size();
        let blocks_per_group = self.sb_builder.blocks_per_group();

        // GDT 占用的块数
        let gdt_blocks =
            (group_count as u64 * EXT2_MIN_DESC_SIZE_64BIT as u64).div_ceil(block_size as u64);

        // 构建所有 group descriptor
        let mut gdt_data = vec![0u8; (gdt_blocks * block_size as u64) as usize];

        for group_idx in 0..group_count {
            let group_start = group_idx as u64 * blocks_per_group as u64;

            // 每个 block group 的元数据位置
            let block_bitmap = group_start + 1 + gdt_blocks;
            let inode_bitmap = block_bitmap + 1;
            let inode_table = inode_bitmap + 1;

            // 计算空闲块数与空闲 inode 数
            let free_blocks = self.block_alloc.get_free_blocks_in_group(group_idx);
            let free_inodes = self.inode_alloc.get_free_inodes_in_group(group_idx);

            let mut gd = Ext4GroupDescriptor::try_read_from_bytes(
                &[0u8; std::mem::size_of::<Ext4GroupDescriptor>()],
            )
            .map_err(|_| Ext4Error::StructInit("ext4 group descriptor"))?;
            gd.bg_block_bitmap_lo = (block_bitmap & 0xFFFFFFFF) as u32;
            gd.bg_block_bitmap_hi = (block_bitmap >> 32) as u32;
            gd.bg_inode_bitmap_lo = (inode_bitmap & 0xFFFFFFFF) as u32;
            gd.bg_inode_bitmap_hi = (inode_bitmap >> 32) as u32;
            gd.bg_inode_table_lo = (inode_table & 0xFFFFFFFF) as u32;
            gd.bg_inode_table_hi = (inode_table >> 32) as u32;
            gd.bg_free_blocks_count_lo = (free_blocks & 0xFFFF) as u16;
            gd.bg_free_blocks_count_hi = (free_blocks >> 16) as u16;
            gd.bg_free_inodes_count_lo = (free_inodes & 0xFFFF) as u16;
            gd.bg_free_inodes_count_hi = (free_inodes >> 16) as u16;
            // 目录计数应按 block group 统计, 此处暂置为 0 (后续可优化)
            gd.bg_used_dirs_count_lo = 0;
            gd.bg_used_dirs_count_hi = 0;
            gd.bg_flags = 0;
            gd.bg_itable_unused_lo = free_inodes as u16;
            gd.bg_itable_unused_hi = (free_inodes >> 16) as u16;

            // 计算 group descriptor 的 checksum
            gd.bg_checksum = self.calc_group_desc_checksum(group_idx, &gd);

            // 写入 GDT 缓冲区
            let gd_offset = group_idx as usize * EXT2_MIN_DESC_SIZE_64BIT as usize;
            let gd_bytes: &[u8] = zerocopy::IntoBytes::as_bytes(&gd);
            gdt_data[gd_offset..gd_offset + gd_bytes.len()].copy_from_slice(gd_bytes);
        }

        // 将 GDT 写入 superblock 之后 (从块 1 开始)
        let gdt_offset = block_size as u64; // 块 1
        self.writer.seek(SeekFrom::Start(gdt_offset))?;
        self.writer.write_all(&gdt_data)?;

        Ok(())
    }

    // 写入 bitmap
    fn write_bitmaps(&mut self) -> Result<()> {
        let group_count = self.sb_builder.group_count();
        let block_size = self.sb_builder.block_size();
        let blocks_per_group = self.sb_builder.blocks_per_group();
        let inodes_per_group = self.sb_builder.inodes_per_group();
        let total_blocks = self.sb_builder.blocks_count();
        let total_inodes = self.sb_builder.inodes_count();

        for group_idx in 0..group_count {
            let group_start = group_idx as u64 * blocks_per_group as u64;
            let gdt_blocks =
                (group_count as u64 * EXT2_MIN_DESC_SIZE_64BIT as u64).div_ceil(block_size as u64);

            // 计算该 block group 中实际的块数与 inode 数
            let blocks_in_group = ((total_blocks - group_start) as u32).min(blocks_per_group);
            let inode_start = group_idx * inodes_per_group;
            let inodes_in_group = (total_inodes - inode_start).min(inodes_per_group);

            // 写入 block bitmap
            let block_bitmap_offset = (group_start + 1 + gdt_blocks) * block_size as u64;
            self.writer.seek(SeekFrom::Start(block_bitmap_offset))?;
            let block_bitmap = self.block_alloc.get_bitmap(group_idx);
            let mut padded_bitmap = vec![0xFFu8; block_size as usize];
            padded_bitmap[..block_bitmap.len()].copy_from_slice(block_bitmap);
            // 将超出范围的位置 1
            for bit in blocks_in_group..blocks_per_group {
                let byte_idx = (bit / 8) as usize;
                let bit_idx = bit % 8;
                if byte_idx < padded_bitmap.len() {
                    padded_bitmap[byte_idx] |= 1 << bit_idx;
                }
            }
            self.writer.write_all(&padded_bitmap)?;

            // 写入 inode bitmap
            let inode_bitmap_offset = block_bitmap_offset + block_size as u64;
            self.writer.seek(SeekFrom::Start(inode_bitmap_offset))?;
            let inode_bitmap = self.inode_alloc.get_bitmap(group_idx);
            let mut padded_bitmap = vec![0xFFu8; block_size as usize];
            padded_bitmap[..inode_bitmap.len()].copy_from_slice(inode_bitmap);
            // 将超出范围的位置 1
            for bit in inodes_in_group..inodes_per_group {
                let byte_idx = (bit / 8) as usize;
                let bit_idx = bit % 8;
                if byte_idx < padded_bitmap.len() {
                    padded_bitmap[byte_idx] |= 1 << bit_idx;
                }
            }
            self.writer.write_all(&padded_bitmap)?;
        }

        Ok(())
    }

    // 计算 group descriptor 的 checksum (CRC16)
    fn calc_group_desc_checksum(&self, group_idx: u32, gd: &Ext4GroupDescriptor) -> u16 {
        let uuid = self.sb_builder.uuid();
        let mut crc = ext4_crc16(!0, &uuid);
        crc = ext4_crc16(crc, &group_idx.to_le_bytes());

        let gd_bytes: &[u8] = zerocopy::IntoBytes::as_bytes(gd);
        // checksum 字段偏移为 30 (bg_checksum 在结构体中的位置)
        crc = ext4_crc16(crc, &gd_bytes[..30]);
        crc = ext4_crc16(crc, &gd_bytes[32..]);
        crc
    }
}

// EXT4 CRC16 计算
fn ext4_crc16(crc: u16, data: &[u8]) -> u16 {
    let mut crc = crc;
    for &byte in data {
        crc = (crc >> 8) ^ CRC16_TABLE[((crc ^ byte as u16) & 0xFF) as usize];
    }
    crc
}

// CRC16 查找表
const CRC16_TABLE: [u16; 256] = [
    0x0000, 0xC0C1, 0xC181, 0x0140, 0xC301, 0x03C0, 0x0280, 0xC241, 0xC601, 0x06C0, 0x0780, 0xC741,
    0x0500, 0xC5C1, 0xC481, 0x0440, 0xCC01, 0x0CC0, 0x0D80, 0xCD41, 0x0F00, 0xCFC1, 0xCE81, 0x0E40,
    0x0A00, 0xCAC1, 0xCB81, 0x0B40, 0xC901, 0x09C0, 0x0880, 0xC841, 0xD801, 0x18C0, 0x1980, 0xD941,
    0x1B00, 0xDBC1, 0xDA81, 0x1A40, 0x1E00, 0xDEC1, 0xDF81, 0x1F40, 0xDD01, 0x1DC0, 0x1C80, 0xDC41,
    0x1400, 0xD4C1, 0xD581, 0x1540, 0xD701, 0x17C0, 0x1680, 0xD641, 0xD201, 0x12C0, 0x1380, 0xD341,
    0x1100, 0xD1C1, 0xD081, 0x1040, 0xF001, 0x30C0, 0x3180, 0xF141, 0x3300, 0xF3C1, 0xF281, 0x3240,
    0x3600, 0xF6C1, 0xF781, 0x3740, 0xF501, 0x35C0, 0x3480, 0xF441, 0x3C00, 0xFCC1, 0xFD81, 0x3D40,
    0xFF01, 0x3FC0, 0x3E80, 0xFE41, 0xFA01, 0x3AC0, 0x3B80, 0xFB41, 0x3900, 0xF9C1, 0xF881, 0x3840,
    0x2800, 0xE8C1, 0xE981, 0x2940, 0xEB01, 0x2BC0, 0x2A80, 0xEA41, 0xEE01, 0x2EC0, 0x2F80, 0xEF41,
    0x2D00, 0xEDC1, 0xEC81, 0x2C40, 0xE401, 0x24C0, 0x2580, 0xE541, 0x2700, 0xE7C1, 0xE681, 0x2640,
    0x2200, 0xE2C1, 0xE381, 0x2340, 0xE101, 0x21C0, 0x2080, 0xE041, 0xA001, 0x60C0, 0x6180, 0xA141,
    0x6300, 0xA3C1, 0xA281, 0x6240, 0x6600, 0xA6C1, 0xA781, 0x6740, 0xA501, 0x65C0, 0x6480, 0xA441,
    0x6C00, 0xACC1, 0xAD81, 0x6D40, 0xAF01, 0x6FC0, 0x6E80, 0xAE41, 0xAA01, 0x6AC0, 0x6B80, 0xAB41,
    0x6900, 0xA9C1, 0xA881, 0x6840, 0x7800, 0xB8C1, 0xB981, 0x7940, 0xBB01, 0x7BC0, 0x7A80, 0xBA41,
    0xBE01, 0x7EC0, 0x7F80, 0xBF41, 0x7D00, 0xBDC1, 0xBC81, 0x7C40, 0xB401, 0x74C0, 0x7580, 0xB541,
    0x7700, 0xB7C1, 0xB681, 0x7640, 0x7200, 0xB2C1, 0xB381, 0x7340, 0xB101, 0x71C0, 0x7080, 0xB041,
    0x5000, 0x90C1, 0x9181, 0x5140, 0x9301, 0x53C0, 0x5280, 0x9241, 0x9601, 0x56C0, 0x5780, 0x9741,
    0x5500, 0x95C1, 0x9481, 0x5440, 0x9C01, 0x5CC0, 0x5D80, 0x9D41, 0x5F00, 0x9FC1, 0x9E81, 0x5E40,
    0x5A00, 0x9AC1, 0x9B81, 0x5B40, 0x9901, 0x59C0, 0x5880, 0x9841, 0x8801, 0x48C0, 0x4980, 0x8941,
    0x4B00, 0x8BC1, 0x8A81, 0x4A40, 0x4E00, 0x8EC1, 0x8F81, 0x4F40, 0x8D01, 0x4DC0, 0x4C80, 0x8C41,
    0x4400, 0x84C1, 0x8581, 0x4540, 0x8701, 0x47C0, 0x4680, 0x8641, 0x8201, 0x42C0, 0x4380, 0x8341,
    0x4100, 0x81C1, 0x8081, 0x4040,
];

// 简化的构建入口函数
pub fn build_ext4_image(
    source_dir: &Path,
    output_path: &Path,
    image_size: u64,
    mount_point: &str,
) -> Result<()> {
    let config = Ext4BuilderConfig {
        source_dir: source_dir.to_path_buf(),
        output_path: output_path.to_path_buf(),
        image_size,
        volume_label: String::new(),
        mount_point: mount_point.to_string(),
        root_uid: 0,
        root_gid: 0,
        file_contexts: None,
        fs_config: None,
        timestamp: None,
    };

    let mut builder = Ext4Builder::new(config)?;
    builder.build()
}
