// F2FS superblock builder
use crate::filesystem::f2fs::consts::*;
//
// Responsible for calculating and building the F2FS superblock structure.

use crate::filesystem::f2fs::types::*;
use crate::filesystem::f2fs::{F2fsError, Result};

// Super block size
const SUPERBLOCK_SIZE: usize = 3072;

// superblock offset
const SUPERBLOCK_OFFSET: u64 = 1024;

// version string
const F2FS_VERSION: &[u8] = b"5.15.0";

// Helper function: calculate log2
fn log_base_2(val: u32) -> u32 {
    if val == 0 {
        return 0;
    }
    31 - val.leading_zeros()
}

// Helper function: align up to segment boundary
fn seg_align(blocks: u32, blocks_per_seg: u32) -> u32 {
    blocks.div_ceil(blocks_per_seg)
}

// Helper function: align upwards
fn size_align(count: u32, per_block: u32) -> u32 {
    count.div_ceil(per_block)
}

// Super block layout information
#[derive(Debug, Clone)]
pub struct SuperblockLayout {
    pub segment0_blkaddr: u32,
    pub cp_blkaddr: u32,
    pub sit_blkaddr: u32,
    pub nat_blkaddr: u32,
    pub ssa_blkaddr: u32,
    pub main_blkaddr: u32,
    pub segment_count: u32,
    pub segment_count_ckpt: u32,
    pub segment_count_sit: u32,
    pub segment_count_nat: u32,
    pub segment_count_ssa: u32,
    pub segment_count_main: u32,
    pub section_count: u32,
    pub block_count: u64,
    pub cp_payload: u32,
}

// super block builder
#[derive(Debug)]
pub struct SuperblockBuilder {
    // Basic parameters
    image_size: u64,
    block_size: u32,
    sector_size: u32,
    blocks_per_seg: u32,
    segs_per_sec: u32,
    secs_per_zone: u32,

    // Feature flag
    features: F2fsFeatures,

    // Volume information
    volume_label: String,
    uuid: [u8; 16],

    // Calculation result
    layout: Option<SuperblockLayout>,
}

impl SuperblockBuilder {
    pub fn new(image_size: u64) -> Self {
        SuperblockBuilder {
            image_size,
            block_size: F2FS_BLKSIZE as u32,
            sector_size: DEFAULT_SECTOR_SIZE,
            blocks_per_seg: DEFAULT_BLOCKS_PER_SEGMENT,
            segs_per_sec: DEFAULT_SEGMENTS_PER_SECTION,
            secs_per_zone: DEFAULT_SECTIONS_PER_ZONE,
            features: F2fsFeatures::default(),
            volume_label: String::new(),
            uuid: [0u8; 16],
            layout: None,
        }
    }

    // Set feature flags
    pub fn with_features(mut self, features: F2fsFeatures) -> Self {
        self.features = features;
        self
    }

    // Set volume label
    pub fn with_label(mut self, label: &str) -> Self {
        self.volume_label = label.to_string();
        self
    }

    // Set UUID
    pub fn with_uuid(mut self, uuid: [u8; 16]) -> Self {
        self.uuid = uuid;
        self
    }

