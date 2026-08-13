// EROFS 常量定义
// 基于 Linux 内核 fs/erofs/erofs_fs.h

// EROFS superblock 魔数
pub const EROFS_SUPER_MAGIC_V1: u32 = 0xE0F5E1E2;

// superblock 偏移 (1KB)
pub const EROFS_SUPER_OFFSET: u64 = 1024;

// 特性标志
pub const EROFS_FEATURE_INCOMPAT_ZERO_PADDING: u32 = 0x00000001;

// inode 布局类型
pub const EROFS_INODE_LAYOUT_COMPACT: u16 = 0;
pub const EROFS_INODE_LAYOUT_EXTENDED: u16 = 1;

// inode 格式位掩码
pub const EROFS_I_VERSION_MASK: u16 = 0x01;
pub const EROFS_I_DATALAYOUT_BIT: u16 = 1;
pub const EROFS_I_DATALAYOUT_MASK: u16 = 0x07;

// 数据布局类型
pub const EROFS_INODE_FLAT_PLAIN: u16 = 0;
pub const EROFS_INODE_FLAT_INLINE: u16 = 2;
pub const EROFS_INODE_FLAT_COMPRESSION_LEGACY: u16 = 3;
pub const EROFS_INODE_CHUNK_BASED: u16 = 4;

// 文件类型
pub const EROFS_FT_UNKNOWN: u8 = 0;
pub const EROFS_FT_REG_FILE: u8 = 1;
pub const EROFS_FT_DIR: u8 = 2;
pub const EROFS_FT_CHRDEV: u8 = 3;
pub const EROFS_FT_BLKDEV: u8 = 4;
pub const EROFS_FT_FIFO: u8 = 5;
pub const EROFS_FT_SOCK: u8 = 6;
pub const EROFS_FT_SYMLINK: u8 = 7;

// 压缩算法
pub const Z_EROFS_COMPRESSION_LZ4: u8 = 0;
pub const Z_EROFS_COMPRESSION_LZMA: u8 = 1;
pub const Z_EROFS_COMPRESSION_DEFLATE: u8 = 2;
pub const Z_EROFS_COMPRESSION_ZSTD: u8 = 3;

// LZMA 最大字典大小 (8MB)
pub const Z_EROFS_LZMA_MAX_DICT_SIZE: u32 = 8 * 1024 * 1024;

// 压缩模式的数据布局
pub const EROFS_INODE_COMPRESSED_FULL: u16 = 1;
pub const EROFS_INODE_COMPRESSED_COMPACT: u16 = 3;

// lcluster 类型
pub const Z_EROFS_LCLUSTER_TYPE_PLAIN: u16 = 0;
pub const Z_EROFS_LCLUSTER_TYPE_HEAD1: u16 = 1;
pub const Z_EROFS_LCLUSTER_TYPE_NONHEAD: u16 = 2;
pub const Z_EROFS_LCLUSTER_TYPE_HEAD2: u16 = 3;

// delta[0] 标志: 用于首个 NONHEAD cluster 记录压缩块数量
pub const Z_EROFS_LI_D0_CBLKCNT: u16 = 1 << 11;

// advise 标志
pub const Z_EROFS_ADVISE_BIG_PCLUSTER_1: u16 = 0x0002;

// ============ 构建器相关常量 ============

// inode 大小
pub const EROFS_INODE_COMPACT_SIZE: usize = 32;
pub const EROFS_INODE_EXTENDED_SIZE: usize = 64;

// superblock 大小 (128 字节, 参考 erofs_fs.h)
pub const EROFS_SUPER_BLOCK_SIZE: usize = 128;

// POSIX 文件模式常量
pub const S_IFMT: u16 = 0o170000;
pub const S_IFREG: u16 = 0o100000;
pub const S_IFDIR: u16 = 0o040000;
pub const S_IFLNK: u16 = 0o120000;
pub const S_IFCHR: u16 = 0o020000;
pub const S_IFBLK: u16 = 0o060000;
pub const S_IFIFO: u16 = 0o010000;
pub const S_IFSOCK: u16 = 0o140000;

// xattr 索引
pub const EROFS_XATTR_INDEX_USER: u8 = 1;
pub const EROFS_XATTR_INDEX_POSIX_ACL_ACCESS: u8 = 2;
pub const EROFS_XATTR_INDEX_POSIX_ACL_DEFAULT: u8 = 3;
pub const EROFS_XATTR_INDEX_TRUSTED: u8 = 4;
pub const EROFS_XATTR_INDEX_LUSTRE: u8 = 5;
pub const EROFS_XATTR_INDEX_SECURITY: u8 = 6;

// EROFS 兼容特性标志
pub const EROFS_FEATURE_COMPAT_SB_CHKSUM: u32 = 0x00000001;
pub const EROFS_FEATURE_COMPAT_MTIME: u32 = 0x00000002;
pub const EROFS_FEATURE_COMPAT_XATTR_FILTER: u32 = 0x00000004;

// EROFS 不兼容特性标志
pub const EROFS_FEATURE_INCOMPAT_COMPR_CFGS: u32 = 0x00000002;
pub const EROFS_FEATURE_INCOMPAT_BIG_PCLUSTER: u32 = 0x00000002;
pub const EROFS_FEATURE_INCOMPAT_CHUNKED_FILE: u32 = 0x00000004;
pub const EROFS_FEATURE_INCOMPAT_DEVICE_TABLE: u32 = 0x00000008;
pub const EROFS_FEATURE_INCOMPAT_COMPR_HEAD2: u32 = 0x00000008;
pub const EROFS_FEATURE_INCOMPAT_ZTAILPACKING: u32 = 0x00000010;
pub const EROFS_FEATURE_INCOMPAT_FRAGMENTS: u32 = 0x00000020;
pub const EROFS_FEATURE_INCOMPAT_DEDUPE: u32 = 0x00000020;

// 压缩配置结构体大小 (不含 2 字节长度前缀)
pub const Z_EROFS_LZ4_CFGS_SIZE: usize = 14;
pub const Z_EROFS_LZMA_CFGS_SIZE: usize = 14;
pub const Z_EROFS_DEFLATE_CFGS_SIZE: usize = 6;
pub const Z_EROFS_ZSTD_CFGS_SIZE: usize = 6;

// DEFLATE 默认窗口位数
pub const Z_EROFS_DEFLATE_DEFAULT_WINDOWBITS: u8 = 15;

// ZSTD 窗口对数最小值
pub const ZSTD_WINDOWLOG_ABSOLUTEMIN: u8 = 10;
