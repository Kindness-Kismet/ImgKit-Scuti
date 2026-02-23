// EXT4 block allocator

use std::collections::HashSet;

// block allocator
pub struct BlockAllocator {
    // total number of blocks
    total_blocks: u64,
    // Number of blocks per group
    blocks_per_group: u32,
    // allocated blocks
    allocated_blocks: HashSet<u64>,
    // bitmap for each block group
    bitmaps: Vec<Vec<u8>>,
    // Next possible free block (optimized search)
    next_free_block: u64,
}

impl BlockAllocator {
    // Create new block allocator
    pub fn new(total_blocks: u64, blocks_per_group: u32) -> Self {
        let group_count = total_blocks.div_ceil(blocks_per_group as u64) as u32;

        // Initialize bitmap for each block group
        let bitmap_size = (blocks_per_group as usize).div_ceil(8);
        let bitmaps = vec![vec![0u8; bitmap_size]; group_count as usize];

        BlockAllocator {
            total_blocks,
            blocks_per_group,
            allocated_blocks: HashSet::new(),
            bitmaps,
            next_free_block: 0,
        }
    }

    // allocate a block
    pub fn alloc_block(&mut self) -> Option<u64> {
        // Start searching from next_free_block
        for block in self.next_free_block..self.total_blocks {
            if !self.allocated_blocks.contains(&block) {
                self.allocated_blocks.insert(block);
                self.mark_block_used(block);
                self.next_free_block = block + 1;
                return Some(block);
            }
        }

        // If not found, search from scratch (processing fragments)
        for block in 0..self.next_free_block {
            if !self.allocated_blocks.contains(&block) {
                self.allocated_blocks.insert(block);
                self.mark_block_used(block);
                self.next_free_block = block + 1;
                return Some(block);
            }
        }

        None
    }

    // allocate contiguous blocks
    pub fn alloc_blocks(&mut self, count: u32) -> Option<Vec<u64>> {
        let mut blocks = Vec::new();
        for _ in 0..count {
            if let Some(block) = self.alloc_block() {
                blocks.push(block);
            } else {
                // Rollback allocated blocks
                for b in &blocks {
                    self.free_block(*b);
                }
                return None;
            }
        }
        Some(blocks)
    }

    // Mark block as used
    fn mark_block_used(&mut self, block: u64) {
        let group_idx = (block / self.blocks_per_group as u64) as usize;
        let block_in_group = (block % self.blocks_per_group as u64) as usize;
        let byte_idx = block_in_group / 8;
        let bit_idx = block_in_group % 8;

        if group_idx < self.bitmaps.len() && byte_idx < self.bitmaps[group_idx].len() {
            self.bitmaps[group_idx][byte_idx] |= 1 << bit_idx;
        }
    }

    // free block
    pub fn free_block(&mut self, block: u64) {
        self.allocated_blocks.remove(&block);

        let group_idx = (block / self.blocks_per_group as u64) as usize;
        let block_in_group = (block % self.blocks_per_group as u64) as usize;
        let byte_idx = block_in_group / 8;
        let bit_idx = block_in_group % 8;

        if group_idx < self.bitmaps.len() && byte_idx < self.bitmaps[group_idx].len() {
            self.bitmaps[group_idx][byte_idx] &= !(1 << bit_idx);
        }
    }

    // reserved metadata block
    pub fn reserve_metadata_blocks(&mut self, _group_idx: u32, blocks: &[u64]) {
        for &block in blocks {
            self.allocated_blocks.insert(block);
            self.mark_block_used(block);
            // Update next_free_block to skip metadata blocks
            if block >= self.next_free_block {
                self.next_free_block = block + 1;
            }
        }
    }

    // Get the bitmap of the block group
    pub fn get_bitmap(&self, group_idx: u32) -> &[u8] {
        &self.bitmaps[group_idx as usize]
    }

    // Get the number of allocated blocks
    pub fn allocated_count(&self) -> u64 {
        self.allocated_blocks.len() as u64
    }

    // Get the number of free blocks
    pub fn free_count(&self) -> u64 {
        self.total_blocks - self.allocated_count()
    }

    // Get the number of free blocks in the block group
    pub fn get_free_blocks_in_group(&self, group_idx: u32) -> u32 {
        let group_start = group_idx as u64 * self.blocks_per_group as u64;
        let group_end = (group_start + self.blocks_per_group as u64).min(self.total_blocks);
        let mut free_count = 0;

        for block in group_start..group_end {
            if !self.allocated_blocks.contains(&block) {
                free_count += 1;
            }
        }

        free_count
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_block_allocator() {
        let mut allocator = BlockAllocator::new(1000, 100);

        let block1 = allocator.alloc_block().unwrap();
        assert_eq!(block1, 0);

        let block2 = allocator.alloc_block().unwrap();
        assert_eq!(block2, 1);

        assert_eq!(allocator.allocated_count(), 2);
    }

    #[test]
    fn test_alloc_blocks() {
        let mut allocator = BlockAllocator::new(1000, 100);

        let blocks = allocator.alloc_blocks(10).unwrap();
        assert_eq!(blocks.len(), 10);
        assert_eq!(allocator.allocated_count(), 10);
    }
}
