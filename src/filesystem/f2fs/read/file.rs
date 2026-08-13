use super::volume::F2fsVolume;
use crate::filesystem::f2fs::*;
use byteorder::{LittleEndian, ReadBytesExt};
use std::io::{Cursor, Read, Seek};

impl<R: Read + Seek + Send> F2fsVolume<R> {
    pub fn read_file_data(&self, inode: &Inode, nid: Nid) -> Result<Vec<u8>> {
        const MAX_FILE_SIZE: u64 = 16 * 1024 * 1024 * 1024;
        if inode.size == 0 {
            return Ok(Vec::new());
        }
        if inode.size > MAX_FILE_SIZE {
            return Err(F2fsError::InvalidData(format!(
                "file size {} exceeds maximum allowed size {}",
                inode.size, MAX_FILE_SIZE
            )));
        }

        // 检查是否为 inline data
        if inode.inline & F2FS_INLINE_DATA != 0 {
            let node_data = self.read_node(nid)?;
            // inline data 起始于 i_addr 保留槽之后, 偏移取决于 extra_attr
            let inline_offset = if inode.inline & F2FS_EXTRA_ATTR != 0 {
                360 + inode.extra_isize as usize + 4
            } else {
                360 + 4
            };
            let inline_end = node_data.len().saturating_sub(24);
            if inline_offset >= inline_end {
                return Ok(Vec::new());
            }
            let data_len = inode.size.min((inline_end - inline_offset) as u64) as usize;
            return Ok(node_data[inline_offset..inline_offset + data_len].to_vec());
        }

        let blocks = self.read_data_blocks(inode, nid)?;

        // 拼接 block 数据
        let mut data = Vec::with_capacity((inode.size as usize).min(8 * 1024 * 1024));
        let mut remaining = inode.size;

        for block in blocks {
            if remaining == 0 {
                break;
            }

            if block.len() as u64 <= remaining {
                data.extend_from_slice(&block);
                remaining -= block.len() as u64;
            } else {
                data.extend_from_slice(&block[..remaining as usize]);
                remaining = 0;
            }
        }

        Ok(data)
    }

    pub fn read_data_blocks(&self, inode: &Inode, nid: Nid) -> Result<Vec<Vec<u8>>> {
        let mut blocks = Vec::new();
        let expected_blocks = inode.size.div_ceil(F2FS_BLKSIZE as u64);
        let total_blocks = inode.blocks.min(expected_blocks);

        // 读取直接地址
        let node_data = self.read_node(nid)?;
        let direct_addrs = self.get_direct_addrs(&node_data, inode, nid);

        let mut blocks_read = 0u64;
        let mut i = 0;

        while i < direct_addrs.len() && blocks_read < total_blocks {
            let addr = direct_addrs[i];

            match BlockAddr::from(addr) {
                BlockAddr::Null => {
                    i += 1;
                    continue;
                }
                BlockAddr::Compress => {
                    // 处理 compression cluster: 定位实际的压缩数据 block
                    let cluster_size = 4; // F2FS 默认每个 cluster 4 个 block
                    let mut compressed_blocks = Vec::new();

                    // 收集 cluster 内的所有 block
                    for j in 0..cluster_size {
                        if i + j >= direct_addrs.len() {
                            break;
                        }
                        let cluster_addr = direct_addrs[i + j];
                        if cluster_addr != COMPRESS_ADDR
                            && cluster_addr != NULL_ADDR
                            && let BlockAddr::Valid(block) = BlockAddr::from(cluster_addr)
                            && self.is_valid_block(block)
                        {
                            compressed_blocks.push(self.read_block(block)?);
                        }
                    }

                    // 解压
                    if !compressed_blocks.is_empty() {
                        let decompressed =
                            self.decompress_cluster(&compressed_blocks, cluster_size)?;
                        for block in decompressed {
                            if blocks_read < total_blocks {
                                blocks.push(block);
                                blocks_read += 1;
                            }
                        }
                    }

                    i += cluster_size;
                }
                BlockAddr::Valid(block) if self.is_valid_block(block) => {
                    blocks.push(self.read_block(block)?);
                    blocks_read += 1;
                    i += 1;
                }
                _ => {
                    blocks.push(vec![0u8; F2FS_BLKSIZE]);
                    blocks_read += 1;
                    i += 1;
                }
            }
        }

        // 读取间接地址
        if blocks_read < total_blocks {
            let indirect =
                self.read_indirect_blocks(inode, nid, blocks_read, total_blocks - blocks_read)?;
            blocks.extend(indirect);
        }

        Ok(blocks)
    }

