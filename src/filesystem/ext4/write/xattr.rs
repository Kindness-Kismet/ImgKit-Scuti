// EXT4 xattr 构建器

use crate::filesystem::ext4::Result;
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

        // e_hash 名称哈希
        buf.extend_from_slice(&0u32.to_le_bytes());

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
        let mut block = vec![0u8; block_size];

        // 写入 xattr header
        let magic = EXT4_XATTR_HEADER_MAGIC;
        block[0..4].copy_from_slice(&magic.to_le_bytes());
        block[4..8].copy_from_slice(&1u32.to_le_bytes()); // h_refcount
        block[8..12].copy_from_slice(&1u32.to_le_bytes()); // h_blocks
        block[12..16].copy_from_slice(&0u32.to_le_bytes()); // h_hash
        block[16..20].copy_from_slice(&0u32.to_le_bytes()); // h_checksum

        let mut offset = 32; // header 大小
        let mut value_offset = block_size;

        // 写入 entry
        for entry in &self.entries {
            let entry_bytes = entry.to_bytes();

            // 更新 value_offset
            value_offset -= entry.value.len();
            value_offset = (value_offset / 4) * 4; // 对齐

            // 写入 entry 头部
            block[offset..offset + entry_bytes.len()].copy_from_slice(&entry_bytes);

            // 更新 e_value_offs
            let value_offs = (value_offset - offset) as u16;
            block[offset + 2..offset + 4].copy_from_slice(&value_offs.to_le_bytes());

            // 写入属性值
            block[value_offset..value_offset + entry.value.len()].copy_from_slice(&entry.value);

            offset += entry_bytes.len();
        }

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
        let mut data = Vec::new();

        // 在头部写入魔数
        data.extend_from_slice(&EXT4_XATTR_HEADER_MAGIC.to_le_bytes());

        let mut value_offset = max_size;

        // 写入 entry
        for entry in &self.entries {
            let entry_bytes = entry.to_bytes();

            // 更新 value_offset
            value_offset -= entry.value.len();
            value_offset = (value_offset / 4) * 4; // 对齐

            // 写入 entry 头部
            data.extend_from_slice(&entry_bytes);

            // 更新 e_value_offs (相对于 inline xattr 区域起始位置)
            let offs_pos = data.len() - entry_bytes.len() + 2;
            let value_offs = (value_offset - 4) as u16; // 减 4 是因为魔数占用
            data[offs_pos..offs_pos + 2].copy_from_slice(&value_offs.to_le_bytes());
        }

        // 添加终止标记
        data.extend_from_slice(&[0u8; 4]);

        // 填充到 max_size
        if data.len() < max_size {
            // 写入属性值
            let mut values_data = vec![0u8; max_size - data.len()];
            let mut write_offset = max_size - data.len();

            for entry in self.entries.iter().rev() {
                write_offset -= entry.value.len();
                write_offset = (write_offset / 4) * 4;
                values_data[write_offset..write_offset + entry.value.len()]
                    .copy_from_slice(&entry.value);
            }

            data.extend_from_slice(&values_data);
        }

        Ok(data)
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
