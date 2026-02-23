// F2FS type definition

use super::consts::*;
use crate::filesystem::f2fs::{F2fsError, Result};
use byteorder::{LittleEndian, ReadBytesExt};
use std::io::Cursor;
use std::path::PathBuf;

// Type packaging
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Nid(pub u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Block(pub u32);

impl From<u32> for Nid {
    fn from(v: u32) -> Self {
        Nid(v)
    }
}

impl From<Nid> for u32 {
    fn from(nid: Nid) -> Self {
        nid.0
    }
}

impl From<u32> for Block {
    fn from(v: u32) -> Self {
        Block(v)
    }
}

impl From<Block> for u32 {
    fn from(blk: Block) -> Self {
        blk.0
    }
}

// Block address enumeration
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockAddr {
    Null,
    New,
    Compress,
    Valid(Block),
}

impl From<u32> for BlockAddr {
    fn from(addr: u32) -> Self {
        match addr {
            NULL_ADDR => BlockAddr::Null,
            NEW_ADDR => BlockAddr::New,
            COMPRESS_ADDR => BlockAddr::Compress,
            _ => BlockAddr::Valid(Block(addr)),
        }
    }
}

// super block
#[derive(Debug)]
pub struct Superblock {
    pub magic: u32,
    pub block_count: u64,
    pub segment_count: u32,
    pub segment0_blkaddr: u32,
    pub cp_blkaddr: u32,
    pub sit_blkaddr: u32,
    pub nat_blkaddr: u32,
    pub ssa_blkaddr: u32,
    pub main_blkaddr: u32,
}

impl Superblock {
    pub fn from_bytes(data: &[u8]) -> Result<Self> {
        let mut cursor = Cursor::new(data);

        // offset 0
        let magic = cursor.read_u32::<LittleEndian>()?;
        if magic != F2FS_MAGIC {
            return Err(F2fsError::InvalidMagic {
                expected: F2FS_MAGIC,
                got: magic,
            });
        }

        // offset 4-7: major_ver, minor_ver
        let _major_ver = cursor.read_u16::<LittleEndian>()?;
        let _minor_ver = cursor.read_u16::<LittleEndian>()?;

        // offset 8-35: log_sectorsize through checksum_offset
        cursor.set_position(36);
        let block_count = cursor.read_u64::<LittleEndian>()?;

        // offset 44-47: section_count
        cursor.set_position(44);
        let _section_count = cursor.read_u32::<LittleEndian>()?;

        // offset 48
        let segment_count = cursor.read_u32::<LittleEndian>()?;

        // Skip to block addresses (offset 72+)
        cursor.set_position(72);
        let segment0_blkaddr = cursor.read_u32::<LittleEndian>()?;
        let cp_blkaddr = cursor.read_u32::<LittleEndian>()?;
        let sit_blkaddr = cursor.read_u32::<LittleEndian>()?;
        let nat_blkaddr = cursor.read_u32::<LittleEndian>()?;
        let ssa_blkaddr = cursor.read_u32::<LittleEndian>()?;
        let main_blkaddr = cursor.read_u32::<LittleEndian>()?;

        Ok(Superblock {
            magic,
            block_count,
            segment_count,
            segment0_blkaddr,
            cp_blkaddr,
            sit_blkaddr,
            nat_blkaddr,
            ssa_blkaddr,
            main_blkaddr,
        })
    }
}

// Inode
#[derive(Debug, Clone)]
pub struct Inode {
    pub mode: u16,
    pub uid: u32,
    pub gid: u32,
    pub size: u64,
    pub blocks: u64,
    pub inline: u8,
    pub extra_isize: u16,
    pub flags: u32,
    pub xattr_nid: u32,
}

impl Inode {
    pub fn from_bytes(data: &[u8]) -> Result<Self> {
        let mut cursor = Cursor::new(data);

        // offset 0: mode
        let mode = cursor.read_u16::<LittleEndian>()?;

        // offset 3: inline flags
        cursor.set_position(3);
        let inline = cursor.read_u8()?;

        // offset 4: uid
        let uid = cursor.read_u32::<LittleEndian>()?;

        // offset 8: gid
        let gid = cursor.read_u32::<LittleEndian>()?;

        // offset 16: size
        cursor.set_position(16);
        let size = cursor.read_u64::<LittleEndian>()?;

        // offset 24: blocks
        let blocks = cursor.read_u64::<LittleEndian>()?;

        // offset 76: xattr_nid
        cursor.set_position(76);
        let xattr_nid = cursor.read_u32::<LittleEndian>()?;

        // offset 116: flags
        cursor.set_position(116);
        let flags = cursor.read_u32::<LittleEndian>()?;

        // offset 360: extra_isize (in extra attr area)
        cursor.set_position(360);
        let extra_isize = cursor.read_u16::<LittleEndian>()?;

        Ok(Inode {
            mode,
            uid,
            gid,
            size,
            blocks,
            inline,
            extra_isize,
            flags,
            xattr_nid,
        })
    }

    pub fn is_dir(&self) -> bool {
        (self.mode >> 12) == 4
    }

    pub fn is_reg(&self) -> bool {
        (self.mode >> 12) == 8
    }

    pub fn is_symlink(&self) -> bool {
        (self.mode >> 12) == 10
    }
}

// XATTR entry header
#[derive(Debug, Clone)]
pub struct XattrEntry {
    pub name_index: u8,
    pub name_len: u8,
    pub value_size: u16,
    pub name: Vec<u8>,
    pub value: Vec<u8>,
}

impl XattrEntry {
    pub fn from_bytes(data: &[u8]) -> anyhow::Result<(Self, usize)> {
        if data.len() < 4 {
            return Err(anyhow::anyhow!("xattr entry 数据太短"));
        }

        let name_index = data[0];
        let name_len = data[1];
        let value_size = u16::from_le_bytes([data[2], data[3]]);

        let name_start = 4;
        let name_end = name_start + name_len as usize;

        if data.len() < name_end {
            return Err(anyhow::anyhow!("xattr name 数据不完整"));
        }

        let name = data[name_start..name_end].to_vec();

        // F2FS xattr: value immediately follows name, not aligned
        let value_start = name_end;
        let value_end = value_start + value_size as usize;

        if data.len() < value_end {
            return Err(anyhow::anyhow!("xattr value 数据不完整"));
        }

        let value = data[value_start..value_end].to_vec();

        // The entire entry is aligned to a 4-byte boundary
        let total_size = (value_end + 3) & !3;

        Ok((
            XattrEntry {
                name_index,
                name_len,
                value_size,
                name,
                value,
            },
            total_size,
        ))
    }

    // Get the full xattr name (including prefix)
    pub fn full_name(&self) -> String {
        let prefix = match self.name_index {
            crate::filesystem::f2fs::F2FS_XATTR_INDEX_USER => "user.",
            crate::filesystem::f2fs::F2FS_XATTR_INDEX_POSIX_ACL_ACCESS => "system.posix_acl_access",
            crate::filesystem::f2fs::F2FS_XATTR_INDEX_POSIX_ACL_DEFAULT => {
                "system.posix_acl_default"
            }
            crate::filesystem::f2fs::F2FS_XATTR_INDEX_TRUSTED => "trusted.",
            crate::filesystem::f2fs::F2FS_XATTR_INDEX_SECURITY => "security.",
            _ => "",
        };

        if self.name_index == crate::filesystem::f2fs::F2FS_XATTR_INDEX_POSIX_ACL_ACCESS
            || self.name_index == crate::filesystem::f2fs::F2FS_XATTR_INDEX_POSIX_ACL_DEFAULT
        {
            prefix.to_string()
        } else {
            format!("{}{}", prefix, String::from_utf8_lossy(&self.name))
        }
    }
}

// NAT entry
#[derive(Debug, Clone, Default)]
pub struct NatEntry {
    pub version: u8,
    pub ino: u32,
    pub block_addr: Block,
}

impl NatEntry {
    pub fn from_bytes(data: &[u8]) -> Result<Self> {
        // NAT entry structure (9 bytes):
        // offset 0: version (1 byte)
        // offset 1-4: ino (4 bytes)
        // offset 5-8: block_addr (4 bytes)
        let mut cursor = Cursor::new(data);
        let version = cursor.read_u8()?;
        let ino = cursor.read_u32::<LittleEndian>()?;
        let block_addr = Block(cursor.read_u32::<LittleEndian>()?);

        Ok(NatEntry {
            version,
            ino,
            block_addr,
        })
    }

    // serialize to bytes
    pub fn to_bytes(&self) -> [u8; NAT_ENTRY_SIZE] {
        let mut buf = [0u8; NAT_ENTRY_SIZE];
        buf[0] = self.version;
        buf[1..5].copy_from_slice(&self.ino.to_le_bytes());
        buf[5..9].copy_from_slice(&self.block_addr.0.to_le_bytes());
        buf
    }
}

// ============ Builder related types ============

// Segment type
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(usize)]
pub enum SegType {
    HotData = CURSEG_HOT_DATA,
    WarmData = CURSEG_WARM_DATA,
    ColdData = CURSEG_COLD_DATA,
    HotNode = CURSEG_HOT_NODE,
    WarmNode = CURSEG_WARM_NODE,
    ColdNode = CURSEG_COLD_NODE,
}

