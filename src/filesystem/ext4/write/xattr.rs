// EXT4 xattr 构建器

use crate::filesystem::ext4::Result;
use crate::filesystem::ext4::error::Ext4Error;
use crate::filesystem::ext4::types::*;

// xattr 的 name index
pub const XATTR_INDEX_USER: u8 = 1;
pub const XATTR_INDEX_POSIX_ACL_ACCESS: u8 = 2;
pub const XATTR_INDEX_POSIX_ACL_DEFAULT: u8 = 3;
pub const XATTR_INDEX_TRUSTED: u8 = 4;
pub const XATTR_INDEX_SECURITY: u8 = 6;

// xattr 条目
#[derive(Clone)]
pub struct XattrEntry {
    pub name_index: u8,
    pub name: Vec<u8>,
    pub value: Vec<u8>,
}

impl XattrEntry {
    // 创建 SELinux 安全上下文 xattr
    pub fn selinux(context: &str) -> Self {
        XattrEntry {
            name_index: XATTR_INDEX_SECURITY,
            name: b"selinux".to_vec(),
            value: context.as_bytes().to_vec(),
        }
    }

    // 计算 entry 大小 (按 4 字节对齐)
    pub fn size(&self) -> usize {
        let base_size = 16 + self.name.len(); // sizeof(Ext4XattrEntry) + 名称长度
        (base_size + 3) & !3
    }

    // 序列化为字节
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut buf = Vec::new();

        // e_name_len 名称长度
        buf.push(self.name.len() as u8);

        // e_name_index 名称前缀索引
        buf.push(self.name_index);

        // e_value_offs 值偏移 (稍后填充)
        buf.extend_from_slice(&0u16.to_le_bytes());

        // e_value_inum 外部值所在 inode
        buf.extend_from_slice(&0u32.to_le_bytes());

        // e_value_size 值长度
        buf.extend_from_slice(&(self.value.len() as u32).to_le_bytes());

        // 外部属性必须带有名称及属性值的校验哈希。
        let mut hash = self
            .name
            .iter()
            .fold(0u32, |hash, byte| hash.rotate_left(5) ^ u32::from(*byte));
        for chunk in self.value.chunks(4) {
            let mut word = [0; 4];
            word[..chunk.len()].copy_from_slice(chunk);
            hash = hash.rotate_left(16) ^ u32::from_le_bytes(word);
        }
        buf.extend_from_slice(&hash.to_le_bytes());

        // e_name 名称
        buf.extend_from_slice(&self.name);

        // 按 4 字节对齐
        while buf.len() % 4 != 0 {
            buf.push(0);
        }

        buf
    }
}

// xattr 块构建器
pub struct XattrBlockBuilder {
    entries: Vec<XattrEntry>,
}

impl XattrBlockBuilder {
    // 创建新的 xattr 块构建器
    pub fn new() -> Self {
        XattrBlockBuilder {
            entries: Vec::new(),
        }
    }

    // 添加 entry
    pub fn add_entry(&mut self, entry: XattrEntry) {
        self.entries.push(entry);
    }

    // 构建 xattr 块
    pub fn build(&self, block_size: usize) -> Result<Vec<u8>> {
        let mut block = build_entries(&self.entries, block_size, 32, 0)?;
        block[4..8].copy_from_slice(&1u32.to_le_bytes()); // h_refcount
        block[8..12].copy_from_slice(&1u32.to_le_bytes()); // h_blocks
        Ok(block)
    }

    // 判断是否为空
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

impl Default for XattrBlockBuilder {
    fn default() -> Self {
        Self::new()
    }
}

// inline xattr 构建器 (存放在 inode 中)
pub struct InlineXattrBuilder {
    entries: Vec<XattrEntry>,
}

impl InlineXattrBuilder {
    // 创建新的 inline xattr 构建器
    pub fn new() -> Self {
        InlineXattrBuilder {
            entries: Vec::new(),
        }
    }

    // 添加 entry
    pub fn add_entry(&mut self, entry: XattrEntry) {
        self.entries.push(entry);
    }

    // 构建 inline xattr 数据
    pub fn build(&self, max_size: usize) -> Result<Vec<u8>> {
        build_entries(&self.entries, max_size, 4, 4)
    }

    // 判断是否为空
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

impl Default for InlineXattrBuilder {
    fn default() -> Self {
        Self::new()
    }
}

// 内联属性相对首条条目寻址，外部属性相对块起点寻址。
fn build_entries(
    entries: &[XattrEntry],
    size: usize,
    header_size: usize,
    value_base: usize,
) -> Result<Vec<u8>> {
    let required = header_size
        + 4
        + entries
            .iter()
            .map(|entry| entry.size() + entry.value.len().next_multiple_of(4))
            .sum::<usize>();
    if required > size || size > u16::MAX as usize + 1 {
        return Err(Ext4Error::InvalidXattr(format!(
            "需要 {required} 字节，容量 {size} 字节"
        )));
    }
    let mut data = vec![0; size];
    data[..4].copy_from_slice(&EXT4_XATTR_HEADER_MAGIC.to_le_bytes());
    let mut sorted_entries: Vec<_> = entries.iter().collect();
    sorted_entries.sort_by(|left, right| {
        (left.name_index, left.name.len(), &left.name).cmp(&(
            right.name_index,
            right.name.len(),
            &right.name,
        ))
    });
    let mut offset = header_size;
    let mut value_offset = size;
    for entry in sorted_entries {
        if entry.name.len() > u8::MAX as usize {
            return Err(Ext4Error::InvalidXattr("属性名称超过 255 字节".into()));
        }
        value_offset = (value_offset - entry.value.len()) & !3;
        let encoded = entry.to_bytes();
        data[offset..offset + encoded.len()].copy_from_slice(&encoded);
        let relative = if entry.value.is_empty() {
            0
        } else {
            (value_offset - value_base) as u16
        };
        data[offset + 2..offset + 4].copy_from_slice(&relative.to_le_bytes());
        data[value_offset..value_offset + entry.value.len()].copy_from_slice(&entry.value);
        offset += encoded.len();
    }
    Ok(data)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_xattr_entry() {
        let entry = XattrEntry::selinux("u:object_r:system_file:s0");
        assert_eq!(entry.name_index, XATTR_INDEX_SECURITY);
        assert_eq!(entry.name, b"selinux");
    }

    #[test]
    fn test_xattr_block_builder() {
        let mut builder = XattrBlockBuilder::new();
        builder.add_entry(XattrEntry::selinux("u:object_r:system_file:s0"));

        let block = builder.build(4096).unwrap();
        assert_eq!(block.len(), 4096);

        // 校验魔数
        let magic = u32::from_le_bytes([block[0], block[1], block[2], block[3]]);
        assert_eq!(magic, EXT4_XATTR_HEADER_MAGIC);
    }
}
