// EROFS Image Builder
use crate::filesystem::erofs::consts::*;
//
// Provides complete EROFS image building functionality.

use crate::compression::Compressor;
use crate::filesystem::erofs::write::compress::{
    PhysicalCluster, build_compress_metadata, compress_file_data, create_compressor,
    get_algorithm_type,
};
use crate::filesystem::erofs::write::{
    ErofsConfig, FsConfig, InodeBuilder, SelinuxContexts, SuperblockBuilder,
};
use crate::filesystem::erofs::{ErofsError, Result};
use crate::utils::symlink::read_symlink_info;
use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::{BufWriter, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

// File information
#[derive(Debug)]
struct FileInfo {
    path: PathBuf,
    fs_path: String,
    is_dir: bool,
    is_symlink: bool,
    symlink_target: Option<String>,
    size: u64,
    mode: u16,
    uid: u32,
    gid: u32,
    mtime: u64,
    children: Vec<String>,
    // Compression related
    file_data: Option<Vec<u8>>,                      // File raw data
    physical_clusters: Option<Vec<PhysicalCluster>>, // Physical cluster list
    compress_meta_size: usize,                       // Compressed metadata size (header + indexes)
    use_compression: bool,                           // Whether to use compression
    // xattr related
    xattr_size: usize, // xattr data size (including ibody header)
}

// EROFS Image Builder
pub struct ErofsBuilder {
    config: ErofsConfig,
    writer: BufWriter<File>,

    // super block
    superblock: SuperblockBuilder,

    // Timestamp
    timestamp: u64,

    // block size
    block_size: u32,

    // metadata starting block
    meta_blkaddr: u32,

    // File information mapping
    files: BTreeMap<String, FileInfo>,

    // NID mapping
    nid_map: BTreeMap<String, u64>,

    // SELinux context
    selinux_contexts: Option<SelinuxContexts>,

    // File system configuration
    fs_config: Option<FsConfig>,

    // Compressor
    compressor: Option<Box<dyn Compressor>>,
}

impl ErofsBuilder {
    pub fn new(config: ErofsConfig) -> Result<Self> {
        let file = File::create(&config.output_path)?;
        let writer = BufWriter::new(file);

        let timestamp = config.timestamp.unwrap_or_else(|| {
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0)
        });

        let block_size = config.block_size;

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

        // Create a super block builder
        let mut superblock = SuperblockBuilder::new(block_size)
            .with_volume_name(&config.volume_label)
            .with_build_time(timestamp)
            .with_feature_compat(EROFS_FEATURE_COMPAT_SB_CHKSUM | EROFS_FEATURE_COMPAT_MTIME);

        if let Some(uuid) = config.uuid {
            superblock = superblock.with_uuid(uuid);
        } else {
            // Generate random UUID
            let mut uuid = [0u8; 16];
            for (i, byte) in uuid.iter_mut().enumerate() {
                *byte = ((timestamp >> (i * 4)) & 0xFF) as u8 ^ (i as u8 * 17);
            }
            uuid[6] = (uuid[6] & 0x0F) | 0x40; // Version 4
            uuid[8] = (uuid[8] & 0x3F) | 0x80; // Variant
            superblock = superblock.with_uuid(uuid);
        }

        // Metadata starts after the superblock
        // meta_blkaddr is set to 0 (meaning metadata starts at block 0, i.e. after EROFS_SUPER_OFFSET)
        let meta_blkaddr = 0;

        // Create a compressor
        let compressor = if let Some(ref algorithm) = config.compress_algorithm {
            let algorithm_type = get_algorithm_type(algorithm)?;
            superblock = superblock
                .with_compression(algorithm_type)
                .with_feature_incompat(EROFS_FEATURE_INCOMPAT_ZERO_PADDING);

            // Non-LZ4 algorithms require the COMPR_CFGS flag to be set
            if algorithm_type != Z_EROFS_COMPRESSION_LZ4 {
                superblock.add_feature_incompat(EROFS_FEATURE_INCOMPAT_COMPR_CFGS);
            }

            Some(create_compressor(algorithm, config.compress_level)?)
        } else {
            None
        };

        Ok(ErofsBuilder {
            config,
            writer,
            superblock,
            timestamp,
            block_size,
            meta_blkaddr,
            files: BTreeMap::new(),
            nid_map: BTreeMap::new(),
            selinux_contexts,
            fs_config,
            compressor,
        })
    }

    // Write data to the specified location
    fn write_at(&mut self, offset: u64, data: &[u8]) -> Result<()> {
        self.writer.seek(SeekFrom::Start(offset))?;
        self.writer.write_all(data)?;
        Ok(())
    }

    // Calculate SELinux xattr data size
    // xattr ibody header (12 bytes) + entry header (4 bytes) + name ("selinux" = 7 bytes) + value
    fn calc_xattr_size(&self, fs_path: &str) -> usize {
        if let Some(ref ctx) = self.selinux_contexts
            && let Some(context) = ctx.lookup_without_mut(fs_path)
        {
            // xattr ibody header: 12 bytes
            // entry: 4 bytes header + 7 bytes name + value length, 4-byte aligned
            let entry_size = 4 + 7 + context.len();
            let aligned_entry_size = (entry_size + 3) & !3;
            return 12 + aligned_entry_size;
        }
        0
    }

    // Scan source directory
    fn scan_directory(&mut self, source_path: &Path, fs_path: &str) -> Result<()> {
        let metadata = fs::symlink_metadata(source_path)?;

        // Get uid/gid/mode
        let (uid, gid, mode) = if let Some(ref cfg) = self.fs_config {
            cfg.get_attrs(fs_path, metadata.is_dir())
        } else {
            (
                self.config.root_uid,
                self.config.root_gid,
                if metadata.is_dir() { 0o755 } else { 0o644 },
            )
        };

        let mtime = metadata
            .modified()
            .ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map(|d| d.as_secs())
            .unwrap_or(self.timestamp);

        if metadata.is_dir() {
            let mut children = Vec::new();

            for entry in fs::read_dir(source_path)? {
                let entry = entry?;
                let name = entry.file_name();
                let name_str = name.to_string_lossy().to_string();
                let child_path = entry.path();
                let child_fs_path = if fs_path == "/" {
                    format!("/{}", name_str)
                } else {
                    format!("{}/{}", fs_path, name_str)
                };

                children.push(name_str);
                self.scan_directory(&child_path, &child_fs_path)?;
            }

            // Calculate xattr size
            let xattr_size = self.calc_xattr_size(fs_path);

            self.files.insert(
                fs_path.to_string(),
                FileInfo {
                    path: source_path.to_path_buf(),
                    fs_path: fs_path.to_string(),
                    is_dir: true,
                    is_symlink: false,
                    symlink_target: None,
                    size: 0,
                    mode: mode as u16,
                    uid,
                    gid,
                    mtime,
                    children,
                    file_data: None,
                    physical_clusters: None,
                    compress_meta_size: 0,
                    use_compression: false,
                    xattr_size,
                },
            );
        } else {
            // Detect symbolic links
            let symlink_info = read_symlink_info(source_path)
                .map_err(|e| ErofsError::Io(std::io::Error::other(e.to_string())))?;

            // Calculate xattr size (required before compression metadata is built)
            let xattr_size = self.calc_xattr_size(fs_path);

            // Read file data and compress (if necessary)
            let (file_data, physical_clusters, compress_meta_size, use_compression) =
                if !symlink_info.is_symlink {
                    let data = fs::read(source_path)?;
                    let use_comp = self.compressor.is_some() && !data.is_empty();

                    if use_comp {
                        let compressor = self.compressor.as_ref().unwrap();
                        let pclusters =
                            compress_file_data(&data, self.block_size, compressor.as_ref())?;

                        // Check if any physical cluster actually uses compression
                        let has_compressed = pclusters
                            .iter()
                            .any(|pc| pc.logical_clusters.iter().any(|lc| lc.is_compressed));

                        if has_compressed {
                            // Get compression algorithm type
                            let algorithm = get_algorithm_type(
                                self.config
                                    .compress_algorithm
                                    .as_ref()
                                    .unwrap_or(&"lz4".to_string()),
                            )?;

                            // Build compression metadata to get accurate size
                            // Use dummy start_blkaddr (1) to avoid u32 underflow
                            let (header, indexes) = build_compress_metadata(
                                data.len() as u64,
                                self.block_size,
                                algorithm,
                                &pclusters,
                                1, // dummy value, avoid 0 - 1 underflow
                                xattr_size,
                            )?;

                            let meta_size = header.len() + indexes.len();

                            (Some(data), Some(pclusters), meta_size, true)
                        } else {
                            (Some(data), Some(pclusters), 0, false)
                        }
                    } else {
                        (Some(data), None, 0, false)
                    }
                } else {
                    (None, None, 0, false)
                };

            self.files.insert(
                fs_path.to_string(),
                FileInfo {
                    path: source_path.to_path_buf(),
                    fs_path: fs_path.to_string(),
                    is_dir: false,
                    is_symlink: symlink_info.is_symlink,
                    symlink_target: symlink_info.target,
                    size: if symlink_info.is_symlink {
                        0
                    } else {
                        metadata.len()
                    },
                    mode: mode as u16,
                    uid,
                    gid,
                    mtime,
                    children: Vec::new(),
                    file_data,
                    physical_clusters,
                    compress_meta_size,
                    use_compression,
                    xattr_size,
                },
            );
        }

        Ok(())
    }

    // Calculate directory entry data size (split in chunks)
    // EROFS directory data is organized in blocks, and the nameoff of each block is the offset relative to the start of the block.
    fn calc_dentry_size(&self, children: &[String]) -> usize {
        let block_size = self.block_size as usize;

        // EROFS requires directory entries to be sorted lexicographically by name
        let mut sorted_children: Vec<&String> = children.iter().collect();
        sorted_children.sort();

        // Collect all entries (. and .. plus children)
        let mut entries: Vec<&[u8]> = Vec::new();
        entries.push(b".");
        entries.push(b"..");
        for child_name in &sorted_children {
            entries.push(child_name.as_bytes());
        }

        // Calculate total size in chunks
        let mut total_size = 0;
        let mut entry_idx = 0;

        while entry_idx < entries.len() {
            // Calculate how many entries the current block can hold
            let mut block_used = 0;
            let mut block_entries = 0;

            while entry_idx + block_entries < entries.len() {
                let name = entries[entry_idx + block_entries];
                let entry_size = 12 + name.len(); // dirent (12) + name

                if block_used + entry_size > block_size {
                    break;
                }
                block_used += entry_size;
                block_entries += 1;
            }

            // If even one entry cannot be placed, it means the name is too long and must be forced to be placed.
            if block_entries == 0 {
                let name = entries[entry_idx];
                block_used = 12 + name.len();
                block_entries = 1;
            }

            entry_idx += block_entries;

            // Current block size (may be less than a full block)
            let remaining_entries = entries.len() - entry_idx;
            if remaining_entries == 0 {
                // Last block, does not need to be padded to block boundaries
                total_size += block_used;
            } else {
                // Not the last block, padded to block boundaries
                total_size += block_size;
            }
        }

        total_size
    }

    // Build directory entry data (split into chunks)
    fn build_dentries(&self, children: &[String], parent_fs_path: &str) -> Vec<u8> {
        let block_size = self.block_size as usize;
        let mut buf = Vec::new();

        // EROFS requires directory entries to be sorted lexicographically by name
        let mut sorted_children: Vec<&String> = children.iter().collect();
        sorted_children.sort();

        // Collect all entries
        let mut entries: Vec<(u64, u8, Vec<u8>)> = Vec::new();

        // .entry (current directory)
        let self_nid = self.nid_map.get(parent_fs_path).copied().unwrap_or(0);
        entries.push((self_nid, EROFS_FT_DIR, b".".to_vec()));

        // .. entry (parent directory)
        let parent_path = if parent_fs_path == "/" {
            "/".to_string()
        } else {
            let parts: Vec<&str> = parent_fs_path.rsplitn(2, '/').collect();
            if parts.len() > 1 && !parts[1].is_empty() {
                parts[1].to_string()
            } else {
                "/".to_string()
            }
        };
        let parent_nid = self.nid_map.get(&parent_path).copied().unwrap_or(self_nid);
        entries.push((parent_nid, EROFS_FT_DIR, b"..".to_vec()));

        // Children (sorted)
        for child_name in &sorted_children {
            let child_fs_path = if parent_fs_path == "/" {
                format!("/{}", child_name)
            } else {
                format!("{}/{}", parent_fs_path, child_name)
            };
            if let Some(nid) = self.nid_map.get(&child_fs_path) {
                let file_info = self.files.get(&child_fs_path);
                let file_type = if let Some(info) = file_info {
                    if info.is_dir {
                        EROFS_FT_DIR
                    } else if info.is_symlink {
                        EROFS_FT_SYMLINK
                    } else {
                        EROFS_FT_REG_FILE
                    }
                } else {
                    EROFS_FT_REG_FILE
                };
                entries.push((*nid, file_type, child_name.as_bytes().to_vec()));
            }
        }

        // Write directory data in blocks
        let mut entry_idx = 0;

        while entry_idx < entries.len() {
            // Calculate how many entries the current block can hold
            let mut block_entries = 0;
            let mut block_used = 0;

            while entry_idx + block_entries < entries.len() {
                let (_, _, ref name) = entries[entry_idx + block_entries];
                let entry_size = 12 + name.len();

                if block_used + entry_size > block_size {
                    break;
                }
                block_used += entry_size;
                block_entries += 1;
            }

            // If no item can fit, force it to fit in
            if block_entries == 0 {
                block_entries = 1;
            }

            // Write the directory entry for the current block
            // nameoff starts after the dirent structure (relative to the start of the block)
            let mut name_offset = 12 * block_entries;
            let block_start = buf.len();

            for i in 0..block_entries {
                let (nid, file_type, ref name) = entries[entry_idx + i];

                // nid (8 bytes)
                buf.extend_from_slice(&nid.to_le_bytes());
                // nameoff (2 bytes) - Offset relative to the start of the block
                buf.extend_from_slice(&(name_offset as u16).to_le_bytes());
                // file_type (1 byte)
                buf.push(file_type);
                // reserved (1 byte)
                buf.push(0);

                name_offset += name.len();
            }

            // write name
            for i in 0..block_entries {
                let (_, _, ref name) = entries[entry_idx + i];
                buf.extend_from_slice(name);
            }

            // Pad to block boundary (if not last block)
            let remaining_entries = entries.len() - entry_idx - block_entries;
            if remaining_entries > 0 {
                let current_block_size = buf.len() - block_start;
                let padding = block_size - current_block_size;
                buf.resize(buf.len() + padding, 0);
            }

            entry_idx += block_entries;
        }

        buf
    }

    // Build image
    pub fn build(&mut self) -> Result<()> {
        let source_dir = self.config.source_dir.clone();
        let mount_point = self.config.mount_point.clone();

        // Scan source directory
        if source_dir.exists() {
            self.scan_directory(&source_dir, &mount_point)?;
        }

        // First pass: Calculate size and offset of each inode, assign NID
        // NID = inode_absolute_offset / 32 (relative to image start)
        // When meta_blkaddr = 0, metadata starts from EROFS_SUPER_OFFSET
        // When meta_blkaddr > 0, metadata starts from meta_blkaddr * block_size
        // Note: The superblock occupies 128 bytes, from 0x400 to 0x47F
        // If there is compression configuration data, it immediately follows the superblock
        // Root inode starts after superblock + compressed configuration data (32-byte aligned)
        let compr_cfgs_size = self.superblock.compr_cfgs_size();
        let meta_base = if self.meta_blkaddr == 0 {
            // Superblock followed + compressed configuration data, then 32-byte alignment
            let base = EROFS_SUPER_OFFSET + 128 + compr_cfgs_size as u64;
            // Aligned to 32 bytes
            base.div_ceil(32) * 32
        } else {
            self.meta_blkaddr as u64 * self.block_size as u64
        };
        let mut current_offset: u64 = 0;

        let paths: Vec<String> = self.files.keys().cloned().collect();
        for path in &paths {
            // Calculate inode size
            let info = self
                .files
                .get(path)
                .ok_or_else(|| ErofsError::Io(std::io::Error::other("文件信息不存在")))?;

            // Base inode size + xattr size
            let base_inode_size = if info.is_dir {
                let dentry_size = self.calc_dentry_size(&info.children);
                // Directory data: only the last part less than one block is inlined
                // But inline data cannot cause inodes to cross block boundaries
                let max_inline = self.block_size as usize - 32 - info.xattr_size;
                let tail_size = dentry_size % self.block_size as usize;
                let actual_inline = if tail_size > max_inline { 0 } else { tail_size };
                32 + actual_inline as u64 // Compact inode (32 bytes) + inline data
            } else if info.is_symlink {
                let target_len = info.symlink_target.as_ref().map(|t| t.len()).unwrap_or(0);
                32 + target_len as u64 // Compact inode (32 bytes)
            } else if info.use_compression {
                // Compressed file: inode + compression metadata (header + indexes)
                32 + info.compress_meta_size as u64
            } else {
                // Ordinary file: The part larger than one block is stored in the external data block, and the part smaller than one block is inline.
                // But inline data cannot cause inodes to cross block boundaries
                let max_inline = self.block_size as u64 - 32 - info.xattr_size as u64;
                let tail_size = info.size % self.block_size as u64;
                let actual_inline = if tail_size > max_inline { 0 } else { tail_size };
                32 + actual_inline // Compact inode (32 bytes) + inline data
            };

            // plus xattr size
            let mut inode_size = base_inode_size + info.xattr_size as u64;

            // If xattr is present and there is compressed metadata, 8-byte alignment padding needs to be considered
            if info.xattr_size > 0 && info.use_compression {
                let before_compress = 32 + info.xattr_size as u64;
                let aligned = (before_compress + 7) & !7;
                let padding = aligned - before_compress;
                inode_size += padding;
            }

            let aligned_inode_size = inode_size.div_ceil(32) * 32;

            // Check if the inode will cross a block boundary
            let mut absolute_offset = meta_base + current_offset;
            let block_offset = absolute_offset % self.block_size as u64;

            if block_offset + aligned_inode_size > self.block_size as u64 {
                // Will cross block boundaries and need to be padded to the beginning of the next block
                let padding = self.block_size as u64 - block_offset;
                current_offset += padding;
                absolute_offset = meta_base + current_offset;
            }

            // Calculate NID (based on absolute offset from mirror start)
            let nid = absolute_offset / 32;
            self.nid_map.insert(path.clone(), nid);

            // Aligned to 32 bytes
            current_offset += aligned_inode_size;
        }

        let meta_size = current_offset;

        // Calculate the starting position of the data block
        let meta_blocks = meta_size.div_ceil(self.block_size as u64) as u32;
        // Reserve 1 guard block to avoid metadata estimation boundary errors causing overlap with the data area
        let data_blkaddr = self.meta_blkaddr + meta_blocks + 1;

        // Set up superblock
        self.superblock.set_meta_blkaddr(self.meta_blkaddr);
        self.superblock
            .set_root_nid(self.nid_map.get(&mount_point).copied().unwrap_or(0));
        self.superblock.set_inos(self.files.len() as u64);

        // Second pass: Build and write data
        let mut inode_entries: Vec<(u64, Vec<u8>)> = Vec::new();
        let mut data_entries: Vec<(u64, Vec<u8>)> = Vec::new();
        let mut data_offset = data_blkaddr as u64 * self.block_size as u64;
        let mut next_ino: u32 = 1;

        // Collect copies of file information
        let file_entries: Vec<(String, FileInfo)> = self
            .files
            .iter()
            .map(|(k, v)| {
                (
                    k.clone(),
                    FileInfo {
                        path: v.path.clone(),
                        fs_path: v.fs_path.clone(),
                        is_dir: v.is_dir,
                        is_symlink: v.is_symlink,
                        symlink_target: v.symlink_target.clone(),
                        size: v.size,
                        mode: v.mode,
                        uid: v.uid,
                        gid: v.gid,
                        mtime: v.mtime,
                        children: v.children.clone(),
                        file_data: v.file_data.clone(),
                        physical_clusters: v.physical_clusters.clone(),
                        compress_meta_size: v.compress_meta_size,
                        use_compression: v.use_compression,
                        xattr_size: v.xattr_size,
                    },
                )
            })
            .collect();

        for (path, info) in &file_entries {
            let nid = self.nid_map.get(path).copied().unwrap_or(0);
            let inode_offset = nid * 32;

            // Get SELinux context
            let selinux_context = if let Some(ref mut ctx) = self.selinux_contexts {
                ctx.lookup(path)
            } else {
                None
            };

            let inode_data = if info.is_dir {
                // build directory inode
                let dentry_data = self.build_dentries(&info.children, &info.fs_path);
                // nlink = 2 (. and ..), excluding subdirectories
                let mut inode = InodeBuilder::new_dir(info.mode, info.uid, info.gid)
                    .with_mtime(info.mtime)
                    .with_nlink(2)
                    .with_ino(next_ino)
                    .with_extended(false)
                    .with_size(dentry_data.len() as u64);

                // Directory data processing: similar to files, parts larger than one block are stored in external data blocks
                // Inline data cannot cause inodes to cross block boundaries
                let max_inline = self.block_size as usize - 32 - info.xattr_size;
                let tail_size = dentry_data.len() % self.block_size as usize;

                // If the tail data is too large, don't use inlining and store all data in external blocks
                let (nblocks, actual_inline) = if tail_size > max_inline {
                    // Store all data in external blocks (rounded up)
                    (dentry_data.len().div_ceil(self.block_size as usize), 0)
                } else {
                    (dentry_data.len() / self.block_size as usize, tail_size)
                };

                if nblocks > 0 {
                    // There is data that needs to be stored in an external data block
                    let data_blk = (data_offset / self.block_size as u64) as u32;
                    // Stores a complete block of data (may need to be padded to block boundaries)
                    let external_size = nblocks * self.block_size as usize;
                    let mut block_data =
                        dentry_data[..external_size.min(dentry_data.len())].to_vec();
                    // If there is insufficient data, fill it with zeros
                    if block_data.len() < external_size {
                        block_data.resize(external_size, 0);
                    }

                    // record data block
                    data_entries.push((data_offset, block_data));
                    data_offset += external_size as u64;

                    // Set raw_blkaddr
                    inode = inode.with_raw_blkaddr(data_blk);
                }

                if actual_inline > 0 {
                    // There is inline data
                    let inline_data = dentry_data[nblocks * self.block_size as usize..].to_vec();
                    inode = inode.with_tail_inline_data(inline_data);
                    if nblocks == 0 {
                        // Only inline data, set raw_blkaddr to 0xffffffff
                        inode = inode.with_raw_blkaddr(0xffffffff);
                    }
                } else if nblocks > 0 {
                    // No inline data, but external blocks, using PLAIN layout
                    inode = inode.with_data_layout(EROFS_INODE_FLAT_PLAIN);
                }

                next_ino += 1;

                if let Some(ref ctx) = selinux_context {
                    inode = inode.with_selinux_context(ctx);
                }

                inode.build()?
            } else if info.is_symlink {
                // Build symbolic link inode
                let target = info
                    .symlink_target
                    .as_ref()
                    .map(|t| t.as_bytes().to_vec())
                    .unwrap_or_default();
                let mut inode = InodeBuilder::new_symlink(info.uid, info.gid)
                    .with_mtime(info.mtime)
                    .with_ino(next_ino)
                    .with_extended(false)
                    .with_inline_data(target);

                next_ino += 1;

                if let Some(ref ctx) = selinux_context {
                    inode = inode.with_selinux_context(ctx);
                }

                inode.build()?
            } else {
                // File processing
                let file_data = info
                    .file_data
                    .as_ref()
                    .ok_or_else(|| ErofsError::Io(std::io::Error::other("文件数据不存在")))?;

                let mut inode = InodeBuilder::new_file(info.mode, info.uid, info.gid)
                    .with_mtime(info.mtime)
                    .with_ino(next_ino)
                    .with_size(file_data.len() as u64)
                    .with_extended(false);

                if info.use_compression {
                    // Use compressed layout
                    let physical_clusters = info
                        .physical_clusters
                        .as_ref()
                        .ok_or_else(|| ErofsError::Io(std::io::Error::other("物理簇数据不存在")))?;

                    // Get compression algorithm type
                    let algorithm = get_algorithm_type(
                        self.config
                            .compress_algorithm
                            .as_ref()
                            .unwrap_or(&"lz4".to_string()),
                    )?;

                    // Write the compressed data block and record the starting block address
                    let data_blk = (data_offset / self.block_size as u64) as u32;

                    // Compressed data is stored continuously.
                    // Only compressed clusters use ZERO_PADDING leading zeros; PLAIN clusters keep data leading and trailing zeros padded.
                    let mut all_compressed_data = Vec::new();
                    for pcluster in physical_clusters {
                        // Calculate the aligned size
                        let plen = pcluster.compressed_size.div_ceil(self.block_size as usize)
                            * self.block_size as usize;
                        let is_compressed = pcluster
                            .logical_clusters
                            .first()
                            .map(|lc| lc.is_compressed)
                            .unwrap_or(false);

                        if is_compressed {
                            // ZERO_PADDING: The compressed stream is placed at the end of the block and is padded with zeros in front.
                            let padding = plen - pcluster.compressed_size;
                            all_compressed_data.resize(all_compressed_data.len() + padding, 0);
                            all_compressed_data.extend_from_slice(&pcluster.compressed_data);
                        } else {
                            // PLAIN: The original data is written from the beginning of the block, with zeros at the end.
                            all_compressed_data.extend_from_slice(&pcluster.compressed_data);
                            if pcluster.compressed_size < plen {
                                all_compressed_data.resize(
                                    all_compressed_data.len() + (plen - pcluster.compressed_size),
                                    0,
                                );
                            }
                        }
                    }

                    // Write all compressed data at once
                    if !all_compressed_data.is_empty() {
                        data_entries.push((data_offset, all_compressed_data.clone()));
                        data_offset += all_compressed_data.len() as u64;
                    }

                    // Rebuild compression metadata (with correct start_blkaddr)
                    let (header, indexes) = build_compress_metadata(
                        file_data.len() as u64,
                        self.block_size,
                        algorithm,
                        physical_clusters,
                        data_blk,
                        info.xattr_size,
                    )?;

                    inode = inode
                        .with_data_layout(EROFS_INODE_COMPRESSED_COMPACT)
                        .with_compress_header(header)
                        .with_compress_indexes(indexes)
                        .with_raw_blkaddr(data_blk);
                } else {
                    // No compression: use only pure external or pure inline to avoid mixed tail layout inconsistencies
                    let max_inline = self.block_size as usize - 32 - info.xattr_size;
                    let (nblocks, actual_inline) = if file_data.len() <= max_inline {
                        (0usize, file_data.len())
                    } else {
                        (file_data.len().div_ceil(self.block_size as usize), 0usize)
                    };

                    if nblocks > 0 {
                        // There is data that needs to be stored in an external data block
                        let data_blk = (data_offset / self.block_size as u64) as u32;
                        // Store data (may need to be padded to block boundaries)
                        let external_size = nblocks * self.block_size as usize;
                        let mut block_data = file_data.to_vec();

                        // Pad to block boundaries
                        if block_data.len() < external_size {
                            block_data.resize(external_size, 0);
                        }

                        // record data block
                        data_entries.push((data_offset, block_data));
                        data_offset += external_size as u64;

                        // Set raw_blkaddr
                        inode = inode.with_raw_blkaddr(data_blk);
                    }

                    if actual_inline > 0 {
                        // small files inline
                        inode = inode.with_tail_inline_data(file_data.clone());
                        if nblocks == 0 {
                            // Only inline data, set raw_blkaddr to 0xffffffff
                            inode = inode.with_raw_blkaddr(0xffffffff);
                        }
                    } else if nblocks > 0 {
                        // No inline data, but external blocks, using PLAIN layout
                        inode = inode.with_data_layout(EROFS_INODE_FLAT_PLAIN);
                    }
                }

                next_ino += 1;

                if let Some(ref ctx) = selinux_context {
                    inode = inode.with_selinux_context(ctx);
                }

                inode.build()?
            };

            // Record inode data
            inode_entries.push((inode_offset, inode_data.clone()));
        }

        // Write all data blocks
        for (offset, data) in data_entries {
            self.write_at(offset, &data)?;
        }

        // Write to all inodes
        for (offset, data) in &inode_entries {
            self.write_at(*offset, data)?;
        }

        // Calculate the total number of blocks
        let total_blocks = data_offset.div_ceil(self.block_size as u64) as u32;
        self.superblock.set_blocks(total_blocks);

        // Data to build the first block (starting after the superblock for CRC calculation)
        // Block data = super block (128 bytes) + compressed configuration data + inode metadata
        let block_data_len = self.block_size as usize - EROFS_SUPER_OFFSET as usize;
        let mut block_data = vec![0u8; block_data_len];

        // If there is compression configuration data, fill it into the block buffer first
        let compr_cfgs_data =
            if self.superblock.feature_incompat() & EROFS_FEATURE_INCOMPAT_COMPR_CFGS != 0 {
                self.superblock.build_compr_cfgs()
            } else {
                Vec::new()
            };
        if !compr_cfgs_data.is_empty() {
            let cfgs_offset = EROFS_SUPER_BLOCK_SIZE;
            let copy_len = compr_cfgs_data.len().min(block_data_len - cfgs_offset);
            block_data[cfgs_offset..cfgs_offset + copy_len]
                .copy_from_slice(&compr_cfgs_data[..copy_len]);
        }

        // Fill inode data into block buffer (offset relative to EROFS_SUPER_OFFSET)
        for (offset, data) in &inode_entries {
            // offset is an absolute offset and needs to be converted to an offset relative to EROFS_SUPER_OFFSET
            let rel_offset = *offset as usize - EROFS_SUPER_OFFSET as usize;
            // Copy only the part within the first block (for CRC calculation)
            if rel_offset >= EROFS_SUPER_BLOCK_SIZE && rel_offset < block_data_len {
                let copy_len = data.len().min(block_data_len - rel_offset);
                block_data[rel_offset..rel_offset + copy_len].copy_from_slice(&data[..copy_len]);
            }
        }

        // Write superblock (write only 128 bytes, including correct checksum)
        let sb_data = self.superblock.build_with_checksum(&block_data)?;
        self.write_at(EROFS_SUPER_OFFSET, &sb_data)?;

        // Write compression configuration data (immediately following the superblock)
        if !compr_cfgs_data.is_empty() {
            let cfgs_offset = EROFS_SUPER_OFFSET + EROFS_SUPER_BLOCK_SIZE as u64;
            self.write_at(cfgs_offset, &compr_cfgs_data)?;
        }

        // Set file size
        self.writer
            .get_ref()
            .set_len(total_blocks as u64 * self.block_size as u64)?;
        self.writer.flush()?;

        Ok(())
    }
}

// Convenience function
pub fn build_erofs_image(config: ErofsConfig) -> Result<()> {
    let mut builder = ErofsBuilder::new(config)?;
    builder.build()
}
