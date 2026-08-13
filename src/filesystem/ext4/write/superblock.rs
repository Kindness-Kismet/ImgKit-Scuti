// EXT4 superblock 构建器

use crate::filesystem::ext4::types::*;
use crate::filesystem::ext4::{Ext4Error, Result};
use std::time::{SystemTime, UNIX_EPOCH};
use zerocopy::TryFromBytes;

// superblock 偏移 (相对分区起始位置)
pub const EXT4_SUPERBLOCK_OFFSET: u64 = 1024;

// 默认块大小
pub const DEFAULT_BLOCK_SIZE: u32 = 4096;

// 默认 inode 大小
pub const DEFAULT_INODE_SIZE: u16 = 256;

// 每个 block group 的默认块数
pub const DEFAULT_BLOCKS_PER_GROUP: u32 = 32768;

// 每个 block group 的默认 inode 数
pub const DEFAULT_INODES_PER_GROUP: u32 = 8192;

// EXT4 特性标志
pub const EXT4_FEATURE_COMPAT_HAS_JOURNAL: u32 = 0x0004;
pub const EXT4_FEATURE_COMPAT_EXT_ATTR: u32 = 0x0008;
pub const EXT4_FEATURE_COMPAT_RESIZE_INODE: u32 = 0x0010;
pub const EXT4_FEATURE_COMPAT_DIR_INDEX: u32 = 0x0020;

pub const EXT4_FEATURE_INCOMPAT_FILETYPE: u32 = 0x0002;
pub const EXT4_FEATURE_INCOMPAT_EXTENTS: u32 = 0x0040;
pub const EXT4_FEATURE_INCOMPAT_64BIT: u32 = 0x0080;
pub const EXT4_FEATURE_INCOMPAT_FLEX_BG: u32 = 0x0200;

pub const EXT4_FEATURE_RO_COMPAT_SPARSE_SUPER: u32 = 0x0001;
pub const EXT4_FEATURE_RO_COMPAT_LARGE_FILE: u32 = 0x0002;
pub const EXT4_FEATURE_RO_COMPAT_HUGE_FILE: u32 = 0x0008;
pub const EXT4_FEATURE_RO_COMPAT_GDT_CSUM: u32 = 0x0010;
pub const EXT4_FEATURE_RO_COMPAT_DIR_NLINK: u32 = 0x0020;
pub const EXT4_FEATURE_RO_COMPAT_EXTRA_ISIZE: u32 = 0x0040;

// superblock 构建器
pub struct SuperblockBuilder {
    block_size: u32,
    inode_size: u16,
    blocks_count: u64,
    inodes_count: u32,
    blocks_per_group: u32,
    inodes_per_group: u32,
    volume_label: String,
    uuid: [u8; 16],
    timestamp: u32,
    free_blocks_count: Option<u64>,
    free_inodes_count: Option<u32>,
}

impl SuperblockBuilder {
    // 创建新的 superblock 构建器
    pub fn new(image_size: u64) -> Self {
        let block_size = DEFAULT_BLOCK_SIZE;
        let blocks_count = image_size / block_size as u64;
        let blocks_per_group = DEFAULT_BLOCKS_PER_GROUP;
        let inodes_per_group = DEFAULT_INODES_PER_GROUP;

        // 计算 block group 数量
        let group_count = blocks_count.div_ceil(blocks_per_group as u64) as u32;

        // 计算 inode 总数 (每个 block group 有 inodes_per_group 个 inode)
        let inodes_count = group_count * inodes_per_group;

        SuperblockBuilder {
            block_size,
            inode_size: DEFAULT_INODE_SIZE,
            blocks_count,
            inodes_count,
            blocks_per_group,
            inodes_per_group,
            volume_label: String::new(),
            uuid: [0u8; 16],
            // 系统时钟早于 epoch 时退化为 0, 不影响镜像有效性
            timestamp: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs() as u32,
            free_blocks_count: None,
            free_inodes_count: None,
        }
    }

    // 设置卷标
    pub fn with_label(mut self, label: &str) -> Self {
        self.volume_label = label.to_string();
        self
    }

