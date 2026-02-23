use crate::filesystem::f2fs::*;
use crate::filesystem::f2fs::{F2fsError, Result};
use std::collections::HashMap;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::sync::{Arc, RwLock};

pub struct F2fsVolume {
    file: Arc<RwLock<File>>,
    pub superblock: Superblock,
    nat_cache: Arc<RwLock<HashMap<Nid, NatEntry>>>,
}

impl F2fsVolume {
    pub fn new(path: &str) -> Result<Self> {
        let file = File::open(path)?;
        let file = Arc::new(RwLock::new(file));

        // Read superblock
        let mut buf = vec![0u8; F2FS_BLKSIZE];
        {
            let mut f = file
                .write()
                .map_err(|e| F2fsError::LockError(format!("文件锁写入失败: {}", e)))?;
            f.seek(SeekFrom::Start(F2FS_SUPER_OFFSET))?;
            f.read_exact(&mut buf)?;
        }
        let superblock = Superblock::from_bytes(&buf)?;

        Ok(F2fsVolume {
            file,
            superblock,
            nat_cache: Arc::new(RwLock::new(HashMap::new())),
        })
    }

    pub fn read_block(&self, block: Block) -> Result<Vec<u8>> {
        let mut buf = vec![0u8; F2FS_BLKSIZE];
        let offset = block.0 as u64 * F2FS_BLKSIZE as u64;

        let mut file = self
            .file
            .write()
            .map_err(|e| F2fsError::LockError(format!("文件锁写入失败: {}", e)))?;
        file.seek(SeekFrom::Start(offset))?;
        file.read_exact(&mut buf)?;

        Ok(buf)
    }

    pub fn read_node(&self, nid: Nid) -> Result<Vec<u8>> {
        let nat_entry = self.get_nat_entry(nid)?;
        self.read_block(nat_entry.block_addr)
    }

    fn get_nat_entry(&self, nid: Nid) -> Result<NatEntry> {
        // Check cache first
        {
            let cache = self
                .nat_cache
                .read()
                .map_err(|e| F2fsError::LockError(format!("NAT 缓存读取失败: {}", e)))?;
            if let Some(entry) = cache.get(&nid) {
                return Ok(entry.clone());
            }
        }

        // Read NAT block
        let nat_block_idx = nid.0 / NAT_ENTRY_PER_BLOCK as u32;
        let entry_idx = (nid.0 % NAT_ENTRY_PER_BLOCK as u32) as usize;
        let nat_block = Block(self.superblock.nat_blkaddr + nat_block_idx);

        let data = self.read_block(nat_block)?;
        let entry_data = &data[entry_idx * 9..(entry_idx + 1) * 9];
        let entry = NatEntry::from_bytes(entry_data)?;

        // cache
        {
            let mut cache = self
                .nat_cache
                .write()
                .map_err(|e| F2fsError::LockError(format!("NAT 缓存写入失败: {}", e)))?;
            cache.insert(nid, entry.clone());
        }

        Ok(entry)
    }

    pub fn is_valid_block(&self, block: Block) -> bool {
        block.0 >= self.superblock.main_blkaddr && block.0 < self.superblock.block_count as u32
    }
}
