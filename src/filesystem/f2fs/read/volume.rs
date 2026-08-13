use crate::filesystem::f2fs::*;
use crate::filesystem::f2fs::{F2fsError, Result};
use std::collections::HashMap;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::sync::{Arc, RwLock};

// 通用读取后端, 支持任意 Read+Seek 数据源 (File, SparseReader 等)
pub struct F2fsVolume<R: Read + Seek + Send = File> {
    file: Arc<RwLock<R>>,
    pub superblock: Superblock,
    nat_cache: Arc<RwLock<HashMap<Nid, NatEntry>>>,
    nat_journal_cache: Arc<RwLock<HashMap<Nid, NatEntry>>>,
    nat_blocks_per_copy: u32,
}

impl F2fsVolume<File> {
    pub fn new(path: &str) -> Result<Self> {
        let file = File::open(path)?;
        Self::from_reader(file)
    }
}

impl<R: Read + Seek + Send> F2fsVolume<R> {
    // 从任意 Read+Seek 流构造 (普通文件或 sparse 虚拟流)
    pub fn from_reader(reader: R) -> Result<Self> {
        let file = Arc::new(RwLock::new(reader));

        // 读取 superblock
        let mut buf = vec![0u8; F2FS_BLKSIZE];
        {
            let mut f = file
                .write()
                .map_err(|e| F2fsError::LockError(format!("file write lock poisoned: {}", e)))?;
            f.seek(SeekFrom::Start(F2FS_SUPER_OFFSET))?;
            f.read_exact(&mut buf)?;
        }
        let superblock = Superblock::from_bytes(&buf)?;
        let log_blocks_per_seg = u32::from_le_bytes([buf[20], buf[21], buf[22], buf[23]]);
        let blocks_per_seg = 1u32
            .checked_shl(log_blocks_per_seg)
            .ok_or_else(|| F2fsError::InvalidData("invalid log_blocks_per_seg".into()))?;
        let segment_count_nat = u32::from_le_bytes([buf[60], buf[61], buf[62], buf[63]]);
        let nat_blocks_per_copy = (segment_count_nat / 2).saturating_mul(blocks_per_seg);

        let cp_primary = Self::read_block_raw(&file, Block(superblock.cp_blkaddr))?;
        let cp_secondary =
            Self::read_block_raw(&file, Block(superblock.cp_blkaddr + blocks_per_seg))?;
        let cp_primary_ver = Self::read_le_u64(&cp_primary, 0)?;
        let cp_secondary_ver = Self::read_le_u64(&cp_secondary, 0)?;
        let (active_cp_blkaddr, active_cp) = if cp_secondary_ver > cp_primary_ver {
            (superblock.cp_blkaddr + blocks_per_seg, cp_secondary)
        } else {
            (superblock.cp_blkaddr, cp_primary)
        };
        let nat_journal = Self::load_nat_journal(&file, active_cp_blkaddr, &active_cp)?;
        let nat_journal_cache = Arc::new(RwLock::new(nat_journal));

        Ok(F2fsVolume {
            file,
            superblock,
            nat_cache: Arc::new(RwLock::new(HashMap::new())),
            nat_journal_cache,
            nat_blocks_per_copy,
        })
    }

    pub fn read_block(&self, block: Block) -> Result<Vec<u8>> {
        Self::read_block_raw(&self.file, block)
    }

    fn read_block_raw(file: &Arc<RwLock<R>>, block: Block) -> Result<Vec<u8>> {
        let mut buf = vec![0u8; F2FS_BLKSIZE];
        let offset = block.0 as u64 * F2FS_BLKSIZE as u64;

        let mut file = file
            .write()
            .map_err(|e| F2fsError::LockError(format!("file write lock poisoned: {}", e)))?;
        file.seek(SeekFrom::Start(offset))?;
        file.read_exact(&mut buf)?;

        Ok(buf)
    }

    fn read_le_u32(data: &[u8], offset: usize) -> Result<u32> {
        if offset + 4 > data.len() {
            return Err(F2fsError::InvalidData("u32 read out of bounds".into()));
        }
        Ok(u32::from_le_bytes([
            data[offset],
            data[offset + 1],
            data[offset + 2],
            data[offset + 3],
        ]))
    }

    fn read_le_u64(data: &[u8], offset: usize) -> Result<u64> {
        if offset + 8 > data.len() {
            return Err(F2fsError::InvalidData("u64 read out of bounds".into()));
        }
        Ok(u64::from_le_bytes([
            data[offset],
            data[offset + 1],
            data[offset + 2],
            data[offset + 3],
            data[offset + 4],
            data[offset + 5],
            data[offset + 6],
            data[offset + 7],
        ]))
    }

