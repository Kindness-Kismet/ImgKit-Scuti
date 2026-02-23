// EROFS directory processing module

use super::volume::ErofsVolume;
use crate::filesystem::erofs::*;
use std::io::{Read, Seek, SeekFrom};
use zerocopy::TryFromBytes;

impl ErofsVolume {
    // Verify the validity of directory entries
    fn is_valid_dirent(&self, dirent: &ErofsDirent, offset: usize, data_len: usize) -> bool {
        let nameoff = dirent.nameoff as usize;
        let dirent_size = std::mem::size_of::<ErofsDirent>();

        // Check nameoff plausibility: should be after dirents area
        if nameoff < offset + dirent_size || nameoff >= data_len {
            log::debug!(
                "  → nameoff 不合理: {} < {} || {} >= {}",
                nameoff,
                offset + dirent_size,
                nameoff,
                data_len
            );
            return false;
        }

        // Check file_type plausibility (0-7 are valid file types)
        if dirent.file_type > 7 {
            log::debug!("  → file_type 不合理: {}", dirent.file_type);
            return false;
        }

        true
    }

    // Read directory data
    fn read_dir_data(&mut self, inode_info: &InodeInfo, data_layout: u16) -> Result<Vec<u8>> {
        let block_size = self.block_size as usize;

        let data = if data_layout == EROFS_INODE_FLAT_INLINE {
            let inode_offset = self.nid_to_offset(inode_info.nid);
            let inode_size = if inode_info.is_compact { 32 } else { 64 };
            let xattr_size = self.xattr_ibody_size(inode_info.xattr_icount);
            let inline_offset = inode_offset + inode_size + xattr_size as u64;
            let total_size = inode_info.size as usize;

            log::debug!(
                "FLAT_INLINE: nid={}, raw_blkaddr={}, i_size={}",
                inode_info.nid,
                inode_info.raw_blkaddr,
                total_size
            );

            // FLAT_INLINE layout may contain two parts of data:
            // 1. External block: the previous complete block is stored in the raw_blkaddr location
            // 2. Inline data: The last data less than one block is inlined behind the inode
            let mut combined_data = Vec::with_capacity(total_size);

            if inode_info.raw_blkaddr != 0xFFFFFFFF {
                // There is an external block
                // Calculate the number and size of external blocks (excluding the last incomplete block)
                let external_blocks = total_size / block_size;
                let external_size = external_blocks * block_size;
                let inline_size = total_size - external_size;

                log::debug!(
                    "  外部块: {} 个块 = {} 字节 (从块地址 {} 开始)",
                    external_blocks,
                    external_size,
                    inode_info.raw_blkaddr
                );
                log::debug!(
                    "  内联数据: {} 字节 (在 offset {} 处)",
                    inline_size,
                    inline_offset
                );

                // 1. Read external block
                if external_size > 0 {
                    let external_offset = inode_info.raw_blkaddr as u64 * block_size as u64;
                    self.file.seek(SeekFrom::Start(external_offset))?;
                    let mut external_data = vec![0u8; external_size];
                    let n = self.file.read(&mut external_data)?;

                    if n < external_size {
                        return Err(ErofsError::Io(std::io::Error::new(
                            std::io::ErrorKind::UnexpectedEof,
                            format!(
                                "期望读取 {} 字节外部数据，实际只读取了 {} 字节",
                                external_size, n
                            ),
                        )));
                    }

                    combined_data.extend_from_slice(&external_data);
                    log::debug!("  ✓ 读取外部块: {} 字节", n);
                }

                // 2. Read inline data
                if inline_size > 0 {
                    self.file.seek(SeekFrom::Start(inline_offset))?;
                    let mut inline_data = vec![0u8; inline_size];
                    let n = self.file.read(&mut inline_data)?;

                    if n < inline_size {
                        return Err(ErofsError::Io(std::io::Error::new(
                            std::io::ErrorKind::UnexpectedEof,
                            format!(
                                "期望读取 {} 字节内联数据，实际只读取了 {} 字节",
                                inline_size, n
                            ),
                        )));
                    }

                    combined_data.extend_from_slice(&inline_data);
                    log::debug!("  ✓ 读取内联数据: {} 字节", n);
                }
            } else {
                // All data is inline (raw_blkaddr == 0xFFFFFFFF)
                log::debug!(
                    "  全部内联: {} 字节 (在 offset {} 处)",
                    total_size,
                    inline_offset
                );

                self.file.seek(SeekFrom::Start(inline_offset))?;
                let mut inline_data = vec![0u8; total_size];
                let n = self.file.read(&mut inline_data)?;

                if n < total_size {
                    return Err(ErofsError::Io(std::io::Error::new(
                        std::io::ErrorKind::UnexpectedEof,
                        format!("期望读取 {} 字节，实际只读取了 {} 字节", total_size, n),
                    )));
                }

                combined_data = inline_data;
                log::debug!("  ✓ 读取内联数据: {} 字节", n);
            }

            combined_data
        } else {
            self.read_file_data(inode_info)?
        };

        if !data.is_empty() {
            log::debug!("目录数据总长度: {} 字节", data.len());
            // Output the first few bytes for debugging
            if data.len() >= 16 {
                log::debug!("前 16 字节: {:02X?}", &data[0..16]);
            }
        }

        Ok(data)
    }

