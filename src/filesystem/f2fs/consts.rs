// F2FS constant definition
//
// Based on constant definitions in Linux kernel f2fs_fs.h.

// F2FS magic number
pub const F2FS_MAGIC: u32 = 0xF2F52010;

// Superblock offset (bytes)
pub const F2FS_SUPER_OFFSET: u64 = 1024;

// F2FS block size (4KB)
pub const F2FS_BLKSIZE: usize = 4096;

// Maximum file name length
pub const F2FS_NAME_LEN: usize = 255;

// Directory slot length
pub const F2FS_SLOT_LEN: usize = 8;

// Number of NAT entries per block (4096 / 9 = 455)
pub const NAT_ENTRY_PER_BLOCK: usize = F2FS_BLKSIZE / 9;

// Number of SIT entries in each block (4096 / 74 = 55)
pub const SIT_ENTRY_PER_BLOCK: usize = F2FS_BLKSIZE / 74;

// Empty address (sparse block)
pub const NULL_ADDR: u32 = 0;

// New address (not assigned)
pub const NEW_ADDR: u32 = 0xFFFFFFFF;

// Compressed address tag
pub const COMPRESS_ADDR: u32 = 0xFFFFFFFE;

// Number of direct addresses in Inode (923)
// Calculation: (4096 - 360 - 20 - 24) / 4
pub const DEF_ADDRS_PER_INODE: usize = (F2FS_BLKSIZE - 360 - 20 - 24) / 4;

// Number of addresses in direct nodes (1018)
// Calculation: (4096 - 24) / 4
pub const DEF_ADDRS_PER_BLOCK: usize = (F2FS_BLKSIZE - 24) / 4;

// Default number of inline xattr addresses
pub const DEFAULT_INLINE_XATTR_ADDRS: usize = 50;

// File type: ordinary file
pub const F2FS_FT_REG_FILE: u8 = 1;

// File type: directory
pub const F2FS_FT_DIR: u8 = 2;

// File type: symbolic link
pub const F2FS_FT_SYMLINK: u8 = 7;

// Compression algorithm: LZO
pub const COMPR_LZO: u8 = 0;

// Compression algorithm: LZ4
pub const COMPR_LZ4: u8 = 1;

// Compression algorithm: ZSTD
pub const COMPR_ZSTD: u8 = 2;

// Inode flag: inline xattr
pub const F2FS_INLINE_XATTR: u8 = 0x01;

// Inode flag: inline data
pub const F2FS_INLINE_DATA: u8 = 0x02;

// Inode flag: inline directory
pub const F2FS_INLINE_DENTRY: u8 = 0x04;

// Inode flag: inline data exists
pub const F2FS_DATA_EXIST: u8 = 0x08;

// Inode flags: additional attributes
pub const F2FS_EXTRA_ATTR: u8 = 0x20;

// File flags: compressed
pub const F2FS_COMPR_FL: u32 = 0x00000004;

// XATTR index
pub const F2FS_XATTR_INDEX_USER: u8 = 1;
pub const F2FS_XATTR_INDEX_POSIX_ACL_ACCESS: u8 = 2;
pub const F2FS_XATTR_INDEX_POSIX_ACL_DEFAULT: u8 = 3;
pub const F2FS_XATTR_INDEX_TRUSTED: u8 = 4;
pub const F2FS_XATTR_INDEX_LUSTRE: u8 = 5;
pub const F2FS_XATTR_INDEX_SECURITY: u8 = 6;
pub const F2FS_XATTR_INDEX_ADVISE: u8 = 7;
pub const F2FS_XATTR_INDEX_ENCRYPTION: u8 = 9;
pub const F2FS_XATTR_INDEX_VERITY: u8 = 11;

// XATTR name
pub const XATTR_SECURITY_PREFIX: &str = "security.";
pub const XATTR_SELINUX_SUFFIX: &str = "selinux";

// XATTR entry size
pub const F2FS_XATTR_ENTRY_SIZE: usize = 4; // Minimum header size for each entry

// ============ Format related constants ============

// super block magic number
pub const F2FS_SUPER_MAGIC: u32 = 0xF2F52010;

