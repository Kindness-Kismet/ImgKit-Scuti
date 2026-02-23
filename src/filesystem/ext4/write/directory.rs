// EXT4 directory builder

use crate::filesystem::ext4::Result;

// Catalog item builder
pub struct DirectoryBuilder {
    entries: Vec<DirEntry>,
    block_size: u32,
}

// directory entry
struct DirEntry {
    inode: u32,
    name: Vec<u8>,
    file_type: u8,
}

// file type constants
pub mod file_type {
    pub const REG: u8 = 1;
    pub const DIR: u8 = 2;
    pub const LNK: u8 = 7;
}

impl DirectoryBuilder {
    // Create a new catalog builder
    pub fn new(block_size: u32) -> Self {
        DirectoryBuilder {
            entries: Vec::new(),
            block_size,
        }
    }

    // Add catalog entry
    pub fn add_entry(&mut self, inode: u32, name: &[u8], file_type: u8) {
        self.entries.push(DirEntry {
            inode,
            name: name.to_vec(),
            file_type,
        });
    }

    // Build directory block
    pub fn build(&self) -> Result<Vec<Vec<u8>>> {
        let mut blocks = Vec::new();
        let mut current_block = vec![0u8; self.block_size as usize];
        let mut offset = 0;
        let mut last_entry_offset = 0;

        for entry in self.entries.iter() {
            let entry_size = Self::calculate_entry_size(&entry.name);

            // Check if new blocks are needed
            if offset + entry_size > self.block_size as usize {
                // Extend rec_len of last entry to fill remaining space
                if last_entry_offset < offset && offset < self.block_size as usize {
                    let remaining = self.block_size as usize - last_entry_offset;
                    current_block[last_entry_offset + 4..last_entry_offset + 6]
                        .copy_from_slice(&(remaining as u16).to_le_bytes());
                }

                blocks.push(current_block);
                current_block = vec![0u8; self.block_size as usize];
                offset = 0;
            }

            // Write directory entry
            Self::write_entry(&mut current_block, offset, entry, entry_size);
            last_entry_offset = offset;
            offset += entry_size;
        }

        // add last block
        if offset > 0 {
            // Extend rec_len of last entry to fill remaining space
            if last_entry_offset < offset && offset < self.block_size as usize {
                let remaining = self.block_size as usize - last_entry_offset;
                current_block[last_entry_offset + 4..last_entry_offset + 6]
                    .copy_from_slice(&(remaining as u16).to_le_bytes());
            }
            blocks.push(current_block);
        }

        Ok(blocks)
    }

    // Calculate directory entry size (8-byte alignment)
    fn calculate_entry_size(name: &[u8]) -> usize {
        let base_size = 8 + name.len(); // 8-byte header + name
        (base_size + 3) & !3 // 4-byte alignment
    }

    // Write directory entry
    fn write_entry(block: &mut [u8], offset: usize, entry: &DirEntry, entry_size: usize) {
        // inode
        block[offset..offset + 4].copy_from_slice(&entry.inode.to_le_bytes());

        // rec_len
        let rec_len = entry_size as u16;
        block[offset + 4..offset + 6].copy_from_slice(&rec_len.to_le_bytes());

        // name_len
        block[offset + 6] = entry.name.len() as u8;

        // file_type
        block[offset + 7] = entry.file_type;

        // name
        block[offset + 8..offset + 8 + entry.name.len()].copy_from_slice(&entry.name);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_directory_builder() {
        let mut builder = DirectoryBuilder::new(4096);

        builder.add_entry(2, b".", file_type::DIR);
        builder.add_entry(2, b"..", file_type::DIR);
        builder.add_entry(11, b"test.txt", file_type::REG);

        let blocks = builder.build().unwrap();
        assert_eq!(blocks.len(), 1);
    }

    #[test]
    fn test_calculate_entry_size() {
        let size = DirectoryBuilder::calculate_entry_size(b"test");
        assert_eq!(size, 12); // 8 + 4, aligned to 4 bytes
    }
}
