// EROFS Super Block Builder
//
// Build the EROFS superblock structure.

#![allow(dead_code)]

use crate::filesystem::erofs::Result;
use crate::filesystem::erofs::consts::*;

// CRC32C polynomial (little endian)
const CRC32C_POLY_LE: u32 = 0x82F63B78;

// EROFS-style software CRC32C implementation
// Consistent with implementation in erofs-utils
fn erofs_crc32c(mut crc: u32, data: &[u8]) -> u32 {
    for &byte in data {
        crc ^= byte as u32;
        for _ in 0..8 {
            if crc & 1 != 0 {
                crc = (crc >> 1) ^ CRC32C_POLY_LE;
            } else {
                crc >>= 1;
            }
        }
    }
    crc
}

// Super block layout information
#[derive(Debug, Clone)]
pub struct SuperblockLayout {
    pub block_size: u32,
    pub blkszbits: u8,
    pub meta_blkaddr: u32,
    pub xattr_blkaddr: u32,
    pub root_nid: u64,
    pub inos: u64,
    pub blocks: u32,
}

// super block builder
#[derive(Debug)]
pub struct SuperblockBuilder {
    // Basic configuration
    block_size: u32,
    blkszbits: u8,

    // Feature flag
    feature_compat: u32,
    feature_incompat: u32,

    // metadata
    uuid: [u8; 16],
    volume_name: [u8; 16],

    // Timestamp
    build_time: u64,
    build_time_nsec: u32,

    // layout information
    meta_blkaddr: u32,
    xattr_blkaddr: u32,
    root_nid: u64,
    inos: u64,
    blocks: u32,

    // Compression configuration
    available_compr_algs: u16,
    lz4_max_distance: u16,
    lz4_max_pclusterblks: u8,
}

impl SuperblockBuilder {
    pub fn new(block_size: u32) -> Self {
        let blkszbits = (block_size as f64).log2() as u8;

        SuperblockBuilder {
            block_size,
            blkszbits,
            feature_compat: 0,
            feature_incompat: 0,
            uuid: [0u8; 16],
            volume_name: [0u8; 16],
            build_time: 0,
            build_time_nsec: 0,
            meta_blkaddr: 0,
            xattr_blkaddr: 0,
            root_nid: 0,
            inos: 0,
            blocks: 0,
            available_compr_algs: 0,
            lz4_max_distance: 0,
            lz4_max_pclusterblks: 1,
        }
    }

    pub fn with_uuid(mut self, uuid: [u8; 16]) -> Self {
        self.uuid = uuid;
        self
    }

    pub fn with_volume_name(mut self, name: &str) -> Self {
        let bytes = name.as_bytes();
        let len = bytes.len().min(15);
        self.volume_name[..len].copy_from_slice(&bytes[..len]);
        self
    }

    pub fn with_build_time(mut self, timestamp: u64) -> Self {
        self.build_time = timestamp;
        self
    }

    pub fn with_feature_compat(mut self, features: u32) -> Self {
        self.feature_compat = features;
        self
    }

    pub fn with_feature_incompat(mut self, features: u32) -> Self {
        self.feature_incompat = features;
        self
    }

    pub fn add_feature_incompat(&mut self, features: u32) {
        self.feature_incompat |= features;
    }

    pub fn with_compression(mut self, algorithm: u8) -> Self {
        self.available_compr_algs |= 1 << algorithm;
        if algorithm == Z_EROFS_COMPRESSION_LZ4 {
            self.lz4_max_distance = 65535; // Default 64KB window
            self.lz4_max_pclusterblks = 1; // 4KB pcluster size
        }
        self
    }

    pub fn set_meta_blkaddr(&mut self, addr: u32) {
        self.meta_blkaddr = addr;
    }

    pub fn set_xattr_blkaddr(&mut self, addr: u32) {
        self.xattr_blkaddr = addr;
    }

    pub fn set_root_nid(&mut self, nid: u64) {
        self.root_nid = nid;
    }

    pub fn set_inos(&mut self, count: u64) {
        self.inos = count;
    }

    pub fn set_blocks(&mut self, count: u32) {
        self.blocks = count;
    }

    pub fn block_size(&self) -> u32 {
        self.block_size
    }

    pub fn blkszbits(&self) -> u8 {
        self.blkszbits
    }

    pub fn available_compr_algs(&self) -> u16 {
        self.available_compr_algs
    }

    pub fn feature_incompat(&self) -> u32 {
        self.feature_incompat
    }