    // Calculate layout
    pub fn calculate_layout(&mut self) -> Result<&SuperblockLayout> {
        let log_sectorsize = log_base_2(self.sector_size);
        let log_sectors_per_block = log_base_2(self.block_size / self.sector_size);
        let log_blocksize = log_sectorsize + log_sectors_per_block;
        let log_blks_per_seg = log_base_2(self.blocks_per_seg);

        let blk_size_bytes = 1u64 << log_blocksize;
        let segment_size_bytes = blk_size_bytes * self.blocks_per_seg as u64;
        let zone_size_bytes = blk_size_bytes
            * self.secs_per_zone as u64
            * self.segs_per_sec as u64
            * self.blocks_per_seg as u64;

        // Calculate the total number of blocks
        let total_sectors = self.image_size / self.sector_size as u64;
        let block_count = total_sectors >> log_sectors_per_block;

        // Calculate segment0 starting address
        let zone_align_start_offset = if self.features.readonly {
            8192u64
        } else {
            (2 * F2FS_BLKSIZE as u64).div_ceil(zone_size_bytes) * zone_size_bytes
        };

        let segment0_blkaddr = (zone_align_start_offset / blk_size_bytes) as u32;
        let cp_blkaddr = segment0_blkaddr;

        // Calculate the total number of segments
        let total_segments = ((self.image_size - zone_align_start_offset) / segment_size_bytes)
            as u32
            / self.segs_per_sec
            * self.segs_per_sec;

        if total_segments < F2FS_MIN_SEGMENTS as u32 {
            return Err(F2fsError::InvalidData(format!(
                "镜像太小，至少需要 {} 个段，当前只有 {} 个",
                F2FS_MIN_SEGMENTS, total_segments
            )));
        }

        // Number of checkpoint segments
        let segment_count_ckpt = F2FS_NUMBER_OF_CHECKPOINT_PACK;

        // SIT starting address
        let sit_blkaddr = segment0_blkaddr + segment_count_ckpt * self.blocks_per_seg;

        // Calculate the number of SIT segments
        let blocks_for_sit = size_align(total_segments, SIT_ENTRY_PER_BLOCK as u32);
        let sit_segments = seg_align(blocks_for_sit, self.blocks_per_seg);
        let segment_count_sit = sit_segments * 2; // double

        // NAT starting address
        let nat_blkaddr = sit_blkaddr + segment_count_sit * self.blocks_per_seg;

        // Calculate the number of NAT segments
        let total_valid_blks_available =
            (total_segments - segment_count_ckpt - segment_count_sit) * self.blocks_per_seg;
        let blocks_for_nat = size_align(total_valid_blks_available, NAT_ENTRY_PER_BLOCK as u32);
        let nat_segments = seg_align(blocks_for_nat, self.blocks_per_seg);
        let segment_count_nat = nat_segments * 2; // double

        // SSA starting address
        let ssa_blkaddr = nat_blkaddr + segment_count_nat * self.blocks_per_seg;

        // Calculate the number of SSA segments
        let total_valid_blks_available2 =
            (total_segments - segment_count_ckpt - segment_count_sit - segment_count_nat)
                * self.blocks_per_seg;

        let blocks_for_ssa = if self.features.readonly {
            0
        } else {
            total_valid_blks_available2 / self.blocks_per_seg + 1
        };
        let mut segment_count_ssa = seg_align(blocks_for_ssa, self.blocks_per_seg);

        // Total number of metadata segments
        let total_meta_segments =
            segment_count_ckpt + segment_count_sit + segment_count_nat + segment_count_ssa;

        // Align to zone boundary
        let diff = total_meta_segments % self.segs_per_sec;
        if diff != 0 {
            segment_count_ssa += self.segs_per_sec - diff;
        }

        let total_meta_segments =
            segment_count_ckpt + segment_count_sit + segment_count_nat + segment_count_ssa;

        // Main area starting address
        let main_blkaddr = segment0_blkaddr + total_meta_segments * self.blocks_per_seg;

        // Calculate the number of segments and sections in the main area
        let total_zones =
            total_segments / self.segs_per_sec - total_meta_segments / self.segs_per_sec;
        let section_count = total_zones * self.secs_per_zone;
        let segment_count_main = section_count * self.segs_per_sec;

        // Calculate cp_payload
        let sit_bitmap_size = ((segment_count_sit / 2) << log_blks_per_seg) / 8;
        let max_sit_bitmap_size = sit_bitmap_size.min(MAX_SIT_BITMAP_SIZE_IN_CKPT as u32);
        let cp_payload = if max_sit_bitmap_size > MAX_SIT_BITMAP_SIZE_IN_CKPT as u32 {
            max_sit_bitmap_size.div_ceil(F2FS_BLKSIZE as u32)
        } else {
            0
        };

        let layout = SuperblockLayout {
            segment0_blkaddr,
            cp_blkaddr,
            sit_blkaddr,
            nat_blkaddr,
            ssa_blkaddr,
            main_blkaddr,
            segment_count: total_segments,
            segment_count_ckpt,
            segment_count_sit,
            segment_count_nat,
            segment_count_ssa,
            segment_count_main,
            section_count,
            block_count,
            cp_payload,
        };

        self.layout = Some(layout);
        self.layout
            .as_ref()
            .ok_or_else(|| F2fsError::InvalidData("布局计算失败".into()))
    }

    // Get layout
    pub fn layout(&self) -> Option<&SuperblockLayout> {
        self.layout.as_ref()
    }

