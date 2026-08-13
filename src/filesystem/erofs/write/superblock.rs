// EROFS superblock 构建器
//
// 构建 EROFS superblock 结构.

#![allow(dead_code)]

use crate::filesystem::erofs::Result;
use crate::filesystem::erofs::consts::*;

// CRC32C 多项式 (小端)
const CRC32C_POLY_LE: u32 = 0x82F63B78;

// EROFS 风格的软件 CRC32C 实现
// 与 erofs-utils 中的实现保持一致
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

// superblock 布局信息
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

// superblock 构建器
#[derive(Debug)]
pub struct SuperblockBuilder {
    // 基础配置
    block_size: u32,
    blkszbits: u8,

    // 特性标志
    feature_compat: u32,
    feature_incompat: u32,

    // 元数据
    uuid: [u8; 16],
    volume_name: [u8; 16],

    // 时间戳
    build_time: u64,
    build_time_nsec: u32,

    // 布局信息
    meta_blkaddr: u32,
    xattr_blkaddr: u32,
    root_nid: u64,
    inos: u64,
    blocks: u32,

    // 压缩配置
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
            self.lz4_max_distance = 65535; // 默认 64KB 窗口
            self.lz4_max_pclusterblks = 1; // 4KB pcluster 大小
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

    // 构建压缩配置数据
    // 设置 EROFS_FEATURE_INCOMPAT_COMPR_CFGS 时, 压缩配置需紧随 superblock 之后写入
    // 格式: 按算法 ID 顺序排列, 每项为 [2 字节长度][配置结构]
    pub fn build_compr_cfgs(&self) -> Vec<u8> {
        let mut data = Vec::new();

        // 按算法 ID 顺序遍历
        for alg in 0..4u8 {
            if self.available_compr_algs & (1 << alg) == 0 {
                continue;
            }

            match alg {
                Z_EROFS_COMPRESSION_LZ4 => {
                    // z_erofs_lz4_cfgs: 14 字节
                    // struct z_erofs_lz4_cfgs {
                    //     __le16 max_distance;
                    //     __le16 max_pclusterblks;
                    //     u8 reserved[10];
                    // }
                    data.extend_from_slice(&(Z_EROFS_LZ4_CFGS_SIZE as u16).to_le_bytes());
                    data.extend_from_slice(&self.lz4_max_distance.to_le_bytes());
                    data.extend_from_slice(&(self.lz4_max_pclusterblks as u16).to_le_bytes());
                    data.extend_from_slice(&[0u8; 10]); // 保留字段
                }
                Z_EROFS_COMPRESSION_LZMA => {
                    // z_erofs_lzma_cfgs: 14 字节
                    // struct z_erofs_lzma_cfgs {
                    //     __le32 dict_size;
                    //     __le16 format;
                    //     u8 reserved[8];
                    // }
                    data.extend_from_slice(&(Z_EROFS_LZMA_CFGS_SIZE as u16).to_le_bytes());
                    data.extend_from_slice(&Z_EROFS_LZMA_MAX_DICT_SIZE.to_le_bytes());
                    data.extend_from_slice(&0u16.to_le_bytes()); // format 字段
                    data.extend_from_slice(&[0u8; 8]); // 保留字段
                }
                Z_EROFS_COMPRESSION_DEFLATE => {
                    // z_erofs_deflate_cfgs: 6 字节
                    // struct z_erofs_deflate_cfgs {
                    //     u8 windowbits;
                    //     u8 reserved[5];
                    // }
                    data.extend_from_slice(&(Z_EROFS_DEFLATE_CFGS_SIZE as u16).to_le_bytes());
                    data.push(Z_EROFS_DEFLATE_DEFAULT_WINDOWBITS);
                    data.extend_from_slice(&[0u8; 5]); // 保留字段
                }
                Z_EROFS_COMPRESSION_ZSTD => {
                    // z_erofs_zstd_cfgs: 6 字节
                    // struct z_erofs_zstd_cfgs {
                    //     u8 format;
                    //     u8 windowlog;  // windowLog - ZSTD_WINDOWLOG_ABSOLUTEMIN(10)
                    //     u8 reserved[4];
                    // }
                    // 默认使用 1MB 窗口 (windowlog = 20 - 10 = 10)
                    data.extend_from_slice(&(Z_EROFS_ZSTD_CFGS_SIZE as u16).to_le_bytes());
                    data.push(0); // format 字段
                    data.push(20 - ZSTD_WINDOWLOG_ABSOLUTEMIN); // windowlog 字段
                    data.extend_from_slice(&[0u8; 4]); // 保留字段
                }
                _ => {}
            }
        }

        // 4 字节对齐
        while data.len() % 4 != 0 {
            data.push(0);
        }

        data
    }

    // 计算压缩配置数据大小
    pub fn compr_cfgs_size(&self) -> usize {
        if self.feature_incompat & EROFS_FEATURE_INCOMPAT_COMPR_CFGS == 0 {
            return 0;
        }
        self.build_compr_cfgs().len()
    }

