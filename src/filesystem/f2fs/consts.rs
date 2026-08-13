// F2FS 常量定义
//
// 基于 Linux 内核 f2fs_fs.h 中的常量定义

// F2FS 魔数
pub const F2FS_MAGIC: u32 = 0xF2F52010;

// superblock 偏移 (字节)
pub const F2FS_SUPER_OFFSET: u64 = 1024;

// F2FS block 大小 (4KB)
pub const F2FS_BLKSIZE: usize = 4096;

// 最大文件名长度
pub const F2FS_NAME_LEN: usize = 255;

// 目录 slot 长度
pub const F2FS_SLOT_LEN: usize = 8;

// 每个 block 内的 NAT 条目数 (4096 / 9 = 455)
pub const NAT_ENTRY_PER_BLOCK: usize = F2FS_BLKSIZE / 9;

// 每个 block 内的 SIT 条目数 (4096 / 74 = 55)
pub const SIT_ENTRY_PER_BLOCK: usize = F2FS_BLKSIZE / 74;

// 空地址 (稀疏 block)
pub const NULL_ADDR: u32 = 0;

// 新地址 (尚未分配)
pub const NEW_ADDR: u32 = 0xFFFFFFFF;

// 压缩地址标记
pub const COMPRESS_ADDR: u32 = 0xFFFFFFFE;

// inode 内直接地址数量 (923)
// 计算方式: (4096 - 360 - 20 - 24) / 4
pub const DEF_ADDRS_PER_INODE: usize = (F2FS_BLKSIZE - 360 - 20 - 24) / 4;

// direct node 内地址数量 (1018)
// 计算方式: (4096 - 24) / 4
pub const DEF_ADDRS_PER_BLOCK: usize = (F2FS_BLKSIZE - 24) / 4;

// 默认 inline xattr 地址数量
pub const DEFAULT_INLINE_XATTR_ADDRS: usize = 50;

// 文件类型: 普通文件
pub const F2FS_FT_REG_FILE: u8 = 1;

// 文件类型: 目录
pub const F2FS_FT_DIR: u8 = 2;

// 文件类型: 符号链接
pub const F2FS_FT_SYMLINK: u8 = 7;

// 压缩算法: LZO
pub const COMPR_LZO: u8 = 0;

// 压缩算法: LZ4
pub const COMPR_LZ4: u8 = 1;

// 压缩算法: ZSTD
pub const COMPR_ZSTD: u8 = 2;

// inode 标志: inline xattr
pub const F2FS_INLINE_XATTR: u8 = 0x01;

// inode 标志: inline data
pub const F2FS_INLINE_DATA: u8 = 0x02;

// inode 标志: inline dentry
pub const F2FS_INLINE_DENTRY: u8 = 0x04;

// inode 标志: inline data 存在
pub const F2FS_DATA_EXIST: u8 = 0x08;

// inode 标志: 额外属性
pub const F2FS_EXTRA_ATTR: u8 = 0x20;

// 文件标志: 已压缩
pub const F2FS_COMPR_FL: u32 = 0x00000004;

// XATTR 索引
pub const F2FS_XATTR_INDEX_USER: u8 = 1;
pub const F2FS_XATTR_INDEX_POSIX_ACL_ACCESS: u8 = 2;
pub const F2FS_XATTR_INDEX_POSIX_ACL_DEFAULT: u8 = 3;
pub const F2FS_XATTR_INDEX_TRUSTED: u8 = 4;
pub const F2FS_XATTR_INDEX_LUSTRE: u8 = 5;
pub const F2FS_XATTR_INDEX_SECURITY: u8 = 6;
pub const F2FS_XATTR_INDEX_ADVISE: u8 = 7;
pub const F2FS_XATTR_INDEX_ENCRYPTION: u8 = 9;
pub const F2FS_XATTR_INDEX_VERITY: u8 = 11;

// XATTR 名称
pub const XATTR_SECURITY_PREFIX: &str = "security.";
pub const XATTR_SELINUX_SUFFIX: &str = "selinux";

// XATTR 条目大小
pub const F2FS_XATTR_ENTRY_SIZE: usize = 4; // 每个条目的最小头部大小

// ============ 格式化相关常量 ============

// superblock 魔数
pub const F2FS_SUPER_MAGIC: u32 = 0xF2F52010;

// 版本号
pub const F2FS_MAJOR_VERSION: u16 = 1;
pub const F2FS_MINOR_VERSION: u16 = 16;

// 默认扇区大小
pub const DEFAULT_SECTOR_SIZE: u32 = 512;
pub const DEFAULT_SECTORS_PER_BLOCK: u32 = 8; // 4096 / 512

// 每个 segment 的 block 数
pub const DEFAULT_BLOCKS_PER_SEGMENT: u32 = 512;

// 每个 section 的 segment 数
pub const DEFAULT_SEGMENTS_PER_SECTION: u32 = 1;

