// EXT4 directory reading module

use super::error::{Ext4Error, Result};
use super::types::*;
use std::io::{Read, Seek};
use std::path::PathBuf;
use zerocopy::TryFromBytes;

impl Inode {
    // Read all entries in a directory
    //
    // Return value: Vec<(file name, inode number, file type)>
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

        // Parse directory entry
        while offset + std::mem::size_of::<Ext4DirEntry2>() <= data.len() {
            if let Ok((dirent, _)) = Ext4DirEntry2::try_ref_from_prefix(&data[offset..]) {
                // rec_len is 0 indicating the end of the directory
                if dirent.rec_len == 0 {
                    break;
                }

                // Check whether the record length exceeds the data range
                if offset + dirent.rec_len as usize > data.len() {
                    break;
                }

                // Skip empty entries and checksum entries
                if dirent.inode != 0 && dirent.file_type != file_type::CHECKSUM {
                    // Check if the name length is legal
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