impl SegType {
    pub fn is_node(&self) -> bool {
        matches!(
            self,
            SegType::HotNode | SegType::WarmNode | SegType::ColdNode
        )
    }

    pub fn is_data(&self) -> bool {
        !self.is_node()
    }
}

// F2FS feature flags
#[derive(Debug, Clone, Default)]
pub struct F2fsFeatures {
    pub encrypt: bool,
    pub blkzoned: bool,
    pub extra_attr: bool,
    pub project_quota: bool,
    pub inode_chksum: bool,
    pub flexible_inline_xattr: bool,
    pub quota_ino: bool,
    pub inode_crtime: bool,
    pub lost_found: bool,
    pub verity: bool,
    pub sb_chksum: bool,
    pub casefold: bool,
    pub compression: bool,
    pub readonly: bool,
}

impl F2fsFeatures {
    // Convert to u32 flag
    pub fn to_bits(&self) -> u32 {
        let mut bits = 0u32;
        if self.encrypt {
            bits |= F2FS_FEATURE_ENCRYPT;
        }
        if self.blkzoned {
            bits |= F2FS_FEATURE_BLKZONED;
        }
        if self.extra_attr {
            bits |= F2FS_FEATURE_EXTRA_ATTR;
        }
        if self.project_quota {
            bits |= F2FS_FEATURE_PRJQUOTA;
        }
        if self.inode_chksum {
            bits |= F2FS_FEATURE_INODE_CHKSUM;
        }
        if self.flexible_inline_xattr {
            bits |= F2FS_FEATURE_FLEXIBLE_INLINE_XATTR;
        }
        if self.quota_ino {
            bits |= F2FS_FEATURE_QUOTA_INO;
        }
        if self.inode_crtime {
            bits |= F2FS_FEATURE_INODE_CRTIME;
        }
        if self.lost_found {
            bits |= F2FS_FEATURE_LOST_FOUND;
        }
        if self.verity {
            bits |= F2FS_FEATURE_VERITY;
        }
        if self.sb_chksum {
            bits |= F2FS_FEATURE_SB_CHKSUM;
        }
        if self.casefold {
            bits |= F2FS_FEATURE_CASEFOLD;
        }
        if self.compression {
            bits |= F2FS_FEATURE_COMPRESSION;
        }
        if self.readonly {
            bits |= F2FS_FEATURE_RO;
        }
        bits
    }

