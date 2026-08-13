// F2FS checkpoint 构建器
use crate::filesystem::f2fs::consts::*;
//
// 负责构建 F2FS checkpoint 结构.

use crate::filesystem::f2fs::Result;

// checkpoint 结构大小
const CHECKPOINT_SIZE: usize = 192;

// 活动日志的最大数量
const MAX_ACTIVE_LOGS: usize = 16;
const MAX_ACTIVE_NODE_LOGS: usize = 8;
const MAX_ACTIVE_DATA_LOGS: usize = 8;

// checkpoint 构建器
#[derive(Debug)]
pub struct CheckpointBuilder {
    // checkpoint 版本
    checkpoint_ver: u64,
    // 用户块数量
    user_block_count: u64,
    // 有效块计数
    valid_block_count: u64,
    // 保留 segment 数量
    rsvd_segment_count: u32,
    // 超额预留 segment 数量
    overprov_segment_count: u32,
    // 空闲 segment 数量
    free_segment_count: u32,
    // 当前 node segment 号
    cur_node_segno: [u32; MAX_ACTIVE_NODE_LOGS],
    // 当前 node 块偏移
    cur_node_blkoff: [u16; MAX_ACTIVE_NODE_LOGS],
    // 当前 data segment 号
    cur_data_segno: [u32; MAX_ACTIVE_DATA_LOGS],
    // 当前 data 块偏移
    cur_data_blkoff: [u16; MAX_ACTIVE_DATA_LOGS],
    // checkpoint 标志位
    ckpt_flags: u32,
    // checkpoint pack 的总块数
    cp_pack_total_block_count: u32,
    // data summary 起始块号
    cp_pack_start_sum: u32,
    // 有效 node 数量
    valid_node_count: u32,
    // 有效 inode 数量
    valid_inode_count: u32,
    // 下一个空闲 nid
    next_free_nid: u32,
    // SIT 版本 bitmap 大小
    sit_ver_bitmap_bytesize: u32,
    // NAT 版本 bitmap 大小
    nat_ver_bitmap_bytesize: u32,
    // 已运行时间
    elapsed_time: u64,
    // 分配类型
    alloc_type: [u8; MAX_ACTIVE_LOGS],
    // SIT 位图
    sit_bitmap: Vec<u8>,
    // NAT 位图
    nat_bitmap: Vec<u8>,
}

impl CheckpointBuilder {
    // 创建新的 checkpoint 构建器
    pub fn new() -> Self {
        CheckpointBuilder {
            checkpoint_ver: 1,
            user_block_count: 0,
            valid_block_count: 0,
            rsvd_segment_count: 0,
            overprov_segment_count: 0,
            free_segment_count: 0,
            cur_node_segno: [0; MAX_ACTIVE_NODE_LOGS],
            cur_node_blkoff: [0; MAX_ACTIVE_NODE_LOGS],
            cur_data_segno: [0; MAX_ACTIVE_DATA_LOGS],
            cur_data_blkoff: [0; MAX_ACTIVE_DATA_LOGS],
            ckpt_flags: CP_UMOUNT_FLAG,
            cp_pack_total_block_count: 2, // 默认 2 块
            cp_pack_start_sum: 1,
            valid_node_count: 0,
            valid_inode_count: 0,
            next_free_nid: F2FS_FIRST_INO,
            sit_ver_bitmap_bytesize: 0,
            nat_ver_bitmap_bytesize: 0,
            elapsed_time: 0,
            alloc_type: [0; MAX_ACTIVE_LOGS],
            sit_bitmap: Vec::new(),
            nat_bitmap: Vec::new(),
        }
    }

    // 设置 checkpoint 版本
    pub fn with_version(mut self, ver: u64) -> Self {
        self.checkpoint_ver = ver;
        self
    }

    // 设置用户块数量
    pub fn with_user_block_count(mut self, count: u64) -> Self {
        self.user_block_count = count;
        self
    }

    // 设置 valid block count
    pub fn with_valid_block_count(mut self, count: u64) -> Self {
        self.valid_block_count = count;
        self
    }

    // 设置空闲 segment 数量
    pub fn with_free_segment_count(mut self, count: u32) -> Self {
        self.free_segment_count = count;
        self
    }

    // 设置保留 segment 数量
    pub fn with_rsvd_segment_count(mut self, count: u32) -> Self {
        self.rsvd_segment_count = count;
        self
    }

    // 设置超额预留 segment 数量
    pub fn with_overprov_segment_count(mut self, count: u32) -> Self {
        self.overprov_segment_count = count;
        self
    }