    // Build superblock byte data
    pub fn build(&self) -> Result<[u8; SUPERBLOCK_SIZE]> {
        let layout = self
            .layout
            .as_ref()
            .ok_or_else(|| F2fsError::InvalidData("请先调用 calculate_layout()".into()))?;

        let mut buf = [0u8; SUPERBLOCK_SIZE];

        let log_sectorsize = log_base_2(self.sector_size);
        let log_sectors_per_block = log_base_2(self.block_size / self.sector_size);
        let log_blocksize = log_sectorsize + log_sectors_per_block;
        let log_blks_per_seg = log_base_2(self.blocks_per_seg);

        // Magic number (offset 0)
        buf[0..4].copy_from_slice(&F2FS_MAGIC.to_le_bytes());

        // Major version number (offset 4)
        buf[4..6].copy_from_slice(&F2FS_MAJOR_VERSION.to_le_bytes());

        // Minor version number (offset 6)
        buf[6..8].copy_from_slice(&F2FS_MINOR_VERSION.to_le_bytes());

        // log_sectorsize (offset 8)
        buf[8..12].copy_from_slice(&log_sectorsize.to_le_bytes());

        // log_sectors_per_block (offset 12)
        buf[12..16].copy_from_slice(&log_sectors_per_block.to_le_bytes());

        // log_blocksize (offset 16)
        buf[16..20].copy_from_slice(&log_blocksize.to_le_bytes());

        // log_blocks_per_seg (offset 20)
        buf[20..24].copy_from_slice(&log_blks_per_seg.to_le_bytes());

        // segs_per_sec (offset 24)
        buf[24..28].copy_from_slice(&self.segs_per_sec.to_le_bytes());

        // secs_per_zone (offset 28)
        buf[28..32].copy_from_slice(&self.secs_per_zone.to_le_bytes());

        // checksum_offset (offset 32) - Pointer to the location of the CRC checksum
        let checksum_offset = if self.features.sb_chksum {
            SB_CHKSUM_OFFSET as u32
        } else {
            0u32
        };
        buf[32..36].copy_from_slice(&checksum_offset.to_le_bytes());

        // block_count (offset 36)
        buf[36..44].copy_from_slice(&layout.block_count.to_le_bytes());

        // section_count (offset 44)
        buf[44..48].copy_from_slice(&layout.section_count.to_le_bytes());

        // segment_count (offset 48)
        buf[48..52].copy_from_slice(&layout.segment_count.to_le_bytes());

        // segment_count_ckpt (offset 52)
        buf[52..56].copy_from_slice(&layout.segment_count_ckpt.to_le_bytes());

        // segment_count_sit (offset 56)
        buf[56..60].copy_from_slice(&layout.segment_count_sit.to_le_bytes());

        // segment_count_nat (offset 60)
        buf[60..64].copy_from_slice(&layout.segment_count_nat.to_le_bytes());

        // segment_count_ssa (offset 64)
        buf[64..68].copy_from_slice(&layout.segment_count_ssa.to_le_bytes());

        // segment_count_main (offset 68)
        buf[68..72].copy_from_slice(&layout.segment_count_main.to_le_bytes());

        // segment0_blkaddr (offset 72)
        buf[72..76].copy_from_slice(&layout.segment0_blkaddr.to_le_bytes());

        // cp_blkaddr (offset 76)
        buf[76..80].copy_from_slice(&layout.cp_blkaddr.to_le_bytes());

        // sit_blkaddr (offset 80)
        buf[80..84].copy_from_slice(&layout.sit_blkaddr.to_le_bytes());

        // nat_blkaddr (offset 84)
        buf[84..88].copy_from_slice(&layout.nat_blkaddr.to_le_bytes());

        // ssa_blkaddr (offset 88)
        buf[88..92].copy_from_slice(&layout.ssa_blkaddr.to_le_bytes());

        // main_blkaddr (offset 92)
        buf[92..96].copy_from_slice(&layout.main_blkaddr.to_le_bytes());

        // root_ino (offset 96)
        buf[96..100].copy_from_slice(&F2FS_ROOT_INO.to_le_bytes());

        // node_ino (offset 100)
        buf[100..104].copy_from_slice(&F2FS_NODE_INO.to_le_bytes());

        // meta_ino (offset 104)
        buf[104..108].copy_from_slice(&F2FS_META_INO.to_le_bytes());

        // uuid (offset 108)
        buf[108..124].copy_from_slice(&self.uuid);

        // volume_name (offset 124, 1024 bytes for UTF-16)
        let volume_name_offset = 124;
        let volume_name_bytes = self.volume_label.encode_utf16().collect::<Vec<u16>>();
        for (i, &ch) in volume_name_bytes
            .iter()
            .take(MAX_VOLUME_NAME / 2)
            .enumerate()
        {
            let offset = volume_name_offset + i * 2;
            buf[offset..offset + 2].copy_from_slice(&ch.to_le_bytes());
        }

        // extension_count (offset 1148)
        buf[1148..1152].copy_from_slice(&0u32.to_le_bytes());

        // extension_list (offset 1152, 512 bytes)
        // remain zero

        // cp_payload (offset 1664)
        buf[1664..1668].copy_from_slice(&layout.cp_payload.to_le_bytes());

        // version (offset 1668, 256 bytes)
        let version_len = F2FS_VERSION.len().min(VERSION_LEN);
        buf[1668..1668 + version_len].copy_from_slice(&F2FS_VERSION[..version_len]);

        // init_version (offset 1924, 256 bytes)
        buf[1924..1924 + version_len].copy_from_slice(&F2FS_VERSION[..version_len]);

        // feature (offset 2180)
        buf[2180..2184].copy_from_slice(&self.features.to_bits().to_le_bytes());

        // encryption_level (offset 2184)
        buf[2184] = 0;

        // encrypt_pw_salt (offset 2185, 16 bytes)
        // remain zero

        // devs (offset 2201, 544 bytes for 8 devices)
        // remain zero

        // qf_ino (offset 2745, 12 bytes)
        // remain zero

        // hot_ext_count (offset 2757)
        buf[2757] = 0;

        // s_encoding (offset 2758)
        buf[2758..2760].copy_from_slice(&0u16.to_le_bytes());

        // s_encoding_flags (offset 2760)
        buf[2760..2762].copy_from_slice(&0u16.to_le_bytes());

        // s_stop_reason (offset 2762, 32 bytes)
        // remain zero

        // s_errors (offset 2794, 16 bytes)
        // remain zero

        // reserved (offset 2810, 258 bytes)
        // remain zero

        // Calculate and set CRC (offset 3068)
        if self.features.sb_chksum {
            let crc = crc32(&buf[..SB_CHKSUM_OFFSET]);
            buf[SB_CHKSUM_OFFSET..SB_CHKSUM_OFFSET + 4].copy_from_slice(&crc.to_le_bytes());
        }

        Ok(buf)
    }

