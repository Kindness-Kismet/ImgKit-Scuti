// EXT4 目录构建器

use crate::filesystem::ext4::Result;

// 目录项 构建器
pub struct DirectoryBuilder {
    entries: Vec<DirEntry>,
    block_size: u32,
}

// 目录项
struct DirEntry {
    inode: u32,
    name: Vec<u8>,
    file_type: u8,
}

// 文件类型常量
pub mod file_type {
    pub const REG: u8 = 1;
    pub const DIR: u8 = 2;
    pub const LNK: u8 = 7;
}

impl DirectoryBuilder {
    // 创建新的目录构建器
    pub fn new(block_size: u32) -> Self {
        DirectoryBuilder {
            entries: Vec::new(),
            block_size,
        }
    }

    // 添加 dir entry
    pub fn add_entry(&mut self, inode: u32, name: &[u8], file_type: u8) {
        self.entries.push(DirEntry {
            inode,
            name: name.to_vec(),
            file_type,
        });
    }

    // 构建目录数据块
    pub fn build(&self) -> Result<Vec<Vec<u8>>> {
        let mut blocks = Vec::new();
        let mut current_block = vec![0u8; self.block_size as usize];
        let mut offset = 0;
        let mut last_entry_offset = 0;

        for entry in self.entries.iter() {
            let entry_size = Self::calculate_entry_size(&entry.name);

            // 检查是否需要新的块
            if offset + entry_size > self.block_size as usize {
                // 扩展上一个 dir entry 的 rec_len 以填满剩余空间
                if last_entry_offset < offset && offset < self.block_size as usize {
                    let remaining = self.block_size as usize - last_entry_offset;
                    current_block[last_entry_offset + 4..last_entry_offset + 6]
                        .copy_from_slice(&(remaining as u16).to_le_bytes());
                }

                blocks.push(current_block);
                current_block = vec![0u8; self.block_size as usize];
                offset = 0;
            }

            // 写入 dir entry
            Self::write_entry(&mut current_block, offset, entry, entry_size);
            last_entry_offset = offset;
            offset += entry_size;
        }

        // 添加最后一个块
        if offset > 0 {
            // 扩展上一个 dir entry 的 rec_len 以填满剩余空间
            if last_entry_offset < offset && offset < self.block_size as usize {
                let remaining = self.block_size as usize - last_entry_offset;
                current_block[last_entry_offset + 4..last_entry_offset + 6]
                    .copy_from_slice(&(remaining as u16).to_le_bytes());
            }
            blocks.push(current_block);
        }

        Ok(blocks)
    }

    // 计算 dir entry 的大小 (按对齐处理)
    fn calculate_entry_size(name: &[u8]) -> usize {
        let base_size = 8 + name.len(); // 8 字节头部 + 名称
        (base_size + 3) & !3 // 4 字节对齐
    }

    // 写入 dir entry
    fn write_entry(block: &mut [u8], offset: usize, entry: &DirEntry, entry_size: usize) {
        // inode 号
        block[offset..offset + 4].copy_from_slice(&entry.inode.to_le_bytes());

        // rec_len 记录长度
        let rec_len = entry_size as u16;
        block[offset + 4..offset + 6].copy_from_slice(&rec_len.to_le_bytes());

        // name_len 名称长度
        block[offset + 6] = entry.name.len() as u8;

        // file_type 文件类型
        block[offset + 7] = entry.file_type;

        // name 名称
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
        assert_eq!(size, 12); // 8 + 4, 按 4 字节对齐
    }
}
