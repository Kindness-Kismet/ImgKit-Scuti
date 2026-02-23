// EROFS Inode Builder
//
// Build the EROFS inode structure.

#![allow(dead_code)]

use crate::filesystem::erofs::Result;
use crate::filesystem::erofs::consts::*;

// Xattr entry
#[derive(Debug, Clone)]
pub struct XattrEntry {
    pub name_index: u8,
    pub name: Vec<u8>,
    pub value: Vec<u8>,
}

impl XattrEntry {
    // Create SELinux context xattr
    pub fn selinux(context: &str) -> Self {
        XattrEntry {
            name_index: EROFS_XATTR_INDEX_SECURITY,
            name: b"selinux".to_vec(),
            value: context.as_bytes().to_vec(),
        }
    }

    // Calculate the aligned size
    pub fn aligned_size(&self) -> usize {
        let raw_size = 4 + self.name.len() + self.value.len();
        (raw_size + 3) & !3
    }

    // serialize to bytes
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut buf = Vec::with_capacity(self.aligned_size());
        buf.push(self.name.len() as u8);
        buf.push(self.name_index);
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
    nlink: u32,
    size: u64,
    mtime: u64,
    mtime_nsec: u32,

    // Data layout
    data_layout: u16,
    is_extended: bool,

    // data location
    raw_blkaddr: u32,
    inline_data: Option<Vec<u8>>,

    // Compression related
    compress_header: Option<Vec<u8>>,
    compress_indexes: Option<Vec<u8>>,

    // Xattr
    xattrs: Vec<XattrEntry>,

    // inode number
    ino: u32,
}

impl InodeBuilder {
    pub fn new() -> Self {
        InodeBuilder {
            mode: 0,
            uid: 0,
            gid: 0,
            nlink: 1,
            size: 0,
            mtime: 0,
            mtime_nsec: 0,
            data_layout: EROFS_INODE_FLAT_PLAIN,
            is_extended: false,
            raw_blkaddr: 0,
            inline_data: None,
            compress_header: None,
            compress_indexes: None,
            xattrs: Vec::new(),
            ino: 0,
        }
    }

    // Create directory inode
    pub fn new_dir(mode: u16, uid: u32, gid: u32) -> Self {
        let mut builder = Self::new();
        builder.mode = S_IFDIR | (mode & 0o7777);
        builder.uid = uid;
        builder.gid = gid;
        builder.nlink = 2;
        builder
    }

    // Create a normal file inode
    pub fn new_file(mode: u16, uid: u32, gid: u32) -> Self {
        let mut builder = Self::new();
        builder.mode = S_IFREG | (mode & 0o7777);
        builder.uid = uid;
        builder.gid = gid;
        builder
    }

    // Create symbolic link inode
    pub fn new_symlink(uid: u32, gid: u32) -> Self {
        let mut builder = Self::new();
        builder.mode = S_IFLNK | 0o777;
        builder.uid = uid;
        builder.gid = gid;
        builder
    }

    pub fn with_mode(mut self, mode: u16) -> Self {
        self.mode = mode;
        self
    }

    pub fn with_owner(mut self, uid: u32, gid: u32) -> Self {
        self.uid = uid;
        self.gid = gid;
        self
    }

    pub fn with_nlink(mut self, nlink: u32) -> Self {
        self.nlink = nlink;
        self
    }

    pub fn with_size(mut self, size: u64) -> Self {
        self.size = size;
        self
    }

    pub fn with_mtime(mut self, mtime: u64) -> Self {
        self.mtime = mtime;
        self
    }

    pub fn with_data_layout(mut self, layout: u16) -> Self {
        self.data_layout = layout;
        self
    }

    pub fn with_raw_blkaddr(mut self, addr: u32) -> Self {
        self.raw_blkaddr = addr;
        self
    }

    pub fn with_inline_data(mut self, data: Vec<u8>) -> Self {
        self.size = data.len() as u64;
        self.inline_data = Some(data);
        self.data_layout = EROFS_INODE_FLAT_INLINE;
        self.raw_blkaddr = 0xffffffff; // Inline data markup
        self
    }

