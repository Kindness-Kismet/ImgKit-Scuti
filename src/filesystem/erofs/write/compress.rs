// EROFS 压缩支持
//
// 实现 EROFS 压缩文件的打包功能.

use crate::compression::Compressor;
use crate::compression::deflate::DeflateCompressor;
use crate::compression::lz4::{Lz4Compressor, Lz4HcCompressor};
use crate::compression::lzma::MicroLzmaCompressor;
use crate::compression::zstd::ZstdCompressor;
use crate::filesystem::erofs::consts::*;
use crate::filesystem::erofs::{ErofsError, Result};

// 压缩建议标志
const Z_EROFS_ADVISE_COMPACTED_2B: u16 = 0x0001;

// 压缩索引结构 (8 字节)
#[derive(Debug, Clone)]
pub struct ZErofsLclusterIndex {
    pub di_advise: u16,
    pub di_clusterofs: u16,
    pub di_u: u32,
}

impl ZErofsLclusterIndex {
    pub fn new_head(cluster_type: u16, cluster_ofs: u16, blkaddr: u32) -> Self {
        ZErofsLclusterIndex {
            di_advise: cluster_type,
            di_clusterofs: cluster_ofs,
            di_u: blkaddr,
        }
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        let mut buf = Vec::with_capacity(8);
        buf.extend_from_slice(&self.di_advise.to_le_bytes());
        buf.extend_from_slice(&self.di_clusterofs.to_le_bytes());
        buf.extend_from_slice(&self.di_u.to_le_bytes());
        buf
    }
}

// 压缩器工厂
pub fn create_compressor(algorithm: &str, level: Option<u32>) -> Result<Box<dyn Compressor>> {
    match algorithm.to_lowercase().as_str() {
        "lz4" => Ok(Box::new(Lz4Compressor)),
        "lz4hc" => {
            // lz4hc: 0-12, 默认 9
            let level = level.unwrap_or(9).min(12) as i32;
            Ok(Box::new(Lz4HcCompressor::new(level)))
        }
        "lzma" => {
            // lzma: 0-9 (常规) 或 100-109 (极限), 默认 6
            let level = level.unwrap_or(6);
            // 校验等级范围
            if level > 9 && !(100..=109).contains(&level) {
                return Err(ErofsError::Io(std::io::Error::other(
                    "lzma compression level must be 0-9 or 100-109",
                )));
            }
            Ok(Box::new(MicroLzmaCompressor::new(level)))
        }
        "deflate" => {
            // deflate: 0-9, 默认 1
            let level = level.unwrap_or(1).min(9);
            Ok(Box::new(DeflateCompressor::new(level)))
        }
        "zstd" => {
            // zstd: 0-22, 默认 3
            let level = level.unwrap_or(3).min(22) as i32;
            Ok(Box::new(ZstdCompressor::new(level)))
        }
        _ => Err(ErofsError::Io(std::io::Error::other(format!(
            "unsupported compression algorithm: {}",
            algorithm
        )))),
    }
}

// 获取压缩算法类型
pub fn get_algorithm_type(algorithm: &str) -> Result<u8> {
    match algorithm.to_lowercase().as_str() {
        "lz4" | "lz4hc" => Ok(Z_EROFS_COMPRESSION_LZ4),
        "lzma" => Ok(Z_EROFS_COMPRESSION_LZMA),
        "deflate" => Ok(Z_EROFS_COMPRESSION_DEFLATE),
        "zstd" => Ok(Z_EROFS_COMPRESSION_ZSTD),
        _ => Err(ErofsError::Io(std::io::Error::other(format!(
            "unsupported compression algorithm: {}",
            algorithm
        )))),
    }
}

// 物理簇 (pcluster) - 存放压缩数据的物理单元
#[derive(Debug, Clone)]
pub struct PhysicalCluster {
    pub compressed_data: Vec<u8>,              // 压缩后的数据
    pub compressed_size: usize,                // 压缩后的大小
    pub logical_clusters: Vec<LogicalCluster>, // 包含的逻辑簇
}

// 逻辑簇 (lcluster) - 4KB 逻辑数据单元
#[derive(Debug, Clone)]
pub struct LogicalCluster {
    pub original_size: usize,    // 原始数据大小 (通常为 4KB 或末块的剩余大小)
    pub offset_in_pcluster: u16, // 在物理簇内的偏移
    pub is_head: bool,           // 是否为物理簇的头部
    pub is_compressed: bool,     // 是否使用压缩
}