    // 构建 superblock 数据
    pub fn build(&self) -> Result<Vec<u8>> {
        let mut buf = vec![0u8; EROFS_SUPER_BLOCK_SIZE];

        // magic (偏移 0, 4 字节)
        buf[0..4].copy_from_slice(&EROFS_SUPER_MAGIC_V1.to_le_bytes());

        // checksum (偏移 4, 4 字节) - 稍后计算
        buf[4..8].copy_from_slice(&0u32.to_le_bytes());

        // feature_compat (偏移 8, 4 字节)
        buf[8..12].copy_from_slice(&self.feature_compat.to_le_bytes());

        // blkszbits (偏移 12, 1 字节)
        buf[12] = self.blkszbits;

        // sb_extslots (偏移 13, 1 字节)
        buf[13] = 0;

        // root_nid (偏移 14, 2 字节) - 低 16 位
        buf[14..16].copy_from_slice(&(self.root_nid as u16).to_le_bytes());

        // inos (偏移 16, 8 字节)
        buf[16..24].copy_from_slice(&self.inos.to_le_bytes());

        // build_time (epoch, 偏移 24, 8 字节)
        buf[24..32].copy_from_slice(&self.build_time.to_le_bytes());

        // build_time_nsec (fixed_nsec, 偏移 32, 4 字节)
        buf[32..36].copy_from_slice(&self.build_time_nsec.to_le_bytes());

        // blocks (偏移 36, 4 字节)
        buf[36..40].copy_from_slice(&self.blocks.to_le_bytes());

        // meta_blkaddr (偏移 40, 4 字节)
        buf[40..44].copy_from_slice(&self.meta_blkaddr.to_le_bytes());

        // xattr_blkaddr (偏移 44, 4 字节)
        buf[44..48].copy_from_slice(&self.xattr_blkaddr.to_le_bytes());

        // uuid (偏移 48, 16 字节)
        buf[48..64].copy_from_slice(&self.uuid);

        // volume_name (偏移 64, 16 字节)
        buf[64..80].copy_from_slice(&self.volume_name);

        // feature_incompat (偏移 80, 4 字节)
        buf[80..84].copy_from_slice(&self.feature_incompat.to_le_bytes());

        // available_compr_algs / lz4_max_distance (偏移 84, 2 字节)
        // 若设置了 COMPR_CFGS 标志, 写入 available_compr_algs
        // 否则写入 lz4_max_distance
        if self.feature_incompat & EROFS_FEATURE_INCOMPAT_COMPR_CFGS != 0 {
            buf[84..86].copy_from_slice(&self.available_compr_algs.to_le_bytes());
        } else {
            buf[84..86].copy_from_slice(&self.lz4_max_distance.to_le_bytes());
        }

        // extra_devices (偏移 86, 2 字节)
        buf[86..88].copy_from_slice(&0u16.to_le_bytes());

        // devt_slotoff (偏移 88, 2 字节)
        buf[88..90].copy_from_slice(&0u16.to_le_bytes());

        // dirblkbits (偏移 90, 1 字节)
        buf[90] = 0;

        // xattr_prefix_count (偏移 91, 1 字节)
        buf[91] = 0;

        // xattr_prefix_start (偏移 92, 4 字节)
        buf[92..96].copy_from_slice(&0u32.to_le_bytes());

        // packed_nid (偏移 96, 8 字节)
        buf[96..104].copy_from_slice(&0u64.to_le_bytes());

        // xattr_filter_reserved (偏移 104, 1 字节)
        buf[104] = 0;

        // reserved (偏移 105, 3 字节)
        buf[105..108].copy_from_slice(&[0u8; 3]);

        // build_time (偏移 108, 4 字节) - 用于 mkfs 时间
        buf[108..112].copy_from_slice(&(self.build_time as u32).to_le_bytes());

        // rootnid_8b (偏移 112, 8 字节) - 48BIT 模式下的 root nid
        buf[112..120].copy_from_slice(&self.root_nid.to_le_bytes());

        // reserved2 (偏移 120, 8 字节)
        buf[120..128].copy_from_slice(&0u64.to_le_bytes());

        Ok(buf)
    }

    // 计算并设置校验和 (需要传入整块数据参与计算)
    pub fn build_with_checksum(&self, block_data: &[u8]) -> Result<Vec<u8>> {
        let mut sb_data = self.build()?;

        // 构建用于 CRC 计算的缓冲区
        // EROFS 校验和针对整块数据计算 (block_size - EROFS_SUPER_OFFSET)
        let checksum_len = self.block_size as usize - EROFS_SUPER_OFFSET as usize;
        let mut buf = vec![0u8; checksum_len];

        // 将 superblock 数据复制到缓冲区起始处
        buf[..sb_data.len()].copy_from_slice(&sb_data);

        // 复制块内 superblock 之后的数据 (inode 等元数据)
        let meta_start = EROFS_SUPER_BLOCK_SIZE;
        if block_data.len() > meta_start {
            let copy_len = (block_data.len() - meta_start).min(checksum_len - meta_start);
            buf[meta_start..meta_start + copy_len]
                .copy_from_slice(&block_data[meta_start..meta_start + copy_len]);
        }

        // 将 checksum 字段置 0
        buf[4..8].copy_from_slice(&0u32.to_le_bytes());

        // 使用 EROFS 风格的软件 CRC32C 计算 (与 erofs-utils 一致)
        let crc = erofs_crc32c(!0, &buf);

        // 将校验和写入 superblock 数据
        sb_data[4..8].copy_from_slice(&crc.to_le_bytes());

        Ok(sb_data)
    }
}

impl Default for SuperblockBuilder {
    fn default() -> Self {
        Self::new(4096)
    }
}
