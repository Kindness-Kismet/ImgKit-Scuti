// EROFS 类型定义
// 基于 Linux 内核 fs/erofs/erofs_fs.h

use super::consts::*;
use zerocopy::{FromZeros, Immutable, IntoBytes, KnownLayout};

// 压缩 map header (8 字节)
#[repr(C, packed)]
#[derive(FromZeros, IntoBytes, Immutable, KnownLayout, Debug, Clone, Copy)]
pub struct ZErofsMapHeader {
    pub h_reserved: u16,     // 保留字段或 fragment 偏移低位
    pub h_idata_size: u16,   // tail packing 数据大小或 advise 低位
    pub h_advise: u16,       // 提示标志
    pub h_algorithmtype: u8, // 算法类型 (bit 0-3: HEAD1; bit 4-7: HEAD2)
    pub h_clusterbits: u8,   // logical cluster 位数 - 12
}

// 压缩索引 (8 字节)
#[repr(C, packed)]
#[derive(FromZeros, IntoBytes, Immutable, KnownLayout, Debug, Clone, Copy)]
pub struct ZErofsLclusterIndex {
    pub di_advise: u16,     // 类型与标志
    pub di_clusterofs: u16, // HEAD lcluster 内的解压偏移
    pub di_u: u32,          // 物理块地址或 delta 信息
}

// EROFS superblock 结构 (128 字节)
#[repr(C, packed)]
#[derive(FromZeros, IntoBytes, Immutable, KnownLayout, Debug, Clone, Copy)]
pub struct ErofsSuperBlock {
    pub magic: u32,                // 魔数 0xE0F5E1E2
    pub checksum: u32,             // CRC32 校验和
    pub feature_compat: u32,       // 兼容特性
    pub blkszbits: u8,             // 块大小位数 (log2(block_size))
    pub sb_extslots: u8,           // superblock 扩展槽数量
    pub root_nid: u16,             // 根目录 inode 号
    pub inos: u64,                 // inode 总数
    pub build_time: u64,           // 构建时间 (秒)
    pub build_time_nsec: u32,      // 构建时间 (纳秒)
    pub blocks: u32,               // 块总数
    pub meta_blkaddr: u32,         // 元数据起始 blkaddr
    pub xattr_blkaddr: u32,        // xattr 起始 blkaddr
    pub uuid: [u8; 16],            // UUID
    pub volume_name: [u8; 16],     // 卷名
    pub feature_incompat: u32,     // 不兼容特性
    pub union2: u16,               // union 字段
    pub extra_devices: u16,        // 额外设备数量
    pub devt_slotoff: u16,         // 设备槽偏移
    pub dirblkbits: u8,            // 目录块位数
    pub xattr_prefix_count: u8,    // xattr 前缀数量
    pub xattr_prefix_start: u32,   // xattr 前缀起始位置
    pub packed_nid: u64,           // packed inode
    pub xattr_filter_reserved: u8, // 保留
    pub reserved: [u8; 23],        // 保留字段
}

// compact inode (32 字节)
#[repr(C, packed)]
#[derive(FromZeros, Debug, Clone, Copy)]
pub struct ErofsInodeCompact {
    pub i_format: u16,       // inode 格式与数据布局
    pub i_xattr_icount: u16, // inline xattr 数量
    pub i_mode: u16,         // 文件模式
    pub i_nb: ErofsInodeNb,  // nlink 或 blocks
    pub i_size: u32,         // 文件大小
    pub i_reserved: [u8; 4], // 保留
    pub i_u: [u8; 4],        // union: raw_blkaddr, rdev 等
    pub i_ino: u32,          // inode 号
    pub i_uid: u16,          // 用户 ID
    pub i_gid: u16,          // 组 ID
    pub i_reserved2: u32,    // 保留
}

// i_nb 联合体
#[repr(C, packed)]
#[derive(FromZeros, Clone, Copy)]
pub union ErofsInodeNb {
    pub nlink: u16,
    pub blocks_hi: u16,
}

impl std::fmt::Debug for ErofsInodeNb {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let nlink = unsafe { self.nlink };
        f.debug_struct("ErofsInodeNb")
            .field("nlink", &nlink)
            .finish()
    }
}

// extended inode (64 字节)
#[repr(C, packed)]
#[derive(FromZeros, IntoBytes, Immutable, KnownLayout, Debug, Clone, Copy)]
pub struct ErofsInodeExtended {
    pub i_format: u16,         // inode 格式与数据布局
    pub i_xattr_icount: u16,   // inline xattr 数量
    pub i_mode: u16,           // 文件模式
    pub i_reserved: u16,       // 保留
    pub i_size: u64,           // 文件大小
    pub i_u: [u8; 4],          // union
    pub i_ino: u32,            // inode 号
    pub i_uid: u32,            // 用户 ID
    pub i_gid: u32,            // 组 ID
    pub i_mtime: u64,          // 修改时间
    pub i_mtime_nsec: u32,     // 修改时间纳秒部分
    pub i_nlink: u32,          // 硬链接数
    pub i_reserved2: [u8; 16], // 保留
}

