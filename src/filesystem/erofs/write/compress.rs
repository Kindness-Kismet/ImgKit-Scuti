// EROFS compression support
//
// Implement the packaging function of EROFS compressed files.

use crate::compression::Compressor;
use crate::compression::deflate::DeflateCompressor;
use crate::compression::lz4::{Lz4Compressor, Lz4HcCompressor};
use crate::compression::lzma::MicroLzmaCompressor;
use crate::compression::zstd::ZstdCompressor;
use crate::filesystem::erofs::consts::*;
use crate::filesystem::erofs::{ErofsError, Result};

// Compression advisory flag
const Z_EROFS_ADVISE_COMPACTED_2B: u16 = 0x0001;

// Compressed index structure (8 bytes)
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

// compressor factory
pub fn create_compressor(algorithm: &str, level: Option<u32>) -> Result<Box<dyn Compressor>> {
    match algorithm.to_lowercase().as_str() {
        "lz4" => Ok(Box::new(Lz4Compressor)),
        "lz4hc" => {
            // lz4hc: 0-12, default 9
            let level = level.unwrap_or(9).min(12) as i32;
            Ok(Box::new(Lz4HcCompressor::new(level)))
        }
        "lzma" => {
            // lzma: 0-9 (normal) or 100-109 (extreme), default 6
            let level = level.unwrap_or(6);
            // Verification level range
            if level > 9 && !(100..=109).contains(&level) {
                return Err(ErofsError::Io(std::io::Error::other(
                    "lzma 压缩等级必须是 0-9 或 100-109",
                )));
            }
            Ok(Box::new(MicroLzmaCompressor::new(level)))
        }
        "deflate" => {
            // deflate: 0-9, default 1
            let level = level.unwrap_or(1).min(9);
            Ok(Box::new(DeflateCompressor::new(level)))
        }
        "zstd" => {
            // zstd: 0-22, default 3
            let level = level.unwrap_or(3).min(22) as i32;
            Ok(Box::new(ZstdCompressor::new(level)))
        }
        _ => Err(ErofsError::Io(std::io::Error::other(format!(
            "不支持的压缩算法: {}",
            algorithm
        )))),
    }
}

// Get compression algorithm type
pub fn get_algorithm_type(algorithm: &str) -> Result<u8> {
    match algorithm.to_lowercase().as_str() {
        "lz4" | "lz4hc" => Ok(Z_EROFS_COMPRESSION_LZ4),
        "lzma" => Ok(Z_EROFS_COMPRESSION_LZMA),
        "deflate" => Ok(Z_EROFS_COMPRESSION_DEFLATE),
        "zstd" => Ok(Z_EROFS_COMPRESSION_ZSTD),
        _ => Err(ErofsError::Io(std::io::Error::other(format!(
            "不支持的压缩算法: {}",
            algorithm
        )))),
    }
}

// Physical cluster (pcluster) - a physical unit that stores compressed data
#[derive(Debug, Clone)]
pub struct PhysicalCluster {
    pub compressed_data: Vec<u8>,              // Compressed data
    pub compressed_size: usize,                // Compressed size
    pub logical_clusters: Vec<LogicalCluster>, // Contains logical clusters
}

// Logical cluster (lcluster) - 4KB logical data unit
#[derive(Debug, Clone)]
pub struct LogicalCluster {
    pub original_size: usize, // Original data size (usually 4KB or the remaining size of the last block)
    pub offset_in_pcluster: u16, // offset in physical cluster
    pub is_head: bool,        // Whether it is the head of the physical cluster
    pub is_compressed: bool,  // Whether compression is used
}

// Compress file data (using destsize strategy)
// Returns a list of physical clusters, each physical cluster contains one or more logical clusters
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

        // Try to compress as much data as possible using destsize mode
        if let Some((compressed, input_size)) =
            compressor.compress_destsize(&data[offset..], block_size_usize)
        {
            // Align input_size down to 4KB boundaries (leaving at least one full logical cluster)
            let aligned_input_size = (input_size / block_size_usize) * block_size_usize;

            // The current implementation is first limited to a single logical cluster pcluster to avoid unpacking inconsistencies caused by multiple logical cluster paths.
            if aligned_input_size == block_size_usize {
                // Recompress aligned data
                let final_compressed = if aligned_input_size < input_size {
                    // Aligned data needs to be recompressed
                    match compressor.compress(&data[offset..offset + aligned_input_size]) {
                        Ok(c) => c,
                        Err(_) => {
                            // Compression failed, fallback to single block
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

                // In non-big pcluster paths, the compressed pcluster must be no more than one block.
                // Otherwise the reader will read in single blocks, resulting in data truncation.
                if final_compressed_len < aligned_input_size
                    && final_compressed_len <= block_size_usize
                {
                    // Compression successful: Calculate how many logical clusters are included
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
                        "destsize 成功: 压缩 {} 字节 -> {} 字节, 包含 {} 个逻辑簇",
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

        // Fallback: Use fixed block size
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

// Build metadata for compressed inodes (header + index)
// Use compacted format
pub fn build_compress_metadata(
    file_size: u64,
    block_size: u32,
    algorithm: u8,
    pclusters: &[PhysicalCluster],
    start_blkaddr: u32,
    xattr_size: usize,
) -> Result<(Vec<u8>, Vec<u8>)> {
    // Calculate cluster bits
    let cluster_bits = block_size.trailing_zeros() as u8;
    let h_clusterbits = 0u8;

    // Build compression header (using COMPACTED_2B format)
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

    // Calculate the total number of logical clusters (calculated by file size)
    let num_lclusters = file_size.div_ceil(block_size as u64) as usize;

    // Calculate the starting physical block address of each physical cluster
    let mut pblk_offsets = Vec::with_capacity(pclusters.len());
    let mut current_pblk = start_blkaddr;
    for pcluster in pclusters {
        pblk_offsets.push(current_pblk);
        let pcluster_blocks = pcluster.compressed_size.div_ceil(block_size as usize) as u32;
        current_pblk += pcluster_blocks;
    }

    // Expand logical cluster information to legacy index semantics
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

    // Compute hybrid index layout parameters according to erofs-utils
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

    // Non-big_pcluster: initial blkaddr needs to be decremented by 1 and dummy_head set
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