// 每个 zone 的 section 数
pub const DEFAULT_SECTIONS_PER_ZONE: u32 = 1;

// checkpoint pack 数量
pub const F2FS_NUMBER_OF_CHECKPOINT_PACK: u32 = 2;

// 保留 inode 号
pub const F2FS_NODE_INO: u32 = 1;
pub const F2FS_META_INO: u32 = 2;
pub const F2FS_ROOT_INO: u32 = 3;
pub const F2FS_FIRST_INO: u32 = 4;

// 当前 segment 类型数量
pub const NR_CURSEG_TYPE: usize = 6;

// 当前 segment 类型
pub const CURSEG_HOT_DATA: usize = 0;
pub const CURSEG_WARM_DATA: usize = 1;
pub const CURSEG_COLD_DATA: usize = 2;
pub const CURSEG_HOT_NODE: usize = 3;
pub const CURSEG_WARM_NODE: usize = 4;
pub const CURSEG_COLD_NODE: usize = 5;

// checkpoint 标志
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

// F2FS 特性标志
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

// NAT 条目大小
pub const NAT_ENTRY_SIZE: usize = 9;

// SIT 条目大小
pub const SIT_ENTRY_SIZE: usize = 74;
pub const SIT_VBLOCK_MAP_SIZE: usize = 64;

// SIT vblocks 字段位定义
pub const SIT_VBLOCKS_SHIFT: u16 = 10;
pub const SIT_VBLOCKS_MASK: u16 = (1 << SIT_VBLOCKS_SHIFT) - 1;

// SSA 条目大小
pub const SUMMARY_SIZE: usize = 7;
pub const SUM_FOOTER_SIZE: usize = 5;
pub const SUM_ENTRY_SIZE: usize = 7;
pub const ENTRIES_IN_SUM: usize = 512;

// summary journal 大小
// SUM_JOURNAL_SIZE = F2FS_BLKSIZE - SUM_FOOTER_SIZE - SUM_ENTRIES_SIZE
// SUM_ENTRIES_SIZE = SUMMARY_SIZE * ENTRIES_IN_SUM = 7 * 512 = 3584
// SUM_JOURNAL_SIZE = 4096 - 5 - 3584 = 507
pub const SUM_ENTRIES_SIZE: usize = SUMMARY_SIZE * ENTRIES_IN_SUM;
pub const SUM_JOURNAL_SIZE: usize = F2FS_BLKSIZE - SUM_FOOTER_SIZE - SUM_ENTRIES_SIZE;

// superblock 校验和偏移
pub const SB_CHKSUM_OFFSET: usize = 3068;

// checkpoint 校验和偏移
pub const CP_CHKSUM_OFFSET: usize = F2FS_BLKSIZE - 4;

// 最大扩展名数量
pub const F2FS_MAX_EXTENSION: usize = 64;

// 扩展名长度
pub const F2FS_EXTENSION_LEN: usize = 8;

// 最大设备数量
pub const MAX_DEVICES: usize = 8;

// 最大卷名长度
pub const MAX_VOLUME_NAME: usize = 512;

// 版本字符串长度
pub const VERSION_LEN: usize = 256;

// quota 类型数量
pub const F2FS_MAX_QUOTAS: usize = 3;

// node block footer 大小
pub const NODE_FOOTER_SIZE: usize = 24;

// inode 结构体大小
pub const F2FS_INODE_SIZE: usize = 360;

// inode 额外属性区大小
pub const F2FS_EXTRA_ISIZE: u16 = 36;

// inline data 大小
pub const MAX_INLINE_DATA_SIZE: usize = 3448;

// inline dentry 大小
pub const NR_INLINE_DENTRY: usize = 61;
pub const INLINE_DENTRY_BITMAP_SIZE: usize = 8;
pub const INLINE_RESERVED_SIZE: usize = 1;

// 目录项大小
pub const F2FS_DIR_ENTRY_SIZE: usize = 11;

// 每个 block 内的目录项数量
pub const NR_DENTRY_IN_BLOCK: usize = 214;
pub const SIZE_OF_DIR_ENTRY: usize = 11;
pub const SIZE_OF_DENTRY_BITMAP: usize = 27;
pub const SIZE_OF_RESERVED: usize = 3;

// inode 模式
pub const S_IFMT: u16 = 0o170000;
pub const S_IFSOCK: u16 = 0o140000;
pub const S_IFLNK: u16 = 0o120000;
pub const S_IFREG: u16 = 0o100000;
pub const S_IFBLK: u16 = 0o060000;
pub const S_IFDIR: u16 = 0o040000;
pub const S_IFCHR: u16 = 0o020000;
pub const S_IFIFO: u16 = 0o010000;

// 权限位
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

// 默认权限模式
pub const DEFAULT_DIR_MODE: u16 = S_IFDIR | 0o755;
pub const DEFAULT_FILE_MODE: u16 = S_IFREG | 0o644;
pub const DEFAULT_SYMLINK_MODE: u16 = S_IFLNK | 0o777;
