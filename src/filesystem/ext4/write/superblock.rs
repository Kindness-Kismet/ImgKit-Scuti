// EXT4 SuperBlock Builder

use crate::filesystem::ext4::Result;
use crate::filesystem::ext4::types::*;
use std::time::{SystemTime, UNIX_EPOCH};
use zerocopy::TryFromBytes;

// Superblock offset (from partition start)
pub const EXT4_SUPERBLOCK_OFFSET: u64 = 1024;

// Default block size
pub const DEFAULT_BLOCK_SIZE: u32 = 4096;

// Default inode size
pub const DEFAULT_INODE_SIZE: u16 = 256;

// Default number of blocks per group
pub const DEFAULT_BLOCKS_PER_GROUP: u32 = 32768;

// Default number of inodes per group
pub const DEFAULT_INODES_PER_GROUP: u32 = 8192;

// EXT4 feature flags
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

// super block builder
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
    // Create a new superblock builder
    pub fn new(image_size: u64) -> Self {
        let block_size = DEFAULT_BLOCK_SIZE;
        let blocks_count = image_size / block_size as u64;
        let blocks_per_group = DEFAULT_BLOCKS_PER_GROUP;
        let inodes_per_group = DEFAULT_INODES_PER_GROUP;

        // Calculate the number of block groups
        let group_count = blocks_count.div_ceil(blocks_per_group as u64) as u32;

        // Count the number of inodes (inodes_per_group inodes per block group)
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
            timestamp: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs() as u32,
            free_blocks_count: None,
            free_inodes_count: None,
        }
    }

    // Set volume label
    pub fn with_label(mut self, label: &str) -> Self {
        self.volume_label = label.to_string();
        self
    }

    // Set UUID
    pub fn with_uuid(mut self, uuid: [u8; 16]) -> Self {
        self.uuid = uuid;
        self
    }

    // Set the actual number of free blocks
    pub fn set_free_blocks_count(&mut self, count: u64) {
        self.free_blocks_count = Some(count);
    }

    // Set the actual number of free inodes
    pub fn set_free_inodes_count(&mut self, count: u32) {
        self.free_inodes_count = Some(count);
    }

    // Set block size
    pub fn with_block_size(mut self, block_size: u32) -> Self {
        self.block_size = block_size;
        self.blocks_count = (self.blocks_count * self.block_size as u64) / block_size as u64;
        self
    }

    // Calculate the number of block groups
    pub fn group_count(&self) -> u32 {
        self.blocks_count.div_ceil(self.blocks_per_group as u64) as u32
    }

    // Calculate log2(block_size) - 10
    fn log_block_size(&self) -> u32 {
        self.block_size.trailing_zeros() - 10
    }

    // Building a superblock
    pub fn build(&self) -> Result<Ext4Superblock> {
        // Use actual idle count or estimate
        let free_blocks = self.free_blocks_count.unwrap_or_else(|| {
            let metadata_blocks = self.estimate_metadata_blocks();
            self.blocks_count.saturating_sub(metadata_blocks)
        });
        let free_inodes = self.free_inodes_count.unwrap_or(self.inodes_count - 11);

        let mut sb =
            Ext4Superblock::try_read_from_bytes(&[0u8; std::mem::size_of::<Ext4Superblock>()])
                .unwrap();

        // Basic information
        sb.s_inodes_count = self.inodes_count;
        sb.s_blocks_count_lo = (self.blocks_count & 0xFFFFFFFF) as u32;
        sb.s_blocks_count_hi = (self.blocks_count >> 32) as u32;
        sb.s_r_blocks_count_lo = 0; // Number of reserved blocks
        sb.s_r_blocks_count_hi = 0;
        sb.s_free_blocks_count_lo = (free_blocks & 0xFFFFFFFF) as u32;
        sb.s_free_blocks_count_hi = (free_blocks >> 32) as u32;
        sb.s_free_inodes_count = free_inodes;

        // Block and inode configuration
        sb.s_first_data_block = if self.block_size == 1024 { 1 } else { 0 };
        sb.s_log_block_size = self.log_block_size();
        sb.s_log_cluster_size = self.log_block_size(); // Usually the same as the block size
        sb.s_blocks_per_group = self.blocks_per_group;
        sb.s_clusters_per_group = self.blocks_per_group;
        sb.s_inodes_per_group = self.inodes_per_group;

        // Timestamp
        sb.s_mtime = 0;
        sb.s_wtime = self.timestamp;
        sb.s_mkfs_time = self.timestamp;

        // mount count
        sb.s_mnt_count = 0;
        sb.s_max_mnt_count = 65535;

        // magic number
        sb.s_magic = EXT4_SUPERBLOCK_MAGIC;

        // state
        sb.s_state = 1; // EXT4_VALID_FS
        sb.s_errors = 1; // EXT4_ERRORS_CONTINUE

        // Version
        sb.s_minor_rev_level = 0;
        sb.s_rev_level = 1; // EXT4_DYNAMIC_REV

        // Default UID/GID
        sb.s_def_resuid = 0;
        sb.s_def_resgid = 0;

        // first non-reserved inode
        sb.s_first_ino = 11;

        // Inode size
        sb.s_inode_size = self.inode_size;

        // Feature flag
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

        // UUID and volume label
        sb.s_uuid = self.uuid;
        let label_bytes = self.volume_label.as_bytes();
        let copy_len = label_bytes.len().min(16);
        sb.s_volume_name[..copy_len].copy_from_slice(&label_bytes[..copy_len]);

        // Hash seed (random)
        sb.s_hash_seed = [0x12345678, 0x9abcdef0, 0x13579bdf, 0x2468ace0];
        sb.s_def_hash_version = 1; // DX_HASH_HALF_MD4

        // Block group descriptor size
        sb.s_desc_size = EXT2_MIN_DESC_SIZE_64BIT;

        // Extra inode size
        sb.s_min_extra_isize = 32;
        sb.s_want_extra_isize = 32;

        // Flex block group
        sb.s_log_groups_per_flex = 4; // 16 blocks group into a flex group

        Ok(sb)
    }

    // Estimated number of metadata blocks
    fn estimate_metadata_blocks(&self) -> u64 {
        let group_count = self.group_count() as u64;

        // Metadata for each chunk group:
        // - Super block backup (certain block groups): 1 block
        // - Block group descriptor table: calculated based on the number of block groups
        // - block bitmap: 1 block
        // - Inode bitmap: 1 block
        // - Inode table: (inodes_per_group * inode_size) / block_size

        let gdt_blocks =
            (group_count * EXT2_MIN_DESC_SIZE_64BIT as u64).div_ceil(self.block_size as u64);

        let inode_table_blocks = (self.inodes_per_group as u64 * self.inode_size as u64)
            .div_ceil(self.block_size as u64);

        // Number of metadata blocks per block group
        let blocks_per_group_metadata = 1 + gdt_blocks + 1 + 1 + inode_table_blocks;

        // Total number of metadata blocks
        group_count * blocks_per_group_metadata
    }

    // Get block size
    pub fn block_size(&self) -> u32 {
        self.block_size
    }

    // Get inode size
    pub fn inode_size(&self) -> u16 {
        self.inode_size
    }

    // Get the number of blocks in each group
    pub fn blocks_per_group(&self) -> u32 {
        self.blocks_per_group
    }

    // Get the number of inodes in each group
    pub fn inodes_per_group(&self) -> u32 {
        self.inodes_per_group
    }

    // Get the total number of blocks
    pub fn blocks_count(&self) -> u64 {
        self.blocks_count
    }

    // Get the total number of inodes
    pub fn inodes_count(&self) -> u32 {
        self.inodes_count
    }

    // Get UUID
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

        // 1GB / 4KB = 262144 blocks
        // 262144 / 32768 = 8 groups
        assert_eq!(group_count, 8);
    }
}
