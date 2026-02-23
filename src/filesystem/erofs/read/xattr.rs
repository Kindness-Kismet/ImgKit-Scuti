// EROFS xattr reading module

use super::volume::ErofsVolume;
use crate::filesystem::erofs::*;
use std::io::{Read, Seek, SeekFrom};
use zerocopy::TryFromBytes;

impl ErofsVolume {
    // Read the xattr (extended attributes) of a file
    pub fn read_xattrs(&mut self, inode_info: &InodeInfo) -> Result<Vec<(String, Vec<u8>)>> {
        if inode_info.xattr_icount == 0 {
            return Ok(Vec::new());
        }

        let inode_offset = self.nid_to_offset(inode_info.nid);
        let inode_size = if inode_info.is_compact { 32 } else { 64 };
        let xattr_header_offset = inode_offset + inode_size;

        self.file.seek(SeekFrom::Start(xattr_header_offset))?;
        let mut header_bytes = vec![0u8; std::mem::size_of::<ErofsXattrIbodyHeader>()];
        self.file.read_exact(&mut header_bytes)?;

        let header =
            ErofsXattrIbodyHeader::try_read_from_bytes(&header_bytes[..]).map_err(|_| {
                ErofsError::Io(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "解析 xattr ibody header 失败",
                ))
            })?;

        let shared_count = header.h_shared_count as usize;
        let xattr_blkaddr = self.superblock.xattr_blkaddr;

        let xattr_isize = self.xattr_ibody_size(inode_info.xattr_icount);
        let header_size = 12 + shared_count * 4;
        let inline_xattr_size = xattr_isize.saturating_sub(header_size);

        log::debug!(
            "读取 xattr: nid={}, xattr_icount={}, shared_count={}, xattr_blkaddr={}, xattr_isize={}, inline_size={}",
            inode_info.nid,
            inode_info.xattr_icount,
            shared_count,
            xattr_blkaddr,
            xattr_isize,
            inline_xattr_size
        );

        let mut xattrs = Vec::new();

        if shared_count > 0 {
            let indices_size = shared_count * 4;
            let mut indices_bytes = vec![0u8; indices_size];
            self.file.read_exact(&mut indices_bytes)?;

            let mut shared_xattr_ids = Vec::with_capacity(shared_count);
            for i in 0..shared_count {
                let id = u32::from_le_bytes([
                    indices_bytes[i * 4],
                    indices_bytes[i * 4 + 1],
                    indices_bytes[i * 4 + 2],
                    indices_bytes[i * 4 + 3],
                ]);
                shared_xattr_ids.push(id);
            }

            log::debug!("  shared_xattr_ids: {:?}", shared_xattr_ids);

            for xattr_id in shared_xattr_ids {
                log::debug!("  尝试读取 shared xattr_id={}", xattr_id);
                match self.read_shared_xattr(xattr_id) {
                    Ok(xattr_list) => {
                        log::debug!("    成功读取 {} 个 xattr", xattr_list.len());
                        xattrs.extend(xattr_list);
                    }
                    Err(e) => {
                        log::debug!("    读取失败: {:?}", e);
                    }
                }
            }
        }

        if inline_xattr_size > 0 {
            log::debug!("  尝试读取 inline xattr，大小 {} 字节", inline_xattr_size);

            let inline_xattr_offset = xattr_header_offset + header_size as u64;

            self.file.seek(SeekFrom::Start(inline_xattr_offset))?;
            let mut inline_data = vec![0u8; inline_xattr_size];
            self.file.read_exact(&mut inline_data)?;

            log::debug!(
                "  inline xattr 数据前16字节: {:02x?}",
                if inline_data.len() >= 16 {
                    &inline_data[0..16]
                } else {
                    &inline_data[..]
                }
            );

            match self.parse_xattr_entries(&inline_data, true) {
                Ok(inline_xattrs) => {
                    log::debug!("    成功解析 {} 个 inline xattr", inline_xattrs.len());
                    xattrs.extend(inline_xattrs);
                }
                Err(e) => {
                    log::debug!("    解析 inline xattr 失败: {:?}", e);
                }
            }
        }

        Ok(xattrs)
    }

    fn read_shared_xattr(&mut self, xattr_id: u32) -> Result<Vec<(String, Vec<u8>)>> {
        let xattr_blkaddr = self.superblock.xattr_blkaddr as u64;
        let xattr_byte_offset = xattr_id as u64 * 4;
        let block_offset = xattr_byte_offset / self.block_size as u64;
        let in_block_offset = (xattr_byte_offset % self.block_size as u64) as usize;

        let physical_blkaddr = xattr_blkaddr + block_offset;
        let physical_offset = physical_blkaddr * self.block_size as u64 + in_block_offset as u64;

        log::debug!(
            "    read_shared_xattr: xattr_id={}, xattr_blkaddr={}, byte_offset={}, physical_offset={}",
            xattr_id,
            xattr_blkaddr,
            xattr_byte_offset,
            physical_offset
        );

        self.file.seek(SeekFrom::Start(physical_offset))?;

        let max_read = self.block_size as usize;
        let mut block_data = vec![0u8; max_read];
        let n = self.file.read(&mut block_data)?;
        block_data.truncate(n);

        log::debug!(
            "    读取了 {} 字节数据，前16字节: {:02x?}",
            n,
            if n >= 16 {
                &block_data[0..16]
            } else {
                &block_data[0..n]
            }
        );

        self.parse_xattr_entries(&block_data, false)
    }

    fn parse_xattr_entries(&self, data: &[u8], parse_all: bool) -> Result<Vec<(String, Vec<u8>)>> {
        let mut xattrs = Vec::new();
        let mut read_offset = 0;

        while read_offset + std::mem::size_of::<ErofsXattrEntry>() <= data.len() {
            let entry_bytes =
                &data[read_offset..read_offset + std::mem::size_of::<ErofsXattrEntry>()];
            let entry = match ErofsXattrEntry::try_read_from_bytes(entry_bytes) {
                Ok(e) => e,
                Err(_) => break,
            };

            let e_name_len = entry.e_name_len;
            let e_name_index = entry.e_name_index;
            let e_value_size = entry.e_value_size;

            log::debug!(
                "    parse_xattr_entry @ offset {}: e_name_len={}, e_name_index={}, e_value_size={}",
                read_offset,
                e_name_len,
                e_name_index,
                e_value_size
            );

            if e_name_len == 0 {
                log::debug!("    遇到空 entry，结束解析");
                break;
            }

            read_offset += std::mem::size_of::<ErofsXattrEntry>();

            let name_len = e_name_len as usize;
            if read_offset + name_len > data.len() {
                log::debug!("    名称长度超出范围");
                break;
            }

            let name_bytes = &data[read_offset..read_offset + name_len];
            let name_suffix = String::from_utf8_lossy(name_bytes).to_string();
            read_offset += name_len;

            let prefix = entry.name_prefix();
            let full_name = if prefix.is_empty() {
                name_suffix
            } else {
                format!("{}{}", prefix, name_suffix)
            };

            let value_size = e_value_size as usize;
            if read_offset + value_size > data.len() {
                log::debug!("    值大小超出范围");
                break;
            }

            let value = data[read_offset..read_offset + value_size].to_vec();
            read_offset += value_size;

            log::debug!("    xattr: name='{}', value_size={}", full_name, value_size);

            xattrs.push((full_name, value));

            read_offset = (read_offset + 3) & !3;

            if !parse_all {
                break;
            }
        }

        Ok(xattrs)
    }
}
