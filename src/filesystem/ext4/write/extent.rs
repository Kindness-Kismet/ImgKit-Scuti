// EXT4 extent 构建器

use crate::filesystem::ext4::types::*;
use zerocopy::IntoBytes;

// extent 构建器
pub struct ExtentBuilder {
    extents: Vec<Ext4Extent>,
}

impl ExtentBuilder {
    // 创建新的 extent 构建器
    pub fn new() -> Self {
        ExtentBuilder {
            extents: Vec::new(),
        }
    }

    // 添加一个 extent
    pub fn add_extent(&mut self, logical_block: u32, physical_block: u64, length: u16) {
        let extent = Ext4Extent {
            ee_block: logical_block,
            ee_len: length,
            ee_start_hi: (physical_block >> 32) as u16,
            ee_start_lo: (physical_block & 0xFFFFFFFF) as u32,
        };
        self.extents.push(extent);
    }

    // 从块列表创建 extent
    pub fn from_blocks(blocks: &[u64]) -> Self {
        let mut builder = ExtentBuilder::new();

        if blocks.is_empty() {
            return builder;
        }

        // 合并连续的块
        let mut start_block = blocks[0];
        let mut logical_block = 0u32;
        let mut length = 1u16;

        for i in 1..blocks.len() {
            if blocks[i] == blocks[i - 1] + 1 && length < 32768 {
                // 块连续, 增加长度
                length += 1;
            } else {
                // 块不连续, 创建新的 extent
                builder.add_extent(logical_block, start_block, length);
                logical_block += length as u32;
                start_block = blocks[i];
                length = 1;
            }
        }

        // 添加最后一个 extent
        builder.add_extent(logical_block, start_block, length);

        builder
    }

    // 构建 extent tree (存放在 inode 的 i_block 中)
    pub fn build_inline(&self) -> [u8; 60] {
        let mut data = [0u8; 60];

        // extent 头部
        let header = Ext4ExtentHeader {
            eh_magic: EXT4_EXTENT_HEADER_MAGIC,
            eh_entries: self.extents.len().min(4) as u16,
            eh_max: 4,   // inode 内最多 4 个 extent
            eh_depth: 0, // 叶子节点
            eh_generation: 0,
        };

        // 写入 extent header
        let header_bytes = header.as_bytes();
        data[..header_bytes.len()].copy_from_slice(header_bytes);

        // 写入 extent
        let mut offset = header_bytes.len();
        for extent in self.extents.iter().take(4) {
            let extent_bytes = extent.as_bytes();
            data[offset..offset + extent_bytes.len()].copy_from_slice(extent_bytes);
            offset += extent_bytes.len();
        }

        data
    }

    // 获取 extent 数量
    pub fn len(&self) -> usize {
        self.extents.len()
    }

    // 判断是否为空
    pub fn is_empty(&self) -> bool {
        self.extents.is_empty()
    }
}

impl Default for ExtentBuilder {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extent_builder() {
        let mut builder = ExtentBuilder::new();
        builder.add_extent(0, 1000, 10);
        builder.add_extent(10, 2000, 20);

        assert_eq!(builder.len(), 2);
    }

    #[test]
    fn test_from_blocks() {
        let blocks = vec![100, 101, 102, 103, 200, 201];
        let builder = ExtentBuilder::from_blocks(&blocks);

        // 应合并为 2 个 extent
        assert_eq!(builder.len(), 2);
    }

    #[test]
    fn test_build_inline() {
        let mut builder = ExtentBuilder::new();
        builder.add_extent(0, 1000, 10);

        let data = builder.build_inline();

        // 校验魔数
        let magic = u16::from_le_bytes([data[0], data[1]]);
        assert_eq!(magic, EXT4_EXTENT_HEADER_MAGIC);
    }
}
