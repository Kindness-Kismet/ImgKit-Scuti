// F2FS Inode Builder
use crate::filesystem::f2fs::consts::*;
//
// Responsible for building F2FS inode blocks.

use crate::filesystem::f2fs::Result;
use crate::filesystem::f2fs::types::*;

// Number of addresses in the Inode block
const ADDRS_PER_INODE: usize = DEF_ADDRS_PER_INODE;

// Number of addresses in the direct node block
const ADDRS_PER_BLOCK: usize = DEF_ADDRS_PER_BLOCK;

// Number of NIDs in the indirect node block
const NIDS_PER_BLOCK: usize = (F2FS_BLKSIZE - NODE_FOOTER_SIZE) / 4;

// Extra inode attribute size
const EXTRA_ISIZE: u16 = 36;

// Default inline xattr size (unit: 4 bytes)
const DEFAULT_INLINE_XATTR_SIZE: u16 = 50;

// Inline xattr entries
#[derive(Debug, Clone)]
pub struct InlineXattrEntry {
    pub name_index: u8,
    pub name: Vec<u8>,
    pub value: Vec<u8>,
}

impl InlineXattrEntry {
    // Create SELinux context xattr
    pub fn selinux(context: &str) -> Self {
        InlineXattrEntry {
            name_index: F2FS_XATTR_INDEX_SECURITY,
            name: b"selinux".to_vec(),
            value: context.as_bytes().to_vec(),
        }
    }

    // Calculate serialized size (aligned to 4 bytes)
    pub fn size(&self) -> usize {
        let raw_size = 4 + self.name.len() + self.value.len();
        (raw_size + 3) & !3
    }

    // serialize to bytes
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut buf = Vec::with_capacity(self.size());
        buf.push(self.name_index);
        buf.push(self.name.len() as u8);
        buf.extend_from_slice(&(self.value.len() as u16).to_le_bytes());
        buf.extend_from_slice(&self.name);
        buf.extend_from_slice(&self.value);
        // Aligned to 4 bytes
        while buf.len() % 4 != 0 {
            buf.push(0);
        }
        buf
    }
}

// Inode builder
#[derive(Debug)]
pub struct InodeBuilder {
    // Basic properties
    mode: u16,
    uid: u32,
    gid: u32,
    links: u32,
    size: u64,
    blocks: u64,

    // Timestamp
    atime: u64,
    atime_nsec: u32,
    ctime: u64,
    ctime_nsec: u32,
    mtime: u64,
    mtime_nsec: u32,
    crtime: u64,
    crtime_nsec: u32,

    // Directory depth (directories only)
    current_depth: u32,

    // Parent inode number
    pino: u32,

    // file name
    name: Vec<u8>,

    // directory level
    dir_level: u8,

    // logo
    flags: u32,
    inline_flags: u8,

    // xattr NID
    xattr_nid: u32,

    // Data block address
    addrs: Vec<u32>,

    // Indirect node NID
    nids: [u32; 5],

    // Whether to enable additional attributes
    has_extra_attr: bool,

    // Project ID
    projid: u32,

    // Inline xattr entries
    inline_xattrs: Vec<InlineXattrEntry>,

    // Symbolic link target (inline data)
    symlink_target: Option<Vec<u8>>,
}

impl InodeBuilder {
    // Create a new Inode builder
    pub fn new() -> Self {
        InodeBuilder {
            mode: 0,
            uid: 0,
            gid: 0,
            links: 1,
            size: 0,
            blocks: 0,
            atime: 0,
            atime_nsec: 0,
            ctime: 0,
            ctime_nsec: 0,
            mtime: 0,
            mtime_nsec: 0,
            crtime: 0,
            crtime_nsec: 0,
            current_depth: 0,
            pino: 0,
            name: Vec::new(),
            dir_level: 0,
            flags: 0,
            inline_flags: 0,
            xattr_nid: 0,
            addrs: Vec::new(),
            nids: [0; 5],
            has_extra_attr: true,
            projid: 0,
            inline_xattrs: Vec::new(),
            symlink_target: None,
        }
    }