// version number
pub const F2FS_MAJOR_VERSION: u16 = 1;
pub const F2FS_MINOR_VERSION: u16 = 16;

// Default sector size
pub const DEFAULT_SECTOR_SIZE: u32 = 512;
pub const DEFAULT_SECTORS_PER_BLOCK: u32 = 8; // 4096 / 512

// Number of blocks per segment
pub const DEFAULT_BLOCKS_PER_SEGMENT: u32 = 512;

// Number of segments per segment
pub const DEFAULT_SEGMENTS_PER_SECTION: u32 = 1;

// Number of sections per area
pub const DEFAULT_SECTIONS_PER_ZONE: u32 = 1;

// Number of checkpoint packages
pub const F2FS_NUMBER_OF_CHECKPOINT_PACK: u32 = 2;

// Reserve inode number
pub const F2FS_NODE_INO: u32 = 1;
pub const F2FS_META_INO: u32 = 2;
pub const F2FS_ROOT_INO: u32 = 3;
pub const F2FS_FIRST_INO: u32 = 4;

// Number of current segment types
pub const NR_CURSEG_TYPE: usize = 6;

// Current segment type
pub const CURSEG_HOT_DATA: usize = 0;
pub const CURSEG_WARM_DATA: usize = 1;
pub const CURSEG_COLD_DATA: usize = 2;
pub const CURSEG_HOT_NODE: usize = 3;
pub const CURSEG_WARM_NODE: usize = 4;
pub const CURSEG_COLD_NODE: usize = 5;

// checkpoint flag
pub const CP_UMOUNT_FLAG: u32 = 0x00000001;
pub const CP_ORPHAN_PRESENT_FLAG: u32 = 0x00000002;
pub const CP_COMPACT_SUM_FLAG: u32 = 0x00000004;
pub const CP_ERROR_FLAG: u32 = 0x00000008;
pub const CP_FSCK_FLAG: u32 = 0x00000010;
pub const CP_FASTBOOT_FLAG: u32 = 0x00000020;
pub const CP_CRC_RECOVERY_FLAG: u32 = 0x00000040;
pub const CP_NAT_BITS_FLAG: u32 = 0x00000080;
pub const CP_TRIMMED_FLAG: u32 = 0x00000100;
pub const CP_NOCRC_RECOVERY_FLAG: u32 = 0x00000200;
pub const CP_LARGE_NAT_BITMAP_FLAG: u32 = 0x00000400;

// F2FS feature flags
pub const F2FS_FEATURE_ENCRYPT: u32 = 0x0001;
pub const F2FS_FEATURE_BLKZONED: u32 = 0x0002;
pub const F2FS_FEATURE_ATOMIC_WRITE: u32 = 0x0004;
pub const F2FS_FEATURE_EXTRA_ATTR: u32 = 0x0008;
pub const F2FS_FEATURE_PRJQUOTA: u32 = 0x0010;
pub const F2FS_FEATURE_INODE_CHKSUM: u32 = 0x0020;
pub const F2FS_FEATURE_FLEXIBLE_INLINE_XATTR: u32 = 0x0040;
pub const F2FS_FEATURE_QUOTA_INO: u32 = 0x0080;
pub const F2FS_FEATURE_INODE_CRTIME: u32 = 0x0100;
pub const F2FS_FEATURE_LOST_FOUND: u32 = 0x0200;
pub const F2FS_FEATURE_VERITY: u32 = 0x0400;
pub const F2FS_FEATURE_SB_CHKSUM: u32 = 0x0800;
pub const F2FS_FEATURE_CASEFOLD: u32 = 0x1000;
pub const F2FS_FEATURE_COMPRESSION: u32 = 0x2000;
pub const F2FS_FEATURE_RO: u32 = 0x4000;

// NAT entry size
pub const NAT_ENTRY_SIZE: usize = 9;

// SIT entry size
pub const SIT_ENTRY_SIZE: usize = 74;
pub const SIT_VBLOCK_MAP_SIZE: usize = 64;

// SIT vblocks field bit definition
pub const SIT_VBLOCKS_SHIFT: u16 = 10;
pub const SIT_VBLOCKS_MASK: u16 = (1 << SIT_VBLOCKS_SHIFT) - 1;