    // Build compressed configuration data
    // When EROFS_FEATURE_INCOMPAT_COMPR_CFGS is set, the compression configuration needs to be written after the superblock
    // Format: In order of algorithm ID, each configuration is [2-byte length][configuration structure]
    pub fn build_compr_cfgs(&self) -> Vec<u8> {
        let mut data = Vec::new();

        // Traverse in order by algorithm ID
        for alg in 0..4u8 {
            if self.available_compr_algs & (1 << alg) == 0 {
                continue;
            }

            match alg {
                Z_EROFS_COMPRESSION_LZ4 => {
                    // z_erofs_lz4_cfgs: 14 bytes
                    // struct z_erofs_lz4_cfgs {
                    //     __le16 max_distance;
                    //     __le16 max_pclusterblks;
                    //     u8 reserved[10];
                    // }
                    data.extend_from_slice(&(Z_EROFS_LZ4_CFGS_SIZE as u16).to_le_bytes());
                    data.extend_from_slice(&self.lz4_max_distance.to_le_bytes());
                    data.extend_from_slice(&(self.lz4_max_pclusterblks as u16).to_le_bytes());
                    data.extend_from_slice(&[0u8; 10]); // reserved
                }
                Z_EROFS_COMPRESSION_LZMA => {
                    // z_erofs_lzma_cfgs: 14 bytes
                    // struct z_erofs_lzma_cfgs {
                    //     __le32 dict_size;
                    //     __le16 format;
                    //     u8 reserved[8];
                    // }
                    data.extend_from_slice(&(Z_EROFS_LZMA_CFGS_SIZE as u16).to_le_bytes());
                    data.extend_from_slice(&Z_EROFS_LZMA_MAX_DICT_SIZE.to_le_bytes());
                    data.extend_from_slice(&0u16.to_le_bytes()); // format
                    data.extend_from_slice(&[0u8; 8]); // reserved
                }
                Z_EROFS_COMPRESSION_DEFLATE => {
                    // z_erofs_deflate_cfgs: 6 bytes
                    // struct z_erofs_deflate_cfgs {
                    //     u8 windowbits;
                    //     u8 reserved[5];
                    // }
                    data.extend_from_slice(&(Z_EROFS_DEFLATE_CFGS_SIZE as u16).to_le_bytes());
                    data.push(Z_EROFS_DEFLATE_DEFAULT_WINDOWBITS);
                    data.extend_from_slice(&[0u8; 5]); // reserved
                }
                Z_EROFS_COMPRESSION_ZSTD => {
                    // z_erofs_zstd_cfgs: 6 bytes
                    // struct z_erofs_zstd_cfgs {
                    //     u8 format;
                    //     u8 windowlog;  // windowLog - ZSTD_WINDOWLOG_ABSOLUTEMIN(10)
                    //     u8 reserved[4];
                    // }
                    // Default uses 1MB window (windowlog = 20 - 10 = 10)
                    data.extend_from_slice(&(Z_EROFS_ZSTD_CFGS_SIZE as u16).to_le_bytes());
                    data.push(0); // format
                    data.push(20 - ZSTD_WINDOWLOG_ABSOLUTEMIN); // windowlog
                    data.extend_from_slice(&[0u8; 4]); // reserved
                }
                _ => {}
            }
        }

        // 4-byte alignment
        while data.len() % 4 != 0 {
            data.push(0);
        }

        data
    }

    // Calculate size of compressed configuration data
    pub fn compr_cfgs_size(&self) -> usize {
        if self.feature_incompat & EROFS_FEATURE_INCOMPAT_COMPR_CFGS == 0 {
            return 0;
        }
        self.build_compr_cfgs().len()
    }