    // 设置 checkpoint 标志位
    pub fn with_flags(mut self, flags: u32) -> Self {
        self.ckpt_flags = flags;
        self
    }

    // 设置有效 node 数量
    pub fn with_valid_node_count(mut self, count: u32) -> Self {
        self.valid_node_count = count;
        self
    }

    // 设置有效 inode 数量
    pub fn with_valid_inode_count(mut self, count: u32) -> Self {
        self.valid_inode_count = count;
        self
    }

    // 设置下一个空闲 nid
    pub fn with_next_free_nid(mut self, nid: u32) -> Self {
        self.next_free_nid = nid;
        self
    }

    // 设置当前 node segment
    pub fn set_cur_node_seg(&mut self, idx: usize, segno: u32, blkoff: u16) {
        if idx < MAX_ACTIVE_NODE_LOGS {
            self.cur_node_segno[idx] = segno;
            self.cur_node_blkoff[idx] = blkoff;
        }
    }

    // 设置当前 data segment
    pub fn set_cur_data_seg(&mut self, idx: usize, segno: u32, blkoff: u16) {
        if idx < MAX_ACTIVE_DATA_LOGS {
            self.cur_data_segno[idx] = segno;
            self.cur_data_blkoff[idx] = blkoff;
        }
    }

    // 设置 SIT bitmap
    pub fn with_sit_bitmap(mut self, bitmap: Vec<u8>) -> Self {
        self.sit_ver_bitmap_bytesize = bitmap.len() as u32;
        self.sit_bitmap = bitmap;
        self
    }

    // 设置 NAT bitmap
    pub fn with_nat_bitmap(mut self, bitmap: Vec<u8>) -> Self {
        self.nat_ver_bitmap_bytesize = bitmap.len() as u32;
        self.nat_bitmap = bitmap;
        self
    }

    // 设置 checkpoint pack 的总块数
    pub fn with_cp_pack_total_block_count(mut self, count: u32) -> Self {
        self.cp_pack_total_block_count = count;
        self
    }

    // 构建 checkpoint 字节数据
    pub fn build(&self) -> Result<Vec<u8>> {
        // checkpoint 块大小
        let mut buf = vec![0u8; F2FS_BLKSIZE];

        // checkpoint_ver (偏移 0)
        buf[0..8].copy_from_slice(&self.checkpoint_ver.to_le_bytes());

        // user_block_count (偏移 8)
        buf[8..16].copy_from_slice(&self.user_block_count.to_le_bytes());

        // valid_block_count (偏移 16)
        buf[16..24].copy_from_slice(&self.valid_block_count.to_le_bytes());

        // rsvd_segment_count (偏移 24)
        buf[24..28].copy_from_slice(&self.rsvd_segment_count.to_le_bytes());

        // overprov_segment_count (偏移 28)
        buf[28..32].copy_from_slice(&self.overprov_segment_count.to_le_bytes());

        // free_segment_count (偏移 32)
        buf[32..36].copy_from_slice(&self.free_segment_count.to_le_bytes());

        // cur_node_segno (偏移 36, 8 个条目共 32 字节)
        for (i, &segno) in self.cur_node_segno.iter().enumerate() {
            let offset = 36 + i * 4;
            buf[offset..offset + 4].copy_from_slice(&segno.to_le_bytes());
        }

        // cur_node_blkoff (偏移 68, 8 个条目共 16 字节)
        for (i, &blkoff) in self.cur_node_blkoff.iter().enumerate() {
            let offset = 68 + i * 2;
            buf[offset..offset + 2].copy_from_slice(&blkoff.to_le_bytes());
        }

        // cur_data_segno (偏移 84, 8 个条目共 32 字节)
        for (i, &segno) in self.cur_data_segno.iter().enumerate() {
            let offset = 84 + i * 4;
            buf[offset..offset + 4].copy_from_slice(&segno.to_le_bytes());
        }

        // cur_data_blkoff (偏移 116, 8 个条目共 16 字节)
        for (i, &blkoff) in self.cur_data_blkoff.iter().enumerate() {
            let offset = 116 + i * 2;
            buf[offset..offset + 2].copy_from_slice(&blkoff.to_le_bytes());
        }

        // ckpt_flags (偏移 132)
        buf[132..136].copy_from_slice(&self.ckpt_flags.to_le_bytes());

        // cp_pack_total_block_count (偏移 136)
        buf[136..140].copy_from_slice(&self.cp_pack_total_block_count.to_le_bytes());

        // cp_pack_start_sum (偏移 140)
        buf[140..144].copy_from_slice(&self.cp_pack_start_sum.to_le_bytes());

        // valid_node_count (偏移 144)
        buf[144..148].copy_from_slice(&self.valid_node_count.to_le_bytes());

        // valid_inode_count (偏移 148)
        buf[148..152].copy_from_slice(&self.valid_inode_count.to_le_bytes());

        // next_free_nid (偏移 152)
        buf[152..156].copy_from_slice(&self.next_free_nid.to_le_bytes());

        // sit_ver_bitmap_bytesize (偏移 156)
        buf[156..160].copy_from_slice(&self.sit_ver_bitmap_bytesize.to_le_bytes());

        // nat_ver_bitmap_bytesize (偏移 160)
        buf[160..164].copy_from_slice(&self.nat_ver_bitmap_bytesize.to_le_bytes());

        // checksum_offset (偏移 164)
        let checksum_offset = CP_CHKSUM_OFFSET as u32;
        buf[164..168].copy_from_slice(&checksum_offset.to_le_bytes());

        // elapsed_time (偏移 168)
        buf[168..176].copy_from_slice(&self.elapsed_time.to_le_bytes());

        // alloc_type (偏移 176, 16 字节)
        buf[176..192].copy_from_slice(&self.alloc_type);

        // sit_nat_version_bitmap (偏移 192)
        let bitmap_offset = CHECKPOINT_SIZE;
        let sit_bitmap_end = bitmap_offset + self.sit_bitmap.len();
        if sit_bitmap_end <= F2FS_BLKSIZE - 4 {
            buf[bitmap_offset..sit_bitmap_end].copy_from_slice(&self.sit_bitmap);
        }

        let nat_bitmap_start = sit_bitmap_end;
        let nat_bitmap_end = nat_bitmap_start + self.nat_bitmap.len();
        if nat_bitmap_end <= F2FS_BLKSIZE - 4 {
            buf[nat_bitmap_start..nat_bitmap_end].copy_from_slice(&self.nat_bitmap);
        }

        // 计算并写入 CRC (偏移 CP_CHKSUM_OFFSET)
        let crc = crc32(&buf[..CP_CHKSUM_OFFSET]);
        buf[CP_CHKSUM_OFFSET..CP_CHKSUM_OFFSET + 4].copy_from_slice(&crc.to_le_bytes());

        Ok(buf)
    }

