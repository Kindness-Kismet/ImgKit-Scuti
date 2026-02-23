// EROFS constant definition
// Based on Linux kernel fs/erofs/erofs_fs.h

// EROFS super block magic number
pub const EROFS_SUPER_MAGIC_V1: u32 = 0xE0F5E1E2;

// Super block offset (1KB)
pub const EROFS_SUPER_OFFSET: u64 = 1024;

// Feature flag
pub const EROFS_FEATURE_INCOMPAT_ZERO_PADDING: u32 = 0x00000001;

// Inode layout type
pub const EROFS_INODE_LAYOUT_COMPACT: u16 = 0;
pub const EROFS_INODE_LAYOUT_EXTENDED: u16 = 1;

// Inode format bit mask
pub const EROFS_I_VERSION_MASK: u16 = 0x01;
pub const EROFS_I_DATALAYOUT_BIT: u16 = 1;
pub const EROFS_I_DATALAYOUT_MASK: u16 = 0x07;

// Data layout type
pub const EROFS_INODE_FLAT_PLAIN: u16 = 0;
pub const EROFS_INODE_FLAT_INLINE: u16 = 2;
pub const EROFS_INODE_FLAT_COMPRESSION_LEGACY: u16 = 3;
pub const EROFS_INODE_CHUNK_BASED: u16 = 4;

// File type
pub const EROFS_FT_UNKNOWN: u8 = 0;
pub const EROFS_FT_REG_FILE: u8 = 1;
pub const EROFS_FT_DIR: u8 = 2;
pub const EROFS_FT_CHRDEV: u8 = 3;
pub const EROFS_FT_BLKDEV: u8 = 4;
pub const EROFS_FT_FIFO: u8 = 5;
pub const EROFS_FT_SOCK: u8 = 6;
pub const EROFS_FT_SYMLINK: u8 = 7;

// Compression algorithm
pub const Z_EROFS_COMPRESSION_LZ4: u8 = 0;
pub const Z_EROFS_COMPRESSION_LZMA: u8 = 1;
pub const Z_EROFS_COMPRESSION_DEFLATE: u8 = 2;
pub const Z_EROFS_COMPRESSION_ZSTD: u8 = 3;

// LZMA maximum dictionary size (8MB)
pub const Z_EROFS_LZMA_MAX_DICT_SIZE: u32 = 8 * 1024 * 1024;

// Compressed mode data layout
pub const EROFS_INODE_COMPRESSED_FULL: u16 = 1;
pub const EROFS_INODE_COMPRESSED_COMPACT: u16 = 3;

// Lcluster type
pub const Z_EROFS_LCLUSTER_TYPE_PLAIN: u16 = 0;
pub const Z_EROFS_LCLUSTER_TYPE_HEAD1: u16 = 1;
pub const Z_EROFS_LCLUSTER_TYPE_NONHEAD: u16 = 2;
pub const Z_EROFS_LCLUSTER_TYPE_HEAD2: u16 = 3;

// Delta[0] flag: used for the first NONHEAD cluster to store the number of compressed blocks
pub const Z_EROFS_LI_D0_CBLKCNT: u16 = 1 << 11;

// Advise logo
pub const Z_EROFS_ADVISE_BIG_PCLUSTER_1: u16 = 0x0002;

// ============ Builder related constants ============

// Inode size
pub const EROFS_INODE_COMPACT_SIZE: usize = 32;
pub const EROFS_INODE_EXTENDED_SIZE: usize = 64;

// Super block size (128 bytes, refer to erofs_fs.h)
pub const EROFS_SUPER_BLOCK_SIZE: usize = 128;

// POSIX file mode constants
pub const S_IFMT: u16 = 0o170000;
pub const S_IFREG: u16 = 0o100000;
pub const S_IFDIR: u16 = 0o040000;
pub const S_IFLNK: u16 = 0o120000;
pub const S_IFCHR: u16 = 0o020000;
pub const S_IFBLK: u16 = 0o060000;
pub const S_IFIFO: u16 = 0o010000;
pub const S_IFSOCK: u16 = 0o140000;

// XATTR index
pub const EROFS_XATTR_INDEX_USER: u8 = 1;
pub const EROFS_XATTR_INDEX_POSIX_ACL_ACCESS: u8 = 2;
pub const EROFS_XATTR_INDEX_POSIX_ACL_DEFAULT: u8 = 3;
pub const EROFS_XATTR_INDEX_TRUSTED: u8 = 4;
pub const EROFS_XATTR_INDEX_LUSTRE: u8 = 5;
pub const EROFS_XATTR_INDEX_SECURITY: u8 = 6;

// EROFS Compatible Feature Flag
pub const EROFS_FEATURE_COMPAT_SB_CHKSUM: u32 = 0x00000001;
pub const EROFS_FEATURE_COMPAT_MTIME: u32 = 0x00000002;
pub const EROFS_FEATURE_COMPAT_XATTR_FILTER: u32 = 0x00000004;

// EROFS incompatible feature flag
pub const EROFS_FEATURE_INCOMPAT_COMPR_CFGS: u32 = 0x00000002;
pub const EROFS_FEATURE_INCOMPAT_BIG_PCLUSTER: u32 = 0x00000002;
pub const EROFS_FEATURE_INCOMPAT_CHUNKED_FILE: u32 = 0x00000004;
pub const EROFS_FEATURE_INCOMPAT_DEVICE_TABLE: u32 = 0x00000008;
pub const EROFS_FEATURE_INCOMPAT_COMPR_HEAD2: u32 = 0x00000008;
pub const EROFS_FEATURE_INCOMPAT_ZTAILPACKING: u32 = 0x00000010;
pub const EROFS_FEATURE_INCOMPAT_FRAGMENTS: u32 = 0x00000020;
pub const EROFS_FEATURE_INCOMPAT_DEDUPE: u32 = 0x00000020;

// Compressed configuration structure size (without 2-byte length prefix)
pub const Z_EROFS_LZ4_CFGS_SIZE: usize = 14;
pub const Z_EROFS_LZMA_CFGS_SIZE: usize = 14;
pub const Z_EROFS_DEFLATE_CFGS_SIZE: usize = 6;
pub const Z_EROFS_ZSTD_CFGS_SIZE: usize = 6;

// DEFLATE default window number of bits
pub const Z_EROFS_DEFLATE_DEFAULT_WINDOWBITS: u8 = 15;

// ZSTD window log minimum value
pub const ZSTD_WINDOWLOG_ABSOLUTEMIN: u8 = 10;
