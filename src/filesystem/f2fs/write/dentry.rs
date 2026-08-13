// F2FS 目录块构建器
use crate::filesystem::f2fs::consts::*;
//
// 负责构建 F2FS 目录数据块.

use crate::filesystem::f2fs::Result;
use crate::filesystem::f2fs::types::*;

// 目录块中的条目数量
const NR_DENTRY_IN_BLOCK_CONST: usize = 214;

// dentry bitmap 大小
const DENTRY_BITMAP_SIZE: usize = 27;

// 保留区域大小
const DENTRY_RESERVED_SIZE: usize = 3;

// 目录项
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

    // 计算所需的 slot 数量
    pub fn slots_needed(&self) -> usize {
        self.name.len().div_ceil(F2FS_SLOT_LEN)
    }
}

// 目录块构建器
#[derive(Debug)]
pub struct DentryBlockBuilder {
    entries: Vec<DentryInfo>,
    // 当前已使用的 slot 数量
    used_slots: usize,
}

impl DentryBlockBuilder {
    pub fn new() -> Self {
        DentryBlockBuilder {
            entries: Vec::new(),
            used_slots: 0,
        }
    }

    // 检查是否还能添加条目
    pub fn can_add(&self, entry: &DentryInfo) -> bool {
        let slots = entry.slots_needed();
        self.used_slots + slots <= NR_DENTRY_IN_BLOCK_CONST
    }

    // 添加目录项
    pub fn add_entry(&mut self, entry: DentryInfo) -> bool {
        if !self.can_add(&entry) {
            return false;
        }

        let slots = entry.slots_needed();
        self.used_slots += slots;
        self.entries.push(entry);
        true
    }

    // 获取已使用的 slot 数量
    pub fn used_slots(&self) -> usize {
        self.used_slots
    }

    // 检查是否为空
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    // 构建目录块
    pub fn build(&self) -> Result<[u8; F2FS_BLKSIZE]> {
        let mut buf = [0u8; F2FS_BLKSIZE];

        // 目录块布局:
        // [0..27]: dentry bitmap (27 字节)
        // [27..30]: 保留 (3 字节)
        // [30..30+214*11]: dentry 数组 (214 * 11 = 2354 字节)
        // [2384..4096]: 文件名区域 (1712 字节)

        let bitmap_offset = 0;
        let dentry_offset = DENTRY_BITMAP_SIZE + DENTRY_RESERVED_SIZE;
        let filename_offset = dentry_offset + NR_DENTRY_IN_BLOCK_CONST * F2FS_DIR_ENTRY_SIZE;

        let mut slot_idx = 0;
        let mut name_offset = 0;

        for entry in &self.entries {
            let slots = entry.slots_needed();
            let hash = dentry_hash(&entry.name);

            // 设置 bitmap
            for i in 0..slots {
                let bit_idx = slot_idx + i;
                let byte_idx = bit_idx / 8;
                let bit_pos = bit_idx % 8;
                buf[bitmap_offset + byte_idx] |= 1 << bit_pos;
            }

            // 写入目录项
            let dentry = DirEntryRaw {
                hash_code: hash,
                ino: entry.ino,
                name_len: entry.name.len() as u16,
                file_type: entry.file_type as u8,
            };
            let dentry_bytes = dentry.to_bytes();
            let entry_offset = dentry_offset + slot_idx * F2FS_DIR_ENTRY_SIZE;
            buf[entry_offset..entry_offset + F2FS_DIR_ENTRY_SIZE].copy_from_slice(&dentry_bytes);

            // 写入文件名
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

// F2FS 哈希冲突位掩码 (64 位, 与 Linux 内核一致)
// 在 32 位哈希中, 该掩码的低 32 位为 0xFFFFFFFF, 因此实际上不会清除任何位
const F2FS_HASH_COL_BIT: u64 = 1 << 63;

// TEA 算法常量
const DELTA: u32 = 0x9E3779B9;

// TEA 变换函数
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

// 将字符串转换为哈希缓冲区
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

    // 处理剩余字节
    let filled = actual_len.div_ceil(4);
    if !actual_len.is_multiple_of(4) {
        buf[actual_len / 4] = val;
    }

    // 填充剩余位置
    for item in buf.iter_mut().skip(filled) {
        *item = pad;
    }
}

// 计算目录项哈希 (TEA hash)
fn dentry_hash(name: &[u8]) -> u32 {
    if name.is_empty() {
        return 0;
    }

    // "." 与 ".." 的哈希固定为 0
    if name == b"." || name == b".." {
        return 0;
    }

    // 初始化哈希缓冲区 (初始值与 ext3/f2fs 一致)
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

    // 先使用 64 位掩码, 再截断为 32 位
    // 由于 F2FS_HASH_COL_BIT 为 1<<63, 低 32 位的掩码为 0xFFFFFFFF
    // 因此实际等价于直接返回 buf[0]
    ((buf[0] as u64) & !F2FS_HASH_COL_BIT) as u32
}

// inline 目录构建器 (用于小目录)
#[derive(Debug)]
pub struct InlineDentryBuilder {
    entries: Vec<DentryInfo>,
    used_slots: usize,
}

// inline 目录的最大条目数量
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

    // 构建 inline 目录数据
    pub fn build(&self) -> Vec<u8> {
        // inline 目录布局:
        // [0..8]: bitmap (8 字节)
        // [8..9]: 保留 (1 字节)
        // [9..9+61*11]: dentry 数组 (61 * 11 = 671 字节)
        // [680..]: 文件名区域

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

            // 设置 bitmap
            for i in 0..slots {
                let bit_idx = slot_idx + i;
                let byte_idx = bit_idx / 8;
                let bit_pos = bit_idx % 8;
                if byte_idx < 8 {
                    buf[bitmap_offset + byte_idx] |= 1 << bit_pos;
                }
            }

            // 写入目录项
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

            // 写入文件名
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
        assert_eq!(entry.slots_needed(), 1); // 8 字节, 1 个 slot

        let long_entry = DentryInfo::new(b"very_long_filename.txt", 101, FileType::RegFile);
        assert_eq!(long_entry.slots_needed(), 3); // 22 字节, 3 个 slot
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

        // 校验 bitmap 非空
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
