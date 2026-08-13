// EROFS 目录处理模块

use super::volume::ErofsVolume;
use crate::filesystem::erofs::*;
use std::io::{Read, Seek, SeekFrom};
use zerocopy::TryFromBytes;

impl ErofsVolume {
    // 校验 dirent 的合法性
    fn is_valid_dirent(&self, dirent: &ErofsDirent, offset: usize, data_len: usize) -> bool {
        let nameoff = dirent.nameoff as usize;
        let dirent_size = std::mem::size_of::<ErofsDirent>();

        // 检查 nameoff 是否合理: 应位于 dirent 区域之后
        if nameoff < offset + dirent_size || nameoff >= data_len {
            log::debug!(
                "  → invalid nameoff: {} < {} || {} >= {}",
                nameoff,
                offset + dirent_size,
                nameoff,
                data_len
            );
            return false;
        }

        // 检查 file_type 是否合理 (0-7 为合法文件类型)
        if dirent.file_type > 7 {
            log::debug!("  → invalid file_type: {}", dirent.file_type);
            return false;
        }

        true
    }

    // 读取目录数据
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

            // FLAT_INLINE 布局可能包含两部分数据:
            // 1. 外部块: 前面的完整块存放在 raw_blkaddr 指向的位置
            // 2. inline 数据: 末尾不足一个块的数据 inline 在 inode 之后
            let mut combined_data = Vec::with_capacity(total_size);

            if inode_info.raw_blkaddr != 0xFFFFFFFF {
                // 存在外部块
                // 计算外部块的数量与大小 (不含末尾不完整的块)
                let external_blocks = total_size / block_size;
                let external_size = external_blocks * block_size;
                let inline_size = total_size - external_size;

                log::debug!(
                    "  external blocks: {} blocks = {} bytes (starting at blkaddr {})",
                    external_blocks,
                    external_size,
                    inode_info.raw_blkaddr
                );
                log::debug!(
                    "  inline data: {} bytes (at offset {})",
                    inline_size,
                    inline_offset
                );

                // 1. 读取外部块
                if external_size > 0 {
                    let external_offset = inode_info.raw_blkaddr as u64 * block_size as u64;
                    self.file.seek(SeekFrom::Start(external_offset))?;
                    let mut external_data = vec![0u8; external_size];
                    let n = self.file.read(&mut external_data)?;

                    if n < external_size {
                        return Err(ErofsError::Io(std::io::Error::new(
                            std::io::ErrorKind::UnexpectedEof,
                            format!(
                                "expected {} bytes of external data, only read {} bytes",
                                external_size, n
                            ),
                        )));
                    }

                    combined_data.extend_from_slice(&external_data);
                    log::debug!("  ✓ read external blocks: {} bytes", n);
                }

                // 2. 读取 inline 数据
                if inline_size > 0 {
                    self.file.seek(SeekFrom::Start(inline_offset))?;
                    let mut inline_data = vec![0u8; inline_size];
                    let n = self.file.read(&mut inline_data)?;

                    if n < inline_size {
                        return Err(ErofsError::Io(std::io::Error::new(
                            std::io::ErrorKind::UnexpectedEof,
                            format!(
                                "expected {} bytes of inline data, only read {} bytes",
                                inline_size, n
                            ),
                        )));
                    }

                    combined_data.extend_from_slice(&inline_data);
                    log::debug!("  ✓ read inline data: {} bytes", n);
                }
            } else {
                // 数据全部 inline (raw_blkaddr == 0xFFFFFFFF)
                log::debug!(
                    "  fully inline: {} bytes (at offset {})",
                    total_size,
                    inline_offset
                );

                self.file.seek(SeekFrom::Start(inline_offset))?;
                let mut inline_data = vec![0u8; total_size];
                let n = self.file.read(&mut inline_data)?;

                if n < total_size {
                    return Err(ErofsError::Io(std::io::Error::new(
                        std::io::ErrorKind::UnexpectedEof,
                        format!("expected {} bytes, only read {} bytes", total_size, n),
                    )));
                }

                combined_data = inline_data;
                log::debug!("  ✓ read inline data: {} bytes", n);
            }

