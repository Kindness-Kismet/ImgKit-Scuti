// F2FS checkpoint builder
use crate::filesystem::f2fs::consts::*;
//
// Responsible for building the F2FS checkpoint structure.

use crate::filesystem::f2fs::Result;

// Checkpoint size
const CHECKPOINT_SIZE: usize = 192;

// Maximum number of activity logs
const MAX_ACTIVE_LOGS: usize = 16;
const MAX_ACTIVE_NODE_LOGS: usize = 8;
const MAX_ACTIVE_DATA_LOGS: usize = 8;

// Checkpoint builder
#[derive(Debug)]
pub struct CheckpointBuilder {
    // checkpoint version
    checkpoint_ver: u64,
    // Number of user blocks
    user_block_count: u64,
    // Number of valid blocks
    valid_block_count: u64,
    // Number of reserved segments
    rsvd_segment_count: u32,
    // Over provisioning of segments
    overprov_segment_count: u32,
    // Number of free segments
    free_segment_count: u32,
    // Current node segment number
    cur_node_segno: [u32; MAX_ACTIVE_NODE_LOGS],
    // Current node block offset
    cur_node_blkoff: [u16; MAX_ACTIVE_NODE_LOGS],
    // Current data segment number
    cur_data_segno: [u32; MAX_ACTIVE_DATA_LOGS],
    // Current data block offset
    cur_data_blkoff: [u16; MAX_ACTIVE_DATA_LOGS],
    // checkpoint flag
    ckpt_flags: u32,
    // Checkpoint package total number of blocks
    cp_pack_total_block_count: u32,
    // Data summary starting block number
    cp_pack_start_sum: u32,
    // Number of valid nodes
    valid_node_count: u32,
    // Number of valid inodes
    valid_inode_count: u32,
    // Next free NID
    next_free_nid: u32,
    // SIT version bitmap size
    sit_ver_bitmap_bytesize: u32,
    // NAT version bitmap size
    nat_ver_bitmap_bytesize: u32,
    // elapsed time
    elapsed_time: u64,
    // allocation type
    alloc_type: [u8; MAX_ACTIVE_LOGS],
    // SIT bitmap
    sit_bitmap: Vec<u8>,
    // NAT bitmap
    nat_bitmap: Vec<u8>,
}

impl CheckpointBuilder {
    // Create a new checkpoint builder
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
            cp_pack_total_block_count: 2, // Default 2 blocks
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

    // Set checkpoint version
    pub fn with_version(mut self, ver: u64) -> Self {
        self.checkpoint_ver = ver;
        self
    }

    // Set the number of user blocks
    pub fn with_user_block_count(mut self, count: u64) -> Self {
        self.user_block_count = count;
        self
    }

    // Set the number of valid blocks
    pub fn with_valid_block_count(mut self, count: u64) -> Self {
        self.valid_block_count = count;
        self
    }

    // Set the number of free segments
    pub fn with_free_segment_count(mut self, count: u32) -> Self {
        self.free_segment_count = count;
        self
    }

    // Set the number of reserved segments
    pub fn with_rsvd_segment_count(mut self, count: u32) -> Self {
        self.rsvd_segment_count = count;
        self
    }

    // Set the number of overprovisioned segments
    pub fn with_overprov_segment_count(mut self, count: u32) -> Self {
        self.overprov_segment_count = count;
        self
    }

    // Set checkpoint flag
    pub fn with_flags(mut self, flags: u32) -> Self {
        self.ckpt_flags = flags;
        self
    }

    // Set the number of valid nodes
    pub fn with_valid_node_count(mut self, count: u32) -> Self {
        self.valid_node_count = count;
        self
    }

    // Set the number of valid inodes
    pub fn with_valid_inode_count(mut self, count: u32) -> Self {
        self.valid_inode_count = count;
        self
    }

    // Set next free NID
    pub fn with_next_free_nid(mut self, nid: u32) -> Self {
        self.next_free_nid = nid;
        self
    }

    // Set current node segment
    pub fn set_cur_node_seg(&mut self, idx: usize, segno: u32, blkoff: u16) {
        if idx < MAX_ACTIVE_NODE_LOGS {
            self.cur_node_segno[idx] = segno;
            self.cur_node_blkoff[idx] = blkoff;
        }
    }

    // Set the current data segment
    pub fn set_cur_data_seg(&mut self, idx: usize, segno: u32, blkoff: u16) {
        if idx < MAX_ACTIVE_DATA_LOGS {
            self.cur_data_segno[idx] = segno;
            self.cur_data_blkoff[idx] = blkoff;
        }
    }

    // Set SIT bitmap
    pub fn with_sit_bitmap(mut self, bitmap: Vec<u8>) -> Self {
        self.sit_ver_bitmap_bytesize = bitmap.len() as u32;
        self.sit_bitmap = bitmap;
        self
    }

    // Set NAT bitmap
    pub fn with_nat_bitmap(mut self, bitmap: Vec<u8>) -> Self {
        self.nat_ver_bitmap_bytesize = bitmap.len() as u32;
        self.nat_bitmap = bitmap;
        self
    }

    // Set the total number of blocks in the checkpoint package
    pub fn with_cp_pack_total_block_count(mut self, count: u32) -> Self {
        self.cp_pack_total_block_count = count;
        self
    }