    // Get superblock offset
    pub fn superblock_offset() -> u64 {
        SUPERBLOCK_OFFSET
    }

    // Get superblock size
    pub fn superblock_size() -> usize {
        SUPERBLOCK_SIZE
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

// Maximum SIT bitmap size (in checkpoint)
const MAX_SIT_BITMAP_SIZE_IN_CKPT: usize = CP_CHKSUM_OFFSET - 192 - 64; // CP_BITMAP_OFFSET - MIN_NAT_BITMAP_SIZE

// Minimum number of segments
const F2FS_MIN_SEGMENTS: usize = 9;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_log_base_2() {
        assert_eq!(log_base_2(512), 9);
        assert_eq!(log_base_2(4096), 12);
        assert_eq!(log_base_2(8), 3);
    }

    #[test]
    fn test_superblock_builder() {
        // Create a 100MB image
        let mut builder = SuperblockBuilder::new(100 * 1024 * 1024);
        let layout = builder.calculate_layout().unwrap();

        assert!(layout.segment_count > 0);
        assert!(layout.main_blkaddr > layout.ssa_blkaddr);
        assert!(layout.ssa_blkaddr > layout.nat_blkaddr);
        assert!(layout.nat_blkaddr > layout.sit_blkaddr);
        assert!(layout.sit_blkaddr > layout.cp_blkaddr);
    }

    #[test]
    fn test_superblock_build() {
        let mut builder = SuperblockBuilder::new(100 * 1024 * 1024);
        builder.calculate_layout().unwrap();
        let sb_data = builder.build().unwrap();

        // Verify the magic number
        let magic = u32::from_le_bytes([sb_data[0], sb_data[1], sb_data[2], sb_data[3]]);
        assert_eq!(magic, F2FS_MAGIC);
    }
}
