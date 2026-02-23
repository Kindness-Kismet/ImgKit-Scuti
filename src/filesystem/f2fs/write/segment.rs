// F2FS segment allocator
use crate::filesystem::f2fs::consts::*;
//
// Responsible for managing the allocation of segments and blocks.

use crate::filesystem::f2fs::types::*;
use crate::filesystem::f2fs::{F2fsError, Result};

// segment allocator
#[derive(Debug)]
pub struct SegmentAllocator {
    // Current segment number (one for each type)
    current_segments: [u32; NR_CURSEG_TYPE],
    // Next block offset within the current segment
    next_blkoff: [u16; NR_CURSEG_TYPE],
    // Main area starting block address
    main_blkaddr: u32,
    // Number of blocks per segment
    blocks_per_seg: u32,
    // Total number of segments
    total_segments: u32,
    // Number of allocated blocks
    allocated_blocks: u64,
    // Used segment set (tracks all allocated segments)
    used_segments: std::collections::HashSet<u32>,
}

impl SegmentAllocator {
    // Create a new segment allocator
    pub fn new(main_blkaddr: u32, total_segments: u32) -> Self {
        let mut allocator = SegmentAllocator {
            current_segments: [0; NR_CURSEG_TYPE],
            next_blkoff: [0; NR_CURSEG_TYPE],
            main_blkaddr,
            blocks_per_seg: DEFAULT_BLOCKS_PER_SEGMENT,
            total_segments,
            allocated_blocks: 0,
            used_segments: std::collections::HashSet::new(),
        };

        // Initialize the current segment
        // Assign a different starting segment to each type
        for i in 0..NR_CURSEG_TYPE {
            allocator.current_segments[i] = i as u32;
            // Mark initial segment as used
            allocator.used_segments.insert(i as u32);
        }

        allocator
    }

    // Set current segment
    pub fn set_current_segment(&mut self, seg_type: SegType, segno: u32, blkoff: u16) {
        let idx = seg_type as usize;
        if idx < NR_CURSEG_TYPE {
            self.current_segments[idx] = segno;
            self.next_blkoff[idx] = blkoff;
        }
    }

    // Get the current segment number
    pub fn current_segno(&self, seg_type: SegType) -> u32 {
        self.current_segments[seg_type as usize]
    }

    // Get the current block offset
    pub fn current_blkoff(&self, seg_type: SegType) -> u16 {
        self.next_blkoff[seg_type as usize]
    }

    // allocate data blocks
    pub fn alloc_data_block(&mut self, seg_type: SegType) -> Result<u32> {
        if !seg_type.is_data() {
            return Err(F2fsError::InvalidData("段类型不是数据类型".into()));
        }
        self.alloc_block(seg_type)
    }

    // Allocate node blocks
    pub fn alloc_node_block(&mut self, seg_type: SegType) -> Result<u32> {
        if !seg_type.is_node() {
            return Err(F2fsError::InvalidData("段类型不是节点类型".into()));
        }
        self.alloc_block(seg_type)
    }

    // Allocate block (internal method)
    fn alloc_block(&mut self, seg_type: SegType) -> Result<u32> {
        let idx = seg_type as usize;

        // Check if you need to switch to a new segment
        if self.next_blkoff[idx] >= self.blocks_per_seg as u16 {
            self.allocate_new_segment(seg_type)?;
        }

        // Calculate block address
        let segno = self.current_segments[idx];
        let blkoff = self.next_blkoff[idx];
        let blkaddr = self.main_blkaddr + segno * self.blocks_per_seg + blkoff as u32;

        // Update offset
        self.next_blkoff[idx] += 1;
        self.allocated_blocks += 1;

        Ok(blkaddr)
    }

    // allocate new segment
    fn allocate_new_segment(&mut self, seg_type: SegType) -> Result<()> {
        let idx = seg_type as usize;

        // Find next available segment
        // Start searching from the segment next to the current segment
        let mut next_segno = self.current_segments[idx] + 1;

        // Make sure you don't go out of scope
        if next_segno >= self.total_segments {
            return Err(F2fsError::InvalidData("没有可用的段".into()));
        }

        // Find unused segments
        loop {
            // Check if it has been used
            if !self.used_segments.contains(&next_segno) {
                break;
            }

            next_segno += 1;
            if next_segno >= self.total_segments {
                return Err(F2fsError::InvalidData("没有可用的段".into()));
            }
        }

        self.current_segments[idx] = next_segno;
        self.next_blkoff[idx] = 0;
        // Mark new segment as used
        self.used_segments.insert(next_segno);

        Ok(())
    }

