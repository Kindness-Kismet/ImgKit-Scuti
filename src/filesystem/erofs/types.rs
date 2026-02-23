// EROFS type definition
// Based on Linux kernel fs/erofs/erofs_fs.h

use super::consts::*;
use zerocopy::{FromZeros, Immutable, IntoBytes, KnownLayout};

// Compressed map header (8 bytes)
#[repr(C, packed)]
#[derive(FromZeros, IntoBytes, Immutable, KnownLayout, Debug, Clone, Copy)]
pub struct ZErofsMapHeader {
    pub h_reserved: u16,     // retain or fragment offset low
    pub h_idata_size: u16,   // tailpacking data size or advise low
    pub h_advise: u16,       // Suggestion flag
    pub h_algorithmtype: u8, // Algorithm type (bit 0-3: HEAD1; bit 4-7: HEAD2)
    pub h_clusterbits: u8,   // logical cluster bits - 12
}

// Compressed index (8 bytes)
#[repr(C, packed)]
#[derive(FromZeros, IntoBytes, Immutable, KnownLayout, Debug, Clone, Copy)]
pub struct ZErofsLclusterIndex {
    pub di_advise: u16,     // Types and flags
    pub di_clusterofs: u16, // Decompression location in HEAD lcluster
    pub di_u: u32,          // Physical block address or delta information
}

// EROFS superblock structure (128 bytes)
#[repr(C, packed)]
#[derive(FromZeros, IntoBytes, Immutable, KnownLayout, Debug, Clone, Copy)]
pub struct ErofsSuperBlock {
    pub magic: u32,                // Magic number 0xE0F5E1E2
    pub checksum: u32,             // CRC32 checksum
    pub feature_compat: u32,       // Compatibility features
    pub blkszbits: u8,             // Block size in digits (log2(block_size))
    pub sb_extslots: u8,           // Number of super block expansion slots
    pub root_nid: u16,             // Root directory inode number
    pub inos: u64,                 // Total number of inodes
    pub build_time: u64,           // Build time (seconds)
    pub build_time_nsec: u32,      // Build time (nanoseconds)
    pub blocks: u32,               // total number of blocks
    pub meta_blkaddr: u32,         // Metadata starting block address
    pub xattr_blkaddr: u32,        // Extended attribute starting block address
    pub uuid: [u8; 16],            // UUID
    pub volume_name: [u8; 16],     // Volume name
    pub feature_incompat: u32,     // Incompatible features
    pub union2: u16,               // union field
    pub extra_devices: u16,        // Number of additional devices
    pub devt_slotoff: u16,         // Device slot offset
    pub dirblkbits: u8,            // Directory block number of bits
    pub xattr_prefix_count: u8,    // xattr prefix number
    pub xattr_prefix_start: u32,   // xattr prefix start
    pub packed_nid: u64,           // packed inode
    pub xattr_filter_reserved: u8, // reserved
    pub reserved: [u8; 23],        // reserved fields
}

// Compact Inode (32 bytes)
#[repr(C, packed)]
#[derive(FromZeros, Debug, Clone, Copy)]
pub struct ErofsInodeCompact {
    pub i_format: u16,       // inode format and data layout
    pub i_xattr_icount: u16, // Inline xattr quantity
    pub i_mode: u16,         // file mode
    pub i_nb: ErofsInodeNb,  // nlink or blocks
    pub i_size: u32,         // file size
    pub i_reserved: [u8; 4], // reserve
    pub i_u: [u8; 4],        // union: raw_blkaddr, rdev, etc
    pub i_ino: u32,          // inode number
    pub i_uid: u16,          // User ID
    pub i_gid: u16,          // Group ID
    pub i_reserved2: u32,    // reserve
}

// i_nb union
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

// Extended Inode (64 bytes)
#[repr(C, packed)]
#[derive(FromZeros, IntoBytes, Immutable, KnownLayout, Debug, Clone, Copy)]
pub struct ErofsInodeExtended {
    pub i_format: u16,         // inode format and data layout
    pub i_xattr_icount: u16,   // Inline xattr quantity
    pub i_mode: u16,           // file mode
    pub i_reserved: u16,       // reserve
    pub i_size: u64,           // file size
    pub i_u: [u8; 4],          // union
    pub i_ino: u32,            // inode number
    pub i_uid: u32,            // User ID
    pub i_gid: u32,            // Group ID
    pub i_mtime: u64,          // modification time
    pub i_mtime_nsec: u32,     // Modification time in nanoseconds
    pub i_nlink: u32,          // Number of hard links
    pub i_reserved2: [u8; 16], // reserve
}

