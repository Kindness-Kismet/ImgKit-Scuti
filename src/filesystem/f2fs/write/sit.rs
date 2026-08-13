// F2FS SIT (segment info table) 管理器
use crate::filesystem::f2fs::consts::*;
//
// 负责管理 segment info table, 跟踪每个 segment 的 valid block count 与类型.

use crate::filesystem::f2fs::types::*;
use crate::filesystem::f2fs::{F2fsError, Result};
use std::io::Write;

// SIT 管理器
#[derive(Debug)]
pub struct SitManager {
    // SIT 条目列表
    entries: Vec<SitEntry>,
    // SIT 区域起始块地址
    sit_blkaddr: u32,
    // 每个 segment 的块数量
    blocks_per_seg: u32,
    // main 区域起始块地址
    main_blkaddr: u32,
}

impl SitManager {
    // 创建新的 SIT 管理器
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

    // 获取段号
    fn get_segno(&self, blkaddr: u32) -> Option<u32> {
        if blkaddr < self.main_blkaddr {
            return None;
        }
        Some((blkaddr - self.main_blkaddr) / self.blocks_per_seg)
    }

    // 获取块在 segment 内的偏移
    fn get_blkoff(&self, blkaddr: u32) -> u32 {
        (blkaddr - self.main_blkaddr) % self.blocks_per_seg
    }

    // 将块标记为已使用
    pub fn mark_block_used(&mut self, blkaddr: u32, seg_type: u16) -> Result<()> {
        let segno = self
            .get_segno(blkaddr)
            .ok_or_else(|| F2fsError::InvalidData(format!("invalid blkaddr: {}", blkaddr)))?;

        if segno as usize >= self.entries.len() {
            return Err(F2fsError::InvalidData(format!(
                "segment number out of range: {} >= {}",
                segno,
                self.entries.len()
            )));
        }

        let blkoff = self.get_blkoff(blkaddr) as usize;
        let entry = &mut self.entries[segno as usize];

        // 将块标记为有效
        entry.mark_block_valid(blkoff);

        // 更新 valid block count 与 segment 类型
        let valid_blocks = entry.valid_blocks() + 1;
        entry.set_vblocks(valid_blocks, seg_type);

        Ok(())
    }

    // 批量将块标记为已使用
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

    // 设置 segment 类型
    pub fn set_seg_type(&mut self, segno: u32, seg_type: u16) -> Result<()> {
        if segno as usize >= self.entries.len() {
            return Err(F2fsError::InvalidData(format!(
                "segment number out of range: {} >= {}",
                segno,
                self.entries.len()
            )));
        }

        let entry = &mut self.entries[segno as usize];
        let valid_blocks = entry.valid_blocks();
        entry.set_vblocks(valid_blocks, seg_type);
        Ok(())
    }

    // 设置 segment 的修改时间
    pub fn set_mtime(&mut self, segno: u32, mtime: u64) -> Result<()> {
        if segno as usize >= self.entries.len() {
            return Err(F2fsError::InvalidData(format!(
                "segment number out of range: {} >= {}",
                segno,
                self.entries.len()
            )));
        }

        self.entries[segno as usize].mtime = mtime;
        Ok(())
    }

    // 获取 segment 的 valid block count
    pub fn get_valid_blocks(&self, segno: u32) -> Option<u16> {
        self.entries.get(segno as usize).map(|e| e.valid_blocks())
    }

    // 获取 segment 类型
    pub fn get_seg_type(&self, segno: u32) -> Option<u16> {
        self.entries.get(segno as usize).map(|e| e.seg_type())
    }

    // 获取 SIT 条目
    pub fn get_entry(&self, segno: u32) -> Option<&SitEntry> {
        self.entries.get(segno as usize)
    }

    // 获取 segment 总数
    pub fn segment_count(&self) -> u32 {
        self.entries.len() as u32
    }

    // 获取 SIT 区域起始块地址
    pub fn sit_blkaddr(&self) -> u32 {
        self.sit_blkaddr
    }

    // 计算 SIT 区域所需的块数量
    pub fn sit_blocks_needed(&self) -> u32 {
        let entries_per_block = F2FS_BLKSIZE / SIT_ENTRY_SIZE;
        (self.entries.len() as u32).div_ceil(entries_per_block as u32)
    }

    // 将 SIT 区域序列化到 writer
    pub fn write_to<W: Write>(&self, writer: &mut W) -> Result<()> {
        let entries_per_block = F2FS_BLKSIZE / SIT_ENTRY_SIZE;
        let mut block_buf = vec![0u8; F2FS_BLKSIZE];

        for (i, entry) in self.entries.iter().enumerate() {
            let entry_idx = i % entries_per_block;

            let entry_bytes = entry.to_bytes();
            let offset = entry_idx * SIT_ENTRY_SIZE;
            block_buf[offset..offset + SIT_ENTRY_SIZE].copy_from_slice(&entry_bytes);

            // 块填满或写到最后一个条目时落盘
            if entry_idx == entries_per_block - 1 || i == self.entries.len() - 1 {
                writer.write_all(&block_buf)?;
                block_buf.fill(0);
            }
        }

        Ok(())
    }

    // 生成 SIT 区域的字节数据
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

    // 生成 SIT bitmap (供 checkpoint 使用)
    pub fn generate_bitmap(&self) -> Vec<u8> {
        // SIT bitmap 标记哪些 SIT 块有效
        // 每个 bit 对应一个 SIT 块
        let blocks_needed = self.sit_blocks_needed();
        let bitmap_size = (blocks_needed as usize).div_ceil(8);
        let mut bitmap = vec![0u8; bitmap_size];

        // 标记所有已使用的 SIT 块
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

        // 标记第一个 segment 的第一个块
        manager.mark_block_used(2048, 0).unwrap();
        assert_eq!(manager.get_valid_blocks(0), Some(1));

        // 标记第一个 segment 的第二个块
        manager.mark_block_used(2049, 0).unwrap();
        assert_eq!(manager.get_valid_blocks(0), Some(2));
    }

    #[test]
    fn test_sit_entry_serialization() {
        let mut manager = SitManager::new(10, 1024, 2048);

        // 标记若干块
        manager.mark_block_used(2048, 1).unwrap();
        manager.mark_block_used(2049, 1).unwrap();

        let data = manager.to_bytes();
        assert!(!data.is_empty());

        // 校验第一个条目
        let entry = SitEntry::from_bytes(&data[..SIT_ENTRY_SIZE]).unwrap();
        assert_eq!(entry.valid_blocks(), 2);
        assert_eq!(entry.seg_type(), 1);
    }
}
