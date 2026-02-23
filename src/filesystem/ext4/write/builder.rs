// EXT4 Image Builder

use crate::filesystem::ext4::Result;
use crate::filesystem::ext4::types::*;
use crate::filesystem::ext4::write::directory::file_type;
use crate::filesystem::ext4::write::*;
use crate::filesystem::f2fs::write::{FsConfig, SelinuxContexts};
use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{BufWriter, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use zerocopy::TryFromBytes;

// Builder configuration
pub struct Ext4BuilderConfig {
    pub source_dir: PathBuf,
    pub output_path: PathBuf,
    pub image_size: u64,
    pub volume_label: String,
    pub mount_point: String,
    pub root_uid: u32,
    pub root_gid: u32,
    pub file_contexts: Option<PathBuf>,
    pub fs_config: Option<PathBuf>,
    pub timestamp: Option<u64>,
}

impl Default for Ext4BuilderConfig {
    fn default() -> Self {
        Ext4BuilderConfig {
            source_dir: PathBuf::new(),
            output_path: PathBuf::new(),
            image_size: 100 * 1024 * 1024, // 100MB
            volume_label: String::new(),
            mount_point: "/".to_string(),
            root_uid: 0,
            root_gid: 0,
            file_contexts: None,
            fs_config: None,
            timestamp: None,
        }
    }
}

// EXT4 Image Builder
pub struct Ext4Builder {
    config: Ext4BuilderConfig,
    writer: BufWriter<File>,
    sb_builder: SuperblockBuilder,
    block_alloc: BlockAllocator,
    inode_alloc: InodeAllocator,
    inode_map: HashMap<String, u32>,
    #[allow(dead_code)]
    selinux_contexts: Option<SelinuxContexts>,
    #[allow(dead_code)]
    fs_config: Option<FsConfig>,
    #[allow(dead_code)]
    timestamp: u32,
    dir_count: u32,
}

impl Ext4Builder {
    // Create new builder
    pub fn new(config: Ext4BuilderConfig) -> Result<Self> {
        let file = File::create(&config.output_path)?;
        let writer = BufWriter::new(file);

        let sb_builder = SuperblockBuilder::new(config.image_size).with_label(&config.volume_label);

        let block_alloc =
            BlockAllocator::new(sb_builder.blocks_count(), sb_builder.blocks_per_group());

        let inode_alloc =
            InodeAllocator::new(sb_builder.inodes_count(), sb_builder.inodes_per_group());

        // Load SELinux context
        let selinux_contexts = config
            .file_contexts
            .as_ref()
            .and_then(|path| SelinuxContexts::from_file(path).ok());

        // Load file system configuration
        let fs_config = config
            .fs_config
            .as_ref()
            .and_then(|path| FsConfig::from_file(path).ok());

        let timestamp = config.timestamp.unwrap_or_else(|| {
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs()
        }) as u32;

        Ok(Ext4Builder {
            config,
            writer,
            sb_builder,
            block_alloc,
            inode_alloc,
            inode_map: HashMap::new(),
            selinux_contexts,
            fs_config,
            timestamp,
            dir_count: 0,
        })
    }

    // Build image
    pub fn build(&mut self) -> Result<()> {
        // Initialize image file
        self.writer.get_ref().set_len(self.config.image_size)?;

        // reserved metadata block
        self.reserve_metadata_blocks()?;

        // Create root directory
        let root_ino = self.create_root_dir()?;

        // Load source directory content
        let source_dir = self.config.source_dir.clone();
        if source_dir.exists() {
            self.load_directory(&source_dir, root_ino, root_ino)?;
        }

        // Set actual idle count
        self.sb_builder
            .set_free_blocks_count(self.block_alloc.free_count());
        self.sb_builder
            .set_free_inodes_count(self.inode_alloc.free_count());

        // Write metadata
        self.write_metadata()?;

        self.writer.flush()?;
        Ok(())
    }

    // reserved metadata block
    fn reserve_metadata_blocks(&mut self) -> Result<()> {
        let group_count = self.sb_builder.group_count();
        let block_size = self.sb_builder.block_size();
        let blocks_per_group = self.sb_builder.blocks_per_group();

        for group_idx in 0..group_count {
            let group_start = group_idx as u64 * blocks_per_group as u64;

            // Super blocks (some block groups have backups)
            if group_idx == 0 || self.has_super_backup(group_idx) {
                self.block_alloc
                    .reserve_metadata_blocks(group_idx, &[group_start]);
            }

            // block group descriptor table
            let gdt_blocks =
                (group_count as u64 * EXT2_MIN_DESC_SIZE_64BIT as u64).div_ceil(block_size as u64);
            let gdt_start = group_start + 1;
            for i in 0..gdt_blocks {
                self.block_alloc
                    .reserve_metadata_blocks(group_idx, &[gdt_start + i]);
            }

            // block bitmap
            let block_bitmap = gdt_start + gdt_blocks;
            self.block_alloc
                .reserve_metadata_blocks(group_idx, &[block_bitmap]);

            // Inode bitmap
            let inode_bitmap = block_bitmap + 1;
            self.block_alloc
                .reserve_metadata_blocks(group_idx, &[inode_bitmap]);

            // Inode table
            let inode_table_start = inode_bitmap + 1;
            let inode_table_blocks = (self.sb_builder.inodes_per_group() as u64
                * self.sb_builder.inode_size() as u64)
                .div_ceil(block_size as u64);
            for i in 0..inode_table_blocks {
                self.block_alloc
                    .reserve_metadata_blocks(group_idx, &[inode_table_start + i]);
            }
        }

        Ok(())
    }

    // Check if block group has super block backup
    fn has_super_backup(&self, group_idx: u32) -> bool {
        if group_idx == 0 {
            return true;
        }
        // Superblocks backed up in power block groups of 3, 5, 7
        for base in [3, 5, 7] {
            let mut power = base;
            while power <= group_idx {
                if power == group_idx {
                    return true;
                }
                power *= base;
            }
        }
        false
    }

    // Create root directory
    fn create_root_dir(&mut self) -> Result<u32> {
        let root_ino = self.inode_alloc.alloc_root_inode();
        self.inode_map
            .insert(self.config.mount_point.clone(), root_ino);
        Ok(root_ino)
    }

    // Load directory contents
    fn load_directory(&mut self, path: &Path, current_ino: u32, parent_ino: u32) -> Result<()> {
        let entries: Vec<_> = fs::read_dir(path)?.collect::<std::io::Result<Vec<_>>>()?;

        // Create directory builder
        let mut dir_builder = DirectoryBuilder::new(self.sb_builder.block_size());

        // Add . and ..
        dir_builder.add_entry(current_ino, b".", file_type::DIR);
        dir_builder.add_entry(parent_ino, b"..", file_type::DIR);

        // Process all entries
        let mut dir_count = 0;

        for entry in &entries {
            let name = entry.file_name();
            let name_bytes = name.as_encoded_bytes();
            let metadata = entry.metadata()?;

            // Detect symbolic links first (supports Windows’ !<symlink> format)
            let symlink_info = crate::utils::symlink::read_symlink_info(&entry.path())
                .map_err(|e| std::io::Error::other(e.to_string()))?;

            if metadata.is_dir() {
                let ino = self
                    .inode_alloc
                    .alloc_inode()
                    .ok_or_else(|| std::io::Error::other("No more inodes"))?;

                dir_builder.add_entry(ino, name_bytes, file_type::DIR);
                dir_count += 1;

                // Process subdirectories recursively
                self.load_directory(&entry.path(), ino, current_ino)?;
            } else if symlink_info.is_symlink {
                // Symbolic links (including Windows' !<symlink> format)
                let ino = self
                    .inode_alloc
                    .alloc_inode()
                    .ok_or_else(|| std::io::Error::other("No more inodes"))?;

                dir_builder.add_entry(ino, name_bytes, file_type::LNK);

                // Create symbolic link inode
                self.create_symlink_inode(ino, &symlink_info.target.unwrap_or_default())?;
            } else if metadata.is_file() {
                let ino = self
                    .inode_alloc
                    .alloc_inode()
                    .ok_or_else(|| std::io::Error::other("No more inodes"))?;

                dir_builder.add_entry(ino, name_bytes, file_type::REG);

                // Create file inode
                self.create_file_inode(ino, &entry.path(), &metadata)?;
            }
        }

        // Write directory block
        let dir_blocks = dir_builder.build()?;

        // Allocate blocks and record block addresses
        let mut block_addrs = Vec::new();
        for block_data in dir_blocks.iter() {
            if let Some(block) = self.block_alloc.alloc_block() {
                self.write_data_block(block, block_data)?;
                block_addrs.push(block);
            } else {
                return Err(std::io::Error::other("No more blocks").into());
            }
        }

        // Create extent and write to inode
        let extents = ExtentBuilder::from_blocks(&block_addrs);
        if extents.len() > 4 {
            log::error!(
                "{} extent 数量超限: {} 个，最多支持 4 个",
                path.display(),
                extents.len()
            );
        }

        let block_size = self.sb_builder.block_size();
        let blocks_512 = (dir_blocks.len() as u32) * (block_size / 512);
        let dir_size = dir_blocks.len() * block_size as usize;

        let builder = InodeBuilder::new_dir(0o755, self.config.root_uid, self.config.root_gid)
            .with_size(dir_size as u64)
            .with_blocks(blocks_512)
            .with_links(2 + dir_count as u16)
            .with_extents(&extents);

        let inode_data = builder.build(self.sb_builder.inode_size())?;
        self.write_inode(current_ino, &inode_data)?;
        self.dir_count += 1;

        Ok(())
    }

    // Create file inode
    fn create_file_inode(&mut self, ino: u32, path: &Path, metadata: &fs::Metadata) -> Result<()> {
        let file_size = metadata.len();
        let file_data = fs::read(path)?;

        // allocate data blocks
        let block_size = self.sb_builder.block_size() as usize;
        let block_count = (file_size as usize).div_ceil(block_size);

        let mut blocks = Vec::new();
        for chunk in file_data.chunks(block_size) {
            if let Some(block) = self.block_alloc.alloc_block() {
                self.write_data_block(block, chunk)?;
                blocks.push(block);
            } else {
                return Err(std::io::Error::other("No more blocks").into());
            }
        }

        // create extent
        let extents = ExtentBuilder::from_blocks(&blocks);

        // Create inode
        let builder = InodeBuilder::new_file(0o644, self.config.root_uid, self.config.root_gid)
            .with_size(file_size)
            .with_blocks((block_count * (block_size / 512)) as u32)
            .with_extents(&extents);

        let inode_data = builder.build(self.sb_builder.inode_size())?;
        self.write_inode(ino, &inode_data)?;

        Ok(())
    }

    // Create symbolic link inode
    fn create_symlink_inode(&mut self, ino: u32, target: &str) -> Result<()> {
        let target_bytes = target.as_bytes();

        let builder = if target_bytes.len() <= 60 {
            // Fast symlinks: target path is stored in i_block
            InodeBuilder::new_symlink(self.config.root_uid, self.config.root_gid)
                .with_symlink_target(target)
        } else {
            // Slow symbolic links: target path is stored in data block
            let block_size = self.sb_builder.block_size() as usize;
            let mut block_data = vec![0u8; block_size];
            block_data[..target_bytes.len()].copy_from_slice(target_bytes);

            // allocate data blocks
            let block = self
                .block_alloc
                .alloc_block()
                .ok_or_else(|| std::io::Error::other("No more blocks"))?;
            self.write_data_block(block, &block_data)?;

            // create extent
            let extents = ExtentBuilder::from_blocks(&[block]);

            InodeBuilder::new_symlink(self.config.root_uid, self.config.root_gid)
                .with_size(target_bytes.len() as u64)
                .with_blocks((block_size / 512) as u32)
                .with_extents(&extents)
                .with_extent_flag()
        };

        let inode_data = builder.build(self.sb_builder.inode_size())?;
        self.write_inode(ino, &inode_data)?;

        Ok(())
    }

    // write inode
    fn write_inode(&mut self, ino: u32, data: &[u8]) -> Result<()> {
        let group_idx = self.inode_alloc.inode_group(ino);
        let inode_idx = self.inode_alloc.inode_index_in_group(ino);

        // Calculate inode table location
        let blocks_per_group = self.sb_builder.blocks_per_group();
        let block_size = self.sb_builder.block_size();
        let group_start = group_idx as u64 * blocks_per_group as u64;

        // Skip superblock, GDT, bitmaps
        let gdt_blocks = (self.sb_builder.group_count() as u64 * EXT2_MIN_DESC_SIZE_64BIT as u64)
            .div_ceil(block_size as u64);
        let inode_table_start = group_start + 1 + gdt_blocks + 2; // +2 for bitmaps

        let inode_offset = inode_table_start * block_size as u64
            + inode_idx as u64 * self.sb_builder.inode_size() as u64;

        self.writer.seek(SeekFrom::Start(inode_offset))?;
        self.writer.write_all(data)?;

        Ok(())
    }

    // Write data block
    fn write_data_block(&mut self, block: u64, data: &[u8]) -> Result<()> {
        let block_size = self.sb_builder.block_size() as usize;
        let offset = block * block_size as u64;

        self.writer.seek(SeekFrom::Start(offset))?;

        if data.len() < block_size {
            let mut padded = vec![0u8; block_size];
            padded[..data.len()].copy_from_slice(data);
            self.writer.write_all(&padded)?;
        } else {
            self.writer.write_all(&data[..block_size])?;
        }

        Ok(())
    }

    // Write metadata
    fn write_metadata(&mut self) -> Result<()> {
        // write superblock
        let sb = self.sb_builder.build()?;
        let sb_bytes: &[u8] = zerocopy::IntoBytes::as_bytes(&sb);

        self.writer.seek(SeekFrom::Start(EXT4_SUPERBLOCK_OFFSET))?;
        self.writer.write_all(sb_bytes)?;

        // Write block group descriptor
        self.write_group_descriptors()?;

        // Write bitmaps
        self.write_bitmaps()?;

        Ok(())
    }

    // Write block group descriptor
    fn write_group_descriptors(&mut self) -> Result<()> {
        let group_count = self.sb_builder.group_count();
        let block_size = self.sb_builder.block_size();
        let blocks_per_group = self.sb_builder.blocks_per_group();

        // Number of blocks occupied by GDT
        let gdt_blocks =
            (group_count as u64 * EXT2_MIN_DESC_SIZE_64BIT as u64).div_ceil(block_size as u64);

        // Build all group descriptors
        let mut gdt_data = vec![0u8; (gdt_blocks * block_size as u64) as usize];

        for group_idx in 0..group_count {
            let group_start = group_idx as u64 * blocks_per_group as u64;

            // Metadata location for each group
            let block_bitmap = group_start + 1 + gdt_blocks;
            let inode_bitmap = block_bitmap + 1;
            let inode_table = inode_bitmap + 1;

            // Calculate the number of free blocks and free inodes
            let free_blocks = self.block_alloc.get_free_blocks_in_group(group_idx);
            let free_inodes = self.inode_alloc.get_free_inodes_in_group(group_idx);

            let mut gd = Ext4GroupDescriptor::try_read_from_bytes(
                &[0u8; std::mem::size_of::<Ext4GroupDescriptor>()],
            )
            .unwrap();
            gd.bg_block_bitmap_lo = (block_bitmap & 0xFFFFFFFF) as u32;
            gd.bg_block_bitmap_hi = (block_bitmap >> 32) as u32;
            gd.bg_inode_bitmap_lo = (inode_bitmap & 0xFFFFFFFF) as u32;
            gd.bg_inode_bitmap_hi = (inode_bitmap >> 32) as u32;
            gd.bg_inode_table_lo = (inode_table & 0xFFFFFFFF) as u32;
            gd.bg_inode_table_hi = (inode_table >> 32) as u32;
            gd.bg_free_blocks_count_lo = (free_blocks & 0xFFFF) as u16;
            gd.bg_free_blocks_count_hi = (free_blocks >> 16) as u16;
            gd.bg_free_inodes_count_lo = (free_inodes & 0xFFFF) as u16;
            gd.bg_free_inodes_count_hi = (free_inodes >> 16) as u16;
            // Directory count should be for each group, temporarily set to 0 (can be optimized later)
            gd.bg_used_dirs_count_lo = 0;
            gd.bg_used_dirs_count_hi = 0;
            gd.bg_flags = 0;
            gd.bg_itable_unused_lo = free_inodes as u16;
            gd.bg_itable_unused_hi = (free_inodes >> 16) as u16;

            // Compute group descriptor checksum
            gd.bg_checksum = self.calc_group_desc_checksum(group_idx, &gd);

            // Write to GDT buffer
            let gd_offset = group_idx as usize * EXT2_MIN_DESC_SIZE_64BIT as usize;
            let gd_bytes: &[u8] = zerocopy::IntoBytes::as_bytes(&gd);
            gdt_data[gd_offset..gd_offset + gd_bytes.len()].copy_from_slice(gd_bytes);
        }

        // Write GDT to the end of the superblock (starting at block 1)
        let gdt_offset = block_size as u64; // Block 1
        self.writer.seek(SeekFrom::Start(gdt_offset))?;
        self.writer.write_all(&gdt_data)?;

        Ok(())
    }

    // Write bitmaps
    fn write_bitmaps(&mut self) -> Result<()> {
        let group_count = self.sb_builder.group_count();
        let block_size = self.sb_builder.block_size();
        let blocks_per_group = self.sb_builder.blocks_per_group();
        let inodes_per_group = self.sb_builder.inodes_per_group();
        let total_blocks = self.sb_builder.blocks_count();
        let total_inodes = self.sb_builder.inodes_count();

        for group_idx in 0..group_count {
            let group_start = group_idx as u64 * blocks_per_group as u64;
            let gdt_blocks =
                (group_count as u64 * EXT2_MIN_DESC_SIZE_64BIT as u64).div_ceil(block_size as u64);

            // Calculate the actual number of blocks and inodes in this group
            let blocks_in_group = ((total_blocks - group_start) as u32).min(blocks_per_group);
            let inode_start = group_idx * inodes_per_group;
            let inodes_in_group = (total_inodes - inode_start).min(inodes_per_group);

            // Write block bitmap
            let block_bitmap_offset = (group_start + 1 + gdt_blocks) * block_size as u64;
            self.writer.seek(SeekFrom::Start(block_bitmap_offset))?;
            let block_bitmap = self.block_alloc.get_bitmap(group_idx);
            let mut padded_bitmap = vec![0xFFu8; block_size as usize];
            padded_bitmap[..block_bitmap.len()].copy_from_slice(block_bitmap);
            // Set out-of-range bits to 1
            for bit in blocks_in_group..blocks_per_group {
                let byte_idx = (bit / 8) as usize;
                let bit_idx = bit % 8;
                if byte_idx < padded_bitmap.len() {
                    padded_bitmap[byte_idx] |= 1 << bit_idx;
                }
            }
            self.writer.write_all(&padded_bitmap)?;

            // Write inode bitmap
            let inode_bitmap_offset = block_bitmap_offset + block_size as u64;
            self.writer.seek(SeekFrom::Start(inode_bitmap_offset))?;
            let inode_bitmap = self.inode_alloc.get_bitmap(group_idx);
            let mut padded_bitmap = vec![0xFFu8; block_size as usize];
            padded_bitmap[..inode_bitmap.len()].copy_from_slice(inode_bitmap);
            // Set out-of-range bits to 1
            for bit in inodes_in_group..inodes_per_group {
                let byte_idx = (bit / 8) as usize;
                let bit_idx = bit % 8;
                if byte_idx < padded_bitmap.len() {
                    padded_bitmap[byte_idx] |= 1 << bit_idx;
                }
            }
            self.writer.write_all(&padded_bitmap)?;
        }

        Ok(())
    }

    // Calculate group descriptor checksum (CRC16)
    fn calc_group_desc_checksum(&self, group_idx: u32, gd: &Ext4GroupDescriptor) -> u16 {
        let uuid = self.sb_builder.uuid();
        let mut crc = ext4_crc16(!0, &uuid);
        crc = ext4_crc16(crc, &group_idx.to_le_bytes());

        let gd_bytes: &[u8] = zerocopy::IntoBytes::as_bytes(gd);
        // The checksum field offset is 30 (the position of bg_checksum in the structure)
        crc = ext4_crc16(crc, &gd_bytes[..30]);
        crc = ext4_crc16(crc, &gd_bytes[32..]);
        crc
    }
}

// EXT4 CRC16 calculation
fn ext4_crc16(crc: u16, data: &[u8]) -> u16 {
    let mut crc = crc;
    for &byte in data {
        crc = (crc >> 8) ^ CRC16_TABLE[((crc ^ byte as u16) & 0xFF) as usize];
    }
    crc
}

// CRC16 lookup table
const CRC16_TABLE: [u16; 256] = [
    0x0000, 0xC0C1, 0xC181, 0x0140, 0xC301, 0x03C0, 0x0280, 0xC241, 0xC601, 0x06C0, 0x0780, 0xC741,
    0x0500, 0xC5C1, 0xC481, 0x0440, 0xCC01, 0x0CC0, 0x0D80, 0xCD41, 0x0F00, 0xCFC1, 0xCE81, 0x0E40,
    0x0A00, 0xCAC1, 0xCB81, 0x0B40, 0xC901, 0x09C0, 0x0880, 0xC841, 0xD801, 0x18C0, 0x1980, 0xD941,
    0x1B00, 0xDBC1, 0xDA81, 0x1A40, 0x1E00, 0xDEC1, 0xDF81, 0x1F40, 0xDD01, 0x1DC0, 0x1C80, 0xDC41,
    0x1400, 0xD4C1, 0xD581, 0x1540, 0xD701, 0x17C0, 0x1680, 0xD641, 0xD201, 0x12C0, 0x1380, 0xD341,
    0x1100, 0xD1C1, 0xD081, 0x1040, 0xF001, 0x30C0, 0x3180, 0xF141, 0x3300, 0xF3C1, 0xF281, 0x3240,
    0x3600, 0xF6C1, 0xF781, 0x3740, 0xF501, 0x35C0, 0x3480, 0xF441, 0x3C00, 0xFCC1, 0xFD81, 0x3D40,
    0xFF01, 0x3FC0, 0x3E80, 0xFE41, 0xFA01, 0x3AC0, 0x3B80, 0xFB41, 0x3900, 0xF9C1, 0xF881, 0x3840,
    0x2800, 0xE8C1, 0xE981, 0x2940, 0xEB01, 0x2BC0, 0x2A80, 0xEA41, 0xEE01, 0x2EC0, 0x2F80, 0xEF41,
    0x2D00, 0xEDC1, 0xEC81, 0x2C40, 0xE401, 0x24C0, 0x2580, 0xE541, 0x2700, 0xE7C1, 0xE681, 0x2640,
    0x2200, 0xE2C1, 0xE381, 0x2340, 0xE101, 0x21C0, 0x2080, 0xE041, 0xA001, 0x60C0, 0x6180, 0xA141,
    0x6300, 0xA3C1, 0xA281, 0x6240, 0x6600, 0xA6C1, 0xA781, 0x6740, 0xA501, 0x65C0, 0x6480, 0xA441,
    0x6C00, 0xACC1, 0xAD81, 0x6D40, 0xAF01, 0x6FC0, 0x6E80, 0xAE41, 0xAA01, 0x6AC0, 0x6B80, 0xAB41,
    0x6900, 0xA9C1, 0xA881, 0x6840, 0x7800, 0xB8C1, 0xB981, 0x7940, 0xBB01, 0x7BC0, 0x7A80, 0xBA41,
    0xBE01, 0x7EC0, 0x7F80, 0xBF41, 0x7D00, 0xBDC1, 0xBC81, 0x7C40, 0xB401, 0x74C0, 0x7580, 0xB541,
    0x7700, 0xB7C1, 0xB681, 0x7640, 0x7200, 0xB2C1, 0xB381, 0x7340, 0xB101, 0x71C0, 0x7080, 0xB041,
    0x5000, 0x90C1, 0x9181, 0x5140, 0x9301, 0x53C0, 0x5280, 0x9241, 0x9601, 0x56C0, 0x5780, 0x9741,
    0x5500, 0x95C1, 0x9481, 0x5440, 0x9C01, 0x5CC0, 0x5D80, 0x9D41, 0x5F00, 0x9FC1, 0x9E81, 0x5E40,
    0x5A00, 0x9AC1, 0x9B81, 0x5B40, 0x9901, 0x59C0, 0x5880, 0x9841, 0x8801, 0x48C0, 0x4980, 0x8941,
    0x4B00, 0x8BC1, 0x8A81, 0x4A40, 0x4E00, 0x8EC1, 0x8F81, 0x4F40, 0x8D01, 0x4DC0, 0x4C80, 0x8C41,
    0x4400, 0x84C1, 0x8581, 0x4540, 0x8701, 0x47C0, 0x4680, 0x8641, 0x8201, 0x42C0, 0x4380, 0x8341,
    0x4100, 0x81C1, 0x8081, 0x4040,
];

// Simplified build function
pub fn build_ext4_image(
    source_dir: &Path,
    output_path: &Path,
    image_size: u64,
    mount_point: &str,
) -> Result<()> {
    let config = Ext4BuilderConfig {
        source_dir: source_dir.to_path_buf(),
        output_path: output_path.to_path_buf(),
        image_size,
        volume_label: String::new(),
        mount_point: mount_point.to_string(),
        root_uid: 0,
        root_gid: 0,
        file_contexts: None,
        fs_config: None,
        timestamp: None,
    };

    let mut builder = Ext4Builder::new(config)?;
    builder.build()
}
