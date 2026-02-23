// F2FS Directory Block Builder
use crate::filesystem::f2fs::consts::*;
//
// Responsible for building F2FS directory data blocks.

use crate::filesystem::f2fs::Result;
use crate::filesystem::f2fs::types::*;

// Number of entries in directory block
const NR_DENTRY_IN_BLOCK_CONST: usize = 214;

// Directory bitmap size
const DENTRY_BITMAP_SIZE: usize = 27;

// reserved area size
const DENTRY_RESERVED_SIZE: usize = 3;

// directory entry
#[derive(Debug, Clone)]
pub struct DentryInfo {
    pub name: Vec<u8>,
    pub ino: u32,
    pub file_type: FileType,
}

impl DentryInfo {
    pub fn new(name: &[u8], ino: u32, file_type: FileType) -> Self {
        DentryInfo {
            name: name.to_vec(),
            ino,
            file_type,
        }
    }

    // Calculate the number of slots required
    pub fn slots_needed(&self) -> usize {
        self.name.len().div_ceil(F2FS_SLOT_LEN)
    }
}

// Directory block builder
#[derive(Debug)]
pub struct DentryBlockBuilder {
    entries: Vec<DentryInfo>,
    // Number of slots currently in use
    used_slots: usize,
}

impl DentryBlockBuilder {
    pub fn new() -> Self {
        DentryBlockBuilder {
            entries: Vec::new(),
            used_slots: 0,
        }
    }

    // Check if entry can be added
    pub fn can_add(&self, entry: &DentryInfo) -> bool {
        let slots = entry.slots_needed();
        self.used_slots + slots <= NR_DENTRY_IN_BLOCK_CONST
    }

    // Add directory entry
    pub fn add_entry(&mut self, entry: DentryInfo) -> bool {
        if !self.can_add(&entry) {
            return false;
        }

        let slots = entry.slots_needed();
        self.used_slots += slots;
        self.entries.push(entry);
        true
    }

    // Get the number of slots used
    pub fn used_slots(&self) -> usize {
        self.used_slots
    }

    // Check if it is empty
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    // Build directory block
    pub fn build(&self) -> Result<[u8; F2FS_BLKSIZE]> {
        let mut buf = [0u8; F2FS_BLKSIZE];

        // Directory block layout:
        // [0..27]: dentry bitmap (27 bytes)
        // [27..30]: reserved (3 bytes)
        // [30..30+214*11]: dentry array (214 * 11 = 2354 bytes)
        // [2384..4096]: filename area (1712 bytes)

        let bitmap_offset = 0;
        let dentry_offset = DENTRY_BITMAP_SIZE + DENTRY_RESERVED_SIZE;
        let filename_offset = dentry_offset + NR_DENTRY_IN_BLOCK_CONST * F2FS_DIR_ENTRY_SIZE;

        let mut slot_idx = 0;
        let mut name_offset = 0;

        for entry in &self.entries {
            let slots = entry.slots_needed();
            let hash = dentry_hash(&entry.name);

            // Set bitmap
            for i in 0..slots {
                let bit_idx = slot_idx + i;
                let byte_idx = bit_idx / 8;
                let bit_pos = bit_idx % 8;
                buf[bitmap_offset + byte_idx] |= 1 << bit_pos;
            }

            // Write directory entry
            let dentry = DirEntryRaw {
                hash_code: hash,
                ino: entry.ino,
                name_len: entry.name.len() as u16,
                file_type: entry.file_type as u8,
            };
            let dentry_bytes = dentry.to_bytes();
            let entry_offset = dentry_offset + slot_idx * F2FS_DIR_ENTRY_SIZE;
            buf[entry_offset..entry_offset + F2FS_DIR_ENTRY_SIZE].copy_from_slice(&dentry_bytes);

            // Write file name
            let name_start = filename_offset + name_offset;
            let name_end = name_start + entry.name.len();
            if name_end <= F2FS_BLKSIZE {
                buf[name_start..name_end].copy_from_slice(&entry.name);
            }

            slot_idx += slots;
            name_offset += slots * F2FS_SLOT_LEN;
        }

        Ok(buf)
    }
}