    // 获取 checkpoint 结构大小
    pub fn checkpoint_size() -> usize {
        CHECKPOINT_SIZE
    }
}

impl Default for CheckpointBuilder {
    fn default() -> Self {
        Self::new()
    }
}

// CRC32 计算 (F2FS 使用 F2FS_SUPER_MAGIC 作为初始值)
fn crc32(data: &[u8]) -> u32 {
    let mut crc = F2FS_MAGIC; // F2FS 使用 magic number 作为 CRC 初始值
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
    crc // F2FS 不对最终结果取反
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_checkpoint_builder_new() {
        let builder = CheckpointBuilder::new();
        assert_eq!(builder.checkpoint_ver, 1);
        assert_eq!(builder.ckpt_flags, CP_UMOUNT_FLAG);
    }

    #[test]
    fn test_checkpoint_build() {
        let builder = CheckpointBuilder::new()
            .with_version(1)
            .with_user_block_count(1000)
            .with_valid_block_count(100)
            .with_free_segment_count(10)
            .with_valid_node_count(5)
            .with_valid_inode_count(3);

        let data = builder.build().unwrap();
        assert_eq!(data.len(), F2FS_BLKSIZE);

        // 校验版本
        let ver = u64::from_le_bytes([
            data[0], data[1], data[2], data[3], data[4], data[5], data[6], data[7],
        ]);
        assert_eq!(ver, 1);

        // 校验用户块数量
        let user_blocks = u64::from_le_bytes([
            data[8], data[9], data[10], data[11], data[12], data[13], data[14], data[15],
        ]);
        assert_eq!(user_blocks, 1000);
    }

    #[test]
    fn test_checkpoint_with_bitmap() {
        let sit_bitmap = vec![0xFF; 8];
        let nat_bitmap = vec![0xAA; 16];

        let builder = CheckpointBuilder::new()
            .with_sit_bitmap(sit_bitmap.clone())
            .with_nat_bitmap(nat_bitmap.clone());

        let data = builder.build().unwrap();

        // 校验 bitmap 大小
        let sit_size = u32::from_le_bytes([data[156], data[157], data[158], data[159]]);
        assert_eq!(sit_size, 8);

        let nat_size = u32::from_le_bytes([data[160], data[161], data[162], data[163]]);
        assert_eq!(nat_size, 16);
    }
}