    // 设置 UUID
    pub fn with_uuid(mut self, uuid: [u8; 16]) -> Self {
        self.uuid = uuid;
        self
    }

    // 设置实际空闲块数
    pub fn set_free_blocks_count(&mut self, count: u64) {
        self.free_blocks_count = Some(count);
    }

    // 设置实际空闲 inode 数
    pub fn set_free_inodes_count(&mut self, count: u32) {
        self.free_inodes_count = Some(count);
    }

    // 设置块大小
    pub fn with_block_size(mut self, block_size: u32) -> Self {
        self.block_size = block_size;
        self.blocks_count = (self.blocks_count * self.block_size as u64) / block_size as u64;
        self
    }

    // 计算 block group 数量
    pub fn group_count(&self) -> u32 {
        self.blocks_count.div_ceil(self.blocks_per_group as u64) as u32
    }

    // 计算 log2(block_size) - 10
    fn log_block_size(&self) -> u32 {
        self.block_size.trailing_zeros() - 10
    }

    // 构建 superblock
    pub fn build(&self) -> Result<Ext4Superblock> {
        // 使用实际空闲数量, 缺省时进行估算
        let free_blocks = self.free_blocks_count.unwrap_or_else(|| {
            let metadata_blocks = self.estimate_metadata_blocks();
            self.blocks_count.saturating_sub(metadata_blocks)
        });
        let free_inodes = self.free_inodes_count.unwrap_or(self.inodes_count - 11);

        let mut sb =
            Ext4Superblock::try_read_from_bytes(&[0u8; std::mem::size_of::<Ext4Superblock>()])
                .map_err(|_| Ext4Error::StructInit("ext4 superblock"))?;

        // 基础信息
        sb.s_inodes_count = self.inodes_count;
        sb.s_blocks_count_lo = (self.blocks_count & 0xFFFFFFFF) as u32;
        sb.s_blocks_count_hi = (self.blocks_count >> 32) as u32;
        sb.s_r_blocks_count_lo = 0; // 保留块数量
        sb.s_r_blocks_count_hi = 0;
        sb.s_free_blocks_count_lo = (free_blocks & 0xFFFFFFFF) as u32;
        sb.s_free_blocks_count_hi = (free_blocks >> 32) as u32;
        sb.s_free_inodes_count = free_inodes;

        // 块与 inode 配置
        sb.s_first_data_block = if self.block_size == 1024 { 1 } else { 0 };
        sb.s_log_block_size = self.log_block_size();
        sb.s_log_cluster_size = self.log_block_size(); // 通常与块大小一致
        sb.s_blocks_per_group = self.blocks_per_group;
        sb.s_clusters_per_group = self.blocks_per_group;
        sb.s_inodes_per_group = self.inodes_per_group;

        // 时间戳
        sb.s_mtime = 0;
        sb.s_wtime = self.timestamp;
        sb.s_mkfs_time = self.timestamp;

        // 挂载计数
        sb.s_mnt_count = 0;
        sb.s_max_mnt_count = 65535;

        // 魔数
        sb.s_magic = EXT4_SUPERBLOCK_MAGIC;

        // 文件系统状态
        sb.s_state = 1; // EXT4_VALID_FS
        sb.s_errors = 1; // EXT4_ERRORS_CONTINUE

        // 版本
        sb.s_minor_rev_level = 0;
        sb.s_rev_level = 1; // EXT4_DYNAMIC_REV

        // 默认 UID/GID
        sb.s_def_resuid = 0;
        sb.s_def_resgid = 0;

        // 第一个非保留 inode
        sb.s_first_ino = 11;

        // inode 大小
        sb.s_inode_size = self.inode_size;

        // 特性标志
        sb.s_feature_compat = EXT4_FEATURE_COMPAT_EXT_ATTR | EXT4_FEATURE_COMPAT_DIR_INDEX;

        sb.s_feature_incompat = EXT4_FEATURE_INCOMPAT_FILETYPE
            | EXT4_FEATURE_INCOMPAT_EXTENTS
            | EXT4_FEATURE_INCOMPAT_64BIT
            | EXT4_FEATURE_INCOMPAT_FLEX_BG;

        sb.s_feature_ro_compat = EXT4_FEATURE_RO_COMPAT_SPARSE_SUPER
            | EXT4_FEATURE_RO_COMPAT_LARGE_FILE
            | EXT4_FEATURE_RO_COMPAT_HUGE_FILE
            | EXT4_FEATURE_RO_COMPAT_GDT_CSUM
            | EXT4_FEATURE_RO_COMPAT_DIR_NLINK
            | EXT4_FEATURE_RO_COMPAT_EXTRA_ISIZE;

        // UUID 与卷标
        sb.s_uuid = self.uuid;
        let label_bytes = self.volume_label.as_bytes();
        let copy_len = label_bytes.len().min(16);
        sb.s_volume_name[..copy_len].copy_from_slice(&label_bytes[..copy_len]);

        // htree hash 种子 (随机值)
        sb.s_hash_seed = [0x12345678, 0x9abcdef0, 0x13579bdf, 0x2468ace0];
        sb.s_def_hash_version = 1; // DX_HASH_HALF_MD4

        // group descriptor 尺寸
        sb.s_desc_size = EXT2_MIN_DESC_SIZE_64BIT;

        // 额外 inode 空间大小
        sb.s_min_extra_isize = 32;
        sb.s_want_extra_isize = 32;

        // flex_bg 配置
        sb.s_log_groups_per_flex = 4; // 16 个 block group 组成一个 flex group

        Ok(sb)
    }