    // Build superblock data
    pub fn build(&self) -> Result<Vec<u8>> {
        let mut buf = vec![0u8; EROFS_SUPER_BLOCK_SIZE];

        // magic (offset 0, 4 bytes)
        buf[0..4].copy_from_slice(&EROFS_SUPER_MAGIC_V1.to_le_bytes());

        // checksum (offset 4, 4 bytes) - calculated later
        buf[4..8].copy_from_slice(&0u32.to_le_bytes());

        // feature_compat (offset 8, 4 bytes)
        buf[8..12].copy_from_slice(&self.feature_compat.to_le_bytes());

        // blkszbits (offset 12, 1 byte)
        buf[12] = self.blkszbits;

        // sb_extslots (offset 13, 1 byte)
        buf[13] = 0;

        // root_nid (offset 14, 2 bytes) - low 16 bits
        buf[14..16].copy_from_slice(&(self.root_nid as u16).to_le_bytes());

        // inos (offset 16, 8 bytes)
        buf[16..24].copy_from_slice(&self.inos.to_le_bytes());

        // build_time (epoch, offset 24, 8 bytes)
        buf[24..32].copy_from_slice(&self.build_time.to_le_bytes());

        // build_time_nsec (fixed_nsec, offset 32, 4 bytes)
        buf[32..36].copy_from_slice(&self.build_time_nsec.to_le_bytes());

        // blocks (offset 36, 4 bytes)
        buf[36..40].copy_from_slice(&self.blocks.to_le_bytes());

        // meta_blkaddr (offset 40, 4 bytes)
        buf[40..44].copy_from_slice(&self.meta_blkaddr.to_le_bytes());

        // xattr_blkaddr (offset 44, 4 bytes)
        buf[44..48].copy_from_slice(&self.xattr_blkaddr.to_le_bytes());

        // uuid (offset 48, 16 bytes)
        buf[48..64].copy_from_slice(&self.uuid);

        // volume_name (offset 64, 16 bytes)
        buf[64..80].copy_from_slice(&self.volume_name);

        // feature_incompat (offset 80, 4 bytes)
        buf[80..84].copy_from_slice(&self.feature_incompat.to_le_bytes());

        // available_compr_algs / lz4_max_distance (offset 84, 2 bytes)
        // If the COMPR_CFGS flag is set, write available_compr_algs
        // Otherwise write lz4_max_distance
        if self.feature_incompat & EROFS_FEATURE_INCOMPAT_COMPR_CFGS != 0 {
            buf[84..86].copy_from_slice(&self.available_compr_algs.to_le_bytes());
        } else {
            buf[84..86].copy_from_slice(&self.lz4_max_distance.to_le_bytes());
        }

        // extra_devices (offset 86, 2 bytes)
        buf[86..88].copy_from_slice(&0u16.to_le_bytes());

        // devt_slotoff (offset 88, 2 bytes)
        buf[88..90].copy_from_slice(&0u16.to_le_bytes());

        // dirblkbits (offset 90, 1 byte)
        buf[90] = 0;

        // xattr_prefix_count (offset 91, 1 byte)
        buf[91] = 0;

        // xattr_prefix_start (offset 92, 4 bytes)
        buf[92..96].copy_from_slice(&0u32.to_le_bytes());

        // packed_nid (offset 96, 8 bytes)
        buf[96..104].copy_from_slice(&0u64.to_le_bytes());

        // xattr_filter_reserved (offset 104, 1 byte)
        buf[104] = 0;

        // reserved (offset 105, 3 bytes)
        buf[105..108].copy_from_slice(&[0u8; 3]);

        // build_time (offset 108, 4 bytes) - for mkfs time
        buf[108..112].copy_from_slice(&(self.build_time as u32).to_le_bytes());

        // rootnid_8b (offset 112, 8 bytes) - root nid in 48BIT mode
        buf[112..120].copy_from_slice(&self.root_nid.to_le_bytes());

        // reserved2 (offset 120, 8 bytes)
        buf[120..128].copy_from_slice(&0u64.to_le_bytes());

        Ok(buf)
    }

    // Calculate and set the checksum (the entire block of data needs to be passed in for calculation)
    pub fn build_with_checksum(&self, block_data: &[u8]) -> Result<Vec<u8>> {
        let mut sb_data = self.build()?;

        // Building a buffer for CRC calculation
        // EROFS checksum is calculated for the entire block (block_size - EROFS_SUPER_OFFSET)
        let checksum_len = self.block_size as usize - EROFS_SUPER_OFFSET as usize;
        let mut buf = vec![0u8; checksum_len];

        // Copy the superblock data to the beginning of the buffer
        buf[..sb_data.len()].copy_from_slice(&sb_data);

        // Copy the data after the superblock in the block (metadata such as inode)
        let meta_start = EROFS_SUPER_BLOCK_SIZE;
        if block_data.len() > meta_start {
            let copy_len = (block_data.len() - meta_start).min(checksum_len - meta_start);
            buf[meta_start..meta_start + copy_len]
                .copy_from_slice(&block_data[meta_start..meta_start + copy_len]);
        }

        // Set checksum field to 0
        buf[4..8].copy_from_slice(&0u32.to_le_bytes());

        // Use erofs-style software for CRC32C calculations (consistent with erofs-utils)
        let crc = erofs_crc32c(!0, &buf);

        // Write checksum to superblock data
        sb_data[4..8].copy_from_slice(&crc.to_le_bytes());

        Ok(sb_data)
    }
}

impl Default for SuperblockBuilder {
    fn default() -> Self {
        Self::new(4096)
    }
}
