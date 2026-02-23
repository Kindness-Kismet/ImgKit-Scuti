// F2FS NAT (Node Address Table) Manager
use crate::filesystem::f2fs::consts::*;
//
// Responsible for managing the node address table and tracking the block address corresponding to each NID.

use crate::filesystem::f2fs::Result;
use crate::filesystem::f2fs::types::*;
use std::collections::HashMap;
use std::io::Write;

// NAT manager
#[derive(Debug)]
pub struct NatManager {
    // NAT entry mapping
    entries: HashMap<u32, NatEntry>,
    // Next available NID
    next_nid: u32,
    // NAT area starting block address
    nat_blkaddr: u32,
}

impl NatManager {
    // Create a new NAT manager
    pub fn new(nat_blkaddr: u32, _nat_segments: u32) -> Self {
        NatManager {
            entries: HashMap::new(),
            next_nid: F2FS_FIRST_INO, // Starting from 4, 0-3 are reserved
            nat_blkaddr,
        }
    }

    // Assign new NID
    pub fn alloc_nid(&mut self) -> Nid {
        let nid = self.next_nid;
        self.next_nid += 1;
        Nid(nid)
    }

    // Get the next available NID (not assigned)
    pub fn next_free_nid(&self) -> u32 {
        self.next_nid
    }

    // Set up NAT entries
    pub fn set_entry(&mut self, nid: Nid, block_addr: u32, ino: u32) {
        let entry = NatEntry {
            version: 0,
            ino,
            block_addr: Block(block_addr),
        };
        self.entries.insert(nid.0, entry);
    }

    // Get NAT entries
    pub fn get_entry(&self, nid: Nid) -> Option<&NatEntry> {
        self.entries.get(&nid.0)
    }

    // Get block address
    pub fn get_block_addr(&self, nid: Nid) -> Option<u32> {
        self.entries.get(&nid.0).map(|e| e.block_addr.0)
    }

    // Check if NID is assigned
    pub fn is_allocated(&self, nid: Nid) -> bool {
        self.entries.contains_key(&nid.0)
    }

    // Get the number of allocated entries
    pub fn entry_count(&self) -> usize {
        self.entries.len()
    }

    // Get NAT area starting block address
    pub fn nat_blkaddr(&self) -> u32 {
        self.nat_blkaddr
    }

    // Calculate the number of blocks required for a NAT zone
    pub fn nat_blocks_needed(&self) -> u32 {
        // Need to cover all possible NIDs
        let max_nid = self.next_nid;
        (max_nid).div_ceil(NAT_ENTRY_PER_BLOCK as u32)
    }

    // Initialize reserved inodes (node_ino, meta_ino, root_ino)
    // node_ino and meta_ino are virtual inodes, block_addr=1 indicates special tags
    pub fn init_reserved_inodes(&mut self, root_blkaddr: u32) {
        // node_ino (NID 1) - virtual inode, block_addr=1 indicates special tag
        self.entries.insert(
            F2FS_NODE_INO,
            NatEntry {
                version: 0,
                ino: F2FS_NODE_INO,
                block_addr: Block(1),
            },
        );

        // meta_ino (NID 2) - virtual inode, block_addr=1 indicates special tag
        self.entries.insert(
            F2FS_META_INO,
            NatEntry {
                version: 0,
                ino: F2FS_META_INO,
                block_addr: Block(1),
            },
        );

        // root_ino (NID 3) - root inode
        self.entries.insert(
            F2FS_ROOT_INO,
            NatEntry {
                version: 0,
                ino: F2FS_ROOT_INO,
                block_addr: Block(root_blkaddr),
            },
        );
    }

    // Serialize NAT zone to writer
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

    // Generate byte data for NAT zone
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

    // Generate NAT bitmap (for checkpointing)
    pub fn generate_bitmap(&self) -> Vec<u8> {
        // NAT bitmap marks which NAT blocks are valid
        let blocks_needed = self.nat_blocks_needed();
        let bitmap_size = (blocks_needed as usize).div_ceil(8);
        let mut bitmap = vec![0u8; bitmap_size];

        // Mark blocks containing valid entries
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

        // Check root_ino
        let root_entry = manager.get_entry(Nid(F2FS_ROOT_INO)).unwrap();
        assert_eq!(root_entry.block_addr.0, 3000);
        assert_eq!(root_entry.ino, F2FS_ROOT_INO);

        // Check node_ino
        let node_entry = manager.get_entry(Nid(F2FS_NODE_INO)).unwrap();
        assert_eq!(node_entry.ino, F2FS_NODE_INO);

        // Check meta_ino
        let meta_entry = manager.get_entry(Nid(F2FS_META_INO)).unwrap();
        assert_eq!(meta_entry.ino, F2FS_META_INO);
    }

    #[test]
    fn test_nat_serialization() {
        let mut manager = NatManager::new(1024, 4);
        manager.init_reserved_inodes(3000);

        let data = manager.to_bytes();
        assert!(!data.is_empty());

        // Verify root_ino entry
        let root_offset = F2FS_ROOT_INO as usize * NAT_ENTRY_SIZE;
        let entry = NatEntry::from_bytes(&data[root_offset..root_offset + NAT_ENTRY_SIZE]).unwrap();
        assert_eq!(entry.block_addr.0, 3000);
    }
}
