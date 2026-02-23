// EXT4 Inode allocator

use std::collections::HashSet;

// Reserved inode number
pub const EXT4_ROOT_INO: u32 = 2;
pub const EXT4_FIRST_INO: u32 = 11;

// Inode allocator
pub struct InodeAllocator {
    // Total number of inodes
    total_inodes: u32,
    // Number of inodes per group
    inodes_per_group: u32,
    // next available inode
    next_inode: u32,
    // allocated inode
    allocated_inodes: HashSet<u32>,
    // bitmap for each block group
    bitmaps: Vec<Vec<u8>>,
}

impl InodeAllocator {
    // Create new inode allocator
    pub fn new(total_inodes: u32, inodes_per_group: u32) -> Self {
        let group_count = total_inodes.div_ceil(inodes_per_group);

        // Initialize bitmap for each block group
        let bitmap_size = (inodes_per_group as usize).div_ceil(8);
        let bitmaps = vec![vec![0u8; bitmap_size]; group_count as usize];

        let mut allocator = InodeAllocator {
            total_inodes,
            inodes_per_group,
            next_inode: EXT4_FIRST_INO,
            allocated_inodes: HashSet::new(),
            bitmaps,
        };

        // Reserve first 11 inodes
        for i in 1..EXT4_FIRST_INO {
            allocator.allocated_inodes.insert(i);
            allocator.mark_inode_used(i);
        }

        allocator
    }

    // allocate an inode
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

    // Allocate root inode
    pub fn alloc_root_inode(&mut self) -> u32 {
        self.allocated_inodes.insert(EXT4_ROOT_INO);
        self.mark_inode_used(EXT4_ROOT_INO);
        EXT4_ROOT_INO
    }

    // Mark inode as used
    fn mark_inode_used(&mut self, ino: u32) {
        let group_idx = ((ino - 1) / self.inodes_per_group) as usize;
        let inode_in_group = ((ino - 1) % self.inodes_per_group) as usize;
        let byte_idx = inode_in_group / 8;
        let bit_idx = inode_in_group % 8;

        if group_idx < self.bitmaps.len() && byte_idx < self.bitmaps[group_idx].len() {
            self.bitmaps[group_idx][byte_idx] |= 1 << bit_idx;
        }
    }

    // Get the bitmap of the block group
    pub fn get_bitmap(&self, group_idx: u32) -> &[u8] {
        &self.bitmaps[group_idx as usize]
    }

    // Get the number of allocated inodes
    pub fn allocated_count(&self) -> u32 {
        self.allocated_inodes.len() as u32
    }

    // Get the number of free inodes
    pub fn free_count(&self) -> u32 {
        self.total_inodes - self.allocated_count()
    }

    // Get the number of free inodes in the block group
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

    // Calculate the block group where the inode is located
    pub fn inode_group(&self, ino: u32) -> u32 {
        (ino - 1) / self.inodes_per_group
    }

    // Calculate the index of the inode within the block group
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