    // Set inline data without overriding size (for hybrid layouts)
    pub fn with_tail_inline_data(mut self, data: Vec<u8>) -> Self {
        self.inline_data = Some(data);
        self.data_layout = EROFS_INODE_FLAT_INLINE;
        self
    }

    pub fn with_ino(mut self, ino: u32) -> Self {
        self.ino = ino;
        self
    }

    pub fn with_extended(mut self, extended: bool) -> Self {
        self.is_extended = extended;
        self
    }

    pub fn add_xattr(&mut self, entry: XattrEntry) {
        self.xattrs.push(entry);
    }

    pub fn with_selinux_context(mut self, context: &str) -> Self {
        self.xattrs.push(XattrEntry::selinux(context));
        self
    }

    // Set compression header
    pub fn with_compress_header(mut self, header: Vec<u8>) -> Self {
        self.compress_header = Some(header);
        self
    }

    // Set up compressed index
    pub fn with_compress_indexes(mut self, indexes: Vec<u8>) -> Self {
        self.compress_indexes = Some(indexes);
        self
    }

    // Get file type
    pub fn file_type(&self) -> u8 {
        match self.mode & S_IFMT {
            S_IFREG => EROFS_FT_REG_FILE,
            S_IFDIR => EROFS_FT_DIR,
            S_IFLNK => EROFS_FT_SYMLINK,
            S_IFCHR => EROFS_FT_CHRDEV,
            S_IFBLK => EROFS_FT_BLKDEV,
            S_IFIFO => EROFS_FT_FIFO,
            S_IFSOCK => EROFS_FT_SOCK,
            _ => EROFS_FT_UNKNOWN,
        }
    }

    // Calculate xattr data size
    fn xattr_data_size(&self) -> usize {
        if self.xattrs.is_empty() {
            return 0;
        }
        // xattr ibody header (12 bytes) + entries
        let mut size = 12;
        for entry in &self.xattrs {
            size += entry.aligned_size();
        }
        size
    }

    // Calculate xattr icount
    fn xattr_icount(&self) -> u16 {
        if self.xattrs.is_empty() {
            return 0;
        }
        // icount = (xattr_data_size - 12) / 4 + 1
        let data_size = self.xattr_data_size();
        ((data_size - 12) / 4 + 1) as u16
    }

    // Calculate total inode size (including xattr, inline data, compression header and index)
    pub fn total_size(&self) -> usize {
        let base_size = if self.is_extended {
            EROFS_INODE_EXTENDED_SIZE
        } else {
            EROFS_INODE_COMPACT_SIZE
        };

        let xattr_size = self.xattr_data_size();
        let inline_size = self.inline_data.as_ref().map(|d| d.len()).unwrap_or(0);
        let compress_header_size = self.compress_header.as_ref().map(|h| h.len()).unwrap_or(0);
        let compress_indexes_size = self.compress_indexes.as_ref().map(|i| i.len()).unwrap_or(0);

        base_size + xattr_size + inline_size + compress_header_size + compress_indexes_size
    }

    // Build compact inode (32 bytes)
    fn build_compact(&self) -> Vec<u8> {
        let mut buf = vec![0u8; EROFS_INODE_COMPACT_SIZE];

        // i_format (offset 0, 2 bytes)
        // bit 0: version (0 = compact)
        // bit 1-3: data layout
        let i_format = (self.data_layout << 1) | EROFS_INODE_LAYOUT_COMPACT;
        buf[0..2].copy_from_slice(&i_format.to_le_bytes());

        // i_xattr_icount (offset 2, 2 bytes)
        buf[2..4].copy_from_slice(&self.xattr_icount().to_le_bytes());

        // i_mode (offset 4, 2 bytes)
        buf[4..6].copy_from_slice(&self.mode.to_le_bytes());

        // i_nlink (offset 6, 2 bytes)
        buf[6..8].copy_from_slice(&(self.nlink as u16).to_le_bytes());

        // i_size (offset 8, 4 bytes)
        buf[8..12].copy_from_slice(&(self.size as u32).to_le_bytes());

        // i_reserved (offset 12, 4 bytes) - actually mtime
        buf[12..16].copy_from_slice(&(self.mtime as u32).to_le_bytes());

        // i_u (offset 16, 4 bytes) - raw_blkaddr
        buf[16..20].copy_from_slice(&self.raw_blkaddr.to_le_bytes());

        // i_ino (offset 20, 4 bytes)
        buf[20..24].copy_from_slice(&self.ino.to_le_bytes());

        // i_uid (offset 24, 2 bytes)
        buf[24..26].copy_from_slice(&(self.uid as u16).to_le_bytes());

        // i_gid (offset 26, 2 bytes)
        buf[26..28].copy_from_slice(&(self.gid as u16).to_le_bytes());

        // i_reserved2 (offset 28, 4 bytes)
        buf[28..32].copy_from_slice(&0u32.to_le_bytes());

        buf
    }

