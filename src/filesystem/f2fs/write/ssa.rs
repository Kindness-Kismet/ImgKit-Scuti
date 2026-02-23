// F2FS SSA (Segment Summary Area) Manager
use crate::filesystem::f2fs::consts::*;
//
// Responsible for managing the segment summary area and recording the ownership information of each block.

use crate::filesystem::f2fs::Result;
use crate::filesystem::f2fs::types::*;
use std::io::Write;

// Number of entries in each digest block (F2FS defined as F2FS_BLKSIZE / 8 = 512)
const ENTRIES_IN_SUM: usize = F2FS_BLKSIZE / 8;

// Summary footer size
const SUM_FOOTER_SIZE_CONST: usize = 5;

// summary type
const SUM_TYPE_NODE: u8 = 1;
const SUM_TYPE_DATA: u8 = 0;

// SSA Manager
#[derive(Debug)]
pub struct SsaManager {
    // Summary entry for each segment
    summaries: Vec<Vec<Summary>>,
    // Type of each segment (node/data)
    seg_types: Vec<u8>,
    // SSA area starting block address
    ssa_blkaddr: u32,
    // Number of blocks per segment
    blocks_per_seg: u32,
    // Main area starting block address
    main_blkaddr: u32,
}

impl SsaManager {
    // Create a new SSA manager
    pub fn new(segment_count: u32, ssa_blkaddr: u32, main_blkaddr: u32) -> Self {
        let mut summaries = Vec::with_capacity(segment_count as usize);
        let mut seg_types = Vec::with_capacity(segment_count as usize);

        for _ in 0..segment_count {
            summaries.push(vec![
                Summary::default();
                DEFAULT_BLOCKS_PER_SEGMENT as usize
            ]);
            seg_types.push(SUM_TYPE_DATA);
        }

        SsaManager {
            summaries,
            seg_types,
            ssa_blkaddr,
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

    // Set the summary information of the data block
    pub fn set_data_summary(&mut self, blkaddr: u32, nid: u32, ofs_in_node: u16) -> Result<()> {
        let segno = self.get_segno(blkaddr).ok_or_else(|| {
            crate::filesystem::f2fs::error::F2fsError::InvalidData("无效的块地址".into())
        })?;
        let blkoff = self.get_blkoff(blkaddr) as usize;

        if segno as usize >= self.summaries.len() {
            return Ok(()); // out of range, ignored
        }

        self.summaries[segno as usize][blkoff] = Summary {
            nid,
            version: 0,
            ofs_in_node,
        };
        self.seg_types[segno as usize] = SUM_TYPE_DATA;

        Ok(())
    }

    // Set summary information for node blocks
    pub fn set_node_summary(&mut self, blkaddr: u32, nid: u32) -> Result<()> {
        let segno = self.get_segno(blkaddr).ok_or_else(|| {
            crate::filesystem::f2fs::error::F2fsError::InvalidData("无效的块地址".into())
        })?;
        let blkoff = self.get_blkoff(blkaddr) as usize;

        if segno as usize >= self.summaries.len() {
            return Ok(());
        }

        self.summaries[segno as usize][blkoff] = Summary {
            nid,
            version: 0,
            ofs_in_node: 0,
        };
        self.seg_types[segno as usize] = SUM_TYPE_NODE;

        Ok(())
    }

    // Set segment type
    pub fn set_seg_type(&mut self, segno: u32, is_node: bool) {
        if (segno as usize) < self.seg_types.len() {
            self.seg_types[segno as usize] = if is_node {
                SUM_TYPE_NODE
            } else {
                SUM_TYPE_DATA
            };
        }
    }

    // Get the summary entry for the specified segment and offset
    pub fn get_summary_entry(&self, segno: usize, blkoff: usize) -> Option<&Summary> {
        if segno < self.summaries.len() && blkoff < self.summaries[segno].len() {
            Some(&self.summaries[segno][blkoff])
        } else {
            None
        }
    }

    // Get SSA area starting block address
    pub fn ssa_blkaddr(&self) -> u32 {
        self.ssa_blkaddr
    }

    // Calculate the number of blocks required for the SSA region
    pub fn ssa_blocks_needed(&self) -> u32 {
        // Each segment requires a summary block
        self.summaries.len() as u32
    }

    // Build a summary block of a single segment
    fn build_summary_block(&self, segno: usize) -> [u8; F2FS_BLKSIZE] {
        let mut buf = [0u8; F2FS_BLKSIZE];

        // Write summary entries (7 bytes per entry, no padding)
        let entries = &self.summaries[segno];
        for (i, entry) in entries.iter().take(ENTRIES_IN_SUM).enumerate() {
            let entry_bytes = entry.to_bytes();
            let offset = i * SUMMARY_SIZE; // 7 byte alignment
            buf[offset..offset + SUMMARY_SIZE].copy_from_slice(&entry_bytes);
        }

        // Write footer
        let footer_offset = F2FS_BLKSIZE - SUM_FOOTER_SIZE_CONST;
        buf[footer_offset] = self.seg_types[segno]; // entry_type

        // Calculate checksum
        let checksum = crc32(&buf[..footer_offset + 1]);
        buf[footer_offset + 1..footer_offset + 5].copy_from_slice(&checksum.to_le_bytes());

        buf
    }

    // Serialize SSA zone to writer
    pub fn write_to<W: Write>(&self, writer: &mut W) -> Result<()> {
        for segno in 0..self.summaries.len() {
            let block = self.build_summary_block(segno);
            writer.write_all(&block)?;
        }
        Ok(())
    }

    // Generate byte data for SSA area
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut data = Vec::with_capacity(self.summaries.len() * F2FS_BLKSIZE);
        for segno in 0..self.summaries.len() {
            let block = self.build_summary_block(segno);
            data.extend_from_slice(&block);
        }
        data
    }

    // Build a summary block of the current segment for the checkpoint package
    // When CP_UMOUNT_FLAG is set, fsck reads SSA data from checkpoint packets
    pub fn build_curseg_summary(&self, segno: usize, is_node: bool) -> Result<[u8; F2FS_BLKSIZE]> {
        let mut buf = [0u8; F2FS_BLKSIZE];

        if segno >= self.summaries.len() {
            // Segment number out of range, return empty summary block
            let footer_offset = F2FS_BLKSIZE - SUM_FOOTER_SIZE_CONST;
            buf[footer_offset] = if is_node {
                SUM_TYPE_NODE
            } else {
                SUM_TYPE_DATA
            };
            let checksum = crc32(&buf[..footer_offset + 1]);
            buf[footer_offset + 1..footer_offset + 5].copy_from_slice(&checksum.to_le_bytes());
            return Ok(buf);
        }

        // Write summary entries (7 bytes per entry, no padding)
        let entries = &self.summaries[segno];
        for (i, entry) in entries.iter().take(ENTRIES_IN_SUM).enumerate() {
            let entry_bytes = entry.to_bytes();
            let offset = i * SUMMARY_SIZE; // 7 byte alignment
            buf[offset..offset + SUMMARY_SIZE].copy_from_slice(&entry_bytes);
        }

        // Write footer
        let footer_offset = F2FS_BLKSIZE - SUM_FOOTER_SIZE_CONST;
        buf[footer_offset] = if is_node {
            SUM_TYPE_NODE
        } else {
            SUM_TYPE_DATA
        };

        // Calculate checksum
        let checksum = crc32(&buf[..footer_offset + 1]);
        buf[footer_offset + 1..footer_offset + 5].copy_from_slice(&checksum.to_le_bytes());

        Ok(buf)
    }
}

// CRC32 calculation (F2FS uses F2FS_MAGIC as initial value)
fn crc32(data: &[u8]) -> u32 {
    let mut crc = F2FS_MAGIC;
    for &byte in data {
        crc ^= byte as u32;
        for _ in 0..8 {
            if crc & 1 != 0 {
                crc = (crc >> 1) ^ 0xEDB88320;
            } else {
                crc >>= 1;
            }
        }
    }
    crc
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ssa_manager_new() {
        let manager = SsaManager::new(10, 1024, 2048);
        assert_eq!(manager.ssa_blkaddr(), 1024);
        assert_eq!(manager.ssa_blocks_needed(), 10);
    }

    #[test]
    fn test_set_data_summary() {
        let mut manager = SsaManager::new(10, 1024, 2048);

        // Set the first block of the first segment
        manager.set_data_summary(2048, 100, 5).unwrap();

        let data = manager.to_bytes();
        assert!(!data.is_empty());

        // Verify first digest entry
        let nid = u32::from_le_bytes([data[0], data[1], data[2], data[3]]);
        assert_eq!(nid, 100);
    }

    #[test]
    fn test_set_node_summary() {
        let mut manager = SsaManager::new(10, 1024, 2048);

        manager.set_node_summary(2048, 200).unwrap();

        let data = manager.to_bytes();

        // Validate types in footer
        let footer_offset = F2FS_BLKSIZE - SUM_FOOTER_SIZE_CONST;
        assert_eq!(data[footer_offset], SUM_TYPE_NODE);
    }
}