// Directory Entry - EROFS Official Format
// Reference: erofs-utils/include/erofs_fs.h
// struct erofs_dirent { __le64 nid; __le16 nameoff; __u8 file_type; __u8 reserved; }
#[repr(C, packed)]
#[derive(FromZeros, IntoBytes, Immutable, KnownLayout, Debug, Clone, Copy)]
pub struct ErofsDirent {
    pub nid: u64,      // node number (offset 0-7)
    pub nameoff: u16,  // File name offset (offset 8-9)
    pub file_type: u8, // File type (offset 10)
    pub reserved: u8,  // Reserved (offset 11)
}

// extended attribute entry
#[repr(C, packed)]
#[derive(FromZeros, IntoBytes, Immutable, KnownLayout, Debug, Clone, Copy)]
pub struct ErofsXattrEntry {
    pub e_name_len: u8,    // name length
    pub e_name_index: u8,  // name index
    pub e_value_size: u16, // value size
}

// Inline extended attribute header
// According to erofs_fs.h, this structure size must be 12 bytes
#[repr(C, packed)]
#[derive(FromZeros, IntoBytes, Immutable, KnownLayout, Debug, Clone, Copy)]
pub struct ErofsXattrIbodyHeader {
    pub h_name_filter: u32, // Name filter bitmap (bit=1 means does not exist)
    pub h_shared_count: u8, // Share xattr quantity
    pub h_reserved2: [u8; 7], // reserve
                            // h_shared_xattrs[0] is a flexible array member and is not in the structure
} // 12 bytes total

// Inode information structure (used at runtime)
#[derive(Debug, Clone)]
pub struct InodeInfo {
    pub nid: u64,          // inode number
    pub mode: u16,         // file mode
    pub uid: u32,          // User ID
    pub gid: u32,          // Group ID
    pub nlink: u32,        // Number of hard links
    pub size: u64,         // file size
    pub format: u16,       // format flag
    pub xattr_icount: u16, // Inline xattr quantity
    pub raw_blkaddr: u32,  // Data block address
    pub is_compact: bool,  // Is it in compact format?
}

impl ErofsSuperBlock {
    // Get block size
    pub fn block_size(&self) -> u32 {
        1u32 << self.blkszbits
    }

    // Get directory block size
    pub fn dir_block_size(&self) -> u32 {
        1u32 << self.dirblkbits
    }
}

impl ErofsInodeCompact {
    // Get data layout type
    pub fn data_layout(&self) -> u16 {
        self.i_format & 0x7
    }

    // Is it a directory
    pub fn is_dir(&self) -> bool {
        (self.i_mode & 0xF000) == 0x4000 // S_IFDIR
    }

    // Is it an ordinary file?
    pub fn is_regular(&self) -> bool {
        (self.i_mode & 0xF000) == 0x8000 // S_IFREG
    }

    // Is it a symbolic link?
    pub fn is_symlink(&self) -> bool {
        (self.i_mode & 0xF000) == 0xA000 // S_IFLNK
    }

    // Get original block address
    pub fn raw_blkaddr(&self) -> u32 {
        u32::from_le_bytes([self.i_u[0], self.i_u[1], self.i_u[2], self.i_u[3]])
    }
}

impl ErofsInodeExtended {
    // Get data layout type
    pub fn data_layout(&self) -> u16 {
        self.i_format & 0x7
    }

    // Is it a directory
    pub fn is_dir(&self) -> bool {
        (self.i_mode & 0xF000) == 0x4000
    }

    // Is it an ordinary file?
    pub fn is_regular(&self) -> bool {
        (self.i_mode & 0xF000) == 0x8000
    }

    // Is it a symbolic link?
    pub fn is_symlink(&self) -> bool {
        (self.i_mode & 0xF000) == 0xA000
    }

    // Get original block address
    pub fn raw_blkaddr(&self) -> u32 {
        u32::from_le_bytes([self.i_u[0], self.i_u[1], self.i_u[2], self.i_u[3]])
    }
}

impl ErofsXattrEntry {
    // Get name prefix
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