            combined_data
        } else {
            self.read_file_data(inode_info)?
        };

        if !data.is_empty() {
            log::debug!("total directory data length: {} bytes", data.len());
            // 输出前若干字节便于调试
            if data.len() >= 16 {
                log::debug!("first 16 bytes: {:02X?}", &data[0..16]);
            }
        }

        Ok(data)
    }

    // 解析 dirent 数组
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
                    "scanning offset {}: nid={}, nameoff={}, type={}",
                    offset,
                    nid,
                    nameoff,
                    file_type
                );

                // 校验 dirent 合法性
                if !self.is_valid_dirent(&dirent, offset, data.len()) {
                    break;
                }

                log::debug!("  → valid dirent");
                dirents.push(dirent);
                offset += dirent_size;
            } else {
                break;
            }
        }

        log::debug!("found {} dirents", dirents.len());
        dirents
    }

    // 从 dirent 中提取目录条目 (解析名称)
    fn extract_dir_entries(&self, data: &[u8], dirents: &[ErofsDirent]) -> Vec<(String, u64, u8)> {
        let mut entries = Vec::new();

        for (idx, dirent) in dirents.iter().enumerate() {
            // 将 packed 结构体字段复制到局部变量, 避免对齐问题
            let nid = dirent.nid;
            let nameoff = dirent.nameoff as usize;
            let file_type = dirent.file_type;

            // 计算名称长度: 查找空字节
            let max_search_len = if idx + 1 < dirents.len() {
                // 存在下一个 dirent: 将查找范围限制到下一个 nameoff
                let next_nameoff = dirents[idx + 1].nameoff as usize;
                if next_nameoff > nameoff {
                    (next_nameoff - nameoff).min(255)
                } else {
                    // nameoff 顺序异常, 跳过
                    log::debug!(
                        "  → nameoff out of order: next={} <= current={}",
                        next_nameoff,
                        nameoff
                    );
                    continue;
                }
            } else {
                // 最后一个: 查找至数据末尾
                (data.len() - nameoff).min(255)
            };

            // 在受限范围内查找文件名结尾
            // EROFS 文件名以空字节或控制字符结束
            let name_bytes_search = &data[nameoff..nameoff + max_search_len];

            // 查找空字节
            let null_pos = name_bytes_search.iter().position(|&b| b == 0);

            // 查找第一个控制字符 (0x01-0x1F, 不含可打印字符)
            // 文件名应仅包含可打印字符 (>= 0x20)
            let ctrl_pos = name_bytes_search.iter().position(|&b| b > 0 && b < 0x20);

            // 取最小的位置作为实际文件名长度
            let name_len = match (null_pos, ctrl_pos) {
                (Some(n), Some(c)) => n.min(c), // 空字节与控制字符均存在, 取较小者
                (Some(n), None) => n,           // 仅有空字节
                (None, Some(c)) => c,           // 仅有控制字符
                (None, None) => max_search_len, // 均不存在, 使用完整范围
            };

            if nameoff + name_len > data.len() {
                log::debug!(
                    "  → name out of range: {} + {} > {}",
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

            // 跳过 "." 与 ".."
            if name != "." && name != ".." && !name.is_empty() {
                entries.push((name, nid, file_type));
            }
        }

        entries
    }

    // 读取目录
    pub fn read_dir(&mut self, inode_info: &InodeInfo) -> Result<Vec<(String, u64, u8)>> {
        log::debug!("\n=== reading directory nid={} ===", inode_info.nid);

        let data_layout = (inode_info.format >> EROFS_I_DATALAYOUT_BIT) & EROFS_I_DATALAYOUT_MASK;

        if data_layout != EROFS_INODE_FLAT_PLAIN && data_layout != EROFS_INODE_FLAT_INLINE {
            return Err(ErofsError::UnsupportedFeature(format!(
                "Compressed directory (layout {})",
                data_layout
            )));
        }

        // 读取整个目录数据
        let data = self.read_dir_data(inode_info, data_layout)?;

        if data.is_empty() {
            return Ok(Vec::new());
        }

        let mut all_entries = Vec::new();
        let block_size = self.block_size as usize;

        // 按逻辑块解析目录 (每个块有独立的 dirent 区与名称表)
        let mut pos = 0;
        while pos < data.len() {
            let block_end = (pos + block_size).min(data.len());
            let block_data = &data[pos..block_end];

            log::debug!(
                "parsing directory block: pos={}, block_size={}, data.len()={}",
                pos,
                block_end - pos,
                data.len()
            );

            // 解析该块内的 dirent
            let dirents = self.parse_dirents(block_data, block_data.len());
            let entries = self.extract_dir_entries(block_data, &dirents);

            log::debug!("block @ pos={} found {} entries", pos, entries.len());

            all_entries.extend(entries);
            pos = block_end;
        }

        log::debug!(
            "=== directory nid={} parsed, found {} valid entries ===\n",
            inode_info.nid,
            all_entries.len()
        );
        Ok(all_entries)
    }
}