    fn load_nat_journal(
        file: &Arc<RwLock<R>>,
        cp_blkaddr: u32,
        cp_header: &[u8],
    ) -> Result<HashMap<Nid, NatEntry>> {
        let mut nat_journal = HashMap::new();

        let ckpt_flags = Self::read_le_u32(cp_header, 132)?;
        if ckpt_flags & CP_COMPACT_SUM_FLAG == 0 {
            return Ok(nat_journal);
        }

        let cp_pack_total_block_count = Self::read_le_u32(cp_header, 136)?;
        let cp_pack_start_sum = Self::read_le_u32(cp_header, 140)?;
        if cp_pack_start_sum == 0 || cp_pack_start_sum >= cp_pack_total_block_count {
            return Ok(nat_journal);
        }

        let compact_sum = Self::read_block_raw(file, Block(cp_blkaddr + cp_pack_start_sum))?;
        if compact_sum.len() < SUM_JOURNAL_SIZE {
            return Ok(nat_journal);
        }

        // compact summary 前 507 字节是 NAT journal
        let nat_count = u16::from_le_bytes([compact_sum[0], compact_sum[1]]) as usize;
        let entry_size = 13usize; // nid(4) + nat_entry(9)
        for index in 0..nat_count {
            let offset = 2 + index * entry_size;
            if offset + entry_size > SUM_JOURNAL_SIZE {
                break;
            }

            let nid = Self::read_le_u32(&compact_sum, offset)?;
            let version = compact_sum[offset + 4];
            let ino = Self::read_le_u32(&compact_sum, offset + 5)?;
            let block_addr = Self::read_le_u32(&compact_sum, offset + 9)?;
            if block_addr == 0 {
                continue;
            }

            nat_journal.insert(
                Nid(nid),
                NatEntry {
                    version,
                    ino,
                    block_addr: Block(block_addr),
                },
            );
        }

        Ok(nat_journal)
    }

    pub fn read_node(&self, nid: Nid) -> Result<Vec<u8>> {
        let nat_entry = self.get_nat_entry(nid)?;
        self.read_block(nat_entry.block_addr)
    }

    fn get_nat_entry(&self, nid: Nid) -> Result<NatEntry> {
        // 优先查询缓存
        {
            let cache = self
                .nat_cache
                .read()
                .map_err(|e| F2fsError::LockError(format!("NAT cache lock poisoned: {}", e)))?;
            if let Some(entry) = cache.get(&nid) {
                return Ok(entry.clone());
            }
        }
        {
            let cache = self
                .nat_journal_cache
                .read()
                .map_err(|e| F2fsError::LockError(format!("NAT journal lock poisoned: {}", e)))?;
            if let Some(entry) = cache.get(&nid) {
                let entry = entry.clone();
                let mut nat_cache = self
                    .nat_cache
                    .write()
                    .map_err(|e| F2fsError::LockError(format!("NAT cache lock poisoned: {}", e)))?;
                nat_cache.insert(nid, entry.clone());
                return Ok(entry);
            }
        }

        // 读取 NAT block
        let nat_block_idx = nid.0 / NAT_ENTRY_PER_BLOCK as u32;
        let entry_idx = (nid.0 % NAT_ENTRY_PER_BLOCK as u32) as usize;
        let mut entry =
            self.read_nat_entry_from_copy(self.superblock.nat_blkaddr, nat_block_idx, entry_idx)?;
        if (entry.block_addr.0 == 0 || !self.is_valid_block(entry.block_addr))
            && self.nat_blocks_per_copy > 0
            && let Ok(secondary) = self.read_nat_entry_from_copy(
                self.superblock.nat_blkaddr + self.nat_blocks_per_copy,
                nat_block_idx,
                entry_idx,
            )
            && secondary.block_addr.0 != 0
            && self.is_valid_block(secondary.block_addr)
        {
            entry = secondary;
        }

        // 写入缓存
        {
            let mut cache = self
                .nat_cache
                .write()
                .map_err(|e| F2fsError::LockError(format!("NAT cache lock poisoned: {}", e)))?;
            cache.insert(nid, entry.clone());
        }

        Ok(entry)
    }

    fn read_nat_entry_from_copy(
        &self,
        nat_base_blkaddr: u32,
        nat_block_idx: u32,
        entry_idx: usize,
    ) -> Result<NatEntry> {
        let nat_block = Block(nat_base_blkaddr + nat_block_idx);
        let data = self.read_block(nat_block)?;
        let entry_offset = entry_idx * NAT_ENTRY_SIZE;
        let entry_end = entry_offset + NAT_ENTRY_SIZE;
        if entry_end > data.len() {
            return Err(F2fsError::InvalidData(
                "NAT entry read out of bounds".into(),
            ));
        }
        NatEntry::from_bytes(&data[entry_offset..entry_end])
    }

    pub fn is_valid_block(&self, block: Block) -> bool {
        block.0 >= self.superblock.main_blkaddr && block.0 < self.superblock.block_count as u32
    }
}
