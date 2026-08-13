// EXT4 文件系统结构定义

use std::io::{Read, Seek};
use zerocopy::{FromZeros, Immutable, IntoBytes, KnownLayout, TryFromBytes};

// ext4 superblock 魔数
pub const EXT4_SUPERBLOCK_MAGIC: u16 = 0xEF53;
// ext4 extent header 魔数
pub const EXT4_EXTENT_HEADER_MAGIC: u16 = 0xF30A;
// ext4 xattr header 魔数
pub const EXT4_XATTR_HEADER_MAGIC: u32 = 0xEA020000;

// 表示一个 ext4 卷
pub struct Ext4Volume<R: Read + Seek> {
    pub stream: R,
    pub superblock: Ext4Superblock,
    pub group_descriptors: Vec<Ext4GroupDescriptor>,
    pub block_size: u64,
}

// 表示一个 inode
#[derive(Clone)]
#[allow(dead_code)]
pub struct Inode {
    pub inode_idx: u32,
    pub inode: Ext4Inode,
    pub data: Vec<u8>,
}

// 表示一个 dir entry
#[repr(C, packed)]
#[derive(FromZeros, IntoBytes, Immutable, KnownLayout, Debug, Clone, Copy)]
pub struct Ext4DirEntry2 {
    pub inode: u32,
    pub rec_len: u16,
    pub name_len: u8,
    pub file_type: u8,
}

// 表示一个 extent
#[repr(C, packed)]
#[derive(FromZeros, IntoBytes, Immutable, KnownLayout, Debug, Clone, Copy)]
pub struct Ext4Extent {
    pub ee_block: u32,
    pub ee_len: u16,
    pub ee_start_hi: u16,
    pub ee_start_lo: u32,
}

impl Ext4Extent {
    // 获取 extent 在磁盘上的完整起始块号
    pub fn ee_start(&self) -> u64 {
        (self.ee_start_hi as u64) << 32 | self.ee_start_lo as u64
    }

    // 判断 extent 是否为 unwritten 状态
    pub fn is_unwritten(&self) -> bool {
        self.ee_len > 32768
    }

    // 获取 extent 的实际长度
    pub fn get_len(&self) -> u16 {
        if self.is_unwritten() {
            self.ee_len - 32768
        } else {
            self.ee_len
        }
    }
}

// 表示 extent tree 的 extent header
#[repr(C, packed)]
#[derive(FromZeros, IntoBytes, Immutable, KnownLayout, Debug, Clone, Copy)]
pub struct Ext4ExtentHeader {
    pub eh_magic: u16,
    pub eh_entries: u16,
    pub eh_max: u16,
    pub eh_depth: u16,
    pub eh_generation: u32,
}

// 表示 extent tree 中的索引节点
#[repr(C, packed)]
#[derive(FromZeros, IntoBytes, Immutable, KnownLayout, Debug, Clone, Copy)]
pub struct Ext4ExtentIdx {
    pub ei_block: u32,
    pub ei_leaf_lo: u32,
    pub ei_leaf_hi: u16,
    pub ei_unused: u16,
}

impl Ext4ExtentIdx {
    // 获取叶子节点的完整块号
    pub fn ei_leaf(&self) -> u64 {
        (self.ei_leaf_hi as u64) << 32 | self.ei_leaf_lo as u64
    }
}

// 表示 64 位文件系统的 group descriptor
#[repr(C, packed)]
#[derive(FromZeros, IntoBytes, Immutable, KnownLayout, Debug, Clone, Copy)]
pub struct Ext4GroupDescriptor {
    pub bg_block_bitmap_lo: u32,
    pub bg_inode_bitmap_lo: u32,
    pub bg_inode_table_lo: u32,
    pub bg_free_blocks_count_lo: u16,
    pub bg_free_inodes_count_lo: u16,
    pub bg_used_dirs_count_lo: u16,
    pub bg_flags: u16,
    pub bg_exclude_bitmap_lo: u32,
    pub bg_block_bitmap_csum_lo: u16,
    pub bg_inode_bitmap_csum_lo: u16,
    pub bg_itable_unused_lo: u16,
    pub bg_checksum: u16,
    pub bg_block_bitmap_hi: u32,
    pub bg_inode_bitmap_hi: u32,
    pub bg_inode_table_hi: u32,
    pub bg_free_blocks_count_hi: u16,
    pub bg_free_inodes_count_hi: u16,
    pub bg_used_dirs_count_hi: u16,
    pub bg_itable_unused_hi: u16,
    pub bg_exclude_bitmap_hi: u32,
    pub bg_block_bitmap_csum_hi: u16,
    pub bg_inode_bitmap_csum_hi: u16,
    pub bg_reserved: u32,
}

