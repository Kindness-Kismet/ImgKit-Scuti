use super::volume::F2fsVolume;
use crate::filesystem::f2fs::*;
use byteorder::{LittleEndian, ReadBytesExt};
use std::io::Cursor;

impl F2fsVolume {
    pub fn read_file_data(&self, inode: &Inode, nid: Nid) -> Result<Vec<u8>> {
        const MAX_FILE_SIZE: u64 = 16 * 1024 * 1024 * 1024;
        if inode.size == 0 {
            return Ok(Vec::new());
        }
        if inode.size > MAX_FILE_SIZE {
            return Err(F2fsError::InvalidData(format!(
                "文件大小 {} 超过最大允许大小 {}",
                inode.size, MAX_FILE_SIZE
            )));
        }

        // Check if it is inline data
        if inode.inline & F2FS_INLINE_DATA != 0 {
            // Inline data is stored inside the inode, offset 400
            let node_data = self.read_node(nid)?;
            let inline_offset = 400;
            let data_len = inode
                .size
                .min((node_data.len() - inline_offset - 24) as u64)
                as usize;
            return Ok(node_data[inline_offset..inline_offset + data_len].to_vec());
        }

        let blocks = self.read_data_blocks(inode, nid)?;

        // Splicing block data
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

        // Read direct address
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
                    // Handling Compressed Clusters: Finding the Actual Compressed Data Blocks
                    let cluster_size = 4; // F2FS defaults to 4 blocks per cluster
                    let mut compressed_blocks = Vec::new();

                    // Collect all blocks in the cluster
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

                    // Unzip
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

        // Read indirect address
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
        // F2FS compression format: 24-byte header + compressed data
        // Header: clen(4) + chksum(4) + reserved(16)
        if compressed_blocks.is_empty() {
            return Ok(vec![vec![0u8; F2FS_BLKSIZE]; cluster_size]);
        }

        // Splice all compressed blocks
        let mut compressed_data = Vec::new();
        for block in compressed_blocks {
            compressed_data.extend_from_slice(block);
        }

        if compressed_data.len() < 24 {
            return Ok(vec![vec![0u8; F2FS_BLKSIZE]; cluster_size]);
        }

        // Read header
        let mut cursor = Cursor::new(&compressed_data[..24]);
        let clen = cursor.read_u32::<LittleEndian>()? as usize;
        let _chksum = cursor.read_u32::<LittleEndian>()?;

        if clen == 0 || clen > compressed_data.len() - 24 {
            return Ok(vec![vec![0u8; F2FS_BLKSIZE]; cluster_size]);
        }

        // Decompress data
        let compressed_payload = &compressed_data[24..24 + clen];
        let decompressed_size = cluster_size * F2FS_BLKSIZE;

        // Try LZ4 decompression
        let decompressed = match lz4_flex::decompress(compressed_payload, decompressed_size) {
            Ok(data) => data,
            Err(_) => {
                // Decompression failed, returning zero blocks
                return Ok(vec![vec![0u8; F2FS_BLKSIZE]; cluster_size]);
            }
        };

        // split into chunks
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

        // i_nid[0-1]: direct node
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

        // i_nid[2-3]: single indirect node (fix: read NID array)
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
                        // Check if all remaining addresses are sparse
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
                        // Handling compressed clusters
                        let cluster_size = 4;
                        let mut compressed_blocks = Vec::new();

                        // Collect all block addresses in the cluster
                        let mut cluster_addrs = vec![addr];
                        for j in 1..cluster_size {
                            if i + j >= max_addrs {
                                break;
                            }
                            if let Ok(cluster_addr) = cursor.read_u32::<LittleEndian>() {
                                cluster_addrs.push(cluster_addr);
                            }
                        }

                        // Read the actual compressed data block
                        for cluster_addr in &cluster_addrs {
                            if *cluster_addr != COMPRESS_ADDR
                                && *cluster_addr != NULL_ADDR
                                && let BlockAddr::Valid(block) = BlockAddr::from(*cluster_addr)
                                && self.is_valid_block(block)
                            {
                                compressed_blocks.push(self.read_block(block)?);
                            }
                        }

                        // Unzip
                        if !compressed_blocks.is_empty() {
                            let decompressed =
                                self.decompress_cluster(&compressed_blocks, cluster_size)?;
                            for block in decompressed {
                                if blocks.len() < count as usize {
                                    blocks.push(block);
                                }
                            }
                        } else {
                            // If there is no compressed data, zero blocks are padded
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

    // Fix: Single indirect node contains NID array, not block address
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

    // Read symbolic link target path
    pub fn read_symlink_target(&self, inode: &Inode, nid: Nid) -> Result<String> {
        let data = self.read_file_data(inode, nid)?;

        // Intercept the actual data length according to inode.size and remove excess zero bytes
        let actual_len = (inode.size as usize).min(data.len());
        let trimmed_data = &data[..actual_len];

        // Remove trailing null terminator
        let target = if !trimmed_data.is_empty() && trimmed_data[trimmed_data.len() - 1] == 0 {
            String::from_utf8_lossy(&trimmed_data[..trimmed_data.len() - 1]).to_string()
        } else {
            String::from_utf8_lossy(trimmed_data).to_string()
        };

        Ok(target)
    }
}
