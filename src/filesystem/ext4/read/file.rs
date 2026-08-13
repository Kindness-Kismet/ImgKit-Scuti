// EXT4 文件读取模块

use crate::filesystem::ext4::error::{Ext4Error, Result};
use crate::filesystem::ext4::types::*;
use std::collections::HashSet;
use std::io::{Read, Seek};
use zerocopy::TryFromBytes;

impl Inode {
    // 判断 inode 是否为目录
    pub fn is_dir(&self) -> bool {
        (self.inode.i_mode & inode_mode::S_IFMT) == inode_mode::S_IFDIR
    }

    // 判断 inode 是否为普通文件
    pub fn is_file(&self) -> bool {
        (self.inode.i_mode & inode_mode::S_IFMT) == inode_mode::S_IFREG
    }

    // 判断 inode 是否为符号链接
    pub fn is_symlink(&self) -> bool {
        (self.inode.i_mode & inode_mode::S_IFMT) == inode_mode::S_IFLNK
    }

    // 读取并返回 inode 对应文件的全部数据
    pub fn open_read<R: Read + Seek>(&self, volume: &mut Ext4Volume<R>) -> Result<Vec<u8>> {
        const MAX_FILE_SIZE: u64 = 16 * 1024 * 1024 * 1024;
        let file_size = self.inode.i_size();
        if file_size > MAX_FILE_SIZE {
            return Err(Ext4Error::InvalidInodeSize {
                size: file_size,
                max: MAX_FILE_SIZE,
            });
        }

        // 非 extent 模式: 数据直接内联存放在 i_block 中
        if (self.inode.i_flags & inode_mode::EXT4_EXTENTS_FL) == 0 {
            let size = usize::try_from(file_size).map_err(|_| Ext4Error::InvalidInodeSize {
                size: file_size,
                max: usize::MAX as u64,
            })?;
            // 确保不超过 i_block 的大小
            let max_inline_size = self.inode.i_block.len();
            if size > max_inline_size {
                return Err(Ext4Error::InvalidInodeSize {
                    size: size as u64,
                    max: max_inline_size as u64,
                });
            }
            return Ok(self.inode.i_block[..size].to_vec());
        }

        // extent 模式: 需要解析 extent tree
        let mut mapping = Vec::new();
        let mut visited_blocks = HashSet::new();
        self.parse_extents(
            volume,
            &self.inode.i_block,
            &mut mapping,
            0,
            &mut visited_blocks,
        )?;
        mapping.sort_by_key(|&(file_block_idx, _, _, _)| file_block_idx);

        let file_size_usize =
            usize::try_from(file_size).map_err(|_| Ext4Error::InvalidInodeSize {
                size: file_size,
                max: usize::MAX as u64,
            })?;
        let mut data = Vec::with_capacity(file_size_usize.min(8 * 1024 * 1024));
        let mut block_buf = vec![0u8; volume.block_size as usize];

        // 遍历所有 extent 并读取数据块
        for (file_block_idx, disk_block_idx, block_count, is_unwritten) in mapping {
            let extent_start = file_block_idx.saturating_mul(volume.block_size);
            if extent_start >= file_size {
                break;
            }

            // 存在空洞时使用零字节填充
            if extent_start > data.len() as u64 {
                let hole_size = (extent_start - data.len() as u64) as usize;
                data.resize(data.len() + hole_size, 0);
            }

            // 读取或填充块数据
            for i in 0..block_count {
                if data.len() as u64 >= file_size {
                    break;
                }

                let remaining = (file_size - data.len() as u64) as usize;
                let to_copy = remaining.min(volume.block_size as usize);
                if is_unwritten {
                    // unwritten extent, 使用零字节填充
                    data.resize(data.len() + to_copy, 0);
                } else {
                    volume.read_block(disk_block_idx + i, &mut block_buf)?;
                    data.extend_from_slice(&block_buf[..to_copy]);
                }
            }
        }

        // 截断到实际文件大小
        data.resize(file_size_usize, 0);
        Ok(data)
    }

    // 解析 extent tree, 获取文件块的映射关系
    //
    // 返回值: (文件逻辑块号, 磁盘物理块号, 块数, 是否为 unwritten)
    fn parse_extents<R: Read + Seek>(
        &self,
        volume: &mut Ext4Volume<R>,
        data: &[u8],
        mapping: &mut Vec<(u64, u64, u64, bool)>,
        depth: u8,
        visited_blocks: &mut HashSet<u64>,
    ) -> Result<()> {
        const MAX_EXTENT_TREE_DEPTH: u8 = 8;
        if depth > MAX_EXTENT_TREE_DEPTH {
            return Err(Ext4Error::ExtentTreeTooDeep { depth });
        }

        let (extent_header, entries_data) = Ext4ExtentHeader::try_ref_from_prefix(data)
            .map_err(|_| Ext4Error::InvalidExtentHeader)?;

        // 校验 extent header 魔数
        if extent_header.eh_magic != EXT4_EXTENT_HEADER_MAGIC {
            return Err(Ext4Error::Magic {
                expected: EXT4_EXTENT_HEADER_MAGIC,
                found: extent_header.eh_magic,
            });
        }

        if extent_header.eh_depth == 0 {
            // 叶子节点: 直接读取 extent
            let (extents, _) = <[Ext4Extent]>::try_ref_from_prefix_with_elems(
                entries_data,
                extent_header.eh_entries as usize,
            )
            .map_err(|_| Ext4Error::InvalidExtent)?;
            for extent in extents {
                mapping.push((
                    extent.ee_block as u64,
                    extent.ee_start(),
                    extent.get_len() as u64,
                    extent.is_unwritten(),
                ));
            }
        } else {
            // 索引节点: 递归读取子节点
            let (indices, _) = <[Ext4ExtentIdx]>::try_ref_from_prefix_with_elems(
                entries_data,
                extent_header.eh_entries as usize,
            )
            .map_err(|_| Ext4Error::InvalidExtent)?;
            for idx in indices {
                let child_block = idx.ei_leaf();
                if !visited_blocks.insert(child_block) {
                    return Err(Ext4Error::ExtentCycleDetected { block: child_block });
                }
                let mut block_data = vec![0u8; volume.block_size as usize];
                volume.read_block(child_block, &mut block_data)?;
                self.parse_extents(volume, &block_data, mapping, depth + 1, visited_blocks)?;
            }
        }
        Ok(())
    }
}
