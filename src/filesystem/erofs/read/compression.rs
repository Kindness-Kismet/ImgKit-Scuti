// EROFS 压缩文件读取器

use super::volume::ErofsVolume;
use crate::compression::{Algorithm, Decompressor};
use crate::filesystem::erofs::*;
use std::io::{Read, Seek, SeekFrom};
use zerocopy::TryFromBytes;

// 压缩文件读取操作所需的参数
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

        // 1. 定位压缩元数据: inode 偏移 + inode 大小 + xattr 大小, 按 8 字节对齐
        // COMPRESSED_COMPACT 格式下, 压缩元数据 (map header 与索引) inline 存放在 inode 之后
        let inode_offset = self.nid_to_offset(inode_info.nid);
        let inode_size = if inode_info.is_compact { 32 } else { 64 };
        let xattr_size = self.xattr_ibody_size(inode_info.xattr_icount);

        // 对齐到 8 字节边界
        let header_offset = ((inode_offset + inode_size + xattr_size as u64) + 7) & !7;

        log::debug!(
            "compression metadata offset: {} (inode={}, inode_size={}, xattr_size={})",
            header_offset,
            inode_offset,
            inode_size,
            xattr_size
        );

        // 2. 读取 z_erofs_map_header
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

        // 3. 读取并解压所需的全部 cluster
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

        // 读取完整的压缩文件 (多 cluster)
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

    // 位压缩索引解码辅助函数
    // 从位压缩缓冲区中取出一条索引项
    // 参数: lobits - 低位位数, buffer - 数据缓冲区, bit_pos - 起始比特位置
    // 返回: (lo_value, type)
    fn decode_compactedbits(lobits: u32, buffer: &[u8], bit_pos: u32) -> (u32, u16) {
        let byte_offset = (bit_pos / 8) as usize;
        let bit_offset = bit_pos % 8;

        // 读取一个 32 位小端值
        if byte_offset + 4 > buffer.len() {
            return (0, 0);
        }

        let v = u32::from_le_bytes([
            buffer[byte_offset],
            buffer[byte_offset + 1],
            buffer[byte_offset + 2],
            buffer[byte_offset + 3],
        ]) >> bit_offset;

        // 取出低位数值
        let lo = v & ((1 << lobits) - 1);

        // 取出 2 位的类型字段
        let lcluster_type = ((v >> lobits) & 3) as u16;

        (lo, lcluster_type)
    }

    fn read_multi_cluster_compressed_file(
        &mut self,
        inode_info: &InodeInfo,
        params: CompressionParams,
    ) -> Result<Vec<u8>> {
        use std::collections::HashMap;

        // 解构参数
        let CompressionParams {
            header_offset,
            algorithm_head1,
            algorithm_head2,
            cluster_size,
            num_clusters,
            z_advise,
        } = params;

        // 已解压 pcluster 数据的缓存: pblk -> 解压后的数据
        // 同一个 pblk 只会使用一种压缩算法, 因此键中不包含算法
        let mut pcluster_cache: HashMap<u32, Vec<u8>> = HashMap::new();

        // 数据块列表: (逻辑地址, 数据)
        // 组装最终文件前按 LA 排序
        let mut data_blocks: Vec<(u64, Vec<u8>)> = Vec::new();

        // 计算位压缩参数
        let cluster_bits = (cluster_size as f32).log2() as u32;
        const Z_EROFS_LI_D0_CBLKCNT: u32 = 1 << 11;
        let lobits = cluster_bits.max((Z_EROFS_LI_D0_CBLKCNT as f32).log2() as u32 + 1);

        log::debug!(
            "bit-packing params: cluster_bits={}, lobits={}",
            cluster_bits,
            lobits
        );

        // 确定 vcnt (每个 pack 的 lcluster 数) 与 amortizedshift (每条索引字节数的 log2)
        // 判断顺序不可颠倒: 先检查较小的 cluster_bits
        let (vcnt, amortizedshift) = if cluster_bits <= 12 {
            (16, 1) // 每条索引 2 字节, 每个 pack 16 条 (compact 模式)
        } else if cluster_bits <= 14 {
            (2, 2) // 每条索引 4 字节, 每个 pack 2 条 (标准模式)
        } else {
            return Err(ErofsError::UnsupportedFeature(format!(
                "cluster_bits {} too large",
                cluster_bits
            )));
        };

        // 计算每条索引占用的编码比特数
        // encodebits = ((vcnt << amortizedshift) - 4) * 8 / vcnt
        let _encodebits = (((vcnt << amortizedshift) - 4) * 8) / vcnt;

        // 索引区起始偏移
        let ebase = header_offset + 8;

        // 计算混合索引格式参数 (参考 erofs-utils lib/zmap.c:126-130)
        // compacted_4b_initial: 起始若干 cluster 使用 4 字节索引以对齐到 32 字节
        let compacted_4b_initial = (((32 - (ebase % 32)) / 4) & 7) as usize;

        // compacted_2b: 中间使用 2 字节索引的 cluster (必须是 16 的倍数)
        let compacted_2b = if (z_advise & 0x1) != 0 && compacted_4b_initial < num_clusters {
            // Z_EROFS_ADVISE_COMPACTED_2B = 0x0001
            ((num_clusters - compacted_4b_initial) / 16) * 16
        } else {
            0
        };

        // compacted_4b_end: 剩余 cluster 使用 4 字节索引
        let compacted_4b_end = num_clusters - compacted_4b_initial - compacted_2b;

        log::debug!(
            "index format: compacted_4b_initial={}, compacted_2b={}, compacted_4b_end={}, total={}",
            compacted_4b_initial,
            compacted_2b,
            compacted_4b_end,
            num_clusters
        );

        // 计算索引缓冲区总大小
        // 每个 pack 占用 pack_size 字节, 含索引数据与 stored_pblk (末尾 4 字节)
        // 4b pack (vcnt=2, amortizedshift=2): pack_size = 2 << 2 = 8 字节
        // 2b pack (vcnt=16, amortizedshift=1): pack_size = 16 << 1 = 32 字节
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

        // 以十六进制打印索引缓冲区前 64 字节
        if indices_buffer.len() >= 64 {
            log::debug!(
                "index buffer first 64 bytes: {:02x?}",
                &indices_buffer[0..64]
            );
        } else {
            log::debug!("index buffer all bytes: {:02x?}", &indices_buffer);
        }

        // 辅助闭包: 计算指定 lcn 对应的 pack 参数
        let calc_pack_params = |target_lcn: usize| -> (usize, usize, usize, usize, usize, u32) {
            let mut adjusted_lcn = target_lcn;
            let mut pos = 0usize;
            let mut amortizedshift_local = 2;
            let mut region_start = 0usize; // 当前区域的起始偏移

            if adjusted_lcn >= compacted_4b_initial {
                pos += compacted_4b_initial * 4;
                region_start = compacted_4b_initial * 4; // 2 字节索引区从此处开始
                adjusted_lcn -= compacted_4b_initial;

                if adjusted_lcn < compacted_2b {
                    amortizedshift_local = 1;
                } else {
                    pos += compacted_2b * 2;
                    region_start = compacted_4b_initial * 4 + compacted_2b * 2; // 4 字节尾部区从此处开始
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

            // pack_start 需相对当前区域的起点对齐
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
                // 不完整的 pack (最后一个 pack) 中, stored_pblk 可能只有部分字节
                // 尝试从备用位置读取
                let encodebits_for_calc = ((pack_size - 4) * 8) / vcnt_local;
                let index_bytes = encodebits_for_calc.div_ceil(8);
                let alt_pblk_offset = pos + index_bytes;

                if alt_pblk_offset + 4 <= indices_buffer.len() {
                    // 可以读取完整的 4 字节
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
                    // 不完整的 pack 没有足够字节存放 stored_pblk, 回退使用 raw_blkaddr
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

        // 解压全部 cluster
        // 分两遍: 第一遍处理所有 HEAD 与 PLAIN cluster (填充缓存),
        //         第二遍处理所有 NONHEAD cluster (使用缓存)

        // 第一遍: 处理 PLAIN 与 HEAD cluster
        for lcn in 0..num_clusters {
            // 计算该 lcn 的索引参数
            let (
                pack_offset,
                in_pack_idx,
                pack_vcnt,
                _pack_amortizedshift,
                pack_encodebits,
                stored_pblk,
            ) = calc_pack_params(lcn);

            // 解码该 cluster 的索引
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

            // 按 lcluster 类型分别处理
            if lcluster_type == Z_EROFS_LCLUSTER_TYPE_NONHEAD {
                // 第一遍跳过 NONHEAD, 留到第二遍处理
                log::debug!(
                    "Cluster {}: NONHEAD (delta={}) - skipped in pass 1",
                    lcn,
                    lo
                );
                continue;
            }

            // 计算物理块地址
            // 参考 erofs-utils lib/zmap.c 中 z_erofs_load_compact_lcluster (第 218-242 行):
            // pblk = stored_pblk + nblk
            // 其中 nblk 由当前 cluster 向前扫描至 pack 起点得出:
            // - 每遇到一个非 NONHEAD cluster: nblk++
            // - 带 CBLKCNT 标志的 NONHEAD: nblk += cblks, i--
            // - 不带 CBLKCNT 的 NONHEAD: i -= (delta - 2) (仅 big pcluster)

            let big_pcluster = (z_advise & Z_EROFS_ADVISE_BIG_PCLUSTER_1) != 0;
            // 注意: 非 big pcluster 模式下 nblk 从 1 开始 (见 erofs-utils zmap.c:207)
            let mut nblk = if !big_pcluster { 1u32 } else { 0u32 };

            if !big_pcluster {
                // 非 big pcluster 模式下的 nblk 计算
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
                // big pcluster 模式下的 nblk 计算
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
                        // 检查 CBLKCNT 标志 (lobits=12 时位于 bit 11)
                        let cblkcnt_bit = 1u32 << (lobits - 1);
                        if (scan_lo & cblkcnt_bit) != 0 {
                            i -= 1;
                            nblk += scan_lo & !(cblkcnt_bit);
                            continue;
                        }
                        // big pcluster 下不应出现普通的 d0 == 1
                        // 边界情况下可能出现 lo=0, 此处暂时跳过
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

            // clusterofs 用于 HEAD 类型
            let clusterofs = lo;

            // 按类型处理数据
            if lcluster_type == 0 {
                // PLAIN: 未压缩数据或特殊情况
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
                    // PLAIN cluster 的逻辑地址 = (lcn << cluster_bits) | clusterofs
                    // 按 erofs-utils zmap.c, PLAIN 与 HEAD 使用相同的 LA 计算方式
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
                // HEAD: 压缩数据, 需确定压缩块数量
                let big_pcluster = (z_advise & Z_EROFS_ADVISE_BIG_PCLUSTER_1) != 0;

                // 依据 HEAD1 或 HEAD2 选择算法
                // 注意: algorithm_head2=0 表示没有独立的 HEAD2 算法, 回退到 HEAD1
                let algorithm = if lcluster_type == Z_EROFS_LCLUSTER_TYPE_HEAD2 {
                    if algorithm_head2 == 0 {
                        algorithm_head1 // HEAD2 无专用算法, 回退到 HEAD1
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

                // 参考 erofs-utils/lib/zmap.c 中 z_erofs_get_extent_compressedlen:
                // 非 big pcluster 模式下, compressedblks 默认为 1
                // big pcluster 模式下, 从 NONHEAD 的 CBLKCNT 标志中读取
                let mut num_blocks = 1u32;

                if big_pcluster && lcn + 1 < num_clusters {
                    // 计算下一个 cluster 的索引参数
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

                    // 若下一个 cluster 为 NONHEAD 且带有 CBLKCNT 标志
                    if next_type == Z_EROFS_LCLUSTER_TYPE_NONHEAD {
                        let cblkcnt_flag = Z_EROFS_LI_D0_CBLKCNT;
                        if (next_lo & cblkcnt_flag) != 0 {
                            // 取出压缩块数量 (去掉标志位)
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

                // 先计算 m_llen (extent 的逻辑长度)
                // 解压时该值作为 expected_size 使用
                let m_la = ((lcn as u64) << cluster_bits) | (clusterofs as u64);
                let mut m_llen = inode_info.size.checked_sub(m_la).ok_or_else(|| {
                    ErofsError::Io(std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        format!("invalid offset: m_la={} > i_size={}", m_la, inode_info.size),
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
                                format!("invalid extent: next_la={} <= m_la={}", next_la, m_la),
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

                // 检查缓存
                let cache_key = pblk;
                let decompressed_pcluster = if let Some(cached_data) =
                    pcluster_cache.get(&cache_key)
                {
                    log::debug!("Cluster {}: pcluster cache hit pblk={}", lcn, pblk);
                    cached_data.clone()
                } else {
                    // 缓存未命中: 从磁盘读取并解压
                    log::debug!(
                        "Cluster {}: cache miss, reading from disk pblk={}, algorithm={}, m_llen={}",
                        lcn,
                        pblk,
                        algorithm,
                        m_llen
                    );

                    let data_offset = (pblk as u64).saturating_mul(self.block_size as u64);

                    // 压缩数据大小 = num_blocks * block_size
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

                    // 以 m_llen 作为 expected_size (pcluster 解压后的大小)
                    let expected_size = usize::try_from(m_llen).map_err(|_| {
                        ErofsError::Io(std::io::Error::new(
                            std::io::ErrorKind::InvalidData,
                            format!("decompressed size exceeds platform limit: {}", m_llen),
                        ))
                    })?;
                    let mut pcluster_data = Vec::new();

                    // 注意: 部分镜像中 algorithm=0 表示使用默认压缩算法 (如 LZ4/LZ4HC),
                    // 而不是"未压缩", 因此仍需尝试 LZ4 解压
                    // 尝试 LZ4 解压 (algorithm=0 或 Z_EROFS_COMPRESSION_LZ4)
                    if algorithm == 0 || algorithm == Z_EROFS_COMPRESSION_LZ4 {
                        match self.decompress_lz4(&compressed_data, expected_size) {
                            Ok(chunk) => {
                                log::debug!(
                                    "Cluster {}: LZ4 decompressed {} -> {} bytes",
                                    lcn,
                                    compressed_data.len(),
                                    chunk.len()
                                );
                                // 输出前 16 字节便于调试
                                if chunk.len() >= 16 {
                                    log::debug!(
                                        "Cluster {}: pcluster first 16 bytes: {:02x?}",
                                        lcn,
                                        &chunk[0..16]
                                    );
                                }
                                pcluster_data = chunk;
                            }
                            Err(e) => {
                                log::debug!("Cluster {}: LZ4 decompression failed: {:?}", lcn, e);
                            }
                        }
                    }
                    // DEFLATE 解压 (使用通用 trait)
                    else if algorithm == Z_EROFS_COMPRESSION_DEFLATE {
                        match self.decompress_with_padding(
                            &compressed_data,
                            expected_size,
                            Algorithm::Deflate.decompressor(),
                        ) {
                            Ok(chunk) => {
                                log::debug!(
                                    "Cluster {}: DEFLATE decompressed {} -> {} bytes",
                                    lcn,
                                    compressed_data.len(),
                                    chunk.len()
                                );
                                pcluster_data = chunk;
                            }
                            Err(e) => {
                                log::debug!("Cluster {}: DEFLATE decompression failed: {}", lcn, e);
                            }
                        }
                    }
                    // LZMA 解压 (使用通用 trait)
                    else if algorithm == Z_EROFS_COMPRESSION_LZMA {
                        match self.decompress_with_padding(
                            &compressed_data,
                            expected_size,
                            Algorithm::MicroLzma.decompressor(),
                        ) {
                            Ok(chunk) => {
                                log::debug!(
                                    "Cluster {}: LZMA decompressed {} -> {} bytes",
                                    lcn,
                                    compressed_data.len(),
                                    chunk.len()
                                );
                                pcluster_data = chunk;
                            }
                            Err(e) => {
                                log::debug!("Cluster {}: LZMA decompression failed: {}", lcn, e);
                            }
                        }
                    }
                    // ZSTD 解压 (使用通用 trait)
                    else if algorithm == Z_EROFS_COMPRESSION_ZSTD {
                        match self.decompress_with_padding(
                            &compressed_data,
                            expected_size,
                            Algorithm::Zstd.decompressor(),
                        ) {
                            Ok(chunk) => {
                                log::debug!(
                                    "Cluster {}: ZSTD decompressed {} -> {} bytes",
                                    lcn,
                                    compressed_data.len(),
                                    chunk.len()
                                );
                                pcluster_data = chunk;
                            }
                            Err(e) => {
                                log::debug!("Cluster {}: ZSTD decompression failed: {}", lcn, e);
                            }
                        }
                    }

                    // 缓存解压后的 pcluster 数据
                    if !pcluster_data.is_empty() {
                        pcluster_cache.insert(cache_key, pcluster_data.clone());
                    }
                    pcluster_data
                };

                // 从解压后的 pcluster 中取出 m_llen 字节
                // 若 pcluster 不足 m_llen, 以零填充
                let expected_size = usize::try_from(m_llen).map_err(|_| {
                    ErofsError::Io(std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        format!("extent size exceeds platform limit: {}", m_llen),
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
                        // pcluster 不足, 需要零填充
                        let mut data = Vec::with_capacity(expected_size);
                        data.extend_from_slice(&decompressed_pcluster[0..extract_len]);
                        data.resize(expected_size, 0);
                        log::debug!(
                            "Cluster {}: pcluster too short, zero-padding {} bytes",
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

        // 按逻辑地址对数据块排序
        data_blocks.sort_by_key(|(la, _)| *la);

        log::debug!("sorted data blocks, {} blocks in total", data_blocks.len());

        // 组装最终文件
        let file_size = usize::try_from(inode_info.size).map_err(|_| {
            ErofsError::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("file size exceeds platform limit: {}", inode_info.size),
            ))
        })?;
        let mut decompressed_data = Vec::with_capacity(file_size.min(8 * 1024 * 1024));
        let mut current_pos = 0u64;

        for (i, (la, data)) in data_blocks.iter().enumerate() {
            // 计算当前 extent 的实际长度:
            // 若存在下一个 extent, 截断到下一个 extent 的 LA
            // 否则截断到文件大小
            let next_la = if i + 1 < data_blocks.len() {
                data_blocks[i + 1].0
            } else {
                inode_info.size
            };

            let actual_len = if *la + data.len() as u64 > next_la {
                // 需要截断
                if next_la > *la {
                    let truncated_len = usize::try_from(next_la - *la).map_err(|_| {
                        ErofsError::Io(std::io::Error::new(
                            std::io::ErrorKind::InvalidData,
                            format!(
                                "extent truncation exceeds platform limit: {}",
                                next_la - *la
                            ),
                        ))
                    })?;
                    log::debug!(
                        "extent truncated: LA={}, old length={}, new length={} (next LA={})",
                        la,
                        data.len(),
                        truncated_len,
                        next_la
                    );
                    truncated_len
                } else {
                    // next_la <= la, 跳过该块
                    log::debug!(
                        "extent skipped: LA={}, next LA={} (out of order)",
                        la,
                        next_la
                    );
                    0
                }
            } else {
                data.len()
            };

            // 跳过长度为 0 的块
            if actual_len == 0 {
                continue;
            }

            log::debug!(
                "assembling data block: LA={}, position={}, data length={} bytes",
                la,
                current_pos,
                actual_len
            );

            // 若 LA 大于当前位置, 用 0 填充空洞
            if *la > current_pos {
                let gap = usize::try_from(*la - current_pos).map_err(|_| {
                    ErofsError::Io(std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        format!(
                            "extent gap size exceeds platform limit: {}",
                            *la - current_pos
                        ),
                    ))
                })?;
                log::debug!("filling gap: from {} to {}, {} bytes", current_pos, la, gap);
                decompressed_data.resize(decompressed_data.len() + gap, 0);
                current_pos = *la;
            }

            // 写入数据 (按截断后的长度)
            decompressed_data.extend_from_slice(&data[..actual_len]);
            current_pos += actual_len as u64;
        }

        // 截断到实际文件大小
        decompressed_data.truncate(file_size);

        log::debug!(
            "multi-cluster decompression done: {} bytes (expected {} bytes)",
            decompressed_data.len(),
            inode_info.size
        );

        Ok(decompressed_data)
    }

    fn decompress_lz4(&self, compressed: &[u8], expected_size: usize) -> Result<Vec<u8>> {
        log::debug!(
            "LZ4 decompression: compressed_size={}, expected_size={}",
            compressed.len(),
            expected_size
        );

        // 仅当启用 ZERO_PADDING 特性标志时才跳过前导零字节
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
                    "compressed data is all zeros",
                )));
            }

            log::debug!(
                "ZERO_PADDING enabled, skipping {} bytes of 0-padding",
                start
            );
        } else {
            log::debug!("ZERO_PADDING not enabled, keeping leading zero bytes");
        }

        // 使用官方 lz4 库解压 (支持 LZ4HC)
        // 依次尝试多个 expected_size: 先用传入值, 再逐步放大
        // 避免使用 None, 否则可能丢失解压数据开头的若干字节
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
                        "decompressed (attempt {}, size={:?}): {} bytes",
                        idx + 1,
                        size_opt,
                        decompressed.len()
                    );
                    return Ok(decompressed);
                }
                Err(e) if idx == sizes_to_try.len() - 1 => {
                    log::debug!("all attempts with the LZ4 library failed: {:?}", e);
                }
                Err(_) => {
                    // 继续尝试下一个大小
                }
            }
        }

        // 回退到 lz4_flex (对部分格式可能有效)
        // 方式一: 使用 lz4_flex 标准解压
        if let Ok(decompressed) = lz4_flex::decompress(&compressed[start..], expected_size) {
            log::debug!("decompressed (lz4_flex): {} bytes", decompressed.len());
            return Ok(decompressed);
        }

        log::debug!("all LZ4 decompression methods failed");
        Err(ErofsError::Io(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "all LZ4 decompression methods failed",
        )))
    }

    // 通用解压辅助函数: 处理 ZERO_PADDING 特性并调用解压器
    fn decompress_with_padding(
        &self,
        compressed: &[u8],
        expected_size: usize,
        decompressor: Box<dyn Decompressor>,
    ) -> Result<Vec<u8>> {
        // 检查 ZERO_PADDING 特性标志
        let has_zero_padding =
            (self.superblock.feature_incompat & EROFS_FEATURE_INCOMPAT_ZERO_PADDING) != 0;
        let mut start = 0;

        if has_zero_padding {
            while start < compressed.len() && compressed[start] == 0 {
                start += 1;
            }

            if start > 0 {
                log::debug!(
                    "{} ZERO_PADDING skipped {} bytes",
                    decompressor.name(),
                    start
                );
            }
        }

        if start >= compressed.len() {
            return Err(ErofsError::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "compressed data is all zeros",
            )));
        }

        // 调用解压器
        decompressor
            .decompress(&compressed[start..], expected_size)
            .map_err(|e| {
                ErofsError::Io(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    format!("{} decompression failed: {}", decompressor.name(), e),
                ))
            })
    }
}
