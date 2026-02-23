// EXT4 extended attribute reading module

use super::error::Result;
use super::types::*;
use std::io::{Read, Seek};
use zerocopy::TryFromBytes;

impl Inode {
    // Read all extended attributes (xattrs) of an inode
    //
    // Return value: Vec<(attribute name, attribute value)>
    pub fn xattrs<R: Read + Seek>(
        &self,
        volume: &mut Ext4Volume<R>,
    ) -> Result<Vec<(String, Vec<u8>)>> {
        let mut xattrs = Vec::new();

        // 1. Read inline xattr (stored inside inode)
        let inline_data_start =
            EXT2_GOOD_OLD_INODE_SIZE as usize + self.inode.i_extra_isize as usize;
        if self.data.len() > inline_data_start {
            let inline_data = &self.data[inline_data_start..];
            if let Ok((header, _)) = Ext4XattrIbodyHeader::try_ref_from_prefix(inline_data)
                && header.h_magic == EXT4_XATTR_HEADER_MAGIC
            {
                // xattr entry after header, 4-byte aligned
                let entries_start = (std::mem::size_of::<Ext4XattrIbodyHeader>() + 3) & !3;
                self.parse_xattr_entries(inline_data, entries_start, &mut xattrs, volume)?;
            }
        }

        // 2. Read external xattr (stored in independent block)
        if self.inode.i_file_acl() != 0 {
            let mut block_data = vec![0u8; volume.block_size as usize];
            volume.read_block(self.inode.i_file_acl(), &mut block_data)?;
            if let Ok((header, _)) = Ext4XattrHeader::try_ref_from_prefix(&block_data)
                && header.h_magic == EXT4_XATTR_HEADER_MAGIC
            {
                // xattr entry after header, 4-byte aligned
                let entries_start = (std::mem::size_of::<Ext4XattrHeader>() + 3) & !3;
                self.parse_xattr_entries(&block_data, entries_start, &mut xattrs, volume)?;
            }
        }
        Ok(xattrs)
    }

    // Parse a list of xattr entries from raw data
    fn parse_xattr_entries<R: Read + Seek>(
        &self,
        raw_data: &[u8],
        mut i: usize,
        xattrs: &mut Vec<(String, Vec<u8>)>,
        volume: &mut Ext4Volume<R>,
    ) -> Result<()> {
        while i + std::mem::size_of::<Ext4XattrEntry>() <= raw_data.len() {
            if let Ok((entry, _)) = Ext4XattrEntry::try_ref_from_prefix(&raw_data[i..]) {
                // An all-zero entry indicates the end of the list
                if entry.e_name_len == 0
                    && entry.e_name_index == 0
                    && entry.e_value_offs == 0
                    && entry.e_value_inum == 0
                {
                    break;
                }

                // Read attribute name
                let name_start = i + std::mem::size_of::<Ext4XattrEntry>();
                if name_start + entry.e_name_len as usize > raw_data.len() {
                    eprintln!("[警告] inode {} 的 xattr 条目名称超出范围", self.inode_idx);
                    break;
                }
                let name = format!(
                    "{}{}",
                    entry.get_name_prefix(),
                    String::from_utf8_lossy(
                        &raw_data[name_start..name_start + entry.e_name_len as usize]
                    )
                );

                // Read attribute value
                if entry.e_value_inum == 0 {
                    // The value is stored in the current block
                    let value_start = entry.e_value_offs as usize;
                    if value_start + entry.e_value_size as usize > raw_data.len() {
                        eprintln!(
                            "[警告] inode {} 的 xattr 值超出范围 (名称: {})",
                            self.inode_idx, name
                        );
                        break;
                    }
                    let value =
                        raw_data[value_start..value_start + entry.e_value_size as usize].to_vec();
                    xattrs.push((name, value));
                } else {
                    // Values ​​are stored in separate inodes (large attributes)
                    match volume.get_inode(entry.e_value_inum) {
                        Ok(xattr_inode) => {
                            let value = xattr_inode.open_read(volume)?;
                            xattrs.push((name, value));
                        }
                        Err(_) => {
                            let invalid_inum = entry.e_value_inum;
                            eprintln!(
                                "\n[警告] 因无效的 inode 引用 {}，跳过 inode {} 的 xattr '{}'",
                                invalid_inum, self.inode_idx, name
                            );
                        }
                    }
                }

                // Move to next item (aligned)
                let entry_size = entry.size();
                if entry_size == 0 {
                    eprintln!(
                        "[警告] inode {} 的 xattr 条目大小为 0，中断。",
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
