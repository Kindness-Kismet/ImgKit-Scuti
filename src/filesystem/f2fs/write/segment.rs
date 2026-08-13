// F2FS segment 分配器
use crate::filesystem::f2fs::consts::*;
//
// 负责管理 segment 与块的分配.

use crate::filesystem::f2fs::types::*;
use crate::filesystem::f2fs::{F2fsError, Result};

// segment 分配器
#[derive(Debug)]
pub struct SegmentAllocator {
    // 当前 segment 号 (每种类型各一个)
    current_segments: [u32; NR_CURSEG_TYPE],
    // 当前 segment 内的下一个块偏移
    next_blkoff: [u16; NR_CURSEG_TYPE],
    // main 区域起始块地址
    main_blkaddr: u32,
    // 每个 segment 的块数量
    blocks_per_seg: u32,
    // segment 总数
    total_segments: u32,
    // 已分配的块数量
    allocated_blocks: u64,
    // 已使用的 segment 集合 (跟踪所有已分配的 segment)
    used_segments: std::collections::HashSet<u32>,
}

impl SegmentAllocator {
    // 创建新的 segment 分配器
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

        // 初始化当前 segment
        // 为每种类型分配不同的起始 segment
        for i in 0..NR_CURSEG_TYPE {
            allocator.current_segments[i] = i as u32;
            // 将初始 segment 标记为已使用
            allocator.used_segments.insert(i as u32);
        }

        allocator
    }

    // 设置当前 segment
    pub fn set_current_segment(&mut self, seg_type: SegType, segno: u32, blkoff: u16) {
        let idx = seg_type as usize;
        if idx < NR_CURSEG_TYPE {
            self.current_segments[idx] = segno;
            self.next_blkoff[idx] = blkoff;
        }
    }

    // 获取当前 segment 号
    pub fn current_segno(&self, seg_type: SegType) -> u32 {
        self.current_segments[seg_type as usize]
    }

    // 获取当前块偏移
    pub fn current_blkoff(&self, seg_type: SegType) -> u16 {
        self.next_blkoff[seg_type as usize]
    }

    // 分配数据块
    pub fn alloc_data_block(&mut self, seg_type: SegType) -> Result<u32> {
        if !seg_type.is_data() {
            return Err(F2fsError::InvalidData(
                "segment type is not a data type".into(),
            ));
        }
        self.alloc_block(seg_type)
    }

    // 分配 node 块
    pub fn alloc_node_block(&mut self, seg_type: SegType) -> Result<u32> {
        if !seg_type.is_node() {
            return Err(F2fsError::InvalidData(
                "segment type is not a node type".into(),
            ));
        }
        self.alloc_block(seg_type)
    }

    // 分配块 (内部方法)
    fn alloc_block(&mut self, seg_type: SegType) -> Result<u32> {
        let idx = seg_type as usize;

        // 检查是否需要切换到新的 segment
        if self.next_blkoff[idx] >= self.blocks_per_seg as u16 {
            self.allocate_new_segment(seg_type)?;
        }

        // 计算块地址
        let segno = self.current_segments[idx];
        let blkoff = self.next_blkoff[idx];
        let blkaddr = self.main_blkaddr + segno * self.blocks_per_seg + blkoff as u32;

        // 更新偏移
        self.next_blkoff[idx] += 1;
        self.allocated_blocks += 1;

        Ok(blkaddr)
    }

    // 分配新的 segment
    fn allocate_new_segment(&mut self, seg_type: SegType) -> Result<()> {
        let idx = seg_type as usize;

        // 查找下一个可用的 segment
        // 从当前 segment 的下一个开始查找
        let mut next_segno = self.current_segments[idx] + 1;

        // 确保不越界
        if next_segno >= self.total_segments {
            return Err(F2fsError::InvalidData("no available segment".into()));
        }

        // 查找未使用的 segment
        loop {
            // 检查是否已被使用
            if !self.used_segments.contains(&next_segno) {
                break;
            }

            next_segno += 1;
            if next_segno >= self.total_segments {
                return Err(F2fsError::InvalidData("no available segment".into()));
            }
        }

        self.current_segments[idx] = next_segno;
        self.next_blkoff[idx] = 0;
        // 将新 segment 标记为已使用
        self.used_segments.insert(next_segno);

        Ok(())
    }

    // 获取已分配的块数量
    pub fn allocated_blocks(&self) -> u64 {
        self.allocated_blocks
    }

    // 获取空闲 segment 数量
    pub fn free_segments(&self) -> u32 {
        // 空闲 segment 数量 = segment 总数 - 已使用 segment 数量
        // 已使用 segment 数量即 used_segments 集合的大小
        self.total_segments
            .saturating_sub(self.used_segments.len() as u32)
    }

    // 获取 main 区域起始块地址
    pub fn main_blkaddr(&self) -> u32 {
        self.main_blkaddr
    }

    // 获取 segment 总数
    pub fn total_segments(&self) -> u32 {
        self.total_segments
    }

    // 获取每个 segment 的块数量
    pub fn blocks_per_seg(&self) -> u32 {
        self.blocks_per_seg
    }

    // 将块地址转换为段号
    pub fn blkaddr_to_segno(&self, blkaddr: u32) -> Option<u32> {
        if blkaddr < self.main_blkaddr {
            return None;
        }
        Some((blkaddr - self.main_blkaddr) / self.blocks_per_seg)
    }

    // 将块地址转换为 segment 内偏移
    pub fn blkaddr_to_blkoff(&self, blkaddr: u32) -> u32 {
        (blkaddr - self.main_blkaddr) % self.blocks_per_seg
    }

    // 获取当前 segment 信息 (供 checkpoint 使用)
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

// 当前 segment 信息
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
        // HotNode 是类型 4, 因此起始 segment 为 3
        assert_eq!(blk1, 1024 + 3 * 512);

        let blk2 = allocator.alloc_node_block(SegType::HotNode).unwrap();
        assert_eq!(blk2, 1024 + 3 * 512 + 1);
    }

    #[test]
    fn test_segment_switch() {
        let mut allocator = SegmentAllocator::new(0, 100);

        // 分配满一个 segment
        for _ in 0..DEFAULT_BLOCKS_PER_SEGMENT {
            allocator.alloc_data_block(SegType::HotData).unwrap();
        }

        // 下一个块应落在新的 segment 中
        let blk = allocator.alloc_data_block(SegType::HotData).unwrap();
        // 新的段号应为 0 + NR_CURSEG_TYPE = 6
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