    // Create directory inode
    pub fn new_dir(mode: u16, uid: u32, gid: u32) -> Self {
        let mut builder = Self::new();
        builder.mode = S_IFDIR | (mode & 0o7777);
        builder.uid = uid;
        builder.gid = gid;
        builder.links = 2; // . and ..
        builder.inline_flags = 0; // No inline flags are set, it is up to the caller
        builder.has_extra_attr = false;
        builder
    }

    // Create a normal file inode
    pub fn new_file(mode: u16, uid: u32, gid: u32) -> Self {
        let mut builder = Self::new();
        builder.mode = S_IFREG | (mode & 0o7777);
        builder.uid = uid;
        builder.gid = gid;
        builder.inline_flags = 0;
        builder.has_extra_attr = false;
        builder
    }

    // Create symbolic link inode
    pub fn new_symlink(uid: u32, gid: u32) -> Self {
        let mut builder = Self::new();
        builder.mode = S_IFLNK | 0o777;
        builder.uid = uid;
        builder.gid = gid;
        // Symbolic link uses inline data to store target path
        // F2FS_INLINE_DATA: Indicates using inline data
        // F2FS_DATA_EXIST: Indicates that the inline data area has actual data
        builder.inline_flags = F2FS_INLINE_DATA | F2FS_DATA_EXIST;
        builder.has_extra_attr = false;
        // The inode itself occupies 1 block for inline data
        builder.blocks = 1;
        builder
    }

    // Enable extra_attr feature
    pub fn with_extra_attr(mut self) -> Self {
        self.has_extra_attr = true;
        self.inline_flags |= F2FS_EXTRA_ATTR;
        self
    }

    // enable inline_xattr
    pub fn with_inline_xattr(mut self) -> Self {
        self.inline_flags |= F2FS_INLINE_XATTR;
        self
    }

    // Set symbolic link target
    pub fn with_symlink_target(mut self, target: &str) -> Self {
        let target_bytes = target.as_bytes().to_vec();
        self.size = target_bytes.len() as u64;
        self.symlink_target = Some(target_bytes);
        self.inline_flags |= F2FS_INLINE_DATA;
        self
    }

    // Setup mode
    pub fn with_mode(mut self, mode: u16) -> Self {
        self.mode = mode;
        self
    }

    // Set UID/GID
    pub fn with_owner(mut self, uid: u32, gid: u32) -> Self {
        self.uid = uid;
        self.gid = gid;
        self
    }

    // Set the number of links
    pub fn with_links(mut self, links: u32) -> Self {
        self.links = links;
        self
    }

    // Set file size
    pub fn with_size(mut self, size: u64) -> Self {
        self.size = size;
        self
    }

    // Set the number of blocks
    pub fn with_blocks(mut self, blocks: u64) -> Self {
        self.blocks = blocks;
        self
    }

    // Set timestamp
    pub fn with_timestamp(mut self, time: u64) -> Self {
        self.atime = time;
        self.ctime = time;
        self.mtime = time;
        self.crtime = time;
        self
    }

    // Set detailed timestamp
    pub fn with_times(mut self, atime: u64, ctime: u64, mtime: u64, crtime: u64) -> Self {
        self.atime = atime;
        self.ctime = ctime;
        self.mtime = mtime;
        self.crtime = crtime;
        self
    }

    // Set parent inode number
    pub fn with_pino(mut self, pino: u32) -> Self {
        self.pino = pino;
        self
    }

    // Set file name
    pub fn with_name(mut self, name: &[u8]) -> Self {
        self.name = name.to_vec();
        self
    }

    // Set directory depth
    pub fn with_depth(mut self, depth: u32) -> Self {
        self.current_depth = depth;
        self
    }

    // set flag
    pub fn with_flags(mut self, flags: u32) -> Self {
        self.flags = flags;
        self
    }

