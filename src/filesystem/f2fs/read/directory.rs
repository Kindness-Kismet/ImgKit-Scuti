use super::volume::F2fsVolume;
use crate::filesystem::f2fs::*;
use byteorder::{LittleEndian, ReadBytesExt};
use std::io::Cursor;

#[derive(Debug)]
pub struct DirEntry {
    pub name: String,
    pub nid: Nid,
    pub file_type: u8,
}

impl F2fsVolume {
    pub fn read_dir(&self, inode: &Inode, nid: Nid) -> Result<Vec<DirEntry>> {
        let blocks = self.read_data_blocks(inode, nid)?;
        let mut entries = Vec::new();

        for block in blocks.iter() {
            entries.extend(self.parse_dir_block(block)?);
        }

        Ok(entries)
    }

    fn parse_dir_block(&self, block: &[u8]) -> Result<Vec<DirEntry>> {
        let mut entries = Vec::new();

        // Directory block structure:
        // - Bitmap: 27 bytes
        // - Reserved: 3 bytes
        // - Directory entries: 214 * 11 bytes
        // - Filenames: 214 * 8 bytes

        let bitmap_size = 27; // (214 + 7) / 8
        let reserved_size = 3;
        let nr_dentry = 214;
        let dentry_size = 11;

        if block.len() < bitmap_size + reserved_size + nr_dentry * dentry_size {
            return Ok(entries);
        }

        let bitmap = &block[..bitmap_size];
        let dentry_offset = bitmap_size + reserved_size;
        let filename_offset = dentry_offset + nr_dentry * dentry_size;

        for i in 0..nr_dentry {
            let byte_idx = i / 8;
            let bit_idx = i % 8;

            if bitmap[byte_idx] & (1 << bit_idx) != 0 {
                let entry_pos = dentry_offset + i * dentry_size;
                if entry_pos + dentry_size > block.len() {
                    break;
                }

                let entry_data = &block[entry_pos..entry_pos + dentry_size];
                let mut cursor = Cursor::new(entry_data);

                let _hash = cursor.read_u32::<LittleEndian>()?;
                let nid = Nid(cursor.read_u32::<LittleEndian>()?);
                let name_len = cursor.read_u16::<LittleEndian>()? as usize;
                let file_type = cursor.read_u8()?;

                let name_pos = filename_offset + i * F2FS_SLOT_LEN;
                if name_pos + name_len > block.len() {
                    break;
                }

                let name_bytes = &block[name_pos..name_pos + name_len];
                let name = String::from_utf8_lossy(name_bytes).to_string();

                if name != "." && name != ".." {
                    entries.push(DirEntry {
                        name,
                        nid,
                        file_type,
                    });
                }
            }
        }

        Ok(entries)
    }
}
