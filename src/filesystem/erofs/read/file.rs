// EROFS Inode file reading module

use super::volume::ErofsVolume;
use crate::filesystem::erofs::*;
use std::io::{Read, Seek, SeekFrom};

impl ErofsVolume {
    // Read file data
    pub fn read_file_data(&mut self, inode_info: &InodeInfo) -> Result<Vec<u8>> {
        // Prevent memory allocation overflow: limit maximum file size to 16GB
        const MAX_FILE_SIZE: u64 = 16 * 1024 * 1024 * 1024;
        if inode_info.size > MAX_FILE_SIZE {
            return Err(ErofsError::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!(
                    "文件大小 {} 超过最大允许大小 {}",
                    inode_info.size, MAX_FILE_SIZE
                ),
            )));
        }

        let data_layout = (inode_info.format >> EROFS_I_DATALAYOUT_BIT) & EROFS_I_DATALAYOUT_MASK;

        log::debug!(
            "read_file_data: nid={}, size={}, layout={}, format=0x{:04X}",
            inode_info.nid,
            inode_info.size,
            data_layout,
            inode_info.format
        );

        match data_layout {
            EROFS_INODE_FLAT_INLINE => self.read_flat_inline(inode_info),
            EROFS_INODE_FLAT_PLAIN => self.read_flat_plain(inode_info),
            EROFS_INODE_FLAT_COMPRESSION_LEGACY => self.read_compressed_file(inode_info),
            _ => Err(ErofsError::UnsupportedFeature(format!(
                "数据布局 {}",
                data_layout
            ))),
        }
    }

    // Read files with FLAT_INLINE layout
    fn read_flat_inline(&mut self, inode_info: &InodeInfo) -> Result<Vec<u8>> {
        if inode_info.raw_blkaddr != 0xFFFFFFFF {
            // Large files: use mixed storage mode (tail packing)
            let block_addr = inode_info.raw_blkaddr as u64;
            let data_offset = block_addr.saturating_mul(self.block_size as u64);
            let block_size = self.block_size as usize;

            let nblocks = (inode_info.size as usize).div_ceil(block_size);
            let tailpacking = !(inode_info.size as usize).is_multiple_of(block_size);
            let external_blocks = if tailpacking { nblocks - 1 } else { nblocks };
            let external_size = external_blocks * block_size;

            log::debug!(
                "FLAT_INLINE (file): block_addr={}, data_offset={}, i_size={}, nblocks={}, external_blocks={}, tailpack={}",
                block_addr,
                data_offset,
                inode_info.size,
                nblocks,
                external_blocks,
                tailpacking
            );

            // Read external block
            let mut data = if external_size > 0 {
                self.file.seek(SeekFrom::Start(data_offset))?;
                let mut external_data = vec![0u8; external_size];
                self.file.read_exact(&mut external_data)?;
                external_data
            } else {
                Vec::new()
            };

            // If there is a tail, read from the inline position
            if tailpacking {
                let tail_size = inode_info.size as usize - external_size;
                let inode_offset = self.nid_to_offset(inode_info.nid);
                let inode_size = if inode_info.is_compact { 32 } else { 64 };
                let xattr_size = self.xattr_ibody_size(inode_info.xattr_icount);
                let inline_offset = inode_offset + inode_size + xattr_size as u64;

                log::debug!(
                    "  读取文件 inline 尾部: offset={}, size={}",
                    inline_offset,
                    tail_size
                );

                self.file.seek(SeekFrom::Start(inline_offset))?;
                let mut tail_data = vec![0u8; tail_size];
                let n = self.file.read(&mut tail_data)?;
                tail_data.truncate(n);
                data.extend_from_slice(&tail_data);
            }

            Ok(data)
        } else {
            // Small files: data inline behind inode
            let inode_offset = self.nid_to_offset(inode_info.nid);
            let inode_size = if inode_info.is_compact { 32 } else { 64 };
            let xattr_size = self.xattr_ibody_size(inode_info.xattr_icount);
            let inline_offset = inode_offset + inode_size + xattr_size as u64;

            log::debug!(
                "FLAT_INLINE: inode_offset={}, inode_size={}, xattr_size={}, inline_offset={}",
                inode_offset,
                inode_size,
                xattr_size,
                inline_offset
            );

            let read_size = inode_info.size as usize;
            self.file.seek(SeekFrom::Start(inline_offset))?;
            let mut data = vec![0u8; read_size];
            let n = self.file.read(&mut data)?;
            data.truncate(n);

            log::debug!("读取了 {} 字节（i_size={}）", n, inode_info.size);
            Ok(data)
        }
    }

    // Read files with FLAT_PLAIN layout
    fn read_flat_plain(&mut self, inode_info: &InodeInfo) -> Result<Vec<u8>> {
        let block_addr = inode_info.raw_blkaddr as u64;
        let data_offset = block_addr.saturating_mul(self.block_size as u64);

        log::debug!(
            "FLAT_PLAIN: block_addr={}, data_offset={}",
            block_addr,
            data_offset
        );

        self.file.seek(SeekFrom::Start(data_offset))?;
        let mut data = vec![0u8; inode_info.size as usize];
        self.file.read_exact(&mut data)?;
        Ok(data)
    }

    // Read symbolic links
    pub fn read_symlink(&mut self, inode_info: &InodeInfo) -> Result<String> {
        let data = self.read_file_data(inode_info)?;
        Ok(String::from_utf8_lossy(&data).to_string())
    }
}