impl Ext4GroupDescriptor {
    // 获取 inode table 的完整起始块号
    pub fn bg_inode_table(&self) -> u64 {
        (self.bg_inode_table_hi as u64) << 32 | self.bg_inode_table_lo as u64
    }
}

// 表示 ext4 inode 的磁盘布局
#[repr(C, packed)]
#[derive(FromZeros, IntoBytes, Immutable, KnownLayout, Debug, Clone, Copy)]
pub struct Ext4Inode {
    pub i_mode: u16,
    pub i_uid_lo: u16,
    pub i_size_lo: u32,
    pub i_atime: u32,
    pub i_ctime: u32,
    pub i_mtime: u32,
    pub i_dtime: u32,
    pub i_gid_lo: u16,
    pub i_links_count: u16,
    pub i_blocks_lo: u32,
    pub i_flags: u32,
    pub osd1: u32,
    pub i_block: [u8; 60],
    pub i_generation: u32,
    pub i_file_acl_lo: u32,
    pub i_size_hi: u32,
    pub i_obso_faddr: u32,
    pub i_osd2_blocks_high: u16,
    pub i_file_acl_hi: u16,
    pub i_uid_hi: u16,
    pub i_gid_hi: u16,
    pub i_osd2_checksum_lo: u16,
    pub i_osd2_reserved: u16,
    pub i_extra_isize: u16,
    pub i_checksum_hi: u16,
    pub i_ctime_extra: u32,
    pub i_mtime_extra: u32,
    pub i_atime_extra: u32,
    pub i_crtime: u32,
    pub i_crtime_extra: u32,
    pub i_version_hi: u32,
    pub i_projid: u32,
    pub i_pad: [u8; 96],
}

impl Ext4Inode {
    // 获取文件的完整大小
    pub fn i_size(&self) -> u64 {
        (self.i_size_hi as u64) << 32 | self.i_size_lo as u64
    }
    // 获取完整的用户 ID
    pub fn i_uid(&self) -> u32 {
        (self.i_uid_hi as u32) << 16 | self.i_uid_lo as u32
    }
    // 获取完整的组 ID
    pub fn i_gid(&self) -> u32 {
        (self.i_gid_hi as u32) << 16 | self.i_gid_lo as u32
    }
    // 获取 xattr 所在的块地址
    pub fn i_file_acl(&self) -> u64 {
        (self.i_file_acl_hi as u64) << 32 | self.i_file_acl_lo as u64
    }
}

// 表示 ext4 superblock 的磁盘布局
#[repr(C, packed)]
#[derive(FromZeros, IntoBytes, Immutable, KnownLayout, Debug, Clone, Copy)]
#[allow(dead_code)]
pub struct Ext4Superblock {
    pub s_inodes_count: u32,
    pub s_blocks_count_lo: u32,
    pub s_r_blocks_count_lo: u32,
    pub s_free_blocks_count_lo: u32,
    pub s_free_inodes_count: u32,
    pub s_first_data_block: u32,
    pub s_log_block_size: u32,
    pub s_log_cluster_size: u32,
    pub s_blocks_per_group: u32,
    pub s_clusters_per_group: u32,
    pub s_inodes_per_group: u32,
    pub s_mtime: u32,
    pub s_wtime: u32,
    pub s_mnt_count: u16,
    pub s_max_mnt_count: u16,
    pub s_magic: u16,
    pub s_state: u16,
    pub s_errors: u16,
    pub s_minor_rev_level: u16,
    pub s_lastcheck: u32,
    pub s_checkinterval: u32,
    pub s_creator_os: u32,
    pub s_rev_level: u32,
    pub s_def_resuid: u16,
    pub s_def_resgid: u16,
    pub s_first_ino: u32,
    pub s_inode_size: u16,
    pub s_block_group_nr: u16,
    pub s_feature_compat: u32,
    pub s_feature_incompat: u32,
    pub s_feature_ro_compat: u32,
    pub s_uuid: [u8; 16],
    pub s_volume_name: [u8; 16],
    pub s_last_mounted: [u8; 64],
    pub s_algorithm_usage_bitmap: u32,
    pub s_prealloc_blocks: u8,
    pub s_prealloc_dir_blocks: u8,
    pub s_reserved_gdt_blocks: u16,
    pub s_journal_uuid: [u8; 16],
    pub s_journal_inum: u32,
    pub s_journal_dev: u32,
    pub s_last_orphan: u32,
    pub s_hash_seed: [u32; 4],
    pub s_def_hash_version: u8,
    pub s_jnl_backup_type: u8,
    pub s_desc_size: u16,
    pub s_default_mount_opts: u32,
    pub s_first_meta_bg: u32,
    pub s_mkfs_time: u32,
    pub s_jnl_blocks: [u32; 17],
    pub s_blocks_count_hi: u32,
    pub s_r_blocks_count_hi: u32,
    pub s_free_blocks_count_hi: u32,
    pub s_min_extra_isize: u16,
    pub s_want_extra_isize: u16,
    pub s_flags: u32,
    pub s_raid_stride: u16,
    pub s_mmp_interval: u16,
    pub s_mmp_block: u64,
    pub s_raid_stripe_width: u32,
    pub s_log_groups_per_flex: u8,
    pub s_checksum_type: u8,
    pub s_reserved_pad: u16,
    pub s_kbytes_written: u64,
    pub s_snapshot_inum: u32,
    pub s_snapshot_id: u32,
    pub s_snapshot_r_blocks_count: u64,
    pub s_snapshot_list: u32,
    pub s_error_count: u32,
    pub s_first_error_time: u32,
    pub s_first_error_ino: u32,
    pub s_first_error_block: u64,
    pub s_first_error_func: [u8; 32],
    pub s_first_error_line: u32,
    pub s_last_error_time: u32,
    pub s_last_error_ino: u32,
    pub s_last_error_line: u32,
    pub s_last_error_block: u64,
    pub s_last_error_func: [u8; 32],
    pub s_mount_opts: [u8; 64],
    pub s_usr_quota_inum: u32,
    pub s_grp_quota_inum: u32,
    pub s_overhead_blocks: u32,
    pub s_backup_bgs: [u32; 2],
    pub s_encrypt_algos: [u8; 4],
    pub s_encrypt_pw_salt: [u8; 16],
    pub s_lpf_ino: u32,
    pub s_prj_quota_inum: u32,
    pub s_checksum_seed: u32,
    pub s_reserved: [u32; 98],
    pub s_checksum: u32,
}