    // Set inline flag
    pub fn with_inline_flags(mut self, flags: u8) -> Self {
        self.inline_flags = flags;
        self
    }

    // Add data block address
    pub fn add_addr(&mut self, addr: u32) {
        self.addrs.push(addr);
    }

    // Set data block address list
    pub fn with_addrs(mut self, addrs: Vec<u32>) -> Self {
        self.addrs = addrs;
        self
    }

    // Set indirect node NID
    pub fn set_nid(&mut self, idx: usize, nid: u32) {
        if idx < 5 {
            self.nids[idx] = nid;
        }
    }

    // Set all indirect node NIDs
    pub fn with_nids(mut self, nids: [u32; 5]) -> Self {
        self.nids = nids;
        self
    }

    // Set xattr NID
    pub fn with_xattr_nid(mut self, nid: u32) -> Self {
        self.xattr_nid = nid;
        self
    }

    // Set project ID
    pub fn with_projid(mut self, projid: u32) -> Self {
        self.projid = projid;
        self
    }

    // Set up SELinux context
    pub fn with_selinux_context(mut self, context: &str) -> Self {
        self.inline_xattrs.push(InlineXattrEntry::selinux(context));
        self.inline_flags |= F2FS_INLINE_XATTR;
        self
    }

    // Add inline xattr
    pub fn add_inline_xattr(&mut self, entry: InlineXattrEntry) {
        self.inline_xattrs.push(entry);
        self.inline_flags |= F2FS_INLINE_XATTR;
    }

    // Get file type
    pub fn file_type(&self) -> FileType {
        FileType::from(self.mode)
    }

    // Calculate the actual number of available addresses
    fn addrs_per_inode(&self) -> usize {
        if self.has_extra_attr {
            ADDRS_PER_INODE - (EXTRA_ISIZE as usize / 4) - DEFAULT_INLINE_XATTR_SIZE as usize
        } else {
            ADDRS_PER_INODE
        }
    }

