// EXT4 Extent Builder

use crate::filesystem::ext4::types::*;
use zerocopy::IntoBytes;

// Extent builder
pub struct ExtentBuilder {
    extents: Vec<Ext4Extent>,
}

impl ExtentBuilder {
    // Create a new extent builder
    pub fn new() -> Self {
        ExtentBuilder {
            extents: Vec::new(),
        }
    }

    // add an extent
    pub fn add_extent(&mut self, logical_block: u32, physical_block: u64, length: u16) {
        let extent = Ext4Extent {
            ee_block: logical_block,
            ee_len: length,
            ee_start_hi: (physical_block >> 32) as u16,
            ee_start_lo: (physical_block & 0xFFFFFFFF) as u32,
        };
        self.extents.push(extent);
    }

    // Create extents from a list of blocks
    pub fn from_blocks(blocks: &[u64]) -> Self {
        let mut builder = ExtentBuilder::new();

        if blocks.is_empty() {
            return builder;
        }

        // Merge consecutive blocks
        let mut start_block = blocks[0];
        let mut logical_block = 0u32;
        let mut length = 1u16;

        for i in 1..blocks.len() {
            if blocks[i] == blocks[i - 1] + 1 && length < 32768 {
                // Continuous blocks, increasing length
                length += 1;
            } else {
                // Discontinuous, create a new extent
                builder.add_extent(logical_block, start_block, length);
                logical_block += length as u32;
                start_block = blocks[i];
                length = 1;
            }
        }

        // Add the last extent
        builder.add_extent(logical_block, start_block, length);

        builder
    }

    // Build extent tree (stored in i_block of inode)
    pub fn build_inline(&self) -> [u8; 60] {
        let mut data = [0u8; 60];

        // Extent header
        let header = Ext4ExtentHeader {
            eh_magic: EXT4_EXTENT_HEADER_MAGIC,
            eh_entries: self.extents.len().min(4) as u16,
            eh_max: 4,   // Up to 4 extents in inode
            eh_depth: 0, // leaf node
            eh_generation: 0,
        };

        // write header
        let header_bytes = header.as_bytes();
        data[..header_bytes.len()].copy_from_slice(header_bytes);

        // write extents
        let mut offset = header_bytes.len();
        for extent in self.extents.iter().take(4) {
            let extent_bytes = extent.as_bytes();
            data[offset..offset + extent_bytes.len()].copy_from_slice(extent_bytes);
            offset += extent_bytes.len();
        }

        data
    }

    // Get extent quantity
    pub fn len(&self) -> usize {
        self.extents.len()
    }

    // Is it empty
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

        // Should be merged into 2 extents
        assert_eq!(builder.len(), 2);
    }

    #[test]
    fn test_build_inline() {
        let mut builder = ExtentBuilder::new();
        builder.add_extent(0, 1000, 10);

        let data = builder.build_inline();

        // Verify the magic number
        let magic = u16::from_le_bytes([data[0], data[1]]);
        assert_eq!(magic, EXT4_EXTENT_HEADER_MAGIC);
    }
}
