// EXT4 目录读取模块

use crate::filesystem::ext4::error::{Ext4Error, Result};
use crate::filesystem::ext4::types::*;
use std::io::{Read, Seek};
use std::path::PathBuf;
use zerocopy::TryFromBytes;

impl Inode {
    // 读取目录下的所有 dir entry
    //
    // 返回值: Vec<(文件名, inode 号, 文件类型)>
    pub fn open_dir<R: Read + Seek>(
        &self,
        volume: &mut Ext4Volume<R>,
    ) -> Result<Vec<(String, u32, u8)>> {
        if !self.is_dir() {
            return Err(Ext4Error::NotADirectory(PathBuf::new()));
        }

        let data = self.open_read(volume)?;
        let mut entries = Vec::new();
        let mut offset = 0;

        // 解析 dir entry
        while offset + std::mem::size_of::<Ext4DirEntry2>() <= data.len() {
            if let Ok((dirent, _)) = Ext4DirEntry2::try_ref_from_prefix(&data[offset..]) {
                // rec_len 为 0 表示目录已结束
                if dirent.rec_len == 0 {
                    break;
                }

                // 检查记录长度是否超出数据范围
                if offset + dirent.rec_len as usize > data.len() {
                    break;
                }

                // 跳过空 dir entry 与 checksum entry
                if dirent.inode != 0 && dirent.file_type != file_type::CHECKSUM {
                    // 检查名称长度是否合法
                    if offset + 8 + dirent.name_len as usize > data.len() {
                        break;
                    }

                    let name = String::from_utf8_lossy(
                        &data[offset + 8..offset + 8 + dirent.name_len as usize],
                    )
                    .to_string();
                    entries.push((name, dirent.inode, dirent.file_type));
                }
                offset += dirent.rec_len as usize;
            } else {
                break;
            }
        }
        Ok(entries)
    }
}