    // Get the number of allocated blocks
    pub fn allocated_blocks(&self) -> u64 {
        self.allocated_blocks
    }

    // Get the number of free segments
    pub fn free_segments(&self) -> u32 {
        // Number of free segments = Total number of segments - Number of used segments
        // The number of used segments is the size of the used_segments collection
        self.total_segments
            .saturating_sub(self.used_segments.len() as u32)
    }

    // Get the main area starting block address
    pub fn main_blkaddr(&self) -> u32 {
        self.main_blkaddr
    }

    // Get the total number of segments
    pub fn total_segments(&self) -> u32 {
        self.total_segments
    }

    // Get the number of blocks in each segment
    pub fn blocks_per_seg(&self) -> u32 {
        self.blocks_per_seg
    }

    // Convert block address to segment number
    pub fn blkaddr_to_segno(&self, blkaddr: u32) -> Option<u32> {
        if blkaddr < self.main_blkaddr {
            return None;
        }
        Some((blkaddr - self.main_blkaddr) / self.blocks_per_seg)
    }

    // Convert block address to intra-segment offset
    pub fn blkaddr_to_blkoff(&self, blkaddr: u32) -> u32 {
        (blkaddr - self.main_blkaddr) % self.blocks_per_seg
    }

    // Get current segment information (for checkpoints)
    pub fn get_curseg_info(&self) -> CursegInfo {
        CursegInfo {
            node_segno: [
                self.current_segments[CURSEG_HOT_NODE],
                self.current_segments[CURSEG_WARM_NODE],
                self.current_segments[CURSEG_COLD_NODE],
            ],
            node_blkoff: [
                self.next_blkoff[CURSEG_HOT_NODE],
                self.next_blkoff[CURSEG_WARM_NODE],
                self.next_blkoff[CURSEG_COLD_NODE],
            ],
            data_segno: [
                self.current_segments[CURSEG_HOT_DATA],
                self.current_segments[CURSEG_WARM_DATA],
                self.current_segments[CURSEG_COLD_DATA],
            ],
            data_blkoff: [
                self.next_blkoff[CURSEG_HOT_DATA],
                self.next_blkoff[CURSEG_WARM_DATA],
                self.next_blkoff[CURSEG_COLD_DATA],
            ],
        }
    }
}

// Current segment information
#[derive(Debug, Clone)]
pub struct CursegInfo {
    pub node_segno: [u32; 3],
    pub node_blkoff: [u16; 3],
    pub data_segno: [u32; 3],
    pub data_blkoff: [u16; 3],
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_segment_allocator_new() {
        let allocator = SegmentAllocator::new(1024, 100);
        assert_eq!(allocator.main_blkaddr(), 1024);
        assert_eq!(allocator.total_segments(), 100);
    }

    #[test]
    fn test_alloc_data_block() {
        let mut allocator = SegmentAllocator::new(1024, 100);

        let blk1 = allocator.alloc_data_block(SegType::HotData).unwrap();
        assert_eq!(blk1, 1024); // main_blkaddr + seg0 * 512 + 0

        let blk2 = allocator.alloc_data_block(SegType::HotData).unwrap();
        assert_eq!(blk2, 1025);
    }

    #[test]
    fn test_alloc_node_block() {
        let mut allocator = SegmentAllocator::new(1024, 100);

        let blk1 = allocator.alloc_node_block(SegType::HotNode).unwrap();
        // HotNode is type 4, so the starting segment is 3
        assert_eq!(blk1, 1024 + 3 * 512);

        let blk2 = allocator.alloc_node_block(SegType::HotNode).unwrap();
        assert_eq!(blk2, 1024 + 3 * 512 + 1);
    }

    #[test]
    fn test_segment_switch() {
        let mut allocator = SegmentAllocator::new(0, 100);

        // Allocate a segment
        for _ in 0..DEFAULT_BLOCKS_PER_SEGMENT {
            allocator.alloc_data_block(SegType::HotData).unwrap();
        }

        // The next chunk should be in the new segment
        let blk = allocator.alloc_data_block(SegType::HotData).unwrap();
        // The new segment number should be 0 + NR_CURSEG_TYPE = 6
        assert_eq!(blk, 6 * DEFAULT_BLOCKS_PER_SEGMENT);
    }

    #[test]
    fn test_curseg_info() {
        let allocator = SegmentAllocator::new(1024, 100);
        let info = allocator.get_curseg_info();

        assert_eq!(info.node_segno[0], CURSEG_HOT_NODE as u32);
        assert_eq!(info.data_segno[0], CURSEG_HOT_DATA as u32);
    }
}