    // Build inode node block
    pub fn build(&self, nid: u32, ino: u32, cp_ver: u64) -> Result<[u8; F2FS_BLKSIZE]> {
        let mut buf = [0u8; F2FS_BLKSIZE];

        // i_mode (offset 0)
        buf[0..2].copy_from_slice(&self.mode.to_le_bytes());

        // i_advise (offset 2)
        buf[2] = 0;

        // i_inline (offset 3)
        buf[3] = self.inline_flags;

        // i_uid (offset 4)
        buf[4..8].copy_from_slice(&self.uid.to_le_bytes());

        // i_gid (offset 8)
        buf[8..12].copy_from_slice(&self.gid.to_le_bytes());

        // i_links (offset 12)
        buf[12..16].copy_from_slice(&self.links.to_le_bytes());

        // i_size (offset 16)
        buf[16..24].copy_from_slice(&self.size.to_le_bytes());

        // i_blocks (offset 24)
        buf[24..32].copy_from_slice(&self.blocks.to_le_bytes());

        // i_atime (offset 32)
        buf[32..40].copy_from_slice(&self.atime.to_le_bytes());

        // i_ctime (offset 40)
        buf[40..48].copy_from_slice(&self.ctime.to_le_bytes());

        // i_mtime (offset 48)
        buf[48..56].copy_from_slice(&self.mtime.to_le_bytes());

        // i_atime_nsec (offset 56)
        buf[56..60].copy_from_slice(&self.atime_nsec.to_le_bytes());

        // i_ctime_nsec (offset 60)
        buf[60..64].copy_from_slice(&self.ctime_nsec.to_le_bytes());

        // i_mtime_nsec (offset 64)
        buf[64..68].copy_from_slice(&self.mtime_nsec.to_le_bytes());

        // i_generation (offset 68)
        buf[68..72].copy_from_slice(&0u32.to_le_bytes());

        // i_current_depth (offset 72)
        buf[72..76].copy_from_slice(&self.current_depth.to_le_bytes());

        // i_xattr_nid (offset 76)
        buf[76..80].copy_from_slice(&self.xattr_nid.to_le_bytes());

        // i_flags (offset 80)
        buf[80..84].copy_from_slice(&self.flags.to_le_bytes());

        // i_pino (offset 84)
        buf[84..88].copy_from_slice(&self.pino.to_le_bytes());

        // i_namelen (offset 88)
        let namelen = self.name.len().min(F2FS_NAME_LEN) as u32;
        buf[88..92].copy_from_slice(&namelen.to_le_bytes());

        // i_name (offset 92, 255 bytes)
        let name_end = 92 + namelen as usize;
        buf[92..name_end].copy_from_slice(&self.name[..namelen as usize]);

        // i_dir_level (offset 347)
        buf[347] = self.dir_level;

        // i_ext (offset 348, 12 bytes) - extent cache, initialized to 0
        // remain zero

        // Extra attribute area (offset 360)
        if self.has_extra_attr {
            // i_extra_isize (offset 360)
            buf[360..362].copy_from_slice(&EXTRA_ISIZE.to_le_bytes());

            // i_inline_xattr_size (offset 362)
            buf[362..364].copy_from_slice(&DEFAULT_INLINE_XATTR_SIZE.to_le_bytes());

            // i_projid (offset 364)
            buf[364..368].copy_from_slice(&self.projid.to_le_bytes());

            // i_inode_checksum (offset 368) - calculated later
            buf[368..372].copy_from_slice(&0u32.to_le_bytes());

            // i_crtime (offset 372)
            buf[372..380].copy_from_slice(&self.crtime.to_le_bytes());

            // i_crtime_nsec (offset 380)
            buf[380..384].copy_from_slice(&self.crtime_nsec.to_le_bytes());

            // i_compr_blocks (offset 384)
            buf[384..392].copy_from_slice(&0u64.to_le_bytes());

            // i_compress_algorithm (offset 392)
            buf[392] = 0;

            // i_log_cluster_size (offset 393)
            buf[393] = 0;

            // i_compress_flag (offset 394)
            buf[394..396].copy_from_slice(&0u16.to_le_bytes());
        }

        // Data block address (offset 396 if extra attributes, 360 otherwise)
        // or inline data (symbolic link target)
        let addr_offset = if self.has_extra_attr { 396 } else { 360 };

        if let Some(ref target) = self.symlink_target {
            // Write symbolic link target as inline data
            // F2FS inline data structures:
            // - i_addr[0..extra_isize] is the extra attribute area (if there is F2FS_EXTRA_ATTR)
            // - i_addr[extra_isize] is a reserved slot (DEF_INLINE_RESERVED_SIZE = 1) and must be 0
            // - i_addr[extra_isize + 1] starts with the actual inline data
            //
            // addr_offset is already the position of i_addr[extra_isize] (extra attributes skipped)
            // None extra_attr: addr_offset=360, i_addr[0] = 0 (reserved), i_addr[1] starts with data (offset 364)
            // There is extra_attr: addr_offset=396, i_addr[9] = 0 (reserved), i_addr[10] starts with data (offset 400)
            let reserved_offset = addr_offset;
            let inline_data_offset = reserved_offset + 4; // Skip reserved slot

            // Reserve slot set to 0
            buf[reserved_offset..reserved_offset + 4].copy_from_slice(&0u32.to_le_bytes());

            // Write inline data
            let max_inline_size = F2FS_BLKSIZE - inline_data_offset - NODE_FOOTER_SIZE;
            let write_len = target.len().min(max_inline_size);
            buf[inline_data_offset..inline_data_offset + write_len]
                .copy_from_slice(&target[..write_len]);
        } else {
            // Write data block address
            let max_addrs = self.addrs_per_inode();
            for (i, &addr) in self.addrs.iter().take(max_addrs).enumerate() {
                let offset = addr_offset + i * 4;
                buf[offset..offset + 4].copy_from_slice(&addr.to_le_bytes());
            }
        }

        // Indirect node NID (after address array)
        // Position of nids[5]: 360 + DEF_ADDRS_PER_INODE * 4
        let nid_offset = 360 + ADDRS_PER_INODE * 4;
        for (i, &n) in self.nids.iter().enumerate() {
            let offset = nid_offset + i * 4;
            buf[offset..offset + 4].copy_from_slice(&n.to_le_bytes());
        }

        // Write inline xattr (before footer)
        // The inline xattr region is calculated from the end of the inode forward
        // Position: F2FS_BLKSIZE - NODE_FOOTER_SIZE - inline_xattr_size * 4
        if !self.inline_xattrs.is_empty() && self.has_extra_attr {
            let inline_xattr_bytes = DEFAULT_INLINE_XATTR_SIZE as usize * 4; // 200 bytes
            let xattr_start = F2FS_BLKSIZE - NODE_FOOTER_SIZE - inline_xattr_bytes;

            // Serialize all xattr entries
            let mut xattr_data = Vec::new();
            // xattr header: magic (4 bytes)
            xattr_data.extend_from_slice(&0xF2F52011u32.to_le_bytes());

            for entry in &self.inline_xattrs {
                xattr_data.extend_from_slice(&entry.to_bytes());
            }

            // Add termination tag (all zeros entry)
            xattr_data.extend_from_slice(&[0u8; 4]);

            // Write xattr data
            let write_len = xattr_data.len().min(inline_xattr_bytes);
            buf[xattr_start..xattr_start + write_len].copy_from_slice(&xattr_data[..write_len]);
        }

        // Node footer (last 24 bytes)
        let footer = NodeFooter {
            nid,
            ino,
            flag: 0,
            cp_ver,
            next_blkaddr: 0,
        };
        let footer_bytes = footer.to_bytes();
        buf[F2FS_BLKSIZE - NODE_FOOTER_SIZE..].copy_from_slice(&footer_bytes);

        // Calculate and write inode checksum (if extra_attr is enabled)
        if self.has_extra_attr {
            let checksum = calculate_inode_checksum(ino, &buf);
            buf[368..372].copy_from_slice(&checksum.to_le_bytes());
        }

        Ok(buf)
    }
}

