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
    nat_journal_cache: Arc<RwLock<HashMap<Nid, NatEntry>>>,
    nat_blocks_per_copy: u32,
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
        let log_blocks_per_seg = u32::from_le_bytes([buf[20], buf[21], buf[22], buf[23]]);
        let blocks_per_seg = 1u32
            .checked_shl(log_blocks_per_seg)
            .ok_or_else(|| F2fsError::InvalidData("无效的 log_blocks_per_seg".into()))?;
        let segment_count_nat = u32::from_le_bytes([buf[60], buf[61], buf[62], buf[63]]);
        let nat_blocks_per_copy = (segment_count_nat / 2).saturating_mul(blocks_per_seg);
        let nat_journal_cache = Arc::new(RwLock::new(
            Self::read_nat_journal_entries(&file, &superblock, blocks_per_seg).unwrap_or_default(),
        ));

        Ok(F2fsVolume {
            file,
            superblock,
            nat_cache: Arc::new(RwLock::new(HashMap::new())),
            nat_journal_cache,
            nat_blocks_per_copy,
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
        {
            let cache = self
                .nat_journal_cache
                .read()
                .map_err(|e| F2fsError::LockError(format!("NAT journal 缓存读取失败: {}", e)))?;
            if let Some(entry) = cache.get(&nid) {
                let entry = entry.clone();
                let mut nat_cache = self
                    .nat_cache
                    .write()
                    .map_err(|e| F2fsError::LockError(format!("NAT 缓存写入失败: {}", e)))?;
                nat_cache.insert(nid, entry.clone());
                return Ok(entry);
            }
        }

        // Read NAT block
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
            return Err(F2fsError::InvalidData("NAT 条目读取越界".into()));
        }
        NatEntry::from_bytes(&data[entry_offset..entry_end])
    }

    fn read_nat_journal_entries(
        file: &Arc<RwLock<File>>,
        superblock: &Superblock,
        blocks_per_seg: u32,
    ) -> Result<HashMap<Nid, NatEntry>> {
        let cp_pack_1 = superblock.cp_blkaddr;
        let cp_pack_2 = superblock.cp_blkaddr + blocks_per_seg;

        let cp1 = Self::read_block_from_file(file, Block(cp_pack_1))?;
        let cp2 = Self::read_block_from_file(file, Block(cp_pack_2))?;
        let cp1_ver = u64::from_le_bytes([
            cp1[0], cp1[1], cp1[2], cp1[3], cp1[4], cp1[5], cp1[6], cp1[7],
        ]);
        let cp2_ver = u64::from_le_bytes([
            cp2[0], cp2[1], cp2[2], cp2[3], cp2[4], cp2[5], cp2[6], cp2[7],
        ]);

        let latest_cp = if cp2_ver > cp1_ver { cp2 } else { cp1 };
        let cp_base = if cp2_ver > cp1_ver {
            cp_pack_2
        } else {
            cp_pack_1
        };
        let cp_pack_start_sum = u32::from_le_bytes([
            latest_cp[140],
            latest_cp[141],
            latest_cp[142],
            latest_cp[143],
        ]);

        let compact_sum = Self::read_block_from_file(file, Block(cp_base + cp_pack_start_sum))?;
        if compact_sum.len() < 2 {
            return Err(F2fsError::InvalidData("checkpoint summary 数据太短".into()));
        }

        let mut journal = HashMap::new();
        let n_nats = u16::from_le_bytes([compact_sum[0], compact_sum[1]]) as usize;
        let nat_journal_entry_size = 4 + NAT_ENTRY_SIZE;
        let max_nats = (SUM_JOURNAL_SIZE.saturating_sub(2)) / nat_journal_entry_size;
        let n_nats = n_nats.min(max_nats);

        for i in 0..n_nats {
            let offset = 2 + i * nat_journal_entry_size;
            let end = offset + nat_journal_entry_size;
            if end > compact_sum.len() {
                break;
            }

            let nid = Nid(u32::from_le_bytes([
                compact_sum[offset],
                compact_sum[offset + 1],
                compact_sum[offset + 2],
                compact_sum[offset + 3],
            ]));
            if nid.0 == 0 {
                continue;
            }

            let nat = NatEntry::from_bytes(&compact_sum[offset + 4..end])?;
            if nat.block_addr.0 != 0 {
                journal.insert(nid, nat);
            }
        }

        Ok(journal)
    }

    fn read_block_from_file(file: &Arc<RwLock<File>>, block: Block) -> Result<Vec<u8>> {
        let mut buf = vec![0u8; F2FS_BLKSIZE];
        let offset = block.0 as u64 * F2FS_BLKSIZE as u64;
        let mut f = file
            .write()
            .map_err(|e| F2fsError::LockError(format!("文件锁写入失败: {}", e)))?;
        f.seek(SeekFrom::Start(offset))?;
        f.read_exact(&mut buf)?;
        Ok(buf)
    }

    pub fn is_valid_block(&self, block: Block) -> bool {
        block.0 >= self.superblock.main_blkaddr && block.0 < self.superblock.block_count as u32
    }
}