// 目录项 - EROFS 官方格式
// 参考: erofs-utils/include/erofs_fs.h
// struct erofs_dirent { __le64 nid; __le16 nameoff; __u8 file_type; __u8 reserved; }
#[repr(C, packed)]
#[derive(FromZeros, IntoBytes, Immutable, KnownLayout, Debug, Clone, Copy)]
pub struct ErofsDirent {
    pub nid: u64,      // 节点号 (偏移 0-7)
    pub nameoff: u16,  // 文件名偏移 (偏移 8-9)
    pub file_type: u8, // 文件类型 (偏移 10)
    pub reserved: u8,  // 保留 (偏移 11)
}

// xattr 条目
#[repr(C, packed)]
#[derive(FromZeros, IntoBytes, Immutable, KnownLayout, Debug, Clone, Copy)]
pub struct ErofsXattrEntry {
    pub e_name_len: u8,    // 名称长度
    pub e_name_index: u8,  // 名称索引
    pub e_value_size: u16, // 值大小
}

// inline xattr 的 ibody header
// 根据 erofs_fs.h, 该结构体大小必须为 12 字节
#[repr(C, packed)]
#[derive(FromZeros, IntoBytes, Immutable, KnownLayout, Debug, Clone, Copy)]
pub struct ErofsXattrIbodyHeader {
    pub h_name_filter: u32, // 名称过滤位图 (bit=1 表示不存在)
    pub h_shared_count: u8, // 共享 xattr 数量
    pub h_reserved2: [u8; 7], // 保留
                            // h_shared_xattrs[0] 是柔性数组成员, 不在结构体内
} // 共 12 字节

// inode 信息结构 (运行时使用)
#[derive(Debug, Clone)]
pub struct InodeInfo {
    pub nid: u64,          // inode 号
    pub mode: u16,         // 文件模式
    pub uid: u32,          // 用户 ID
    pub gid: u32,          // 组 ID
    pub nlink: u32,        // 硬链接数
    pub size: u64,         // 文件大小
    pub format: u16,       // 格式标志
    pub xattr_icount: u16, // inline xattr 数量
    pub raw_blkaddr: u32,  // 数据块地址
    pub is_compact: bool,  // 是否为 compact 格式
}

impl ErofsSuperBlock {
    // 获取块大小
    pub fn block_size(&self) -> u32 {
        1u32 << self.blkszbits
    }

    // 获取目录块大小
    pub fn dir_block_size(&self) -> u32 {
        1u32 << self.dirblkbits
    }
}

impl ErofsInodeCompact {
    // 获取数据布局类型
    pub fn data_layout(&self) -> u16 {
        self.i_format & 0x7
    }

    // 是否为目录
    pub fn is_dir(&self) -> bool {
        (self.i_mode & 0xF000) == 0x4000 // S_IFDIR
    }

    // 是否为普通文件
    pub fn is_regular(&self) -> bool {
        (self.i_mode & 0xF000) == 0x8000 // S_IFREG
    }

    // 是否为符号链接
    pub fn is_symlink(&self) -> bool {
        (self.i_mode & 0xF000) == 0xA000 // S_IFLNK
    }

    // 获取原始块地址
    pub fn raw_blkaddr(&self) -> u32 {
        u32::from_le_bytes([self.i_u[0], self.i_u[1], self.i_u[2], self.i_u[3]])
    }
}

impl ErofsInodeExtended {
    // 获取数据布局类型
    pub fn data_layout(&self) -> u16 {
        self.i_format & 0x7
    }

    // 是否为目录
    pub fn is_dir(&self) -> bool {
        (self.i_mode & 0xF000) == 0x4000
    }

    // 是否为普通文件
    pub fn is_regular(&self) -> bool {
        (self.i_mode & 0xF000) == 0x8000
    }

    // 是否为符号链接
    pub fn is_symlink(&self) -> bool {
        (self.i_mode & 0xF000) == 0xA000
    }

    // 获取原始块地址
    pub fn raw_blkaddr(&self) -> u32 {
        u32::from_le_bytes([self.i_u[0], self.i_u[1], self.i_u[2], self.i_u[3]])
    }
}

impl ErofsXattrEntry {
    // 获取名称前缀
    pub fn name_prefix(&self) -> &'static str {
        match self.e_name_index {
            EROFS_XATTR_INDEX_USER => "user.",
            EROFS_XATTR_INDEX_POSIX_ACL_ACCESS => "system.posix_acl_access",
            EROFS_XATTR_INDEX_POSIX_ACL_DEFAULT => "system.posix_acl_default",
            EROFS_XATTR_INDEX_TRUSTED => "trusted.",
            EROFS_XATTR_INDEX_LUSTRE => "lustre.",
            EROFS_XATTR_INDEX_SECURITY => "security.",
            _ => "",
        }
    }
}