// 压缩文件数据 (采用 destsize 策略)
// 返回物理簇列表, 每个物理簇包含一个或多个逻辑簇
pub fn compress_file_data(
    data: &[u8],
    block_size: u32,
    compressor: &dyn Compressor,
) -> Result<Vec<PhysicalCluster>> {
    let mut pclusters = Vec::new();
    let mut offset = 0;
    let block_size_usize = block_size as usize;

    while offset < data.len() {
        let remaining = data.len() - offset;

        // 使用 destsize 模式尽可能多地压缩数据
        if let Some((compressed, input_size)) =
            compressor.compress_destsize(&data[offset..], block_size_usize)
        {
            // 将 input_size 向下对齐到 4KB 边界 (至少保留一个完整逻辑簇)
            let aligned_input_size = (input_size / block_size_usize) * block_size_usize;

            // 当前实现先限定为单逻辑簇 pcluster, 避免多逻辑簇路径导致解包不一致.
            if aligned_input_size == block_size_usize {
                // 对对齐后的数据重新压缩
                let final_compressed = if aligned_input_size < input_size {
                    // 对齐后的数据需要重新压缩
                    match compressor.compress(&data[offset..offset + aligned_input_size]) {
                        Ok(c) => c,
                        Err(_) => {
                            // 压缩失败, 回退为单块处理
                            let chunk_size = std::cmp::min(block_size_usize, remaining);
                            let chunk = &data[offset..offset + chunk_size];
                            let compressed = compressor.compress(chunk).map_err(|e| {
                                ErofsError::Io(std::io::Error::other(e.to_string()))
                            })?;
                            let compressed_len = compressed.len();
                            let (use_compressed, final_data, final_size) =
                                if compressed_len < chunk_size {
                                    (true, compressed, compressed_len)
                                } else {
                                    (false, chunk.to_vec(), chunk_size)
                                };
                            pclusters.push(PhysicalCluster {
                                compressed_data: final_data,
                                compressed_size: final_size,
                                logical_clusters: vec![LogicalCluster {
                                    original_size: chunk_size,
                                    offset_in_pcluster: 0,
                                    is_head: true,
                                    is_compressed: use_compressed,
                                }],
                            });
                            offset += chunk_size;
                            continue;
                        }
                    }
                } else {
                    compressed
                };

                let final_compressed_len = final_compressed.len();

                // 在非 big pcluster 路径下, 压缩后的 pcluster 不得超过一个 block.
                // 否则读取端会按单块读取, 导致数据被截断.
                if final_compressed_len < aligned_input_size
                    && final_compressed_len <= block_size_usize
                {
                    // 压缩成功: 计算包含多少个逻辑簇
                    let num_lclusters = aligned_input_size / block_size_usize;
                    let mut logical_clusters = Vec::with_capacity(num_lclusters);

                    for i in 0..num_lclusters {
                        logical_clusters.push(LogicalCluster {
                            original_size: block_size_usize,
                            offset_in_pcluster: 0,
                            is_head: i == 0,
                            is_compressed: true,
                        });
                    }

                    log::debug!(
                        "destsize succeeded: compressed {} bytes -> {} bytes, {} lclusters",
                        aligned_input_size,
                        final_compressed_len,
                        num_lclusters
                    );

                    pclusters.push(PhysicalCluster {
                        compressed_data: final_compressed,
                        compressed_size: final_compressed_len,
                        logical_clusters,
                    });

                    offset += aligned_input_size;
                    continue;
                }
            }
        }

        // 回退: 使用固定块大小
        let chunk_size = std::cmp::min(block_size_usize, remaining);
        let chunk = &data[offset..offset + chunk_size];

        let compressed = compressor
            .compress(chunk)
            .map_err(|e| ErofsError::Io(std::io::Error::other(e.to_string())))?;

        let compressed_len = compressed.len();
        let (use_compressed, final_data, final_size) = if compressed_len < chunk_size {
            (true, compressed, compressed_len)
        } else {
            (false, chunk.to_vec(), chunk_size)
        };

        pclusters.push(PhysicalCluster {
            compressed_data: final_data,
            compressed_size: final_size,
            logical_clusters: vec![LogicalCluster {
                original_size: chunk_size,
                offset_in_pcluster: 0,
                is_head: true,
                is_compressed: use_compressed,
            }],
        });

        offset += chunk_size;
    }

    Ok(pclusters)
}