    // Build checkpoint byte data
    pub fn build(&self) -> Result<Vec<u8>> {
        // Checkpoint block size
        let mut buf = vec![0u8; F2FS_BLKSIZE];

        // checkpoint_ver (offset 0)
        buf[0..8].copy_from_slice(&self.checkpoint_ver.to_le_bytes());

        // user_block_count (offset 8)
        buf[8..16].copy_from_slice(&self.user_block_count.to_le_bytes());

        // valid_block_count (offset 16)
        buf[16..24].copy_from_slice(&self.valid_block_count.to_le_bytes());

        // rsvd_segment_count (offset 24)
        buf[24..28].copy_from_slice(&self.rsvd_segment_count.to_le_bytes());

        // overprov_segment_count (offset 28)
        buf[28..32].copy_from_slice(&self.overprov_segment_count.to_le_bytes());

        // free_segment_count (offset 32)
        buf[32..36].copy_from_slice(&self.free_segment_count.to_le_bytes());

        // cur_node_segno (offset 36, 32 bytes for 8 entries)
        for (i, &segno) in self.cur_node_segno.iter().enumerate() {
            let offset = 36 + i * 4;
            buf[offset..offset + 4].copy_from_slice(&segno.to_le_bytes());
        }

        // cur_node_blkoff (offset 68, 16 bytes for 8 entries)
        for (i, &blkoff) in self.cur_node_blkoff.iter().enumerate() {
            let offset = 68 + i * 2;
            buf[offset..offset + 2].copy_from_slice(&blkoff.to_le_bytes());
        }

        // cur_data_segno (offset 84, 32 bytes for 8 entries)
        for (i, &segno) in self.cur_data_segno.iter().enumerate() {
            let offset = 84 + i * 4;
            buf[offset..offset + 4].copy_from_slice(&segno.to_le_bytes());
        }

        // cur_data_blkoff (offset 116, 16 bytes for 8 entries)
        for (i, &blkoff) in self.cur_data_blkoff.iter().enumerate() {
            let offset = 116 + i * 2;
            buf[offset..offset + 2].copy_from_slice(&blkoff.to_le_bytes());
        }

        // ckpt_flags (offset 132)
        buf[132..136].copy_from_slice(&self.ckpt_flags.to_le_bytes());

        // cp_pack_total_block_count (offset 136)
        buf[136..140].copy_from_slice(&self.cp_pack_total_block_count.to_le_bytes());

        // cp_pack_start_sum (offset 140)
        buf[140..144].copy_from_slice(&self.cp_pack_start_sum.to_le_bytes());

        // valid_node_count (offset 144)
        buf[144..148].copy_from_slice(&self.valid_node_count.to_le_bytes());

        // valid_inode_count (offset 148)
        buf[148..152].copy_from_slice(&self.valid_inode_count.to_le_bytes());

        // next_free_nid (offset 152)
        buf[152..156].copy_from_slice(&self.next_free_nid.to_le_bytes());

        // sit_ver_bitmap_bytesize (offset 156)
        buf[156..160].copy_from_slice(&self.sit_ver_bitmap_bytesize.to_le_bytes());

        // nat_ver_bitmap_bytesize (offset 160)
        buf[160..164].copy_from_slice(&self.nat_ver_bitmap_bytesize.to_le_bytes());

        // checksum_offset (offset 164)
        let checksum_offset = CP_CHKSUM_OFFSET as u32;
        buf[164..168].copy_from_slice(&checksum_offset.to_le_bytes());

        // elapsed_time (offset 168)
        buf[168..176].copy_from_slice(&self.elapsed_time.to_le_bytes());

        // alloc_type (offset 176, 16 bytes)
        buf[176..192].copy_from_slice(&self.alloc_type);

        // sit_nat_version_bitmap (offset 192)
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

        // Calculate and set CRC (offset CP_CHKSUM_OFFSET)
        let crc = crc32(&buf[..CP_CHKSUM_OFFSET]);
        buf[CP_CHKSUM_OFFSET..CP_CHKSUM_OFFSET + 4].copy_from_slice(&crc.to_le_bytes());

        Ok(buf)
    }

    // Get checkpoint size
    pub fn checkpoint_size() -> usize {
        CHECKPOINT_SIZE
    }
}

impl Default for CheckpointBuilder {
    fn default() -> Self {
        Self::new()
    }
}

// CRC32 calculation (F2FS uses F2FS_SUPER_MAGIC as initial value)
fn crc32(data: &[u8]) -> u32 {
    let mut crc = F2FS_MAGIC; // F2FS uses magic numbers as CRC initial values
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
    crc // F2FS does not invert the final result
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

        // Verify version
        let ver = u64::from_le_bytes([
            data[0], data[1], data[2], data[3], data[4], data[5], data[6], data[7],
        ]);
        assert_eq!(ver, 1);

        // Verify user block number
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

        // Verify bitmap size
        let sit_size = u32::from_le_bytes([data[156], data[157], data[158], data[159]]);
        assert_eq!(sit_size, 8);

        let nat_size = u32::from_le_bytes([data[160], data[161], data[162], data[163]]);
        assert_eq!(nat_size, 16);
    }
}