    // 估算元数据块数量
    fn estimate_metadata_blocks(&self) -> u64 {
        let group_count = self.group_count() as u64;

        // 每个 block group 的元数据:
        // - superblock 备份 (部分 block group): 1 块
        // - group descriptor 表: 按 block group 数量计算
        // - block bitmap: 1 块
        // - inode bitmap: 1 块
        // - inode table: (inodes_per_group * inode_size) / block_size

        let gdt_blocks =
            (group_count * EXT2_MIN_DESC_SIZE_64BIT as u64).div_ceil(self.block_size as u64);

        let inode_table_blocks = (self.inodes_per_group as u64 * self.inode_size as u64)
            .div_ceil(self.block_size as u64);

        // 每个 block group 的元数据块数
        let blocks_per_group_metadata = 1 + gdt_blocks + 1 + 1 + inode_table_blocks;

        // 元数据块总数
        group_count * blocks_per_group_metadata
    }

    // 获取块大小
    pub fn block_size(&self) -> u32 {
        self.block_size
    }

    // 获取 inode 大小
    pub fn inode_size(&self) -> u16 {
        self.inode_size
    }

    // 获取每个 block group 的块数
    pub fn blocks_per_group(&self) -> u32 {
        self.blocks_per_group
    }

    // 获取每个 block group 的 inode 数
    pub fn inodes_per_group(&self) -> u32 {
        self.inodes_per_group
    }

    // 获取总块数
    pub fn blocks_count(&self) -> u64 {
        self.blocks_count
    }

    // 获取 inode 总数
    pub fn inodes_count(&self) -> u32 {
        self.inodes_count
    }

    // 获取 UUID
    pub fn uuid(&self) -> [u8; 16] {
        self.uuid
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_superblock_builder() {
        let builder = SuperblockBuilder::new(100 * 1024 * 1024); // 100MB
        let sb = builder.build().unwrap();

        assert_eq!({ sb.s_magic }, EXT4_SUPERBLOCK_MAGIC);
        assert!(sb.s_blocks_count() > 0);
        assert!({ sb.s_inodes_count } > 0);
    }

    #[test]
    fn test_group_count() {
        let builder = SuperblockBuilder::new(1024 * 1024 * 1024); // 1GB
        let group_count = builder.group_count();

        // 1GB / 4KB = 262144 个块
        // 262144 / 32768 = 8 个 block group
        assert_eq!(group_count, 8);
    }
}
