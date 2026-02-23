// F2FS SIT (Segment Information Table) Manager
use crate::filesystem::f2fs::consts::*;
//
// Responsible for managing the segment information table and tracking the number and type of valid blocks for each segment.

use crate::filesystem::f2fs::types::*;
use crate::filesystem::f2fs::{F2fsError, Result};
use std::io::Write;

// SIT Manager
#[derive(Debug)]
pub struct SitManager {
    // SIT entry list
    entries: Vec<SitEntry>,
    // SIT area starting block address
    sit_blkaddr: u32,
    // Number of blocks per segment
    blocks_per_seg: u32,
    // Main area starting block address
    main_blkaddr: u32,
}

impl SitManager {
    // Create a new SIT manager
    pub fn new(segment_count: u32, sit_blkaddr: u32, main_blkaddr: u32) -> Self {
        let mut entries = Vec::with_capacity(segment_count as usize);
        for _ in 0..segment_count {
            entries.push(SitEntry::default());
        }

        SitManager {
            entries,
            sit_blkaddr,
            blocks_per_seg: DEFAULT_BLOCKS_PER_SEGMENT,
            main_blkaddr,
        }
    }

    // Get segment number
    fn get_segno(&self, blkaddr: u32) -> Option<u32> {
        if blkaddr < self.main_blkaddr {
            return None;
        }
        Some((blkaddr - self.main_blkaddr) / self.blocks_per_seg)
    }

    // Get the offset of the block within the segment
    fn get_blkoff(&self, blkaddr: u32) -> u32 {
        (blkaddr - self.main_blkaddr) % self.blocks_per_seg
    }

    // Mark block as used
    pub fn mark_block_used(&mut self, blkaddr: u32, seg_type: u16) -> Result<()> {
        let segno = self
            .get_segno(blkaddr)
            .ok_or_else(|| F2fsError::InvalidData(format!("无效的块地址: {}", blkaddr)))?;

        if segno as usize >= self.entries.len() {
            return Err(F2fsError::InvalidData(format!(
                "段号超出范围: {} >= {}",
                segno,
                self.entries.len()
            )));
        }

        let blkoff = self.get_blkoff(blkaddr) as usize;
        let entry = &mut self.entries[segno as usize];

        // Mark block as valid
        entry.mark_block_valid(blkoff);

        // Update valid block number and segment type
        let valid_blocks = entry.valid_blocks() + 1;
        entry.set_vblocks(valid_blocks, seg_type);

        Ok(())
    }

    // Batch mark blocks as used
    pub fn mark_blocks_used(
        &mut self,
        start_blkaddr: u32,
        count: u32,
        seg_type: u16,
    ) -> Result<()> {
        for i in 0..count {
            self.mark_block_used(start_blkaddr + i, seg_type)?;
        }
        Ok(())
    }

    // Set segment type
    pub fn set_seg_type(&mut self, segno: u32, seg_type: u16) -> Result<()> {
        if segno as usize >= self.entries.len() {
            return Err(F2fsError::InvalidData(format!(
                "段号超出范围: {} >= {}",
                segno,
                self.entries.len()
            )));
        }

        let entry = &mut self.entries[segno as usize];
        let valid_blocks = entry.valid_blocks();
        entry.set_vblocks(valid_blocks, seg_type);
        Ok(())
    }

    // Set the modification time of the segment
    pub fn set_mtime(&mut self, segno: u32, mtime: u64) -> Result<()> {
        if segno as usize >= self.entries.len() {
            return Err(F2fsError::InvalidData(format!(
                "段号超出范围: {} >= {}",
                segno,
                self.entries.len()
            )));
        }

        self.entries[segno as usize].mtime = mtime;
        Ok(())
    }

    // Get the effective block number of a segment
    pub fn get_valid_blocks(&self, segno: u32) -> Option<u16> {
        self.entries.get(segno as usize).map(|e| e.valid_blocks())
    }

    // Get segment type
    pub fn get_seg_type(&self, segno: u32) -> Option<u16> {
        self.entries.get(segno as usize).map(|e| e.seg_type())
    }

    // Get SIT entry
    pub fn get_entry(&self, segno: u32) -> Option<&SitEntry> {
        self.entries.get(segno as usize)
    }