    fn decompress_cluster(
        &self,
        compressed_blocks: &[Vec<u8>],
        cluster_size: usize,
    ) -> Result<Vec<Vec<u8>>> {
        // F2FS 压缩格式: 24 字节头部 + 压缩数据
        // 头部: clen(4) + chksum(4) + reserved(16)
        if compressed_blocks.is_empty() {
            return Ok(vec![vec![0u8; F2FS_BLKSIZE]; cluster_size]);
        }

        // 拼接所有压缩 block
        let mut compressed_data = Vec::new();
        for block in compressed_blocks {
            compressed_data.extend_from_slice(block);
        }

        if compressed_data.len() < 24 {
            return Ok(vec![vec![0u8; F2FS_BLKSIZE]; cluster_size]);
        }

        // 读取头部
        let mut cursor = Cursor::new(&compressed_data[..24]);
        let clen = cursor.read_u32::<LittleEndian>()? as usize;
        let _chksum = cursor.read_u32::<LittleEndian>()?;

        if clen == 0 || clen > compressed_data.len() - 24 {
            return Ok(vec![vec![0u8; F2FS_BLKSIZE]; cluster_size]);
        }

        // 解压数据
        let compressed_payload = &compressed_data[24..24 + clen];
        let decompressed_size = cluster_size * F2FS_BLKSIZE;

        // 尝试 LZ4 解压
        let decompressed = match lz4_flex::decompress(compressed_payload, decompressed_size) {
            Ok(data) => data,
            Err(_) => {
                // 解压失败, 返回全零 block
                return Ok(vec![vec![0u8; F2FS_BLKSIZE]; cluster_size]);
            }
        };

        // 按 block 切分
        let mut blocks = Vec::new();
        for i in 0..cluster_size {
            let start = i * F2FS_BLKSIZE;
            let end = start + F2FS_BLKSIZE;
            if end <= decompressed.len() {
                blocks.push(decompressed[start..end].to_vec());
            } else if start < decompressed.len() {
                let mut block = decompressed[start..].to_vec();
                block.resize(F2FS_BLKSIZE, 0);
                blocks.push(block);
            } else {
                blocks.push(vec![0u8; F2FS_BLKSIZE]);
            }
        }

        Ok(blocks)
    }

    fn get_direct_addrs(&self, node_data: &[u8], inode: &Inode, _nid: Nid) -> Vec<u32> {
        let mut offset = 360;
        if inode.inline & F2FS_EXTRA_ATTR != 0 {
            offset += inode.extra_isize as usize;
        }

        let mut count = DEF_ADDRS_PER_INODE;
        if inode.inline & F2FS_EXTRA_ATTR != 0 {
            count -= inode.extra_isize as usize / 4;
        }
        if inode.inline & F2FS_INLINE_XATTR != 0 {
            count -= DEFAULT_INLINE_XATTR_ADDRS;
        }

        let mut addrs = Vec::new();
        let mut cursor = Cursor::new(&node_data[offset..]);

        for _ in 0..count.min((node_data.len() - offset - 24) / 4) {
            if let Ok(addr) = cursor.read_u32::<LittleEndian>() {
                addrs.push(addr);
            } else {
                break;
            }
        }

        addrs
    }

    fn read_indirect_blocks(
        &self,
        inode: &Inode,
        nid: Nid,
        _start: u64,
        count: u64,
    ) -> Result<Vec<Vec<u8>>> {
        let mut blocks = Vec::new();
        let node_data = self.read_node(nid)?;

        let nid_offset = 360 + DEF_ADDRS_PER_INODE * 4;
        let mut cursor = Cursor::new(&node_data[nid_offset..]);

        let mut blocks_read = 0u64;

        // i_nid[0-1]: 指向 direct node
        for _i in 0..2 {
            if blocks_read >= count {
                break;
            }

            if let Ok(nid_val) = cursor.read_u32::<LittleEndian>()
                && nid_val != 0
            {
                let direct = self.read_direct_node(Nid(nid_val), count - blocks_read, inode)?;
                blocks_read += direct.len() as u64;
                blocks.extend(direct);
            }
        }

        // i_nid[2-3]: 指向 single indirect node (其中存放的是 nid 数组)
        for _i in 2..4 {
            if blocks_read >= count {
                break;
            }

            if let Ok(nid_val) = cursor.read_u32::<LittleEndian>()
                && nid_val != 0
            {
                let indirect =
                    self.read_single_indirect(Nid(nid_val), count - blocks_read, inode)?;
                blocks_read += indirect.len() as u64;
                blocks.extend(indirect);
            }
        }

        Ok(blocks)
    }

