// EROFS compressed file reader

use super::volume::ErofsVolume;
use crate::compression::{Algorithm, Decompressor};
use crate::filesystem::erofs::*;
use std::io::{Read, Seek, SeekFrom};
use zerocopy::TryFromBytes;

// Parameters for a compressed file read operation.
struct CompressionParams {
    header_offset: u64,
    algorithm_head1: u8,
    algorithm_head2: u8,
    cluster_size: u32,
    num_clusters: usize,
    z_advise: u16,
}

impl ErofsVolume {
    pub(crate) fn read_compressed_file(&mut self, inode_info: &InodeInfo) -> Result<Vec<u8>> {
        use crate::filesystem::erofs::types::ZErofsMapHeader;

        log::debug!(
            "read_compressed_file: nid={}, size={}, raw_blkaddr={}",
            inode_info.nid,
            inode_info.size,
            inode_info.raw_blkaddr
        );

        // 1. Locate compression metadata: inode offset + inode size + xattr size, aligned to 8 bytes.
        // For COMPRESSED_COMPACT format, compression metadata (header + indexes) is stored inline after the inode.
        let inode_offset = self.nid_to_offset(inode_info.nid);
        let inode_size = if inode_info.is_compact { 32 } else { 64 };
        let xattr_size = self.xattr_ibody_size(inode_info.xattr_icount);

        // Align to 8-byte boundary.
        let header_offset = ((inode_offset + inode_size + xattr_size as u64) + 7) & !7;

        log::debug!(
            "compression metadata offset: {} (inode={}, inode_size={}, xattr_size={})",
            header_offset,
            inode_offset,
            inode_size,
            xattr_size
        );

        // 2. Read z_erofs_map_header
        self.file.seek(SeekFrom::Start(header_offset))?;
        let mut header_bytes = vec![0u8; std::mem::size_of::<ZErofsMapHeader>()];
        self.file.read_exact(&mut header_bytes)?;

        let header = ZErofsMapHeader::try_read_from_bytes(&header_bytes[..]).map_err(|_| {
            ErofsError::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "failed to parse z_erofs_map_header",
            ))
        })?;

        let algorithm_head1 = header.h_algorithmtype & 0x0F; // bit 0-3
        let algorithm_head2 = (header.h_algorithmtype >> 4) & 0x0F; // bit 4-7
        let cluster_bits = header.h_clusterbits + self.superblock.blkszbits;
        let cluster_size = 1u32 << cluster_bits;
        let z_advise = header.h_advise;

        // 3. Read and decompress all required clusters.
        let num_clusters = inode_info.size.div_ceil(cluster_size as u64) as usize;

        log::debug!(
            "compression header: algorithm_head1={}, algorithm_head2={}, cluster_bits={}, cluster_size={}, num_clusters={}, z_advise=0x{:x}",
            algorithm_head1,
            algorithm_head2,
            cluster_bits,
            cluster_size,
            num_clusters,
            z_advise
        );

        // Read the full compressed file (multi-cluster).
        self.read_multi_cluster_compressed_file(
            inode_info,
            CompressionParams {
                header_offset,
                algorithm_head1,
                algorithm_head2,
                cluster_size,
                num_clusters,
                z_advise,
            },
        )
    }

    // Bit-packed index decode helper.
    // Extracts one index entry from a bit-packed buffer.
    // Args: lobits - number of low bits, buffer - data buffer, bit_pos - starting bit position.
    // Returns: (lo_value, type)
    fn decode_compactedbits(lobits: u32, buffer: &[u8], bit_pos: u32) -> (u32, u16) {
        let byte_offset = (bit_pos / 8) as usize;
        let bit_offset = bit_pos % 8;

        // Read a 32-bit little-endian value.
        if byte_offset + 4 > buffer.len() {
            return (0, 0);
        }

        let v = u32::from_le_bytes([
            buffer[byte_offset],
            buffer[byte_offset + 1],
            buffer[byte_offset + 2],
            buffer[byte_offset + 3],
        ]) >> bit_offset;

        // Extract the low-bit value.
        let lo = v & ((1 << lobits) - 1);

        // Extract the 2-bit type field.
        let lcluster_type = ((v >> lobits) & 3) as u16;

        (lo, lcluster_type)
    }

    fn read_multi_cluster_compressed_file(
        &mut self,
        inode_info: &InodeInfo,
        params: CompressionParams,
    ) -> Result<Vec<u8>> {
        use std::collections::HashMap;

        // Destructure params.
        let CompressionParams {
            header_offset,
            algorithm_head1,
            algorithm_head2,
            cluster_size,
            num_clusters,
            z_advise,
        } = params;

        // Cache of decompressed pcluster data: pblk -> decompressed data.
        // A single pblk always uses one compression algorithm, so algorithm is not part of the key.
        let mut pcluster_cache: HashMap<u32, Vec<u8>> = HashMap::new();

        // Data block list: (logical address, data).
        // Sorted by LA before assembling the final file.
        let mut data_blocks: Vec<(u64, Vec<u8>)> = Vec::new();

        // Compute bit-packing parameters.
        let cluster_bits = (cluster_size as f32).log2() as u32;
        const Z_EROFS_LI_D0_CBLKCNT: u32 = 1 << 11;
        let lobits = cluster_bits.max((Z_EROFS_LI_D0_CBLKCNT as f32).log2() as u32 + 1);

        log::debug!(
            "bit-packing params: cluster_bits={}, lobits={}",
            cluster_bits,
            lobits
        );

        // Determine vcnt (lclusters per pack) and amortizedshift (log2 of bytes per index).
        // Order matters: check smaller cluster_bits first.
        let (vcnt, amortizedshift) = if cluster_bits <= 12 {
            (16, 1) // 2 bytes/index, 16 per pack (compact mode)
        } else if cluster_bits <= 14 {
            (2, 2) // 4 bytes/index, 2 per pack (standard mode)
        } else {
            return Err(ErofsError::UnsupportedFeature(format!(
                "cluster_bits {} too large",
                cluster_bits
            )));
        };

        // Compute encoded bits per index.
        // encodebits = ((vcnt << amortizedshift) - 4) * 8 / vcnt
        let _encodebits = (((vcnt << amortizedshift) - 4) * 8) / vcnt;

        // Index region start offset.
        let ebase = header_offset + 8;

        // Compute mixed index format parameters (per erofs-utils lib/zmap.c:126-130).
        // compacted_4b_initial: first few clusters use 4-byte indexes for 32-byte alignment.
        let compacted_4b_initial = (((32 - (ebase % 32)) / 4) & 7) as usize;

        // compacted_2b: middle clusters using 2-byte indexes (must be a multiple of 16).
        let compacted_2b = if (z_advise & 0x1) != 0 && compacted_4b_initial < num_clusters {
            // Z_EROFS_ADVISE_COMPACTED_2B = 0x0001
            ((num_clusters - compacted_4b_initial) / 16) * 16
        } else {
            0
        };

        // compacted_4b_end: remaining clusters use 4-byte indexes.
        let compacted_4b_end = num_clusters - compacted_4b_initial - compacted_2b;

        log::debug!(
            "index format: compacted_4b_initial={}, compacted_2b={}, compacted_4b_end={}, total={}",
            compacted_4b_initial,
            compacted_2b,
            compacted_4b_end,
            num_clusters
        );

        // Compute total index buffer size.
        // Each pack occupies pack_size bytes, including index data and stored_pblk (last 4 bytes).
        // 4b pack (vcnt=2, amortizedshift=2): pack_size = 2 << 2 = 8 bytes
        // 2b pack (vcnt=16, amortizedshift=1): pack_size = 16 << 1 = 32 bytes
        let num_packs_4b_initial = compacted_4b_initial.div_ceil(2); // vcnt=2
        let num_packs_2b = compacted_2b.div_ceil(16); // vcnt=16
        let num_packs_4b_end = compacted_4b_end.div_ceil(2); // vcnt=2

        let indices_size = num_packs_4b_initial * 8 + num_packs_2b * 32 + num_packs_4b_end * 8;

        self.file.seek(SeekFrom::Start(ebase))?;
        let mut indices_buffer = vec![0u8; indices_size];
        let n = self.file.read(&mut indices_buffer)?;
        indices_buffer.truncate(n);

        log::debug!(
            "read index buffer: offset={}, size={}, actual={}",
            ebase,
            indices_size,
            n
        );

        // Print first 64 bytes of index buffer in hex.
        if indices_buffer.len() >= 64 {
            log::debug!(
                "index buffer first 64 bytes: {:02x?}",
                &indices_buffer[0..64]
            );
        } else {
            log::debug!("index buffer all bytes: {:02x?}", &indices_buffer);
        }

        // Helper closure: compute pack parameters for a given lcn.
        let calc_pack_params = |target_lcn: usize| -> (usize, usize, usize, usize, usize, u32) {
            let mut adjusted_lcn = target_lcn;
            let mut pos = 0usize;
            let mut amortizedshift_local = 2;
            let mut region_start = 0usize; // start offset of the current region

            if adjusted_lcn >= compacted_4b_initial {
                pos += compacted_4b_initial * 4;
                region_start = compacted_4b_initial * 4; // 2-byte region starts here
                adjusted_lcn -= compacted_4b_initial;

                if adjusted_lcn < compacted_2b {
                    amortizedshift_local = 1;
                } else {
                    pos += compacted_2b * 2;
                    region_start = compacted_4b_initial * 4 + compacted_2b * 2; // 4-byte tail region starts here
                    adjusted_lcn -= compacted_2b;
                }
            }

            pos += adjusted_lcn * (1 << amortizedshift_local);

            let vcnt_local = if (1 << amortizedshift_local) == 4 {
                2
            } else {
                16
            };
            let pack_size = vcnt_local << amortizedshift_local;

            // pack_start should be aligned relative to the start of the current region.
            let pos_in_region = pos - region_start;
            let pack_start_in_region = (pos_in_region / pack_size) * pack_size;
            let pack_start = region_start + pack_start_in_region;
            let in_pack_idx = (pos - pack_start) >> amortizedshift_local;

            let pblk_offset = pack_start + pack_size - 4;
            let stored_pblk = if pblk_offset + 4 <= indices_buffer.len() {
                u32::from_le_bytes([
                    indices_buffer[pblk_offset],
                    indices_buffer[pblk_offset + 1],
                    indices_buffer[pblk_offset + 2],
                    indices_buffer[pblk_offset + 3],
                ])
            } else {
                // For an incomplete pack (last pack), stored_pblk may only have partial bytes.
                // Try to read from an alternate position.
                let encodebits_for_calc = ((pack_size - 4) * 8) / vcnt_local;
                let index_bytes = encodebits_for_calc.div_ceil(8);
                let alt_pblk_offset = pos + index_bytes;

                if alt_pblk_offset + 4 <= indices_buffer.len() {
                    // Can read a full 4 bytes.
                    log::debug!(
                        "Cluster {}: incomplete pack, reading full stored_pblk from alt offset: offset={}",
                        target_lcn,
                        alt_pblk_offset
                    );
                    u32::from_le_bytes([
                        indices_buffer[alt_pblk_offset],
                        indices_buffer[alt_pblk_offset + 1],
                        indices_buffer[alt_pblk_offset + 2],
                        indices_buffer[alt_pblk_offset + 3],
                    ])
                } else {
                    // Incomplete pack not enough bytes to store stored_pblk, use raw_blkaddr as fallback
                    log::debug!(
                        "Cluster {}: incomplete pack has no stored_pblk, falling back to raw_blkaddr={}",
                        target_lcn,
                        inode_info.raw_blkaddr
                    );
                    inode_info.raw_blkaddr
                }
            };

            let encodebits_local = ((pack_size - 4) * 8) / vcnt_local;

            (
                pack_start,
                in_pack_idx,
                vcnt_local,
                amortizedshift_local,
                encodebits_local,
                stored_pblk,
            )
        };

        // Decompress all clusters.
        // Two passes: pass 1 handles all HEAD and PLAIN clusters (fills cache),
        //             pass 2 handles all NONHEAD clusters (uses cache).

        // Pass 1: handle PLAIN and HEAD clusters.
        for lcn in 0..num_clusters {
            // Compute index parameters for this lcn.
            let (
                pack_offset,
                in_pack_idx,
                pack_vcnt,
                _pack_amortizedshift,
                pack_encodebits,
                stored_pblk,
            ) = calc_pack_params(lcn);

            // Decode the index for this cluster.
            let bit_pos = (in_pack_idx * pack_encodebits) as u32;
            let (lo, lcluster_type) =
                Self::decode_compactedbits(lobits, &indices_buffer[pack_offset..], bit_pos);

            log::debug!(
                "Cluster {}: pack_offset={}, in_pack={}, vcnt={}, type={}, lo={}, stored_pblk={}",
                lcn,
                pack_offset,
                in_pack_idx,
                pack_vcnt,
                lcluster_type,
                lo,
                stored_pblk
            );

            // Handle different lcluster types.
            if lcluster_type == Z_EROFS_LCLUSTER_TYPE_NONHEAD {
                // Skip NONHEAD in pass 1; handled in pass 2.
                log::debug!(
                    "Cluster {}: NONHEAD (delta={}) - skipped in pass 1",
                    lcn,
                    lo
                );
                continue;
            }

            // Compute physical block address.
            // Per erofs-utils lib/zmap.c z_erofs_load_compact_lcluster (lines 218-242):
            // pblk = stored_pblk + nblk
            // where nblk is computed by scanning backward from the current cluster to pack start:
            // - for each non-NONHEAD cluster: nblk++
            // - for NONHEAD with CBLKCNT flag: nblk += cblks, i--
            // - for NONHEAD without CBLKCNT: i -= (delta - 2) (big_pcluster only)

            let big_pcluster = (z_advise & Z_EROFS_ADVISE_BIG_PCLUSTER_1) != 0;
            // Important: in non-big_pcluster mode nblk starts at 1 (see erofs-utils zmap.c:207).
            let mut nblk = if !big_pcluster { 1u32 } else { 0u32 };

            if !big_pcluster {
                // nblk calculation for non-big_pcluster mode.
                let mut i = in_pack_idx as i32;
                while i > 0 {
                    i -= 1;
                    let scan_bit_pos = (i as usize * pack_encodebits) as u32;
                    let (scan_lo, scan_type) = Self::decode_compactedbits(
                        lobits,
                        &indices_buffer[pack_offset..],
                        scan_bit_pos,
                    );

                    if scan_type == Z_EROFS_LCLUSTER_TYPE_NONHEAD {
                        i -= scan_lo as i32;
                    }

                    if i >= 0 {
                        nblk += 1;
                    }
                }
            } else {
                // nblk calculation for big_pcluster mode.
                let mut i = in_pack_idx as i32;
                while i > 0 {
                    i -= 1;
                    let scan_bit_pos = (i as usize * pack_encodebits) as u32;
                    let (scan_lo, scan_type) = Self::decode_compactedbits(
                        lobits,
                        &indices_buffer[pack_offset..],
                        scan_bit_pos,
                    );

                    if scan_type == Z_EROFS_LCLUSTER_TYPE_NONHEAD {
                        // Check CBLKCNT flag (bit 11 for lobits=12).
                        let cblkcnt_bit = 1u32 << (lobits - 1);
                        if (scan_lo & cblkcnt_bit) != 0 {
                            i -= 1;
                            nblk += scan_lo & !(cblkcnt_bit);
                            continue;
                        }
                        // big_pcluster should not have plain d0 == 1.
                        // lo=0 may appear in edge cases; skip for now.
                        if scan_lo == 0 {
                            log::debug!(
                                "[WARN] big_pcluster NONHEAD with lo=0 at cluster {}, i={}",
                                lcn,
                                i
                            );
                            continue;
                        }
                        if scan_lo == 1 {
                            log::debug!(
                                "[WARN] big_pcluster NONHEAD with lo=1, scan_lo={}",
                                scan_lo
                            );
                            return Err(ErofsError::UnsupportedFeature(format!(
                                "invalid NONHEAD delta {} in big_pcluster",
                                scan_lo
                            )));
                        }
                        i -= scan_lo as i32 - 2;
                        continue;
                    }
                    nblk += 1;
                }
            }

            let pblk = stored_pblk + nblk;

            log::debug!(
                "Cluster {}: stored_pblk={}, nblk={}, pblk={}",
                lcn,
                stored_pblk,
                nblk,
                pblk
            );

            // clusterofs is used for HEAD type.
            let clusterofs = lo;

            // Handle data by type.
            if lcluster_type == 0 {
                // PLAIN: Uncompressed data or special cases
                if pblk == 0 || pblk == 0xFFFFFFFF {
                    continue;
                }

                let data_offset = (pblk as u64).saturating_mul(self.block_size as u64);
                let read_size = cluster_size as usize;

                log::debug!(
                    "Cluster {}: PLAIN pblk={}, offset={}, size={}",
                    lcn,
                    pblk,
                    data_offset,
                    read_size
                );

                self.file.seek(SeekFrom::Start(data_offset))?;
                let mut chunk = vec![0u8; read_size];
                let n = self.file.read(&mut chunk)?;
                chunk.truncate(n);
                if n > 0 {
                    // Logical address of a PLAIN cluster = (lcn << cluster_bits) | clusterofs.
                    // Per erofs-utils zmap.c, PLAIN and HEAD use the same LA calculation.
                    let logical_address = ((lcn as u64) << cluster_bits) | (clusterofs as u64);
                    log::debug!(
                        "Cluster {}: PLAIN LA={} (lcn={}, clusterofs={}), read {} bytes",
                        lcn,
                        logical_address,
                        lcn,
                        clusterofs,
                        chunk.len()
                    );
                    data_blocks.push((logical_address, chunk));
                }
            } else if lcluster_type == Z_EROFS_LCLUSTER_TYPE_HEAD1
                || lcluster_type == Z_EROFS_LCLUSTER_TYPE_HEAD2
            {
                // HEAD: compressed data; determine the number of compressed blocks.
                let big_pcluster = (z_advise & Z_EROFS_ADVISE_BIG_PCLUSTER_1) != 0;

                // Select algorithm based on HEAD1 or HEAD2.
                // Note: if algorithm_head2=0, no separate HEAD2 algorithm; fall back to HEAD1.
                let algorithm = if lcluster_type == Z_EROFS_LCLUSTER_TYPE_HEAD2 {
                    if algorithm_head2 == 0 {
                        algorithm_head1 // HEAD2 has no dedicated algorithm; fall back to HEAD1
                    } else {
                        algorithm_head2
                    }
                } else {
                    algorithm_head1
                };

                log::debug!(
                    "Cluster {}: HEAD type={}, selected algorithm={}",
                    lcn,
                    lcluster_type,
                    algorithm
                );

                // Per erofs-utils/lib/zmap.c z_erofs_get_extent_compressedlen:
                // in non-big_pcluster mode, default compressedblks = 1.
                // in big_pcluster mode, read from NONHEAD CBLKCNT flag.
                let mut num_blocks = 1u32;

                if big_pcluster && lcn + 1 < num_clusters {
                    // Compute index parameters for the next cluster.
                    let (
                        next_pack_offset,
                        next_in_pack_idx,
                        _next_pack_vcnt,
                        _next_pack_amortizedshift,
                        next_pack_encodebits,
                        _next_stored_pblk,
                    ) = calc_pack_params(lcn + 1);

                    let next_bit_pos = (next_in_pack_idx * next_pack_encodebits) as u32;
                    let (next_lo, next_type) = Self::decode_compactedbits(
                        lobits,
                        &indices_buffer[next_pack_offset..],
                        next_bit_pos,
                    );

                    log::debug!(
                        "Cluster {}: next cluster lcn={}, type={}, lo={}",
                        lcn,
                        lcn + 1,
                        next_type,
                        next_lo
                    );

                    // If the next cluster is NONHEAD and has the CBLKCNT flag.
                    if next_type == Z_EROFS_LCLUSTER_TYPE_NONHEAD {
                        let cblkcnt_flag = Z_EROFS_LI_D0_CBLKCNT;
                        if (next_lo & cblkcnt_flag) != 0 {
                            // Extract compressed block count (strip the flag bit).
                            num_blocks = next_lo & !cblkcnt_flag;
                            log::debug!(
                                "Cluster {}: detected CBLKCNT flag, next_lo=0x{:x}, num_blocks={}",
                                lcn,
                                next_lo,
                                num_blocks
                            );
                        }
                    }
                }

                log::debug!("Cluster {}: final num_blocks={}", lcn, num_blocks);

                // Compute m_llen (logical length of the extent) first.
                // This value is used as expected_size during decompression.
                let m_la = ((lcn as u64) << cluster_bits) | (clusterofs as u64);
                let mut m_llen = inode_info.size.checked_sub(m_la).ok_or_else(|| {
                    ErofsError::Io(std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        format!("无效的逻辑偏移: m_la={} > i_size={}", m_la, inode_info.size),
                    ))
                })?;
                let mut scan_lcn = lcn + 1;

                while scan_lcn < num_clusters {
                    let (
                        scan_pack_offset,
                        scan_in_pack_idx,
                        _scan_pack_vcnt,
                        _scan_pack_amortizedshift,
                        scan_pack_encodebits,
                        _scan_stored_pblk,
                    ) = calc_pack_params(scan_lcn);

                    let scan_bit_pos = (scan_in_pack_idx * scan_pack_encodebits) as u32;
                    let (scan_lo, scan_type) = Self::decode_compactedbits(
                        lobits,
                        &indices_buffer[scan_pack_offset..],
                        scan_bit_pos,
                    );

                    if scan_type == Z_EROFS_LCLUSTER_TYPE_HEAD1
                        || scan_type == Z_EROFS_LCLUSTER_TYPE_HEAD2
                        || scan_type == 0
                    {
                        let next_la = ((scan_lcn as u64) << cluster_bits) | (scan_lo as u64);
                        if next_la <= m_la {
                            return Err(ErofsError::Io(std::io::Error::new(
                                std::io::ErrorKind::InvalidData,
                                format!("无效的 extent 边界: next_la={} <= m_la={}", next_la, m_la),
                            )));
                        }
                        m_llen = next_la - m_la;
                        log::debug!(
                            "Cluster {}: found next HEAD/PLAIN @ lcn={}, next_la={}, m_llen={}",
                            lcn,
                            scan_lcn,
                            next_la,
                            m_llen
                        );
                        break;
                    }

                    if scan_type == Z_EROFS_LCLUSTER_TYPE_NONHEAD {
                        scan_lcn += 1;
                    } else {
                        break;
                    }
                }

                // Check cache.
                let cache_key = pblk;
                let decompressed_pcluster = if let Some(cached_data) =
                    pcluster_cache.get(&cache_key)
                {
                    log::debug!("Cluster {}: pcluster cache hit pblk={}", lcn, pblk);
                    cached_data.clone()
                } else {
                    // Cache miss: read from disk and decompress.
                    log::debug!(
                        "Cluster {}: cache miss, reading from disk pblk={}, algorithm={}, m_llen={}",
                        lcn,
                        pblk,
                        algorithm,
                        m_llen
                    );

                    let data_offset = (pblk as u64).saturating_mul(self.block_size as u64);

                    // Compressed data size = num_blocks * block_size.
                    let compressed_size = num_blocks as usize * self.block_size as usize;

                    log::debug!(
                        "Cluster {}: reading compressed data offset={}, compressed_size={} (num_blocks={}, clusterofs={} [logical offset])",
                        lcn,
                        data_offset,
                        compressed_size,
                        num_blocks,
                        clusterofs
                    );

                    self.file.seek(SeekFrom::Start(data_offset))?;
                    let mut compressed_data = vec![0u8; compressed_size];
                    let n = self.file.read(&mut compressed_data)?;
                    compressed_data.truncate(n);

                    log::debug!(
                        "Cluster {}: actually read {} bytes of compressed data",
                        lcn,
                        n
                    );

                    // Use m_llen as expected_size (decompressed size of the pcluster).
                    let expected_size = usize::try_from(m_llen).map_err(|_| {
                        ErofsError::Io(std::io::Error::new(
                            std::io::ErrorKind::InvalidData,
                            format!("解压目标大小超过平台限制: {}", m_llen),
                        ))
                    })?;
                    let mut pcluster_data = Vec::new();

                    // Note: algorithm=0 in some images means using the default compression algorithm (such as LZ4/LZ4HC)
                    // Instead of "uncompressed"! So you need to try LZ4 decompression
                    // Try LZ4 decompression (algorithm=0 or Z_EROFS_COMPRESSION_LZ4)
                    if algorithm == 0 || algorithm == Z_EROFS_COMPRESSION_LZ4 {
                        match self.decompress_lz4(&compressed_data, expected_size) {
                            Ok(chunk) => {
                                log::debug!(
                                    "Cluster {}: LZ4 解压缩成功 {} -> {} 字节",
                                    lcn,
                                    compressed_data.len(),
                                    chunk.len()
                                );
                                // Output the first 16 bytes for debugging
                                if chunk.len() >= 16 {
                                    log::debug!(
                                        "Cluster {}: pcluster前16字节: {:02x?}",
                                        lcn,
                                        &chunk[0..16]
                                    );
                                }
                                pcluster_data = chunk;
                            }
                            Err(e) => {
                                log::debug!("Cluster {}: LZ4 解压缩失败: {:?}", lcn, e);
                            }
                        }
                    }
                    // DEFLATE decompression (using common trait)
                    else if algorithm == Z_EROFS_COMPRESSION_DEFLATE {
                        match self.decompress_with_padding(
                            &compressed_data,
                            expected_size,
                            Algorithm::Deflate.decompressor(),
                        ) {
                            Ok(chunk) => {
                                log::debug!(
                                    "Cluster {}: DEFLATE 解压缩成功 {} -> {} 字节",
                                    lcn,
                                    compressed_data.len(),
                                    chunk.len()
                                );
                                pcluster_data = chunk;
                            }
                            Err(e) => {
                                log::debug!("Cluster {}: DEFLATE 解压缩失败: {}", lcn, e);
                            }
                        }
                    }
                    // LZMA decompression (using common trait)
                    else if algorithm == Z_EROFS_COMPRESSION_LZMA {
                        match self.decompress_with_padding(
                            &compressed_data,
                            expected_size,
                            Algorithm::MicroLzma.decompressor(),
                        ) {
                            Ok(chunk) => {
                                log::debug!(
                                    "Cluster {}: LZMA 解压缩成功 {} -> {} 字节",
                                    lcn,
                                    compressed_data.len(),
                                    chunk.len()
                                );
                                pcluster_data = chunk;
                            }
                            Err(e) => {
                                log::debug!("Cluster {}: LZMA 解压缩失败: {}", lcn, e);
                            }
                        }
                    }
                    // ZSTD decompression (using common trait)
                    else if algorithm == Z_EROFS_COMPRESSION_ZSTD {
                        match self.decompress_with_padding(
                            &compressed_data,
                            expected_size,
                            Algorithm::Zstd.decompressor(),
                        ) {
                            Ok(chunk) => {
                                log::debug!(
                                    "Cluster {}: ZSTD 解压缩成功 {} -> {} 字节",
                                    lcn,
                                    compressed_data.len(),
                                    chunk.len()
                                );
                                pcluster_data = chunk;
                            }
                            Err(e) => {
                                log::debug!("Cluster {}: ZSTD 解压缩失败: {}", lcn, e);
                            }
                        }
                    }

                    // Cache decompressed pcluster data
                    if !pcluster_data.is_empty() {
                        pcluster_cache.insert(cache_key, pcluster_data.clone());
                    }
                    pcluster_data
                };

                // Extract m_llen bytes from decompressed pcluster
                // If pcluster is less than m_llen, fill it with zeros
                let expected_size = usize::try_from(m_llen).map_err(|_| {
                    ErofsError::Io(std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        format!("extent 大小超过平台限制: {}", m_llen),
                    ))
                })?;
                let extract_len = expected_size.min(decompressed_pcluster.len());

                log::debug!(
                    "Cluster {}: HEAD m_la={}, m_llen={}, pcluster_size={}, extract_len={}",
                    lcn,
                    m_la,
                    m_llen,
                    decompressed_pcluster.len(),
                    extract_len
                );

                if m_llen > 0 {
                    let extent_data = if extract_len < expected_size {
                        // Insufficient pcluster, zero padding required
                        let mut data = Vec::with_capacity(expected_size);
                        data.extend_from_slice(&decompressed_pcluster[0..extract_len]);
                        data.resize(expected_size, 0);
                        log::debug!(
                            "Cluster {}: pcluster不足，零填充 {} 字节",
                            lcn,
                            expected_size - extract_len
                        );
                        data
                    } else {
                        decompressed_pcluster[0..expected_size].to_vec()
                    };
                    data_blocks.push((m_la, extent_data));
                }
            }
        }

        // Sort data blocks by logical address
        data_blocks.sort_by_key(|(la, _)| *la);

        log::debug!("数据块排序完成，共 {} 个块", data_blocks.len());

        // Assemble final files
        let file_size = usize::try_from(inode_info.size).map_err(|_| {
            ErofsError::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("文件大小超过平台限制: {}", inode_info.size),
            ))
        })?;
        let mut decompressed_data = Vec::with_capacity(file_size.min(8 * 1024 * 1024));
        let mut current_pos = 0u64;

        for (i, (la, data)) in data_blocks.iter().enumerate() {
            // Calculate the actual length of the current extent:
            // If there is a next extent, truncate to the LA of the next extent
            // Otherwise truncate to file size
            let next_la = if i + 1 < data_blocks.len() {
                data_blocks[i + 1].0
            } else {
                inode_info.size
            };

            let actual_len = if *la + data.len() as u64 > next_la {
                // Need to truncate
                if next_la > *la {
                    let truncated_len = usize::try_from(next_la - *la).map_err(|_| {
                        ErofsError::Io(std::io::Error::new(
                            std::io::ErrorKind::InvalidData,
                            format!("extent 截断长度超过平台限制: {}", next_la - *la),
                        ))
                    })?;
                    log::debug!(
                        "Extent 截断: LA={}, 原长度={}, 截断后={} (下一个LA={})",
                        la,
                        data.len(),
                        truncated_len,
                        next_la
                    );
                    truncated_len
                } else {
                    // next_la <= la, skip this block
                    log::debug!("Extent 跳过: LA={}, 下一个LA={} (重叠或乱序)", la, next_la);
                    0
                }
            } else {
                data.len()
            };

            // skip blocks of length 0
            if actual_len == 0 {
                continue;
            }

            log::debug!(
                "组装数据块: LA={}, 当前位置={}, 数据长度={} 字节",
                la,
                current_pos,
                actual_len
            );

            // If LA is greater than current position, fill with 0
            if *la > current_pos {
                let gap = usize::try_from(*la - current_pos).map_err(|_| {
                    ErofsError::Io(std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        format!("extent 空洞大小超过平台限制: {}", *la - current_pos),
                    ))
                })?;
                log::debug!("填充空洞: 从 {} 到 {}，共 {} 字节", current_pos, la, gap);
                decompressed_data.resize(decompressed_data.len() + gap, 0);
                current_pos = *la;
            }

            // Write data (truncated length)
            decompressed_data.extend_from_slice(&data[..actual_len]);
            current_pos += actual_len as u64;
        }

        // Truncate to actual file size
        decompressed_data.truncate(file_size);

        log::debug!(
            "多 cluster 解压缩完成: {} 字节（期望 {} 字节）",
            decompressed_data.len(),
            inode_info.size
        );

        Ok(decompressed_data)
    }

    fn decompress_lz4(&self, compressed: &[u8], expected_size: usize) -> Result<Vec<u8>> {
        log::debug!(
            "LZ4 解压缩: compressed_size={}, expected_size={}",
            compressed.len(),
            expected_size
        );

        // Skip leading 0 bytes only if ZERO_PADDING feature flag is set
        let has_zero_padding =
            (self.superblock.feature_incompat & EROFS_FEATURE_INCOMPAT_ZERO_PADDING) != 0;
        let mut start = 0;

        if has_zero_padding {
            while start < compressed.len() && compressed[start] == 0 {
                start += 1;
            }

            if start >= compressed.len() {
                return Err(ErofsError::Io(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "压缩数据全部为零",
                )));
            }

            log::debug!("ZERO_PADDING特性启用，跳过 {} 字节的 0-padding", start);
        } else {
            log::debug!("ZERO_PADDING特性未启用，不跳过前导零字节");
        }

        // Use the official lz4 library to decompress (supports LZ4HC)
        // Try various expected_sizes: first use the provided one, then use a larger one
        // Avoid using None as it may cause the first few bytes of the decompressed data to be lost
        let sizes_to_try = [
            Some(expected_size as i32),
            Some((expected_size * 2) as i32),
            Some((expected_size * 4) as i32),
            Some((expected_size * 6) as i32),
            Some((expected_size * 8) as i32),
            Some((expected_size * 10) as i32),
            Some((expected_size * 16) as i32),
        ];

        for (idx, size_opt) in sizes_to_try.iter().enumerate() {
            match lz4::block::decompress(&compressed[start..], *size_opt) {
                Ok(decompressed) => {
                    log::debug!(
                        "解压缩成功（尝试{}，size={:?}）: {} 字节",
                        idx + 1,
                        size_opt,
                        decompressed.len()
                    );
                    return Ok(decompressed);
                }
                Err(e) if idx == sizes_to_try.len() - 1 => {
                    log::debug!("LZ4 官方库所有尝试都失败: {:?}", e);
                }
                Err(_) => {
                    // Continue to try the next size
                }
            }
        }

        // Fallback to lz4_flex (may work for some formats)
        // Method 1: Use lz4_flex standard decompression
        if let Ok(decompressed) = lz4_flex::decompress(&compressed[start..], expected_size) {
            log::debug!("解压缩成功（lz4_flex）: {} 字节", decompressed.len());
            return Ok(decompressed);
        }

        log::debug!("所有 LZ4 解压缩方法都失败");
        Err(ErofsError::Io(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "所有 LZ4 解压缩方法都失败",
        )))
    }

    // General decompression helper functions: handle the ZERO_PADDING attribute and call the decompressor
    fn decompress_with_padding(
        &self,
        compressed: &[u8],
        expected_size: usize,
        decompressor: Box<dyn Decompressor>,
    ) -> Result<Vec<u8>> {
        // Check ZERO_PADDING feature flag
        let has_zero_padding =
            (self.superblock.feature_incompat & EROFS_FEATURE_INCOMPAT_ZERO_PADDING) != 0;
        let mut start = 0;

        if has_zero_padding {
            while start < compressed.len() && compressed[start] == 0 {
                start += 1;
            }

            if start > 0 {
                log::debug!("{} ZERO_PADDING 跳过 {} 字节", decompressor.name(), start);
            }
        }

        if start >= compressed.len() {
            return Err(ErofsError::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "压缩数据全部为零",
            )));
        }

        // Call decompressor
        decompressor
            .decompress(&compressed[start..], expected_size)
            .map_err(|e| {
                ErofsError::Io(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    format!("{} 解压缩失败: {}", decompressor.name(), e),
                ))
            })
    }
}