    // Build extended inode (64 bytes)
    fn build_extended(&self) -> Vec<u8> {
        let mut buf = vec![0u8; EROFS_INODE_EXTENDED_SIZE];

        // i_format (offset 0, 2 bytes)
        let i_format = (self.data_layout << 1) | EROFS_INODE_LAYOUT_EXTENDED;
        buf[0..2].copy_from_slice(&i_format.to_le_bytes());

        // i_xattr_icount (offset 2, 2 bytes)
        buf[2..4].copy_from_slice(&self.xattr_icount().to_le_bytes());

        // i_mode (offset 4, 2 bytes)
        buf[4..6].copy_from_slice(&self.mode.to_le_bytes());

        // i_reserved (offset 6, 2 bytes)
        buf[6..8].copy_from_slice(&0u16.to_le_bytes());

        // i_size (offset 8, 8 bytes)
        buf[8..16].copy_from_slice(&self.size.to_le_bytes());

        // i_u (offset 16, 4 bytes) - raw_blkaddr
        buf[16..20].copy_from_slice(&self.raw_blkaddr.to_le_bytes());

        // i_ino (offset 20, 4 bytes)
        buf[20..24].copy_from_slice(&self.ino.to_le_bytes());

        // i_uid (offset 24, 4 bytes)
        buf[24..28].copy_from_slice(&self.uid.to_le_bytes());

        // i_gid (offset 28, 4 bytes)
        buf[28..32].copy_from_slice(&self.gid.to_le_bytes());

        // i_mtime (offset 32, 8 bytes)
        buf[32..40].copy_from_slice(&self.mtime.to_le_bytes());

        // i_mtime_nsec (offset 40, 4 bytes)
        buf[40..44].copy_from_slice(&self.mtime_nsec.to_le_bytes());

        // i_nlink (offset 44, 4 bytes)
        buf[44..48].copy_from_slice(&self.nlink.to_le_bytes());

        // i_reserved2 (offset 48, 16 bytes)
        buf[48..64].copy_from_slice(&[0u8; 16]);

        buf
    }

    // Build xattr data
    fn build_xattr(&self) -> Vec<u8> {
        if self.xattrs.is_empty() {
            return Vec::new();
        }

        let mut buf = Vec::new();

        // xattr ibody header (12 bytes)
        // h_name_filter (4 bytes)
        buf.extend_from_slice(&0xFFFFFFFFu32.to_le_bytes());
        // h_shared_count (1 byte)
        buf.push(0);
        // h_reserved2 (7 bytes)
        buf.extend_from_slice(&[0u8; 7]);

        // xattr entries
        for entry in &self.xattrs {
            buf.extend_from_slice(&entry.to_bytes());
        }

        buf
    }

    // Build complete inode data
    pub fn build(&self) -> Result<Vec<u8>> {
        let mut buf = if self.is_extended {
            self.build_extended()
        } else {
            self.build_compact()
        };

        // Add xattr data
        buf.extend_from_slice(&self.build_xattr());

        // Add compression header (if any)
        // Compressed metadata needs to be aligned to 8-byte boundaries
        if let Some(ref header) = self.compress_header {
            // Aligned to 8 bytes
            while buf.len() % 8 != 0 {
                buf.push(0);
            }
            buf.extend_from_slice(header);
        }

        // Add compressed index (if any)
        if let Some(ref indexes) = self.compress_indexes {
            buf.extend_from_slice(indexes);
        }

        // Add inline data
        if let Some(ref data) = self.inline_data {
            buf.extend_from_slice(data);
        }

        Ok(buf)
    }
}

impl Default for InodeBuilder {
    fn default() -> Self {
        Self::new()
    }
}