impl Default for DentryBlockBuilder {
    fn default() -> Self {
        Self::new()
    }
}

// F2FS hash collision bit mask (64-bit, consistent with Linux kernel)
// In a 32-bit hash, the lower 32 bits of this mask are 0xFFFFFFFF, so no bits are actually cleared
const F2FS_HASH_COL_BIT: u64 = 1 << 63;

// TEA algorithm constants
const DELTA: u32 = 0x9E3779B9;

// TEA transformation function
fn tea_transform(buf: &mut [u32; 4], input: &[u32; 4]) {
    let mut sum: u32 = 0;
    let mut b0 = buf[0];
    let mut b1 = buf[1];
    let (a, b, c, d) = (input[0], input[1], input[2], input[3]);

    for _ in 0..16 {
        sum = sum.wrapping_add(DELTA);
        b0 = b0.wrapping_add(
            ((b1 << 4).wrapping_add(a)) ^ (b1.wrapping_add(sum)) ^ ((b1 >> 5).wrapping_add(b)),
        );
        b1 = b1.wrapping_add(
            ((b0 << 4).wrapping_add(c)) ^ (b0.wrapping_add(sum)) ^ ((b0 >> 5).wrapping_add(d)),
        );
    }

    buf[0] = buf[0].wrapping_add(b0);
    buf[1] = buf[1].wrapping_add(b1);
}

// Convert string to hash buffer
fn str2hashbuf(msg: &[u8], len: usize, buf: &mut [u32; 4]) {
    let pad = (len as u32) | ((len as u32) << 8);
    let pad = pad | (pad << 16);

    let mut val = pad;
    let actual_len = len.min(16);

    for (i, &byte) in msg.iter().take(actual_len).enumerate() {
        if i % 4 == 0 {
            val = pad;
        }
        val = (byte as u32).wrapping_add(val << 8);
        if i % 4 == 3 {
            buf[i / 4] = val;
            val = pad;
        }
    }

    // Process remaining bytes
    let filled = actual_len.div_ceil(4);
    if !actual_len.is_multiple_of(4) {
        buf[actual_len / 4] = val;
    }

    // fill remaining positions
    for item in buf.iter_mut().skip(filled) {
        *item = pad;
    }
}

// Calculate directory entry hash (TEA hash)
fn dentry_hash(name: &[u8]) -> u32 {
    if name.is_empty() {
        return 0;
    }

    // The hash of "." and ".." is fixed to 0
    if name == b"." || name == b".." {
        return 0;
    }

    // Initialize hash buffer (same initial values ​​as ext3/f2fs)
    let mut buf: [u32; 4] = [0x67452301, 0xefcdab89, 0x98badcfe, 0x10325476];

    let mut p = name;
    let mut len = name.len();

    loop {
        let mut input: [u32; 4] = [0; 4];
        str2hashbuf(p, len, &mut input);
        tea_transform(&mut buf, &input);

        if len <= 16 {
            break;
        }
        p = &p[16..];
        len -= 16;
    }

    // Use 64-bit mask, then truncate to 32-bit
    // Since F2FS_HASH_COL_BIT is 1<<63, the mask of the lower 32 bits is 0xFFFFFFFF
    // So this is actually equivalent to returning buf[0] directly
    ((buf[0] as u64) & !F2FS_HASH_COL_BIT) as u32
}

// Inline directory builder (for small directories)
#[derive(Debug)]
pub struct InlineDentryBuilder {
    entries: Vec<DentryInfo>,
    used_slots: usize,
}

// Maximum number of entries for an inline directory
const NR_INLINE_DENTRY_CONST: usize = 61;

impl InlineDentryBuilder {
    pub fn new() -> Self {
        InlineDentryBuilder {
            entries: Vec::new(),
            used_slots: 0,
        }
    }

    pub fn can_add(&self, entry: &DentryInfo) -> bool {
        let slots = entry.slots_needed();
        self.used_slots + slots <= NR_INLINE_DENTRY_CONST
    }