    // Parse directory entry array
    fn parse_dirents(&self, data: &[u8], max_parse_size: usize) -> Vec<ErofsDirent> {
        let dirent_size = std::mem::size_of::<ErofsDirent>();
        let mut dirents = Vec::new();

        let mut offset = 0;
        while offset + dirent_size <= max_parse_size {
            if let Ok(dirent) =
                ErofsDirent::try_read_from_bytes(&data[offset..offset + dirent_size])
            {
                let nameoff = dirent.nameoff as usize;
                let nid = dirent.nid;
                let file_type = dirent.file_type;

                log::debug!(
                    "扫描 offset {}: nid={}, nameoff={}, type={}",
                    offset,
                    nid,
                    nameoff,
                    file_type
                );

                // Verify dirent legality
                if !self.is_valid_dirent(&dirent, offset, data.len()) {
                    break;
                }

                log::debug!("  → 有效 dirent");
                dirents.push(dirent);
                offset += dirent_size;
            } else {
                break;
            }
        }

        log::debug!("找到 {} 个 dirent", dirents.len());
        dirents
    }

    // Extract directory entries from dirents (resolve names)
    fn extract_dir_entries(&self, data: &[u8], dirents: &[ErofsDirent]) -> Vec<(String, u64, u8)> {
        let mut entries = Vec::new();

        for (idx, dirent) in dirents.iter().enumerate() {
            // Copy packed struct fields to local variables to avoid alignment issues
            let nid = dirent.nid;
            let nameoff = dirent.nameoff as usize;
            let file_type = dirent.file_type;

            // Calculating name length: looking for null bytes
            let max_search_len = if idx + 1 < dirents.len() {
                // has next dirent: limit search to next nameoff
                let next_nameoff = dirents[idx + 1].nameoff as usize;
                if next_nameoff > nameoff {
                    (next_nameoff - nameoff).min(255)
                } else {
                    // nameoff order is wrong, skip
                    log::debug!(
                        "  → nameoff 顺序错误: next={} <= current={}",
                        next_nameoff,
                        nameoff
                    );
                    continue;
                }
            } else {
                // The last one: Find the end of the data
                (data.len() - nameoff).min(255)
            };

            // Find the end of a filename within a limited range
            // EROFS filename terminated by null byte or control character
            let name_bytes_search = &data[nameoff..nameoff + max_search_len];

            // Find null bytes
            let null_pos = name_bytes_search.iter().position(|&b| b == 0);

            // Find the first control character (0x01-0x1F, excluding printable characters)
            // Filename should contain only printable characters (>= 0x20)
            let ctrl_pos = name_bytes_search.iter().position(|&b| b > 0 && b < 0x20);

            // Use the smallest position as the actual file name length
            let name_len = match (null_pos, ctrl_pos) {
                (Some(n), Some(c)) => n.min(c), // There are null and control characters, whichever is the smallest
                (Some(n), None) => n,           // only null
                (None, Some(c)) => c,           // only control characters
                (None, None) => max_search_len, // None, use full range
            };

            if nameoff + name_len > data.len() {
                log::debug!(
                    "  → 名称超出范围: {} + {} > {}",
                    nameoff,
                    name_len,
                    data.len()
                );
                continue;
            }

            let name_bytes = &data[nameoff..nameoff + name_len];
            let name = String::from_utf8_lossy(name_bytes).to_string();

            log::debug!(
                "dirent[{}]: nid={}, nameoff={}, type={}, name='{}'",
                idx,
                nid,
                nameoff,
                file_type,
                name
            );

            // Skip "." and ".."
            if name != "." && name != ".." && !name.is_empty() {
                entries.push((name, nid, file_type));
            }
        }

        entries
    }

    // Read directory
    pub fn read_dir(&mut self, inode_info: &InodeInfo) -> Result<Vec<(String, u64, u8)>> {
        log::debug!("\n=== 读取目录 nid={} ===", inode_info.nid);

        let data_layout = (inode_info.format >> EROFS_I_DATALAYOUT_BIT) & EROFS_I_DATALAYOUT_MASK;

        if data_layout != EROFS_INODE_FLAT_PLAIN && data_layout != EROFS_INODE_FLAT_INLINE {
            return Err(ErofsError::UnsupportedFeature(format!(
                "Compressed directory (layout {})",
                data_layout
            )));
        }

        // Read entire directory data
        let data = self.read_dir_data(inode_info, data_layout)?;

        if data.is_empty() {
            return Ok(Vec::new());
        }

        let mut all_entries = Vec::new();
        let block_size = self.block_size as usize;

        // Parse directories in logical chunks (each chunk has its own dirents and names table)
        let mut pos = 0;
        while pos < data.len() {
            let block_end = (pos + block_size).min(data.len());
            let block_data = &data[pos..block_end];

            log::debug!(
                "解析目录块: pos={}, block_size={}, data.len()={}",
                pos,
                block_end - pos,
                data.len()
            );

            // Parse the dirents of this block
            let dirents = self.parse_dirents(block_data, block_data.len());
            let entries = self.extract_dir_entries(block_data, &dirents);

            log::debug!("块 @ pos={} 找到 {} 个条目", pos, entries.len());

            all_entries.extend(entries);
            pos = block_end;
        }

        log::debug!(
            "=== 目录 nid={} 解析完成，找到 {} 个有效条目 ===\n",
            inode_info.nid,
            all_entries.len()
        );
        Ok(all_entries)
    }
}