// 构建压缩 inode 的元数据 (map header + 索引)
// 采用 compacted 格式
pub fn build_compress_metadata(
    file_size: u64,
    block_size: u32,
    algorithm: u8,
    pclusters: &[PhysicalCluster],
    start_blkaddr: u32,
    xattr_size: usize,
) -> Result<(Vec<u8>, Vec<u8>)> {
    // 计算簇位宽
    let cluster_bits = block_size.trailing_zeros() as u8;
    let h_clusterbits = 0u8;

    // 构建压缩 map header (使用 COMPACTED_2B 格式)
    let mut header_bytes = vec![0u8; 8];
    header_bytes[0..2].copy_from_slice(&0u16.to_le_bytes());
    header_bytes[2..4].copy_from_slice(&0u16.to_le_bytes());
    let h_advise = if cluster_bits <= 12 {
        Z_EROFS_ADVISE_COMPACTED_2B
    } else {
        0
    };
    header_bytes[4..6].copy_from_slice(&h_advise.to_le_bytes());
    header_bytes[6] = algorithm;
    header_bytes[7] = h_clusterbits;

    #[derive(Clone, Copy, Debug, Default)]
    struct CompactIndexVec {
        clustertype: u8,
        clusterofs: u16,
        blkaddr: u32,
        delta0: u16,
        delta1: u16,
    }

    fn write_compacted_pack(
        out: &mut Vec<u8>,
        entries: &[CompactIndexVec],
        destsize: usize,
        lclusterbits: u32,
        final_pack: bool,
        dummy_head: &mut bool,
        blkaddr_ret: &mut u32,
    ) -> Result<()> {
        let vcnt = match destsize {
            4 => 2usize,
            2 if lclusterbits <= 12 => 16usize,
            _ => {
                return Err(ErofsError::Io(std::io::Error::other(
                    "invalid compacted index pack size",
                )));
            }
        };

        if entries.len() > vcnt {
            return Err(ErofsError::Io(std::io::Error::other(
                "too many entries in compacted index pack",
            )));
        }

        if entries.len() < vcnt && !final_pack {
            return Err(ErofsError::Io(std::io::Error::other(
                "unexpected short compacted index pack",
            )));
        }

        let lobits = lclusterbits.max(12);
        let encodebits = ((vcnt * destsize * 8) - 32) / vcnt;
        let mut pack = vec![0u8; destsize * vcnt];
        let stored_blkaddr = *blkaddr_ret;
        let mut blkaddr = *blkaddr_ret;

        for i in 0..vcnt {
            let entry = entries.get(i).copied().unwrap_or_default();
            let clustertype = entry.clustertype as u16;
            let offset: u32;

            if clustertype == Z_EROFS_LCLUSTER_TYPE_NONHEAD {
                if (entry.delta0 & Z_EROFS_LI_D0_CBLKCNT) != 0 {
                    return Err(ErofsError::Io(std::io::Error::other(
                        "big pcluster is not supported in compacted writer",
                    )));
                }
                if i + 1 == vcnt {
                    offset = entry.delta1.min(Z_EROFS_LI_D0_CBLKCNT - 1) as u32;
                } else {
                    offset = entry.delta0 as u32;
                }
            } else {
                offset = entry.clusterofs as u32;
                if *dummy_head {
                    blkaddr = blkaddr.saturating_add(1);
                }
                *dummy_head = true;
                if entry.blkaddr != 0 && entry.blkaddr != blkaddr && !(final_pack && i + 1 == vcnt)
                {
                    return Err(ErofsError::Io(std::io::Error::other(format!(
                        "unexpected blkaddr in compacted index pack: expect {}, got {}",
                        blkaddr, entry.blkaddr
                    ))));
                }
            }

            let v = ((clustertype as u32) << lobits) | offset;
            let pos = encodebits * i;
            let rem = pos & 7;
            let byte_pos = pos / 8;

            let data_bytes = destsize * vcnt - 4;
            if byte_pos < data_bytes {
                let ch = pack[byte_pos] & ((1 << rem) - 1);
                pack[byte_pos] = ((v << rem) as u8) | ch;
            }
            if byte_pos + 1 < data_bytes {
                pack[byte_pos + 1] = (v >> (8 - rem)) as u8;
            }
            if byte_pos + 2 < data_bytes {
                pack[byte_pos + 2] = (v >> (16 - rem)) as u8;
            }
        }

        let tail = destsize * vcnt - 4;
        pack[tail..tail + 4].copy_from_slice(&stored_blkaddr.to_le_bytes());
        *blkaddr_ret = blkaddr;
        out.extend_from_slice(&pack);
        Ok(())
    }

    // 计算逻辑簇总数 (按文件大小计算)
    let num_lclusters = file_size.div_ceil(block_size as u64) as usize;

    // 计算每个物理簇的起始物理块地址
    let mut pblk_offsets = Vec::with_capacity(pclusters.len());
    let mut current_pblk = start_blkaddr;
    for pcluster in pclusters {
        pblk_offsets.push(current_pblk);
        let pcluster_blocks = pcluster.compressed_size.div_ceil(block_size as usize) as u32;
        current_pblk += pcluster_blocks;
    }

    // 将逻辑簇信息展开为 legacy 索引语义
    let mut cv = Vec::with_capacity(num_lclusters);
    for (pcluster_idx, pcluster) in pclusters.iter().enumerate() {
        let total_lc = pcluster.logical_clusters.len();
        for (local_idx, lcluster) in pcluster.logical_clusters.iter().enumerate() {
            let clustertype = if lcluster.is_compressed {
                if lcluster.is_head {
                    Z_EROFS_LCLUSTER_TYPE_HEAD1 as u8
                } else {
                    Z_EROFS_LCLUSTER_TYPE_NONHEAD as u8
                }
            } else {
                Z_EROFS_LCLUSTER_TYPE_PLAIN as u8
            };

            let mut entry = CompactIndexVec {
                clustertype,
                clusterofs: lcluster.offset_in_pcluster,
                blkaddr: pblk_offsets[pcluster_idx],
                delta0: 0,
                delta1: 0,
            };

            if clustertype as u16 == Z_EROFS_LCLUSTER_TYPE_NONHEAD {
                entry.delta0 = local_idx as u16;
                entry.delta1 = (total_lc - local_idx) as u16;
            }

            cv.push(entry);
        }
    }

    if cv.len() != num_lclusters {
        return Err(ErofsError::Io(std::io::Error::other(format!(
            "logical cluster count mismatch: expect {}, got {}",
            num_lclusters,
            cv.len()
        ))));
    }

    // 按照 erofs-utils 计算混合索引布局参数
    let inode_plus_xattr = 32u64 + xattr_size as u64;
    let aligned_inode_xattr = (inode_plus_xattr + 7) & !7;
    let mpos = aligned_inode_xattr + 8;

    let mut compacted_4b_initial = ((32 - (mpos % 32)) / 4) as usize;
    if compacted_4b_initial == 8 {
        compacted_4b_initial = 0;
    }
    if compacted_4b_initial > num_lclusters {
        compacted_4b_initial = 0;
    }

    let compacted_2b =
        if (h_advise & Z_EROFS_ADVISE_COMPACTED_2B) != 0 && compacted_4b_initial < num_lclusters {
            ((num_lclusters - compacted_4b_initial) / 16) * 16
        } else {
            0
        };
    let mut compacted_4b_end = num_lclusters - compacted_4b_initial - compacted_2b;

    if !compacted_4b_initial.is_multiple_of(2) {
        return Err(ErofsError::Io(std::io::Error::other(
            "compacted_4b_initial is not aligned to 2 entries",
        )));
    }

    let mut indexes = Vec::new();
    let mut cursor = 0usize;

    // 非 big_pcluster: 初始 blkaddr 需减 1, 并置位 dummy_head
    let mut blkaddr = start_blkaddr.saturating_sub(1);
    let mut dummy_head = true;

    while compacted_4b_initial > 0 {
        let entries = &cv[cursor..cursor + 2];
        write_compacted_pack(
            &mut indexes,
            entries,
            4,
            cluster_bits as u32,
            false,
            &mut dummy_head,
            &mut blkaddr,
        )?;
        cursor += 2;
        compacted_4b_initial -= 2;
    }

    let mut remain_2b = compacted_2b;
    while remain_2b > 0 {
        let entries = &cv[cursor..cursor + 16];
        write_compacted_pack(
            &mut indexes,
            entries,
            2,
            cluster_bits as u32,
            false,
            &mut dummy_head,
            &mut blkaddr,
        )?;
        cursor += 16;
        remain_2b -= 16;
    }

    while compacted_4b_end > 1 {
        let entries = &cv[cursor..cursor + 2];
        write_compacted_pack(
            &mut indexes,
            entries,
            4,
            cluster_bits as u32,
            false,
            &mut dummy_head,
            &mut blkaddr,
        )?;
        cursor += 2;
        compacted_4b_end -= 2;
    }

    if compacted_4b_end == 1 {
        let entries = &cv[cursor..cursor + 1];
        write_compacted_pack(
            &mut indexes,
            entries,
            4,
            cluster_bits as u32,
            true,
            &mut dummy_head,
            &mut blkaddr,
        )?;
        cursor += 1;
    }

    if cursor != cv.len() {
        return Err(ErofsError::Io(std::io::Error::other(format!(
            "compacted index conversion did not consume all entries: {} / {}",
            cursor,
            cv.len()
        ))));
    }

    Ok((header_bytes, indexes))
}