    fn read_direct_node(&self, nid: Nid, count: u64, _inode: &Inode) -> Result<Vec<Vec<u8>>> {
        let node_data = self.read_node(nid)?;
        let mut blocks = Vec::new();
        let max_addrs = (node_data.len() - 24) / 4;

        let mut cursor = Cursor::new(&node_data[..node_data.len() - 24]);
        let mut i = 0;

        while i < max_addrs && blocks.len() < count as usize {
            if let Ok(addr) = cursor.read_u32::<LittleEndian>() {
                match BlockAddr::from(addr) {
                    BlockAddr::Null => {
                        // 检查剩余地址是否全部为稀疏
                        let mut all_sparse = true;
                        let mut temp_cursor = cursor.clone();
                        for _ in (i + 1)..max_addrs {
                            if let Ok(check_addr) = temp_cursor.read_u32::<LittleEndian>()
                                && check_addr != NULL_ADDR
                            {
                                all_sparse = false;
                                break;
                            }
                        }

                        if all_sparse {
                            break;
                        }

                        blocks.push(vec![0u8; F2FS_BLKSIZE]);
                        i += 1;
                    }
                    BlockAddr::Compress => {
                        // 处理 compression cluster
                        let cluster_size = 4;
                        let mut compressed_blocks = Vec::new();

                        // 收集 cluster 内的所有 block 地址
                        let mut cluster_addrs = vec![addr];
                        for j in 1..cluster_size {
                            if i + j >= max_addrs {
                                break;
                            }
                            if let Ok(cluster_addr) = cursor.read_u32::<LittleEndian>() {
                                cluster_addrs.push(cluster_addr);
                            }
                        }

                        // 读取实际的压缩数据 block
                        for cluster_addr in &cluster_addrs {
                            if *cluster_addr != COMPRESS_ADDR
                                && *cluster_addr != NULL_ADDR
                                && let BlockAddr::Valid(block) = BlockAddr::from(*cluster_addr)
                                && self.is_valid_block(block)
                            {
                                compressed_blocks.push(self.read_block(block)?);
                            }
                        }

                        // 解压
                        if !compressed_blocks.is_empty() {
                            let decompressed =
                                self.decompress_cluster(&compressed_blocks, cluster_size)?;
                            for block in decompressed {
                                if blocks.len() < count as usize {
                                    blocks.push(block);
                                }
                            }
                        } else {
                            // 没有压缩数据时以全零 block 填充
                            for _ in 0..cluster_size {
                                if blocks.len() < count as usize {
                                    blocks.push(vec![0u8; F2FS_BLKSIZE]);
                                }
                            }
                        }

                        i += cluster_size;
                    }
                    BlockAddr::Valid(block) if self.is_valid_block(block) => {
                        blocks.push(self.read_block(block)?);
                        i += 1;
                    }
                    _ => {
                        blocks.push(vec![0u8; F2FS_BLKSIZE]);
                        i += 1;
                    }
                }
            } else {
                i += 1;
            }
        }

        Ok(blocks)
    }

    // single indirect node 中存放的是 nid 数组, 而非 block 地址
    fn read_single_indirect(&self, nid: Nid, count: u64, inode: &Inode) -> Result<Vec<Vec<u8>>> {
        let node_data = self.read_node(nid)?;
        let mut blocks = Vec::new();
        let max_nids = (node_data.len() - 24) / 4;

        let mut cursor = Cursor::new(&node_data[..node_data.len() - 24]);

        for _ in 0..max_nids {
            if blocks.len() >= count as usize {
                break;
            }

            if let Ok(nid_val) = cursor.read_u32::<LittleEndian>()
                && nid_val != 0
            {
                let direct =
                    self.read_direct_node(Nid(nid_val), count - blocks.len() as u64, inode)?;
                blocks.extend(direct);
            }
        }

        Ok(blocks)
    }

    // 读取符号链接目标路径
    pub fn read_symlink_target(&self, inode: &Inode, nid: Nid) -> Result<String> {
        let data = self.read_file_data(inode, nid)?;

        // 按 inode.size 截取实际数据长度, 去除多余的零字节
        let actual_len = (inode.size as usize).min(data.len());
        let trimmed_data = &data[..actual_len];

        // 去除末尾的 NUL 终止符
        let target = if !trimmed_data.is_empty() && trimmed_data[trimmed_data.len() - 1] == 0 {
            String::from_utf8_lossy(&trimmed_data[..trimmed_data.len() - 1]).to_string()
        } else {
            String::from_utf8_lossy(trimmed_data).to_string()
        };

        Ok(target)
    }
}
