// F2FS NAT (node address table) 管理器
use crate::filesystem::f2fs::consts::*;
//
// 负责管理 node address table, 跟踪每个 NID 对应的块地址.

use crate::filesystem::f2fs::Result;
use crate::filesystem::f2fs::types::*;
use std::collections::HashMap;
use std::io::Write;

// NAT 管理器
#[derive(Debug)]
pub struct NatManager {
    // NAT 条目映射
    entries: HashMap<u32, NatEntry>,
    // 下一个可用的 NID
    next_nid: u32,
    // NAT 区域起始块地址
    nat_blkaddr: u32,
}

impl NatManager {
    // 创建新的 NAT 管理器
    pub fn new(nat_blkaddr: u32, _nat_segments: u32) -> Self {
        NatManager {
            entries: HashMap::new(),
            next_nid: F2FS_FIRST_INO, // 从 4 开始, 0-3 为保留值
            nat_blkaddr,
        }
    }

    // 分配新的 NID
    pub fn alloc_nid(&mut self) -> Nid {
        let nid = self.next_nid;
        self.next_nid += 1;
        Nid(nid)
    }

    // 获取下一个可用的 NID (尚未分配)
    pub fn next_free_nid(&self) -> u32 {
        self.next_nid
    }

    // 设置 NAT 条目
    pub fn set_entry(&mut self, nid: Nid, block_addr: u32, ino: u32) {
        let entry = NatEntry {
            version: 0,
            ino,
            block_addr: Block(block_addr),
        };
        self.entries.insert(nid.0, entry);
    }

    // 获取 NAT 条目
    pub fn get_entry(&self, nid: Nid) -> Option<&NatEntry> {
        self.entries.get(&nid.0)
    }

    // 获取块地址
    pub fn get_block_addr(&self, nid: Nid) -> Option<u32> {
        self.entries.get(&nid.0).map(|e| e.block_addr.0)
    }

    // 检查 NID 是否已分配
    pub fn is_allocated(&self, nid: Nid) -> bool {
        self.entries.contains_key(&nid.0)
    }

    // 获取已分配的条目数量
    pub fn entry_count(&self) -> usize {
        self.entries.len()
    }

    // 获取 NAT 区域起始块地址
    pub fn nat_blkaddr(&self) -> u32 {
        self.nat_blkaddr
    }

    // 计算 NAT 区域所需的块数量
    pub fn nat_blocks_needed(&self) -> u32 {
        // 需要覆盖所有可能的 NID
        let max_nid = self.next_nid;
        (max_nid).div_ceil(NAT_ENTRY_PER_BLOCK as u32)
    }

    // 初始化保留 inode (node_ino, meta_ino, root_ino)
    // node_ino 与 meta_ino 为虚拟 inode, block_addr=1 表示特殊标记
    pub fn init_reserved_inodes(&mut self, root_blkaddr: u32) {
        // node_ino (NID 1) - 虚拟 inode, block_addr=1 表示特殊标记
        self.entries.insert(
            F2FS_NODE_INO,
            NatEntry {
                version: 0,
                ino: F2FS_NODE_INO,
                block_addr: Block(1),
            },
        );

        // meta_ino (NID 2) - 虚拟 inode, block_addr=1 表示特殊标记
        self.entries.insert(
            F2FS_META_INO,
            NatEntry {
                version: 0,
                ino: F2FS_META_INO,
                block_addr: Block(1),
            },
        );

        // root_ino (NID 3) - 根 inode
        self.entries.insert(
            F2FS_ROOT_INO,
            NatEntry {
                version: 0,
                ino: F2FS_ROOT_INO,
                block_addr: Block(root_blkaddr),
            },
        );
    }

    // 将 NAT 区域序列化到 writer
    pub fn write_to<W: Write>(&self, writer: &mut W) -> Result<()> {
        let blocks_needed = self.nat_blocks_needed() as usize;

        for block_idx in 0..blocks_needed {
            let mut block_buf = vec![0u8; F2FS_BLKSIZE];

            for entry_idx in 0..NAT_ENTRY_PER_BLOCK {
                let nid = (block_idx * NAT_ENTRY_PER_BLOCK + entry_idx) as u32;

                if let Some(entry) = self.entries.get(&nid) {
                    let entry_bytes = entry.to_bytes();
                    let offset = entry_idx * NAT_ENTRY_SIZE;
                    block_buf[offset..offset + NAT_ENTRY_SIZE].copy_from_slice(&entry_bytes);
                }
            }

            writer.write_all(&block_buf)?;
        }

        Ok(())
    }