    // Parsed from u32 flag bit
    pub fn from_bits(bits: u32) -> Self {
        F2fsFeatures {
            encrypt: bits & F2FS_FEATURE_ENCRYPT != 0,
            blkzoned: bits & F2FS_FEATURE_BLKZONED != 0,
            extra_attr: bits & F2FS_FEATURE_EXTRA_ATTR != 0,
            project_quota: bits & F2FS_FEATURE_PRJQUOTA != 0,
            inode_chksum: bits & F2FS_FEATURE_INODE_CHKSUM != 0,
            flexible_inline_xattr: bits & F2FS_FEATURE_FLEXIBLE_INLINE_XATTR != 0,
            quota_ino: bits & F2FS_FEATURE_QUOTA_INO != 0,
            inode_crtime: bits & F2FS_FEATURE_INODE_CRTIME != 0,
            lost_found: bits & F2FS_FEATURE_LOST_FOUND != 0,
            verity: bits & F2FS_FEATURE_VERITY != 0,
            sb_chksum: bits & F2FS_FEATURE_SB_CHKSUM != 0,
            casefold: bits & F2FS_FEATURE_CASEFOLD != 0,
            compression: bits & F2FS_FEATURE_COMPRESSION != 0,
            readonly: bits & F2FS_FEATURE_RO != 0,
        }
    }

