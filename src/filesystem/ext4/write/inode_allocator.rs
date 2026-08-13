// EXT4 inode 分配器

use std::collections::HashSet;

// 保留的 inode 号
pub const EXT4_ROOT_INO: u32 = 2;
pub const EXT4_FIRST_INO: u32 = 11;

// inode 分配器
pub struct InodeAllocator {
    // inode 总数
    total_inodes: u32,
    // 每个 block group 的 inode 数
    inodes_per_group: u32,
    // 下一个可用的 inode
    next_inode: u32,
    // 已分配的 inode
    allocated_inodes: HashSet<u32>,
    // 每个 block group 的 bitmap
    bitmaps: Vec<Vec<u8>>,
}

impl InodeAllocator {
    // 创建新的 inode 分配器
    pub fn new(total_inodes: u32, inodes_per_group: u32) -> Self {
        let group_count = total_inodes.div_ceil(inodes_per_group);

        // 初始化每个 block group 的 bitmap
        let bitmap_size = (inodes_per_group as usize).div_ceil(8);
        let bitmaps = vec![vec![0u8; bitmap_size]; group_count as usize];

        let mut allocator = InodeAllocator {
            total_inodes,
            inodes_per_group,
            next_inode: EXT4_FIRST_INO,
            allocated_inodes: HashSet::new(),
            bitmaps,
        };

        // 预留前 11 个 inode
        for i in 1..EXT4_FIRST_INO {
            allocator.allocated_inodes.insert(i);
            allocator.mark_inode_used(i);
        }

        allocator
    }

    // 分配一个 inode
    pub fn alloc_inode(&mut self) -> Option<u32> {
        if self.next_inode > self.total_inodes {
            return None;
        }

        let ino = self.next_inode;
        self.next_inode += 1;
        self.allocated_inodes.insert(ino);
        self.mark_inode_used(ino);
        Some(ino)
    }

    // 分配根目录 inode
    pub fn alloc_root_inode(&mut self) -> u32 {
        self.allocated_inodes.insert(EXT4_ROOT_INO);
        self.mark_inode_used(EXT4_ROOT_INO);
        EXT4_ROOT_INO
    }

    // 将 inode 标记为已使用
    fn mark_inode_used(&mut self, ino: u32) {
        let group_idx = ((ino - 1) / self.inodes_per_group) as usize;
        let inode_in_group = ((ino - 1) % self.inodes_per_group) as usize;
        let byte_idx = inode_in_group / 8;
        let bit_idx = inode_in_group % 8;

        if group_idx < self.bitmaps.len() && byte_idx < self.bitmaps[group_idx].len() {
            self.bitmaps[group_idx][byte_idx] |= 1 << bit_idx;
        }
    }

    // 获取指定 block group 的 bitmap
    pub fn get_bitmap(&self, group_idx: u32) -> &[u8] {
        &self.bitmaps[group_idx as usize]
    }

    // 获取已分配的 inode 数
    pub fn allocated_count(&self) -> u32 {
        self.allocated_inodes.len() as u32
    }

    // 获取空闲 inode 数
    pub fn free_count(&self) -> u32 {
        self.total_inodes - self.allocated_count()
    }

    // 获取指定 block group 中的空闲 inode 数
    pub fn get_free_inodes_in_group(&self, group_idx: u32) -> u32 {
        let group_start = group_idx * self.inodes_per_group + 1;
        let group_end = group_start + self.inodes_per_group;
        let mut free_count = 0;

        for ino in group_start..group_end {
            if ino > self.total_inodes {
                break;
            }
            if !self.allocated_inodes.contains(&ino) {
                free_count += 1;
            }
        }

        free_count
    }

    // 计算 inode 所在的 block group
    pub fn inode_group(&self, ino: u32) -> u32 {
        (ino - 1) / self.inodes_per_group
    }

    // 计算 inode 在 block group 内的索引
    pub fn inode_index_in_group(&self, ino: u32) -> u32 {
        (ino - 1) % self.inodes_per_group
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_inode_allocator() {
        let mut allocator = InodeAllocator::new(1000, 100);

        let ino1 = allocator.alloc_inode().unwrap();
        assert_eq!(ino1, EXT4_FIRST_INO);

        let ino2 = allocator.alloc_inode().unwrap();
        assert_eq!(ino2, EXT4_FIRST_INO + 1);
    }

    #[test]
    fn test_root_inode() {
        let mut allocator = InodeAllocator::new(1000, 100);

        let root = allocator.alloc_root_inode();
        assert_eq!(root, EXT4_ROOT_INO);
    }
}