    // Get the total number of segments
    pub fn segment_count(&self) -> u32 {
        self.entries.len() as u32
    }

    // Get the SIT area starting block address
    pub fn sit_blkaddr(&self) -> u32 {
        self.sit_blkaddr
    }

    // Calculate the number of blocks required for the SIT area
    pub fn sit_blocks_needed(&self) -> u32 {
        let entries_per_block = F2FS_BLKSIZE / SIT_ENTRY_SIZE;
        (self.entries.len() as u32).div_ceil(entries_per_block as u32)
    }

    // Serialize SIT region to writer
    pub fn write_to<W: Write>(&self, writer: &mut W) -> Result<()> {
        let entries_per_block = F2FS_BLKSIZE / SIT_ENTRY_SIZE;
        let mut block_buf = vec![0u8; F2FS_BLKSIZE];

        for (i, entry) in self.entries.iter().enumerate() {
            let entry_idx = i % entries_per_block;

            let entry_bytes = entry.to_bytes();
            let offset = entry_idx * SIT_ENTRY_SIZE;
            block_buf[offset..offset + SIT_ENTRY_SIZE].copy_from_slice(&entry_bytes);

            // Write to block when block is full or last entry
            if entry_idx == entries_per_block - 1 || i == self.entries.len() - 1 {
                writer.write_all(&block_buf)?;
                block_buf.fill(0);
            }
        }

        Ok(())
    }

    // Generate byte data of SIT area
    pub fn to_bytes(&self) -> Vec<u8> {
        let entries_per_block = F2FS_BLKSIZE / SIT_ENTRY_SIZE;
        let blocks_needed = self.sit_blocks_needed() as usize;
        let mut data = vec![0u8; blocks_needed * F2FS_BLKSIZE];

        for (i, entry) in self.entries.iter().enumerate() {
            let block_idx = i / entries_per_block;
            let entry_idx = i % entries_per_block;

            let entry_bytes = entry.to_bytes();
            let offset = block_idx * F2FS_BLKSIZE + entry_idx * SIT_ENTRY_SIZE;
            data[offset..offset + SIT_ENTRY_SIZE].copy_from_slice(&entry_bytes);
        }

        data
    }

    // Generate SIT bitmap (for checkpointing)
    pub fn generate_bitmap(&self) -> Vec<u8> {
        // SIT bitmap marks which SIT blocks are valid
        // Each bit corresponds to a SIT block
        let blocks_needed = self.sit_blocks_needed();
        let bitmap_size = (blocks_needed as usize).div_ceil(8);
        let mut bitmap = vec![0u8; bitmap_size];

        // Mark all used SIT blocks
        for i in 0..blocks_needed {
            let byte_idx = i as usize / 8;
            let bit_idx = i as usize % 8;
            bitmap[byte_idx] |= 1 << bit_idx;
        }

        bitmap
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sit_manager_new() {
        let manager = SitManager::new(100, 1024, 2048);
        assert_eq!(manager.segment_count(), 100);
        assert_eq!(manager.sit_blkaddr(), 1024);
    }

    #[test]
    fn test_mark_block_used() {
        let mut manager = SitManager::new(10, 1024, 2048);

        // Mark the first block of the first segment
        manager.mark_block_used(2048, 0).unwrap();
        assert_eq!(manager.get_valid_blocks(0), Some(1));

        // Mark the second block of the first segment
        manager.mark_block_used(2049, 0).unwrap();
        assert_eq!(manager.get_valid_blocks(0), Some(2));
    }

    #[test]
    fn test_sit_entry_serialization() {
        let mut manager = SitManager::new(10, 1024, 2048);

        // mark some blocks
        manager.mark_block_used(2048, 1).unwrap();
        manager.mark_block_used(2049, 1).unwrap();

        let data = manager.to_bytes();
        assert!(!data.is_empty());

        // Verify first entry
        let entry = SitEntry::from_bytes(&data[..SIT_ENTRY_SIZE]).unwrap();
        assert_eq!(entry.valid_blocks(), 2);
        assert_eq!(entry.seg_type(), 1);
    }
}