    // Android default features
    pub fn android_default() -> Self {
        F2fsFeatures {
            encrypt: true,
            extra_attr: true,
            project_quota: true,
            verity: true,
            quota_ino: true,
            inode_crtime: true,
            sb_chksum: true,
            inode_chksum: true,
            ..Default::default()
        }
    }

    // Android RO features
    pub fn android_ro() -> Self {
        F2fsFeatures {
            readonly: true,
            extra_attr: true,
            sb_chksum: true,
            inode_chksum: true,
            ..Default::default()
        }
    }
}

// Compression configuration
#[derive(Debug, Clone)]
pub struct CompressionConfig {
    pub algorithm: CompressionAlgorithm,
    pub log_cluster_size: u8,
    pub min_blocks: u32,
}

impl Default for CompressionConfig {
    fn default() -> Self {
        CompressionConfig {
            algorithm: CompressionAlgorithm::Lz4,
            log_cluster_size: 2, // 4 blocks per cluster
            min_blocks: 1,
        }
    }
}

// Compression algorithm
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CompressionAlgorithm {
    Lzo = 0,
    #[default]
    Lz4 = 1,
    Zstd = 2,
}

// Builder configuration
#[derive(Debug, Clone)]
pub struct F2fsBuilderConfig {
    pub source_dir: PathBuf,
    pub output_path: PathBuf,
    pub image_size: u64,
    pub mount_point: String,
    pub file_contexts: Option<PathBuf>,
    pub fs_config: Option<PathBuf>,
    pub sparse_mode: bool,
    pub features: F2fsFeatures,
    pub compression: Option<CompressionConfig>,
    pub volume_label: String,
    pub root_uid: u32,
    pub root_gid: u32,
    pub timestamp: Option<u64>,
}

impl Default for F2fsBuilderConfig {
    fn default() -> Self {
        F2fsBuilderConfig {
            source_dir: PathBuf::new(),
            output_path: PathBuf::new(),
            image_size: 0,
            mount_point: String::from("/"),
            file_contexts: None,
            fs_config: None,
            sparse_mode: false,
            features: F2fsFeatures::default(),
            compression: None,
            volume_label: String::new(),
            root_uid: 0,
            root_gid: 0,
            timestamp: None,
        }
    }
}