// Calculate inode checksum
// F2FS uses crc32(ino, inode_data) method to calculate
fn calculate_inode_checksum(ino: u32, inode_data: &[u8]) -> u32 {
    let mut crc = F2FS_MAGIC;

    // First calculate the CRC of ino
    for &byte in &ino.to_le_bytes() {
        crc ^= byte as u32;
        for _ in 0..8 {
            if crc & 1 != 0 {
                crc = (crc >> 1) ^ 0xEDB88320;
            } else {
                crc >>= 1;
            }
        }
    }

    // Then calculate the CRC of the inode data (skipping the checksum field)
    for (i, &byte) in inode_data.iter().enumerate() {
        // Skip checksum field (offset 368-371)
        if (368..372).contains(&i) {
            continue;
        }
        crc ^= byte as u32;
        for _ in 0..8 {
            if crc & 1 != 0 {
                crc = (crc >> 1) ^ 0xEDB88320;
            } else {
                crc >>= 1;
            }
        }
    }

    crc
}

impl Default for InodeBuilder {
    fn default() -> Self {
        Self::new()
    }
}

// Direct node block builder
#[derive(Debug)]
pub struct DirectNodeBuilder {
    addrs: Vec<u32>,
}

impl DirectNodeBuilder {
    pub fn new() -> Self {
        DirectNodeBuilder { addrs: Vec::new() }
    }

    pub fn add_addr(&mut self, addr: u32) {
        if self.addrs.len() < ADDRS_PER_BLOCK {
            self.addrs.push(addr);
        }
    }

    pub fn with_addrs(mut self, addrs: Vec<u32>) -> Self {
        self.addrs = addrs;
        self
    }

