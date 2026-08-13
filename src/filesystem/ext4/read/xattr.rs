// EXT4 扩展属性读取模块

use crate::filesystem::ext4::error::Result;
use crate::filesystem::ext4::types::*;
use std::io::{Read, Seek};
use zerocopy::TryFromBytes;

impl Inode {
    // 读取 inode 的全部 xattr
    //
    // 返回值: Vec<(属性名, 属性值)>
    pub fn xattrs<R: Read + Seek>(
        &self,
        volume: &mut Ext4Volume<R>,
    ) -> Result<Vec<(String, Vec<u8>)>> {
        let mut xattrs = Vec::new();

        // 1. 读取 inline xattr (存放在 inode 内部)
        let inline_data_start =
            EXT2_GOOD_OLD_INODE_SIZE as usize + self.inode.i_extra_isize as usize;
        if self.data.len() > inline_data_start {
            let inline_data = &self.data[inline_data_start..];
            if let Ok((header, _)) = Ext4XattrIbodyHeader::try_ref_from_prefix(inline_data)
                && header.h_magic == EXT4_XATTR_HEADER_MAGIC
            {
                // xattr entry 位于 header 之后, 按 4 字节对齐
                let entries_start = (std::mem::size_of::<Ext4XattrIbodyHeader>() + 3) & !3;
                self.parse_xattr_entries(inline_data, entries_start, &mut xattrs, volume)?;
            }
        }

        // 2. 读取外部 xattr (存放在独立块中)
        if self.inode.i_file_acl() != 0 {
            let mut block_data = vec![0u8; volume.block_size as usize];
            volume.read_block(self.inode.i_file_acl(), &mut block_data)?;
            if let Ok((header, _)) = Ext4XattrHeader::try_ref_from_prefix(&block_data)
                && header.h_magic == EXT4_XATTR_HEADER_MAGIC
            {
                // xattr entry 位于 header 之后, 按 4 字节对齐
                let entries_start = (std::mem::size_of::<Ext4XattrHeader>() + 3) & !3;
                self.parse_xattr_entries(&block_data, entries_start, &mut xattrs, volume)?;
            }
        }
        Ok(xattrs)
    }

    // 从原始数据中解析 xattr entry 列表
    fn parse_xattr_entries<R: Read + Seek>(
        &self,
        raw_data: &[u8],
        mut i: usize,
        xattrs: &mut Vec<(String, Vec<u8>)>,
        volume: &mut Ext4Volume<R>,
    ) -> Result<()> {
        while i + std::mem::size_of::<Ext4XattrEntry>() <= raw_data.len() {
            if let Ok((entry, _)) = Ext4XattrEntry::try_ref_from_prefix(&raw_data[i..]) {
                // 全零 entry 表示列表结束
                if entry.e_name_len == 0
                    && entry.e_name_index == 0
                    && entry.e_value_offs == 0
                    && entry.e_value_inum == 0
                {
                    break;
                }

                // 读取属性名
                let name_start = i + std::mem::size_of::<Ext4XattrEntry>();
                if name_start + entry.e_name_len as usize > raw_data.len() {
                    eprintln!(
                        "[warning] xattr entry name out of range for inode {}",
                        self.inode_idx
                    );
                    break;
                }
                let name = format!(
                    "{}{}",
                    entry.get_name_prefix(),
                    String::from_utf8_lossy(
                        &raw_data[name_start..name_start + entry.e_name_len as usize]
                    )
                );

                // 读取属性值
                if entry.e_value_inum == 0 {
                    // 属性值存放在当前块中
                    let value_start = entry.e_value_offs as usize;
                    if value_start + entry.e_value_size as usize > raw_data.len() {
                        eprintln!(
                            "[warning] xattr value out of range for inode {} (name: {})",
                            self.inode_idx, name
                        );
                        break;
                    }
                    let value =
                        raw_data[value_start..value_start + entry.e_value_size as usize].to_vec();
                    xattrs.push((name, value));
                } else {
                    // 属性值存放在独立 inode 中 (大属性)
                    match volume.get_inode(entry.e_value_inum) {
                        Ok(xattr_inode) => {
                            let value = xattr_inode.open_read(volume)?;
                            xattrs.push((name, value));
                        }
                        Err(_) => {
                            let invalid_inum = entry.e_value_inum;
                            eprintln!(
                                "\n[warning] invalid inode reference {}, skipping xattr of inode {}: '{}'",
                                invalid_inum, self.inode_idx, name
                            );
                        }
                    }
                }

                // 移动到下一项 (按对齐处理)
                let entry_size = entry.size();
                if entry_size == 0 {
                    eprintln!(
                        "[warning] xattr entry size is 0 for inode {}, aborting",
                        self.inode_idx
                    );
                    break;
                }
                i += entry_size;
            } else {
                break;
            }
        }
        Ok(())
    }
}
