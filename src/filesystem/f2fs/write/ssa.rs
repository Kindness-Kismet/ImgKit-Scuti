// F2FS SSA (segment summary area) 管理器
use crate::filesystem::f2fs::consts::*;
//
// 负责管理 segment summary area, 记录每个块的归属信息.

use crate::filesystem::f2fs::Result;
use crate::filesystem::f2fs::error::F2fsError;
use crate::filesystem::f2fs::types::*;
use std::io::Write;

// 每个 summary 块中的条目数量 (F2FS 定义为 F2FS_BLKSIZE / 8 = 512)
const ENTRIES_IN_SUM: usize = F2FS_BLKSIZE / 8;

// summary footer 大小
const SUM_FOOTER_SIZE_CONST: usize = 5;

// summary 类型
const SUM_TYPE_NODE: u8 = 1;
const SUM_TYPE_DATA: u8 = 0;

// SSA 管理器
#[derive(Debug)]
pub struct SsaManager {
    // 每个 segment 的 summary 条目
    summaries: Vec<Vec<Summary>>,
    // 每个 segment 的类型 (node/data)
    seg_types: Vec<u8>,
    // SSA 区域起始块地址
    ssa_blkaddr: u32,
    // 每个 segment 的块数量
    blocks_per_seg: u32,
    // main 区域起始块地址
    main_blkaddr: u32,
}

impl SsaManager {
    // 创建新的 SSA 管理器
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

    // 设置数据块的 summary 信息
    pub fn set_data_summary(&mut self, blkaddr: u32, nid: u32, ofs_in_node: u16) -> Result<()> {
        let segno = self
            .get_segno(blkaddr)
            .ok_or_else(|| F2fsError::InvalidData("invalid blkaddr".into()))?;
        let blkoff = self.get_blkoff(blkaddr) as usize;

        if segno as usize >= self.summaries.len() {
            return Ok(()); // 超出范围, 忽略
        }

        self.summaries[segno as usize][blkoff] = Summary {
            nid,
            version: 0,
            ofs_in_node,
        };
        self.seg_types[segno as usize] = SUM_TYPE_DATA;

        Ok(())
    }

    // 设置 node 块的 summary 信息
    pub fn set_node_summary(&mut self, blkaddr: u32, nid: u32) -> Result<()> {
        let segno = self
            .get_segno(blkaddr)
            .ok_or_else(|| F2fsError::InvalidData("invalid blkaddr".into()))?;
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

    // 设置 segment 类型
    pub fn set_seg_type(&mut self, segno: u32, is_node: bool) {
        if (segno as usize) < self.seg_types.len() {
            self.seg_types[segno as usize] = if is_node {
                SUM_TYPE_NODE
            } else {
                SUM_TYPE_DATA
            };
        }
    }

    // 获取指定 segment 与偏移处的 summary 条目
    pub fn get_summary_entry(&self, segno: usize, blkoff: usize) -> Option<&Summary> {
        if segno < self.summaries.len() && blkoff < self.summaries[segno].len() {
            Some(&self.summaries[segno][blkoff])
        } else {
            None
        }
    }

    // 获取 SSA 区域起始块地址
    pub fn ssa_blkaddr(&self) -> u32 {
        self.ssa_blkaddr
    }

    // 计算 SSA 区域所需的块数量
    pub fn ssa_blocks_needed(&self) -> u32 {
        // 每个 segment 需要一个 summary 块
        self.summaries.len() as u32
    }

    // 构建单个 segment 的 summary 块
    fn build_summary_block(&self, segno: usize) -> [u8; F2FS_BLKSIZE] {
        let mut buf = [0u8; F2FS_BLKSIZE];

        // 写入 summary 条目 (每条 7 字节, 无填充)
        let entries = &self.summaries[segno];
        for (i, entry) in entries.iter().take(ENTRIES_IN_SUM).enumerate() {
            let entry_bytes = entry.to_bytes();
            let offset = i * SUMMARY_SIZE; // 按 7 字节对齐
            buf[offset..offset + SUMMARY_SIZE].copy_from_slice(&entry_bytes);
        }

        // 写入 footer
        let footer_offset = F2FS_BLKSIZE - SUM_FOOTER_SIZE_CONST;
        buf[footer_offset] = self.seg_types[segno]; // entry_type

        // 计算校验和
        let checksum = crc32(&buf[..footer_offset + 1]);
        buf[footer_offset + 1..footer_offset + 5].copy_from_slice(&checksum.to_le_bytes());

        buf
    }

    // 将 SSA 区域序列化到 writer
    pub fn write_to<W: Write>(&self, writer: &mut W) -> Result<()> {
        for segno in 0..self.summaries.len() {
            let block = self.build_summary_block(segno);
            writer.write_all(&block)?;
        }
        Ok(())
    }

    // 生成 SSA 区域的字节数据
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut data = Vec::with_capacity(self.summaries.len() * F2FS_BLKSIZE);
        for segno in 0..self.summaries.len() {
            let block = self.build_summary_block(segno);
            data.extend_from_slice(&block);
        }
        data
    }

    // 为 checkpoint pack 构建当前 segment 的 summary 块
    // 设置 CP_UMOUNT_FLAG 时, fsck 会从 checkpoint pack 中读取 SSA 数据
    pub fn build_curseg_summary(&self, segno: usize, is_node: bool) -> Result<[u8; F2FS_BLKSIZE]> {
        let mut buf = [0u8; F2FS_BLKSIZE];

        if segno >= self.summaries.len() {
            // 段号超出范围, 返回空的 summary 块
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

        // 写入 summary 条目 (每条 7 字节, 无填充)
        let entries = &self.summaries[segno];
        for (i, entry) in entries.iter().take(ENTRIES_IN_SUM).enumerate() {
            let entry_bytes = entry.to_bytes();
            let offset = i * SUMMARY_SIZE; // 按 7 字节对齐
            buf[offset..offset + SUMMARY_SIZE].copy_from_slice(&entry_bytes);
        }

        // 写入 footer
        let footer_offset = F2FS_BLKSIZE - SUM_FOOTER_SIZE_CONST;
        buf[footer_offset] = if is_node {
            SUM_TYPE_NODE
        } else {
            SUM_TYPE_DATA
        };

        // 计算校验和
        let checksum = crc32(&buf[..footer_offset + 1]);
        buf[footer_offset + 1..footer_offset + 5].copy_from_slice(&checksum.to_le_bytes());

        Ok(buf)
    }
}

// CRC32 计算 (F2FS 使用 F2FS_MAGIC 作为初始值)
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

        // 设置第一个 segment 的第一个块
        manager.set_data_summary(2048, 100, 5).unwrap();

        let data = manager.to_bytes();
        assert!(!data.is_empty());

        // 校验第一个 summary 条目
        let nid = u32::from_le_bytes([data[0], data[1], data[2], data[3]]);
        assert_eq!(nid, 100);
    }

    #[test]
    fn test_set_node_summary() {
        let mut manager = SsaManager::new(10, 1024, 2048);

        manager.set_node_summary(2048, 200).unwrap();

        let data = manager.to_bytes();

        // 校验 footer 中的类型
        let footer_offset = F2FS_BLKSIZE - SUM_FOOTER_SIZE_CONST;
        assert_eq!(data[footer_offset], SUM_TYPE_NODE);
    }
}