// SIT entry (Segment Information Table)
#[derive(Debug, Clone)]
pub struct SitEntry {
    pub vblocks: u16, // [15:10] seg_type, [9:0] valid_blocks
    pub valid_map: [u8; SIT_VBLOCK_MAP_SIZE],
    pub mtime: u64,
}

impl Default for SitEntry {
    fn default() -> Self {
        SitEntry {
            vblocks: 0,
            valid_map: [0u8; SIT_VBLOCK_MAP_SIZE],
            mtime: 0,
        }
    }
}

impl SitEntry {
    // Get the number of valid blocks
    pub fn valid_blocks(&self) -> u16 {
        self.vblocks & SIT_VBLOCKS_MASK
    }

    // Get segment type
    pub fn seg_type(&self) -> u16 {
        (self.vblocks & !SIT_VBLOCKS_MASK) >> SIT_VBLOCKS_SHIFT
    }

    // Set the number of valid blocks and segment type
    pub fn set_vblocks(&mut self, valid_blocks: u16, seg_type: u16) {
        self.vblocks = (seg_type << SIT_VBLOCKS_SHIFT) | (valid_blocks & SIT_VBLOCKS_MASK);
    }

    // Mark block as used
    // F2FS uses big-endian bit order: bit 7 = block 0, bit 6 = block 1, ..., bit 0 = block 7
    pub fn mark_block_valid(&mut self, offset: usize) {
        if offset < DEFAULT_BLOCKS_PER_SEGMENT as usize {
            let byte_idx = offset / 8;
            let bit_idx = 7 - (offset % 8); // big endian
            self.valid_map[byte_idx] |= 1 << bit_idx;
        }
    }

    // serialize to bytes
    pub fn to_bytes(&self) -> [u8; SIT_ENTRY_SIZE] {
        let mut buf = [0u8; SIT_ENTRY_SIZE];
        buf[0..2].copy_from_slice(&self.vblocks.to_le_bytes());
        buf[2..66].copy_from_slice(&self.valid_map);
        buf[66..74].copy_from_slice(&self.mtime.to_le_bytes());
        buf
    }

    // parse from bytes
    pub fn from_bytes(data: &[u8]) -> Result<Self> {
        if data.len() < SIT_ENTRY_SIZE {
            return Err(F2fsError::InvalidData("SIT entry 数据太短".into()));
        }
        let vblocks = u16::from_le_bytes([data[0], data[1]]);
        let mut valid_map = [0u8; SIT_VBLOCK_MAP_SIZE];
        valid_map.copy_from_slice(&data[2..66]);
        let mtime = u64::from_le_bytes([
            data[66], data[67], data[68], data[69], data[70], data[71], data[72], data[73],
        ]);
        Ok(SitEntry {
            vblocks,
            valid_map,
            mtime,
        })
    }
}

// Summary entry
#[derive(Debug, Clone, Copy, Default)]
pub struct Summary {
    pub nid: u32,
    pub version: u8,
    pub ofs_in_node: u16,
}

impl Summary {
    pub fn to_bytes(&self) -> [u8; SUMMARY_SIZE] {
        let mut buf = [0u8; SUMMARY_SIZE];
        buf[0..4].copy_from_slice(&self.nid.to_le_bytes());
        buf[4] = self.version;
        buf[5..7].copy_from_slice(&self.ofs_in_node.to_le_bytes());
        buf
    }

    pub fn from_bytes(data: &[u8]) -> Result<Self> {
        if data.len() < SUMMARY_SIZE {
            return Err(F2fsError::InvalidData("Summary 数据太短".into()));
        }
        Ok(Summary {
            nid: u32::from_le_bytes([data[0], data[1], data[2], data[3]]),
            version: data[4],
            ofs_in_node: u16::from_le_bytes([data[5], data[6]]),
        })
    }
}

// node footer
#[derive(Debug, Clone, Copy, Default)]
pub struct NodeFooter {
    pub nid: u32,
    pub ino: u32,
    pub flag: u32,
    pub cp_ver: u64,
    pub next_blkaddr: u32,
}