#[allow(dead_code)]
impl Ext4Superblock {
    // 获取文件系统的总块数
    pub fn s_blocks_count(&self) -> u64 {
        (self.s_blocks_count_hi as u64) << 32 | self.s_blocks_count_lo as u64
    }
}

// 32 位文件系统中 group descriptor 的最小尺寸
pub const EXT2_MIN_DESC_SIZE: u16 = 32;
// 64 位文件系统中 group descriptor 的最小尺寸
pub const EXT2_MIN_DESC_SIZE_64BIT: u16 = 64;

// incompat 标志, 表示文件系统支持 64 位特性
pub const INCOMPAT_64BIT: u32 = 0x80;

// 传统 inode 的标准尺寸
pub const EXT2_GOOD_OLD_INODE_SIZE: u16 = 128;

// 表示一个 xattr entry
#[repr(C, packed)]
#[derive(FromZeros, IntoBytes, Immutable, KnownLayout, Debug, Clone, Copy)]
pub struct Ext4XattrEntry {
    pub e_name_len: u8,
    pub e_name_index: u8,
    pub e_value_offs: u16,
    pub e_value_inum: u32,
    pub e_value_size: u32,
    pub e_hash: u32,
}

impl Ext4XattrEntry {
    // 根据 name index 获取 xattr 的名称前缀
    pub fn get_name_prefix(&self) -> &'static str {
        match self.e_name_index {
            1 => "user.",
            2 => "system.posix_acl_access",
            3 => "system.posix_acl_default",
            4 => "trusted.",
            5 => "lustre.",
            6 => "security.",
            _ => "",
        }
    }

    // 计算该 entry 的总尺寸
    pub fn size(&self) -> usize {
        (std::mem::size_of::<Self>() + self.e_name_len as usize + 3) & !3
    }
}

// 表示存放在外部块中的 xattr header
#[repr(C, packed)]
#[derive(FromZeros, IntoBytes, Immutable, KnownLayout, Debug, Clone, Copy)]
pub struct Ext4XattrHeader {
    pub h_magic: u32,
    pub h_refcount: u32,
    pub h_blocks: u32,
    pub h_hash: u32,
    pub h_checksum: u32,
    pub h_reserved: [u32; 3],
}

// 表示存放在 inode body 中的 xattr header
#[repr(C, packed)]
#[derive(FromZeros, IntoBytes, Immutable, KnownLayout, Debug, Clone, Copy)]
pub struct Ext4XattrIbodyHeader {
    pub h_magic: u32,
}