    pub fn build(&self, nid: u32, ino: u32, cp_ver: u64) -> [u8; F2FS_BLKSIZE] {
        let mut buf = [0u8; F2FS_BLKSIZE];

        // write address
        for (i, &addr) in self.addrs.iter().take(ADDRS_PER_BLOCK).enumerate() {
            let offset = i * 4;
            buf[offset..offset + 4].copy_from_slice(&addr.to_le_bytes());
        }

        // node footer
        let footer = NodeFooter {
            nid,
            ino,
            flag: 0,
            cp_ver,
            next_blkaddr: 0,
        };
        let footer_bytes = footer.to_bytes();
        buf[F2FS_BLKSIZE - NODE_FOOTER_SIZE..].copy_from_slice(&footer_bytes);

        buf
    }
}

impl Default for DirectNodeBuilder {
    fn default() -> Self {
        Self::new()
    }
}

// Indirect node block builder
#[derive(Debug)]
pub struct IndirectNodeBuilder {
    nids: Vec<u32>,
}

impl IndirectNodeBuilder {
    pub fn new() -> Self {
        IndirectNodeBuilder { nids: Vec::new() }
    }

    pub fn add_nid(&mut self, nid: u32) {
        if self.nids.len() < NIDS_PER_BLOCK {
            self.nids.push(nid);
        }
    }

    pub fn len(&self) -> usize {
        self.nids.len()
    }

    pub fn is_empty(&self) -> bool {
        self.nids.is_empty()
    }

    pub fn build(&self, nid: u32, ino: u32, cp_ver: u64) -> [u8; F2FS_BLKSIZE] {
        let mut buf = [0u8; F2FS_BLKSIZE];

        // Write NID
        for (i, &n) in self.nids.iter().take(NIDS_PER_BLOCK).enumerate() {
            let offset = i * 4;
            buf[offset..offset + 4].copy_from_slice(&n.to_le_bytes());
        }

        // node footer
        let footer = NodeFooter {
            nid,
            ino,
            flag: 0,
            cp_ver,
            next_blkaddr: 0,
        };
        let footer_bytes = footer.to_bytes();
        buf[F2FS_BLKSIZE - NODE_FOOTER_SIZE..].copy_from_slice(&footer_bytes);

        buf
    }
}

impl Default for IndirectNodeBuilder {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_inode_builder_new_dir() {
        let builder = InodeBuilder::new_dir(0o755, 1000, 1000);
        assert_eq!(builder.mode, S_IFDIR | 0o755);
        assert_eq!(builder.uid, 1000);
        assert_eq!(builder.gid, 1000);
        assert_eq!(builder.links, 2);
    }

    #[test]
    fn test_inode_builder_new_file() {
        let builder = InodeBuilder::new_file(0o644, 1000, 1000);
        assert_eq!(builder.mode, S_IFREG | 0o644);
        assert_eq!(builder.links, 1);
    }

    #[test]
    fn test_inode_build() {
        let builder = InodeBuilder::new_dir(0o755, 0, 0)
            .with_timestamp(1234567890)
            .with_pino(3)
            .with_name(b"test");

        let data = builder.build(4, 4, 1).unwrap();
        assert_eq!(data.len(), F2FS_BLKSIZE);

        // Authentication mode
        let mode = u16::from_le_bytes([data[0], data[1]]);
        assert_eq!(mode, S_IFDIR | 0o755);

        // Validate footer
        let footer_offset = F2FS_BLKSIZE - NODE_FOOTER_SIZE;
        let nid = u32::from_le_bytes([
            data[footer_offset],
            data[footer_offset + 1],
            data[footer_offset + 2],
            data[footer_offset + 3],
        ]);
        assert_eq!(nid, 4);
    }

    #[test]
    fn test_direct_node_builder() {
        let mut builder = DirectNodeBuilder::new();
        builder.add_addr(100);
        builder.add_addr(101);

        let data = builder.build(5, 4, 1);
        assert_eq!(data.len(), F2FS_BLKSIZE);

        // Verify first address
        let addr = u32::from_le_bytes([data[0], data[1], data[2], data[3]]);
        assert_eq!(addr, 100);
    }
}