// SSA entry size
pub const SUMMARY_SIZE: usize = 7;
pub const SUM_FOOTER_SIZE: usize = 5;
pub const SUM_ENTRY_SIZE: usize = 7;
pub const ENTRIES_IN_SUM: usize = 512;

// Summary journal size
// SUM_JOURNAL_SIZE = F2FS_BLKSIZE - SUM_FOOTER_SIZE - SUM_ENTRIES_SIZE
// SUM_ENTRIES_SIZE = SUMMARY_SIZE * ENTRIES_IN_SUM = 7 * 512 = 3584
// SUM_JOURNAL_SIZE = 4096 - 5 - 3584 = 507
pub const SUM_ENTRIES_SIZE: usize = SUMMARY_SIZE * ENTRIES_IN_SUM;
pub const SUM_JOURNAL_SIZE: usize = F2FS_BLKSIZE - SUM_FOOTER_SIZE - SUM_ENTRIES_SIZE;

// superblock checksum offset
pub const SB_CHKSUM_OFFSET: usize = 3068;

// Checkpoint checksum offset
pub const CP_CHKSUM_OFFSET: usize = F2FS_BLKSIZE - 4;

// Maximum number of extensions
pub const F2FS_MAX_EXTENSION: usize = 64;

// extension length
pub const F2FS_EXTENSION_LEN: usize = 8;

// Maximum number of devices
pub const MAX_DEVICES: usize = 8;

// Maximum volume name length
pub const MAX_VOLUME_NAME: usize = 512;

// version string length
pub const VERSION_LEN: usize = 256;

// Quota type
pub const F2FS_MAX_QUOTAS: usize = 3;

// Node block header size
pub const NODE_FOOTER_SIZE: usize = 24;

// Inode structure size
pub const F2FS_INODE_SIZE: usize = 360;

// Extra inode size
pub const F2FS_EXTRA_ISIZE: u16 = 36;

// Inline data size
pub const MAX_INLINE_DATA_SIZE: usize = 3448;

// Inline directory size
pub const NR_INLINE_DENTRY: usize = 61;
pub const INLINE_DENTRY_BITMAP_SIZE: usize = 8;
pub const INLINE_RESERVED_SIZE: usize = 1;

// directory entry size
pub const F2FS_DIR_ENTRY_SIZE: usize = 11;

// Number of directory entries per block
pub const NR_DENTRY_IN_BLOCK: usize = 214;
pub const SIZE_OF_DIR_ENTRY: usize = 11;
pub const SIZE_OF_DENTRY_BITMAP: usize = 27;
pub const SIZE_OF_RESERVED: usize = 3;

// Inode mode
pub const S_IFMT: u16 = 0o170000;
pub const S_IFSOCK: u16 = 0o140000;
pub const S_IFLNK: u16 = 0o120000;
pub const S_IFREG: u16 = 0o100000;
pub const S_IFBLK: u16 = 0o060000;
pub const S_IFDIR: u16 = 0o040000;
pub const S_IFCHR: u16 = 0o020000;
pub const S_IFIFO: u16 = 0o010000;

// Permission bit
pub const S_ISUID: u16 = 0o4000;
pub const S_ISGID: u16 = 0o2000;
pub const S_ISVTX: u16 = 0o1000;
pub const S_IRWXU: u16 = 0o0700;
pub const S_IRUSR: u16 = 0o0400;
pub const S_IWUSR: u16 = 0o0200;
pub const S_IXUSR: u16 = 0o0100;
pub const S_IRWXG: u16 = 0o0070;
pub const S_IRGRP: u16 = 0o0040;
pub const S_IWGRP: u16 = 0o0020;
pub const S_IXGRP: u16 = 0o0010;
pub const S_IRWXO: u16 = 0o0007;
pub const S_IROTH: u16 = 0o0004;
pub const S_IWOTH: u16 = 0o0002;
pub const S_IXOTH: u16 = 0o0001;

// Default directory permissions
pub const DEFAULT_DIR_MODE: u16 = S_IFDIR | 0o755;
pub const DEFAULT_FILE_MODE: u16 = S_IFREG | 0o644;
pub const DEFAULT_SYMLINK_MODE: u16 = S_IFLNK | 0o777;