// ext4 文件类型常量定义
#[allow(dead_code)]
pub mod file_type {
    pub const UNKNOWN: u8 = 0x0;
    pub const REG: u8 = 0x1;
    pub const DIR: u8 = 0x2;
    pub const CHR: u8 = 0x3;
    pub const BLK: u8 = 0x4;
    pub const FIFO: u8 = 0x5;
    pub const SOCK: u8 = 0x6;
    pub const LNK: u8 = 0x7;
    pub const CHECKSUM: u8 = 0xDE;
}

// inode mode 中与文件类型相关的常量定义
pub mod inode_mode {
    pub const S_IFLNK: u16 = 0xA000;
    pub const S_IFREG: u16 = 0x8000;
    pub const S_IFDIR: u16 = 0x4000;
    pub const S_IFMT: u16 = 0xF000;

    pub const EXT4_EXTENTS_FL: u32 = 0x80000;
}

// 表示 32 位文件系统的 group descriptor
#[repr(C, packed)]
#[derive(FromZeros, IntoBytes, Immutable, KnownLayout, Debug, Clone, Copy)]
pub struct Ext4GroupDescriptor32 {
    pub bg_block_bitmap_lo: u32,
    pub bg_inode_bitmap_lo: u32,
    pub bg_inode_table_lo: u32,
    pub bg_free_blocks_count_lo: u16,
    pub bg_free_inodes_count_lo: u16,
    pub bg_used_dirs_count_lo: u16,
    pub bg_flags: u16,
    pub bg_exclude_bitmap_lo: u32,
    pub bg_block_bitmap_csum_lo: u16,
    pub bg_inode_bitmap_csum_lo: u16,
    pub bg_itable_unused_lo: u16,
    pub bg_checksum: u16,
}

// 实现 32 位 group descriptor 到 64 位 group descriptor 的转换
impl From<Ext4GroupDescriptor32> for Ext4GroupDescriptor {
    fn from(gd32: Ext4GroupDescriptor32) -> Self {
        Ext4GroupDescriptor {
            bg_block_bitmap_lo: gd32.bg_block_bitmap_lo,
            bg_inode_bitmap_lo: gd32.bg_inode_bitmap_lo,
            bg_inode_table_lo: gd32.bg_inode_table_lo,
            bg_free_blocks_count_lo: gd32.bg_free_blocks_count_lo,
            bg_free_inodes_count_lo: gd32.bg_free_inodes_count_lo,
            bg_used_dirs_count_lo: gd32.bg_used_dirs_count_lo,
            bg_flags: gd32.bg_flags,
            bg_exclude_bitmap_lo: gd32.bg_exclude_bitmap_lo,
            bg_block_bitmap_csum_lo: gd32.bg_block_bitmap_csum_lo,
            bg_inode_bitmap_csum_lo: gd32.bg_inode_bitmap_csum_lo,
            bg_itable_unused_lo: gd32.bg_itable_unused_lo,
            bg_checksum: gd32.bg_checksum,
            bg_block_bitmap_hi: 0,
            bg_inode_bitmap_hi: 0,
            bg_inode_table_hi: 0,
            bg_free_blocks_count_hi: 0,
            bg_free_inodes_count_hi: 0,
            bg_used_dirs_count_hi: 0,
            bg_itable_unused_hi: 0,
            bg_exclude_bitmap_hi: 0,
            bg_block_bitmap_csum_hi: 0,
            bg_inode_bitmap_csum_hi: 0,
            bg_reserved: 0,
        }
    }
}

// 定义 security.capability 扩展属性的结构
#[repr(C, packed)]
#[derive(FromZeros, IntoBytes, Immutable, KnownLayout, Debug, Clone, Copy)]
pub struct VfsCapData {
    pub magic_etc: u32,
    pub data: [CapData; 2],
}

// 定义 capability 数据的具体内容
#[repr(C, packed)]
#[derive(FromZeros, IntoBytes, Immutable, KnownLayout, Debug, Clone, Copy)]
pub struct CapData {
    pub permitted: u32,
    pub inheritable: u32,
}

impl VfsCapData {
    // 从原始字节数据解析 VfsCapData
    pub fn from_bytes(bytes: &[u8]) -> Option<Self> {
        Self::try_read_from_bytes(bytes).ok()
    }

    // 获取有效的 capability 集合
    pub fn effective(&self) -> u64 {
        let magic = self.magic_etc;
        let version = magic & 0xFF000000;
        let effective_bit = (magic & 0x00000001) != 0;

        if version == 0x02000000 {
            let mut effective_caps = self.data[0].permitted as u64;
            if effective_bit {
                effective_caps |= (self.data[1].permitted as u64) << 32;
            }
            effective_caps
        } else {
            0
        }
    }
}
