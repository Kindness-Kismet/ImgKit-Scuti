// EXT4 卷操作模块

use crate::filesystem::ext4::error::{Ext4Error, Result};
use crate::filesystem::ext4::types::*;
use std::io::{Read, Seek, SeekFrom};
use zerocopy::TryFromBytes;

impl<R: Read + Seek> Ext4Volume<R> {
    // 创建新的卷实例, 表示一个 ext4 文件系统
    pub fn new(mut stream: R) -> Result<Self> {
        // 读取 superblock (位于偏移 1024 字节处)
        stream.seek(SeekFrom::Start(1024))?;
        let mut superblock_bytes = [0u8; std::mem::size_of::<Ext4Superblock>()];
        stream.read_exact(&mut superblock_bytes)?;
        let superblock =
            Ext4Superblock::try_read_from_bytes(&superblock_bytes[..]).map_err(|_| {
                Ext4Error::Io(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "failed to parse superblock",
                ))
            })?;

        // 校验魔数
        if superblock.s_magic != EXT4_SUPERBLOCK_MAGIC {
            return Err(Ext4Error::Magic {
                expected: EXT4_SUPERBLOCK_MAGIC,
                found: superblock.s_magic,
            });
        }

        // 计算块大小
        let shift = 10u32
            .checked_add(superblock.s_log_block_size)
            .ok_or_else(|| {
                Ext4Error::Io(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "invalid EXT4 block size parameter",
                ))
            })?;
        if shift >= 63 {
            return Err(Ext4Error::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("invalid EXT4 block size shift: {}", shift),
            )));
        }
        let block_size = 1u64 << shift;
        if !(1024..=65536).contains(&block_size) {
            return Err(Ext4Error::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("EXT4 block size out of range: {}", block_size),
            )));
        }

        // 确定 group descriptor 的尺寸
        let mut desc_size = superblock.s_desc_size;
        if desc_size == 0 {
            desc_size = if (superblock.s_feature_incompat & INCOMPAT_64BIT) == 0 {
                EXT2_MIN_DESC_SIZE
            } else {
                EXT2_MIN_DESC_SIZE_64BIT
            };
        }

        // 读取 group descriptor 表
        let group_desc_table_offset = (1024 / block_size + 1) * block_size;
        let num_groups = superblock
            .s_inodes_count
            .div_ceil(superblock.s_inodes_per_group);
        let mut group_descriptors = Vec::with_capacity(num_groups as usize);

        stream.seek(SeekFrom::Start(group_desc_table_offset))?;
        for i in 0..num_groups {
            let is_64bit = (superblock.s_feature_incompat & INCOMPAT_64BIT) != 0;

            if !is_64bit {
                // 32 位 group descriptor
                let mut gd_bytes = vec![0u8; std::mem::size_of::<Ext4GroupDescriptor32>()];
                if stream.read_exact(&mut gd_bytes).is_err() {
                    eprintln!("failed to read group descriptor {}", i);
                    break;
                }
                let gd32 =
                    Ext4GroupDescriptor32::try_read_from_bytes(&gd_bytes[..]).map_err(|_| {
                        Ext4Error::Io(std::io::Error::new(
                            std::io::ErrorKind::InvalidData,
                            format!("failed to parse group descriptor {}", i),
                        ))
                    })?;
                group_descriptors.push(gd32.into());
            } else {
                // 64 位 group descriptor
                let mut gd_bytes = vec![0u8; desc_size as usize];
                if stream.read_exact(&mut gd_bytes).is_err() {
                    eprintln!("failed to read group descriptor {}", i);
                    break;
                }
                let gd = Ext4GroupDescriptor::try_read_from_bytes(&gd_bytes[..]).map_err(|_| {
                    Ext4Error::Io(std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        format!("failed to parse group descriptor {}", i),
                    ))
                })?;
                group_descriptors.push(gd);
            }
        }

        Ok(Self {
            stream,
            superblock,
            group_descriptors,
            block_size,
        })
    }

    // 从卷中读取指定块的数据
    pub fn read_block(&mut self, block_idx: u64, buf: &mut [u8]) -> Result<()> {
        self.stream
            .seek(SeekFrom::Start(block_idx * self.block_size))?;
        self.stream.read_exact(buf)?;
        Ok(())
    }

    // 根据 inode 号获取 Inode 实例
    pub fn get_inode(&mut self, inode_idx: u32) -> Result<Inode> {
        if inode_idx == 0 {
            return Err(Ext4Error::InodeNotFound(inode_idx));
        }

        // 计算 inode 所在的 block group
        let group_idx = (inode_idx - 1) / self.superblock.s_inodes_per_group;
        let inode_table_entry_idx = (inode_idx - 1) % self.superblock.s_inodes_per_group;

        if group_idx as usize >= self.group_descriptors.len() {
            return Err(Ext4Error::InodeNotFound(inode_idx));
        }

        // 计算 inode 在磁盘上的偏移
        let inode_table_offset =
            self.group_descriptors[group_idx as usize].bg_inode_table() * self.block_size;
        let inode_offset =
            inode_table_offset + inode_table_entry_idx as u64 * self.superblock.s_inode_size as u64;

        // 读取 inode
        self.stream.seek(SeekFrom::Start(inode_offset))?;
        let mut inode_bytes = vec![0u8; self.superblock.s_inode_size as usize];
        self.stream.read_exact(&mut inode_bytes)?;
        let inode_struct = Ext4Inode::try_read_from_bytes(&inode_bytes[..]).map_err(|_| {
            Ext4Error::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("failed to parse inode {}", inode_idx),
            ))
        })?;

        Ok(Inode {
            inode_idx,
            inode: inode_struct,
            data: inode_bytes,
        })
    }

    // 获取根目录的 Inode (inode 号为 2)
    pub fn root(&mut self) -> Result<Inode> {
        self.get_inode(2)
    }
}