    pub fn add_entry(&mut self, entry: DentryInfo) -> bool {
        if !self.can_add(&entry) {
            return false;
        }

        let slots = entry.slots_needed();
        self.used_slots += slots;
        self.entries.push(entry);
        true
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    // Build inline catalog data
    pub fn build(&self) -> Vec<u8> {
        // Inline directory layout:
        // [0..8]: bitmap (8 bytes)
        // [8..9]: reserved (1 byte)
        // [9..9+61*11]: dentry array (61 * 11 = 671 bytes)
        // [680..]: filename area

        let total_size = 8
            + 1
            + NR_INLINE_DENTRY_CONST * F2FS_DIR_ENTRY_SIZE
            + NR_INLINE_DENTRY_CONST * F2FS_SLOT_LEN;
        let mut buf = vec![0u8; total_size];

        let bitmap_offset = 0;
        let dentry_offset = 8 + 1;
        let filename_offset = dentry_offset + NR_INLINE_DENTRY_CONST * F2FS_DIR_ENTRY_SIZE;

        let mut slot_idx = 0;
        let mut name_offset = 0;

        for entry in &self.entries {
            let slots = entry.slots_needed();
            let hash = dentry_hash(&entry.name);

            // Set bitmap
            for i in 0..slots {
                let bit_idx = slot_idx + i;
                let byte_idx = bit_idx / 8;
                let bit_pos = bit_idx % 8;
                if byte_idx < 8 {
                    buf[bitmap_offset + byte_idx] |= 1 << bit_pos;
                }
            }

            // Write directory entry
            let dentry = DirEntryRaw {
                hash_code: hash,
                ino: entry.ino,
                name_len: entry.name.len() as u16,
                file_type: entry.file_type as u8,
            };
            let dentry_bytes = dentry.to_bytes();
            let entry_offset = dentry_offset + slot_idx * F2FS_DIR_ENTRY_SIZE;
            if entry_offset + F2FS_DIR_ENTRY_SIZE <= buf.len() {
                buf[entry_offset..entry_offset + F2FS_DIR_ENTRY_SIZE]
                    .copy_from_slice(&dentry_bytes);
            }

            // Write file name
            let name_start = filename_offset + name_offset;
            let name_end = name_start + entry.name.len();
            if name_end <= buf.len() {
                buf[name_start..name_end].copy_from_slice(&entry.name);
            }

            slot_idx += slots;
            name_offset += slots * F2FS_SLOT_LEN;
        }

        buf
    }
}

impl Default for InlineDentryBuilder {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_dentry_info() {
        let entry = DentryInfo::new(b"test.txt", 100, FileType::RegFile);
        assert_eq!(entry.slots_needed(), 1); // 8 bytes, 1 slot

        let long_entry = DentryInfo::new(b"very_long_filename.txt", 101, FileType::RegFile);
        assert_eq!(long_entry.slots_needed(), 3); // 22 bytes, 3 slots
    }

    #[test]
    fn test_dentry_block_builder() {
        let mut builder = DentryBlockBuilder::new();

        let entry1 = DentryInfo::new(b".", 3, FileType::Dir);
        let entry2 = DentryInfo::new(b"..", 3, FileType::Dir);
        let entry3 = DentryInfo::new(b"test.txt", 4, FileType::RegFile);

        assert!(builder.add_entry(entry1));
        assert!(builder.add_entry(entry2));
        assert!(builder.add_entry(entry3));

        let data = builder.build().unwrap();
        assert_eq!(data.len(), F2FS_BLKSIZE);

        // Verify bitmap is not empty
        assert_ne!(data[0], 0);
    }

    #[test]
    fn test_dentry_hash() {
        let hash1 = dentry_hash(b"test");
        let hash2 = dentry_hash(b"test");
        assert_eq!(hash1, hash2);

        let hash3 = dentry_hash(b"other");
        assert_ne!(hash1, hash3);
    }

    #[test]
    fn test_inline_dentry_builder() {
        let mut builder = InlineDentryBuilder::new();

        let entry1 = DentryInfo::new(b".", 3, FileType::Dir);
        let entry2 = DentryInfo::new(b"..", 3, FileType::Dir);

        assert!(builder.add_entry(entry1));
        assert!(builder.add_entry(entry2));

        let data = builder.build();
        assert!(!data.is_empty());
    }
}