impl NodeFooter {
    pub fn to_bytes(&self) -> [u8; NODE_FOOTER_SIZE] {
        let mut buf = [0u8; NODE_FOOTER_SIZE];
        buf[0..4].copy_from_slice(&self.nid.to_le_bytes());
        buf[4..8].copy_from_slice(&self.ino.to_le_bytes());
        buf[8..12].copy_from_slice(&self.flag.to_le_bytes());
        buf[12..20].copy_from_slice(&self.cp_ver.to_le_bytes());
        buf[20..24].copy_from_slice(&self.next_blkaddr.to_le_bytes());
        buf
    }

    pub fn from_bytes(data: &[u8]) -> Result<Self> {
        if data.len() < NODE_FOOTER_SIZE {
            return Err(F2fsError::InvalidData("NodeFooter 数据太短".into()));
        }
        Ok(NodeFooter {
            nid: u32::from_le_bytes([data[0], data[1], data[2], data[3]]),
            ino: u32::from_le_bytes([data[4], data[5], data[6], data[7]]),
            flag: u32::from_le_bytes([data[8], data[9], data[10], data[11]]),
            cp_ver: u64::from_le_bytes([
                data[12], data[13], data[14], data[15], data[16], data[17], data[18], data[19],
            ]),
            next_blkaddr: u32::from_le_bytes([data[20], data[21], data[22], data[23]]),
        })
    }
}

// directory entry
#[derive(Debug, Clone)]
pub struct DirEntryRaw {
    pub hash_code: u32,
    pub ino: u32,
    pub name_len: u16,
    pub file_type: u8,
}

impl DirEntryRaw {
    pub fn to_bytes(&self) -> [u8; F2FS_DIR_ENTRY_SIZE] {
        let mut buf = [0u8; F2FS_DIR_ENTRY_SIZE];
        buf[0..4].copy_from_slice(&self.hash_code.to_le_bytes());
        buf[4..8].copy_from_slice(&self.ino.to_le_bytes());
        buf[8..10].copy_from_slice(&self.name_len.to_le_bytes());
        buf[10] = self.file_type;
        buf
    }
}

// File type
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileType {
    Unknown = 0,
    RegFile = 1,
    Dir = 2,
    Chrdev = 3,
    Blkdev = 4,
    Fifo = 5,
    Sock = 6,
    Symlink = 7,
}

impl From<u8> for FileType {
    fn from(v: u8) -> Self {
        match v {
            1 => FileType::RegFile,
            2 => FileType::Dir,
            3 => FileType::Chrdev,
            4 => FileType::Blkdev,
            5 => FileType::Fifo,
            6 => FileType::Sock,
            7 => FileType::Symlink,
            _ => FileType::Unknown,
        }
    }
}

impl From<u16> for FileType {
    // Convert from mode
    fn from(mode: u16) -> Self {
        match mode & S_IFMT {
            S_IFREG => FileType::RegFile,
            S_IFDIR => FileType::Dir,
            S_IFCHR => FileType::Chrdev,
            S_IFBLK => FileType::Blkdev,
            S_IFIFO => FileType::Fifo,
            S_IFSOCK => FileType::Sock,
            S_IFLNK => FileType::Symlink,
            _ => FileType::Unknown,
        }
    }
}

// Extended information
#[derive(Debug, Clone, Default)]
pub struct ExtraIsize {
    pub extra_isize: u16,
    pub inline_xattr_size: u16,
    pub projid: u32,
    pub inode_checksum: u32,
    pub crtime: u64,
    pub crtime_nsec: u32,
    pub compr_blocks: u64,
    pub compress_algorithm: u8,
    pub log_cluster_size: u8,
    pub compress_flag: u16,
}

impl ExtraIsize {
    pub fn size() -> usize {
        F2FS_EXTRA_ISIZE as usize
    }
}