    // 生成 NAT 区域的字节数据
    pub fn to_bytes(&self) -> Vec<u8> {
        let blocks_needed = self.nat_blocks_needed() as usize;
        let mut data = vec![0u8; blocks_needed * F2FS_BLKSIZE];

        for (&nid, entry) in &self.entries {
            let block_idx = nid as usize / NAT_ENTRY_PER_BLOCK;
            let entry_idx = nid as usize % NAT_ENTRY_PER_BLOCK;

            if block_idx < blocks_needed {
                let entry_bytes = entry.to_bytes();
                let offset = block_idx * F2FS_BLKSIZE + entry_idx * NAT_ENTRY_SIZE;
                data[offset..offset + NAT_ENTRY_SIZE].copy_from_slice(&entry_bytes);
            }
        }

        data
    }

    // 生成 NAT bitmap (供 checkpoint 使用)
    pub fn generate_bitmap(&self) -> Vec<u8> {
        // NAT bitmap 标记哪些 NAT 块有效
        let blocks_needed = self.nat_blocks_needed();
        let bitmap_size = (blocks_needed as usize).div_ceil(8);
        let mut bitmap = vec![0u8; bitmap_size];

        // 标记包含有效条目的块
        for &nid in self.entries.keys() {
            let block_idx = nid as usize / NAT_ENTRY_PER_BLOCK;
            let byte_idx = block_idx / 8;
            let bit_idx = block_idx % 8;
            if byte_idx < bitmap.len() {
                bitmap[byte_idx] |= 1 << bit_idx;
            }
        }

        bitmap
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_nat_manager_new() {
        let manager = NatManager::new(1024, 4);
        assert_eq!(manager.nat_blkaddr(), 1024);
        assert_eq!(manager.next_free_nid(), F2FS_FIRST_INO);
    }

    #[test]
    fn test_alloc_nid() {
        let mut manager = NatManager::new(1024, 4);

        let nid1 = manager.alloc_nid();
        assert_eq!(nid1.0, F2FS_FIRST_INO);

        let nid2 = manager.alloc_nid();
        assert_eq!(nid2.0, F2FS_FIRST_INO + 1);
    }

    #[test]
    fn test_set_and_get_entry() {
        let mut manager = NatManager::new(1024, 4);

        let nid = manager.alloc_nid();
        manager.set_entry(nid, 2048, nid.0);

        let entry = manager.get_entry(nid).unwrap();
        assert_eq!(entry.block_addr.0, 2048);
        assert_eq!(entry.ino, nid.0);
    }

    #[test]
    fn test_init_reserved_inodes() {
        let mut manager = NatManager::new(1024, 4);
        manager.init_reserved_inodes(3000);

        // 检查 root_ino
        let root_entry = manager.get_entry(Nid(F2FS_ROOT_INO)).unwrap();
        assert_eq!(root_entry.block_addr.0, 3000);
        assert_eq!(root_entry.ino, F2FS_ROOT_INO);

        // 检查 node_ino
        let node_entry = manager.get_entry(Nid(F2FS_NODE_INO)).unwrap();
        assert_eq!(node_entry.ino, F2FS_NODE_INO);

        // 检查 meta_ino
        let meta_entry = manager.get_entry(Nid(F2FS_META_INO)).unwrap();
        assert_eq!(meta_entry.ino, F2FS_META_INO);
    }

    #[test]
    fn test_nat_serialization() {
        let mut manager = NatManager::new(1024, 4);
        manager.init_reserved_inodes(3000);

        let data = manager.to_bytes();
        assert!(!data.is_empty());

        // 校验 root_ino 条目
        let root_offset = F2FS_ROOT_INO as usize * NAT_ENTRY_SIZE;
        let entry = NatEntry::from_bytes(&data[root_offset..root_offset + NAT_ENTRY_SIZE]).unwrap();
        assert_eq!(entry.block_addr.0, 3000);
    }
}
