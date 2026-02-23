// F2FS Image Builder
use crate::filesystem::f2fs::consts::*;
//
// Provides complete F2FS image building functionality.

use crate::filesystem::f2fs::types::*;
use crate::filesystem::f2fs::write::{
    CheckpointBuilder, CursegInfo, DentryBlockBuilder, DentryInfo, DirectNodeBuilder, FsConfig,
    IndirectNodeBuilder, InodeBuilder, NatManager, SegmentAllocator, SelinuxContexts, SitManager,
    SsaManager, SuperblockBuilder,
};
use crate::filesystem::f2fs::{F2fsError, Result};
use crate::utils::symlink::read_symlink_info;
use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{BufWriter, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

// Directory information (for delayed writing to inodes)
struct DirInfo {
    path: PathBuf,
    fs_path: String,
    ino: u32,
    blkaddr: u32,
}

// F2FS Image Builder
pub struct F2fsBuilder {
    config: F2fsBuilderConfig,
    writer: BufWriter<File>,

    // Metadata Manager
    superblock: SuperblockBuilder,
    sit: SitManager,
    nat: NatManager,
    ssa: SsaManager,
    segment_alloc: SegmentAllocator,

    // Current status
    cp_ver: u64,
    timestamp: u64,

    // inode mapping (path -> ino)
    inode_map: HashMap<String, u32>,

    // SELinux context
    selinux_contexts: Option<SelinuxContexts>,

    // File system configuration
    fs_config: Option<FsConfig>,
}

impl F2fsBuilder {
    // Create new builder
    pub fn new(config: F2fsBuilderConfig) -> Result<Self> {
        // Create output file
        let file = File::create(&config.output_path)?;
        let writer = BufWriter::new(file);

        // Get timestamp
        let timestamp = config.timestamp.unwrap_or_else(|| {
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0)
        });

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

        // Create superblock builder and calculate layout
        let mut superblock = SuperblockBuilder::new(config.image_size)
            .with_features(config.features.clone())
            .with_label(&config.volume_label);
        superblock.calculate_layout()?;

        let layout = superblock
            .layout()
            .ok_or_else(|| F2fsError::InvalidData("布局计算失败".into()))?;

        // Initialization manager
        let sit = SitManager::new(
            layout.segment_count_main,
            layout.sit_blkaddr,
            layout.main_blkaddr,
        );
        let nat = NatManager::new(layout.nat_blkaddr, layout.segment_count_nat);
        let ssa = SsaManager::new(
            layout.segment_count_main,
            layout.ssa_blkaddr,
            layout.main_blkaddr,
        );
        let segment_alloc = SegmentAllocator::new(layout.main_blkaddr, layout.segment_count_main);

        Ok(F2fsBuilder {
            config,
            writer,
            superblock,
            sit,
            nat,
            ssa,
            segment_alloc,
            cp_ver: 1,
            timestamp,
            inode_map: HashMap::new(),
            selinux_contexts,
            fs_config,
        })
    }

    // Build image
    pub fn build(&mut self) -> Result<()> {
        // Initialize image file
        self.writer.get_ref().set_len(self.config.image_size)?;

        // Create root directory and load content
        let root_blkaddr = self.create_root_dir()?;
        let source_dir = self.config.source_dir.clone();
        let mount_point = self.config.mount_point.clone();

        let (root_data_addrs, subdir_count) = if source_dir.exists() {
            self.load_directory(&source_dir, F2FS_ROOT_INO, F2FS_ROOT_INO, &mount_point)?
        } else {
            // Create root directory data block containing "." and ".." even if there is no source directory
            let mut dentry_block = DentryBlockBuilder::new();
            dentry_block.add_entry(DentryInfo::new(b".", F2FS_ROOT_INO, FileType::Dir));
            dentry_block.add_entry(DentryInfo::new(b"..", F2FS_ROOT_INO, FileType::Dir));

            let data_blkaddr = self.segment_alloc.alloc_data_block(SegType::HotData)?;
            self.sit
                .mark_block_used(data_blkaddr, CURSEG_HOT_DATA as u16)?;
            self.ssa.set_data_summary(data_blkaddr, F2FS_ROOT_INO, 0)?;

            let dentry_data = dentry_block.build()?;
            self.write_block(data_blkaddr, &dentry_data)?;

            (vec![data_blkaddr], 0)
        };

        // Write to root directory inode
        self.write_dir_inode(
            F2FS_ROOT_INO,
            root_blkaddr,
            F2FS_ROOT_INO,
            b"/",
            1,
            &root_data_addrs,
            subdir_count,
            &mount_point,
        )?;

        // Write final metadata
        self.finalize()
    }

    // Create root directory (allocate inode blocks only)
    fn create_root_dir(&mut self) -> Result<u32> {
        let root_blkaddr = self.segment_alloc.alloc_node_block(SegType::HotNode)?;
        self.nat.init_reserved_inodes(root_blkaddr);
        self.sit
            .mark_block_used(root_blkaddr, CURSEG_HOT_NODE as u16)?;
        self.ssa.set_node_summary(root_blkaddr, F2FS_ROOT_INO)?;
        self.inode_map
            .insert(self.config.mount_point.clone(), F2FS_ROOT_INO);
        Ok(root_blkaddr)
    }

    // write directory inode
    #[allow(clippy::too_many_arguments)]
    fn write_dir_inode(
        &mut self,
        ino: u32,
        blkaddr: u32,
        pino: u32,
        name: &[u8],
        depth: u32,
        data_addrs: &[u32],
        child_count: u32,
        fs_path: &str,
    ) -> Result<()> {
        // Get uid/gid/mode
        let (uid, gid, mode) = if let Some(ref cfg) = self.fs_config {
            cfg.get_attrs(fs_path, true)
        } else {
            (self.config.root_uid, self.config.root_gid, 0o755)
        };

        let mut inode = InodeBuilder::new_dir(mode as u16, uid, gid)
            .with_timestamp(self.timestamp)
            .with_pino(pino)
            .with_name(name)
            .with_depth(depth)
            .with_links(2 + child_count)
            .with_size(F2FS_BLKSIZE as u64 * data_addrs.len() as u64)
            .with_blocks((1 + data_addrs.len()) as u64)
            .with_addrs(data_addrs.to_vec());

        // Set up SELinux context
        if let Some(ref mut ctx) = self.selinux_contexts
            && let Some(context) = ctx.lookup(fs_path)
        {
            inode = inode.with_selinux_context(&context);
        }

        self.write_block(blkaddr, &inode.build(ino, ino, self.cp_ver)?)
    }

    // Load directory contents
    fn load_directory(
        &mut self,
        source_path: &Path,
        parent_ino: u32,
        parent_pino: u32,
        parent_fs_path: &str,
    ) -> Result<(Vec<u32>, u32)> {
        let entries: Vec<_> = fs::read_dir(source_path)?.filter_map(|e| e.ok()).collect();

        // Supports multiple dentry blocks
        let mut dentry_blocks: Vec<DentryBlockBuilder> = vec![DentryBlockBuilder::new()];
        dentry_blocks[0].add_entry(DentryInfo::new(b".", parent_ino, FileType::Dir));
        dentry_blocks[0].add_entry(DentryInfo::new(b"..", parent_pino, FileType::Dir));

        let mut subdirs: Vec<DirInfo> = Vec::new();
        let mut subdir_count = 0u32;

        for entry in &entries {
            let name_bytes = entry.file_name();
            let name = name_bytes.as_encoded_bytes();
            let metadata = entry.metadata()?;

            // Calculate file system path
            let name_str = String::from_utf8_lossy(name);
            let fs_path = if parent_fs_path == "/" {
                format!("/{}", name_str)
            } else {
                format!("{}/{}", parent_fs_path, name_str)
            };

            let (ino, file_type) = if metadata.is_dir() {
                subdir_count += 1;
                let (ino, blkaddr) = self.alloc_dir_inode()?;
                subdirs.push(DirInfo {
                    path: entry.path(),
                    fs_path: fs_path.clone(),
                    ino,
                    blkaddr,
                });
                self.inode_map
                    .insert(entry.path().to_string_lossy().to_string(), ino);
                (ino, FileType::Dir)
            } else {
                // Detect symbolic links (supports Windows !<symlink> format)
                let symlink_info = read_symlink_info(&entry.path())
                    .map_err(|e| F2fsError::Io(std::io::Error::other(e.to_string())))?;

                let file_type = if symlink_info.is_symlink {
                    FileType::Symlink
                } else {
                    FileType::RegFile
                };
                let ino = self.create_file_inode(
                    &entry.path(),
                    parent_ino,
                    &metadata,
                    &fs_path,
                    symlink_info.target.as_deref(),
                )?;
                (ino, file_type)
            };

            // Try to add to current block, create new block if full
            let dentry = DentryInfo::new(name, ino, file_type);
            let current_block = dentry_blocks.last_mut().unwrap();
            if !current_block.add_entry(dentry.clone()) {
                let mut new_block = DentryBlockBuilder::new();
                new_block.add_entry(dentry);
                dentry_blocks.push(new_block);
            }
        }

        // Write all directory data blocks
        let mut data_addrs = Vec::new();
        for builder in &dentry_blocks {
            if !builder.is_empty() {
                let blkaddr = self.segment_alloc.alloc_data_block(SegType::HotData)?;
                self.sit.mark_block_used(blkaddr, CURSEG_HOT_DATA as u16)?;
                self.ssa
                    .set_data_summary(blkaddr, parent_ino, data_addrs.len() as u16)?;
                self.write_block(blkaddr, &builder.build()?)?;
                data_addrs.push(blkaddr);
            }
        }

        // Process subdirectories recursively
        for dir in subdirs {
            let name = dir
                .path
                .file_name()
                .map(|n| n.as_encoded_bytes().to_vec())
                .unwrap_or_default();
            let (sub_addrs, sub_count) =
                self.load_directory(&dir.path, dir.ino, parent_ino, &dir.fs_path)?;
            self.write_dir_inode(
                dir.ino,
                dir.blkaddr,
                parent_ino,
                &name,
                2,
                &sub_addrs,
                sub_count,
                &dir.fs_path,
            )?;
        }

        Ok((data_addrs, subdir_count))
    }

    // Allocate directory inode blocks
    fn alloc_dir_inode(&mut self) -> Result<(u32, u32)> {
        let nid = self.nat.alloc_nid();
        let blkaddr = self.segment_alloc.alloc_node_block(SegType::HotNode)?;
        self.nat.set_entry(nid, blkaddr, nid.0);
        self.sit.mark_block_used(blkaddr, CURSEG_HOT_NODE as u16)?;
        self.ssa.set_node_summary(blkaddr, nid.0)?;
        Ok((nid.0, blkaddr))
    }

    // Create file inode
    fn create_file_inode(
        &mut self,
        path: &Path,
        parent_ino: u32,
        metadata: &fs::Metadata,
        fs_path: &str,
        symlink_target: Option<&str>,
    ) -> Result<u32> {
        let nid = self.nat.alloc_nid();
        let ino = nid.0;
        let blkaddr = self.segment_alloc.alloc_node_block(SegType::WarmNode)?;
        self.nat.set_entry(nid, blkaddr, ino);
        self.sit.mark_block_used(blkaddr, CURSEG_WARM_NODE as u16)?;
        self.ssa.set_node_summary(blkaddr, nid.0)?;

        let file_name = path
            .file_name()
            .map(|n| n.as_encoded_bytes().to_vec())
            .unwrap_or_default();

        // Get uid/gid/mode
        let (uid, gid, mode) = if let Some(ref cfg) = self.fs_config {
            cfg.get_attrs(fs_path, false)
        } else {
            (self.config.root_uid, self.config.root_gid, 0o644)
        };

        // Create inode
        let mut inode = if let Some(target) = symlink_target {
            // symbolic link
            InodeBuilder::new_symlink(uid, gid).with_symlink_target(target)
        } else {
            // Ordinary document
            let file_size = metadata.len();

            // Write file data blocks (only for normal files)
            let (direct_addrs, nids) = if metadata.is_file() && file_size > 0 {
                let all_addrs = self.write_file_data(path)?;
                self.organize_file_addrs(ino, all_addrs)?
            } else {
                (vec![], [0; 5])
            };

            InodeBuilder::new_file(mode as u16, uid, gid)
                .with_size(file_size)
                // i_blocks contains the inode block itself + the number of data blocks
                .with_blocks(file_size.div_ceil(F2FS_BLKSIZE as u64) + 1)
                .with_addrs(direct_addrs)
                .with_nids(nids)
        }
        .with_timestamp(self.timestamp)
        .with_pino(parent_ino)
        .with_name(&file_name);

        // Set up SELinux context
        if let Some(ref mut ctx) = self.selinux_contexts
            && let Some(context) = ctx.lookup(fs_path)
        {
            inode = inode.with_selinux_context(&context);
        }

        self.write_block(blkaddr, &inode.build(ino, ino, self.cp_ver)?)?;
        self.inode_map
            .insert(path.to_string_lossy().to_string(), ino);
        Ok(ino)
    }

    // Write file data block
    fn write_file_data(&mut self, path: &Path) -> Result<Vec<u32>> {
        let data = fs::read(path)?;
        let mut all_addrs = Vec::new();

        // Allocate all data blocks
        for chunk in data.chunks(F2FS_BLKSIZE) {
            let blkaddr = self.segment_alloc.alloc_data_block(SegType::WarmData)?;
            self.sit.mark_block_used(blkaddr, CURSEG_WARM_DATA as u16)?;
            // SSA records will be set in organize_file_addrs because ino needs to be known

            let mut block = vec![0u8; F2FS_BLKSIZE];
            block[..chunk.len()].copy_from_slice(chunk);
            self.write_block(blkaddr, &block)?;
            all_addrs.push(blkaddr);
        }

        Ok(all_addrs)
    }

    // Organization file addresses (handles direct and indirect addresses)
    fn organize_file_addrs(
        &mut self,
        ino: u32,
        all_addrs: Vec<u32>,
    ) -> Result<(Vec<u32>, [u32; 5])> {
        const ADDRS_PER_INODE: usize = 864; // When there are extra_attr and inline_xattr
        const ADDRS_PER_BLOCK: usize = 1018;
        const NIDS_PER_BLOCK: usize = 1018;

        let mut direct_addrs = Vec::new();
        let mut nids = [0u32; 5];

        // Set SSA records for all data blocks
        for (idx, &blkaddr) in all_addrs.iter().enumerate() {
            self.ssa.set_data_summary(blkaddr, ino, idx as u16)?;
        }

        // 1. Direct address (stored in inode)
        let direct_count = all_addrs.len().min(ADDRS_PER_INODE);
        direct_addrs.extend_from_slice(&all_addrs[..direct_count]);

        if all_addrs.len() <= ADDRS_PER_INODE {
            return Ok((direct_addrs, nids));
        }

        // 2. The first direct indirect node (nids[0])
        let mut remaining = &all_addrs[direct_count..];
        if !remaining.is_empty() {
            let count = remaining.len().min(ADDRS_PER_BLOCK);
            let nid = self.nat.alloc_nid();
            let blkaddr = self.segment_alloc.alloc_node_block(SegType::WarmNode)?;
            self.nat.set_entry(nid, blkaddr, ino);
            self.sit.mark_block_used(blkaddr, CURSEG_WARM_NODE as u16)?;
            self.ssa.set_node_summary(blkaddr, nid.0)?;

            let direct_node = DirectNodeBuilder::new()
                .with_addrs(remaining[..count].to_vec())
                .build(nid.0, ino, self.cp_ver);
            self.write_block(blkaddr, &direct_node)?;
            nids[0] = nid.0;

            remaining = &remaining[count..];
        }

        // 3. The second direct indirect node (nids[1])
        if !remaining.is_empty() {
            let count = remaining.len().min(ADDRS_PER_BLOCK);
            let nid = self.nat.alloc_nid();
            let blkaddr = self.segment_alloc.alloc_node_block(SegType::WarmNode)?;
            self.nat.set_entry(nid, blkaddr, ino);
            self.sit.mark_block_used(blkaddr, CURSEG_WARM_NODE as u16)?;
            self.ssa.set_node_summary(blkaddr, nid.0)?;

            let direct_node = DirectNodeBuilder::new()
                .with_addrs(remaining[..count].to_vec())
                .build(nid.0, ino, self.cp_ver);
            self.write_block(blkaddr, &direct_node)?;
            nids[1] = nid.0;

            remaining = &remaining[count..];
        }

        // 4. The first double indirect node (nids[2])
        if !remaining.is_empty() {
            nids[2] = self.alloc_double_indirect_node(ino, remaining, ADDRS_PER_BLOCK)?;
            let consumed = remaining.len().min(NIDS_PER_BLOCK * ADDRS_PER_BLOCK);
            remaining = &remaining[consumed..];
        }

        // 5. The second double indirect node (nids[3])
        if !remaining.is_empty() {
            nids[3] = self.alloc_double_indirect_node(ino, remaining, ADDRS_PER_BLOCK)?;
            let consumed = remaining.len().min(NIDS_PER_BLOCK * ADDRS_PER_BLOCK);
            remaining = &remaining[consumed..];
        }

        // 6. Triple indirect nodes (nids[4]) - if required
        if !remaining.is_empty() {
            log::warn!(
                "文件需要三重间接节点 (剩余 {} 个块)，暂未实现",
                remaining.len()
            );
        }

        Ok((direct_addrs, nids))
    }

    // Allocate double indirect nodes
    fn alloc_double_indirect_node(
        &mut self,
        ino: u32,
        addrs: &[u32],
        addrs_per_block: usize,
    ) -> Result<u32> {
        const NIDS_PER_BLOCK: usize = 1018;

        // Allocate double indirect nodes
        let double_indirect_nid = self.nat.alloc_nid();
        let double_indirect_blkaddr = self.segment_alloc.alloc_node_block(SegType::WarmNode)?;
        self.nat
            .set_entry(double_indirect_nid, double_indirect_blkaddr, ino);
        self.sit
            .mark_block_used(double_indirect_blkaddr, CURSEG_WARM_NODE as u16)?;
        self.ssa
            .set_node_summary(double_indirect_blkaddr, double_indirect_nid.0)?;

        // Create an indirect node builder
        let mut indirect_builder = IndirectNodeBuilder::new();

        // Assign an address to each direct node
        let mut offset = 0;
        while offset < addrs.len() && indirect_builder.len() < NIDS_PER_BLOCK {
            let chunk_size = (addrs.len() - offset).min(addrs_per_block);
            let chunk = &addrs[offset..offset + chunk_size];

            // Assign direct node
            let direct_nid = self.nat.alloc_nid();
            let direct_blkaddr = self.segment_alloc.alloc_node_block(SegType::WarmNode)?;
            self.nat.set_entry(direct_nid, direct_blkaddr, ino);
            self.sit
                .mark_block_used(direct_blkaddr, CURSEG_WARM_NODE as u16)?;
            self.ssa.set_node_summary(direct_blkaddr, direct_nid.0)?;

            // Write to direct node
            let direct_node = DirectNodeBuilder::new().with_addrs(chunk.to_vec()).build(
                direct_nid.0,
                ino,
                self.cp_ver,
            );
            self.write_block(direct_blkaddr, &direct_node)?;

            // Add to indirect node
            indirect_builder.add_nid(direct_nid.0);

            offset += chunk_size;
        }

        // Write to double indirect node
        let double_indirect_node = indirect_builder.build(double_indirect_nid.0, ino, self.cp_ver);
        self.write_block(double_indirect_blkaddr, &double_indirect_node)?;

        Ok(double_indirect_nid.0)
    }

    // Complete build
    fn finalize(&mut self) -> Result<()> {
        let layout = self
            .superblock
            .layout()
            .ok_or_else(|| F2fsError::InvalidData("布局未计算".into()))?
            .clone();

        // write superblock
        let sb_data = self.superblock.build()?;
        self.writer.seek(SeekFrom::Start(F2FS_SUPER_OFFSET))?;
        self.writer.write_all(&sb_data)?;
        self.writer
            .seek(SeekFrom::Start(F2FS_SUPER_OFFSET + F2FS_BLKSIZE as u64))?;
        self.writer.write_all(&sb_data)?;

        // CP packet structure: cp_header(1) + data_sum(1) + node_sum(3) + cp_footer(1) = 6 blocks
        let cp_pack_blocks = 6u32;

        // Calculate reserved and over-provisioned segments
        let ovp_segment_count = (layout.segment_count_main as f64 * 0.05) as u32;
        let ovp_segment_count = ovp_segment_count.max(2); // at least 2
        let rsvd_segment_count = ovp_segment_count.max(2); // at least 2

        // Calculate the number of user blocks (number of main area blocks - number of reserved blocks)
        let user_block_count = (layout.segment_count_main - ovp_segment_count) as u64
            * DEFAULT_BLOCKS_PER_SEGMENT as u64;

        // Calculate bitmap size
        // sit_ver_bitmap_bytesize = (segment_count_sit / 2) * blocks_per_seg / 8
        // nat_ver_bitmap_bytesize = (segment_count_nat / 2) * blocks_per_seg / 8
        let sit_bitmap_size = ((layout.segment_count_sit / 2) * DEFAULT_BLOCKS_PER_SEGMENT) / 8;
        let nat_bitmap_size = ((layout.segment_count_nat / 2) * DEFAULT_BLOCKS_PER_SEGMENT) / 8;

        // Generate the correct size bitmap
        let sit_bitmap = vec![0u8; sit_bitmap_size as usize];
        let nat_bitmap = vec![0u8; nat_bitmap_size as usize];

        // Get current segment allocation information
        let curseg_info = self.segment_alloc.get_curseg_info();

        // Set the correct SIT type for all curseg
        // The type needs to be set even if there are no allocation blocks in the segment
        self.sit
            .set_seg_type(curseg_info.node_segno[0], CURSEG_HOT_NODE as u16)?;
        self.sit
            .set_seg_type(curseg_info.node_segno[1], CURSEG_WARM_NODE as u16)?;
        self.sit
            .set_seg_type(curseg_info.node_segno[2], CURSEG_COLD_NODE as u16)?;
        self.sit
            .set_seg_type(curseg_info.data_segno[0], CURSEG_HOT_DATA as u16)?;
        self.sit
            .set_seg_type(curseg_info.data_segno[1], CURSEG_WARM_DATA as u16)?;
        self.sit
            .set_seg_type(curseg_info.data_segno[2], CURSEG_COLD_DATA as u16)?;

        // Build checkpoint
        let mut checkpoint = CheckpointBuilder::new()
            .with_version(self.cp_ver)
            .with_user_block_count(user_block_count)
            .with_valid_block_count(self.segment_alloc.allocated_blocks())
            .with_free_segment_count(self.segment_alloc.free_segments())
            .with_rsvd_segment_count(rsvd_segment_count)
            .with_overprov_segment_count(ovp_segment_count)
            .with_next_free_nid(self.nat.next_free_nid())
            // valid_node_count: actual number of node blocks (excluding node_ino and meta_ino)
            // There are node_ino(1), meta_ino(2), root_ino(3), and other inodes in NAT
            // The block_addr=1 of node_ino and meta_ino is a special mark and is not counted.
            .with_valid_node_count(self.nat.entry_count() as u32 - 2)
            // valid_inode_count: actual inode number (directory and file)
            // inode_map already contains the root directory, so use inode_map.len() directly
            .with_valid_inode_count(self.inode_map.len() as u32)
            .with_sit_bitmap(sit_bitmap)
            .with_nat_bitmap(nat_bitmap)
            .with_cp_pack_total_block_count(cp_pack_blocks)
            // Use CP_UMOUNT_FLAG | CP_COMPACT_SUM_FLAG
            // CP_COMPACT_SUM_FLAG indicates that the DATA summary uses compact format
            .with_flags(CP_UMOUNT_FLAG | CP_COMPACT_SUM_FLAG);

        // Set current segment information
        checkpoint.set_cur_node_seg(0, curseg_info.node_segno[0], curseg_info.node_blkoff[0]);
        checkpoint.set_cur_node_seg(1, curseg_info.node_segno[1], curseg_info.node_blkoff[1]);
        checkpoint.set_cur_node_seg(2, curseg_info.node_segno[2], curseg_info.node_blkoff[2]);
        // Unused segments are set to 0xFFFFFFFF
        for i in 3..8 {
            checkpoint.set_cur_node_seg(i, 0xFFFFFFFF, 0);
        }

        checkpoint.set_cur_data_seg(0, curseg_info.data_segno[0], curseg_info.data_blkoff[0]);
        checkpoint.set_cur_data_seg(1, curseg_info.data_segno[1], curseg_info.data_blkoff[1]);
        checkpoint.set_cur_data_seg(2, curseg_info.data_segno[2], curseg_info.data_blkoff[2]);
        // Unused segments are set to 0xFFFFFFFF
        for i in 3..8 {
            checkpoint.set_cur_data_seg(i, 0xFFFFFFFF, 0);
        }

        let cp_data = checkpoint.build()?;

        // Get the SSA data of the current segment for checkpoint package
        let hot_node_segno = curseg_info.node_segno[0] as usize;
        let warm_node_segno = curseg_info.node_segno[1] as usize;
        let cold_node_segno = curseg_info.node_segno[2] as usize;
        let hot_data_segno = curseg_info.data_segno[0] as usize;
        let warm_data_segno = curseg_info.data_segno[1] as usize;
        let cold_data_segno = curseg_info.data_segno[2] as usize;

        // Construct a compact format DATA summary block
        // Structure: nat_journal(SUM_JOURNAL_SIZE) + sit_journal(SUM_JOURNAL_SIZE) + data summaries
        let compact_sum = self.build_compact_data_summary(
            &curseg_info,
            hot_data_segno,
            warm_data_segno,
            cold_data_segno,
        )?;

        // Build NODE summary blocks (normal format)
        let node_sum_hot = self.ssa.build_curseg_summary(hot_node_segno, true)?;
        let node_sum_warm = self.ssa.build_curseg_summary(warm_node_segno, true)?;
        let node_sum_cold = self.ssa.build_curseg_summary(cold_node_segno, true)?;

        // Write the first checkpoint packet
        let cp_offset = layout.cp_blkaddr as u64 * F2FS_BLKSIZE as u64;
        self.writer.seek(SeekFrom::Start(cp_offset))?;
        self.writer.write_all(&cp_data)?; // block 0: CP header

        self.writer.write_all(&compact_sum)?; // block 1: compact data summary
        self.writer.write_all(&node_sum_hot)?; // block 2: hot node summary
        self.writer.write_all(&node_sum_warm)?; // block 3: warm node summary
        self.writer.write_all(&node_sum_cold)?; // block 4: cold node summary
        self.writer.write_all(&cp_data)?; // block 5: CP footer

        // Write the second checkpoint packet (in the next segment)
        let cp2_offset =
            (layout.cp_blkaddr + DEFAULT_BLOCKS_PER_SEGMENT) as u64 * F2FS_BLKSIZE as u64;
        self.writer.seek(SeekFrom::Start(cp2_offset))?;
        self.writer.write_all(&cp_data)?; // block 0: CP header
        self.writer.write_all(&compact_sum)?; // block 1: compact data summary
        self.writer.write_all(&node_sum_hot)?; // block 2: hot node summary
        self.writer.write_all(&node_sum_warm)?; // block 3: warm node summary
        self.writer.write_all(&node_sum_cold)?; // block 4: cold node summary
        self.writer.write_all(&cp_data)?; // block 5: CP footer

        // The SIT area remains empty and the data is stored in the checkpoint's SIT journal.

        // Write to NAT zone
        let nat_data = self.nat.to_bytes();
        let nat_offset = layout.nat_blkaddr as u64 * F2FS_BLKSIZE as u64;
        self.writer.seek(SeekFrom::Start(nat_offset))?;
        self.writer.write_all(&nat_data)?;

        // Write second NAT
        let nat_blocks_per_copy =
            (layout.segment_count_nat / 2) * DEFAULT_BLOCKS_PER_SEGMENT / F2FS_BLKSIZE as u32;
        let nat_offset_2 = (layout.nat_blkaddr + nat_blocks_per_copy) as u64 * F2FS_BLKSIZE as u64;
        self.writer.seek(SeekFrom::Start(nat_offset_2))?;
        self.writer.write_all(&nat_data)?;

        // The SSA area remains empty and data is stored in the checkpoint's summary blocks

        self.writer.flush()?;
        Ok(())
    }

    // Construct a compact format DATA summary block
    // Structure: n_nats + n_sits + NAT entries + SIT entries + DATA summaries + footer
    fn build_compact_data_summary(
        &self,
        curseg_info: &CursegInfo,
        hot_data_segno: usize,
        warm_data_segno: usize,
        cold_data_segno: usize,
    ) -> Result<Vec<u8>> {
        let mut buf = vec![0u8; F2FS_BLKSIZE];
        let mut offset = 0usize;

        // 1. NAT journal (507 bytes)
        buf[offset..offset + 2].copy_from_slice(&0u16.to_le_bytes());
        offset = 507; // Skip entire NAT journal space

        // 2. SIT journal (507 bytes)
        let n_sits: u16 = 6;
        buf[offset..offset + 2].copy_from_slice(&n_sits.to_le_bytes());
        offset += 2;

        for i in 0..6 {
            let (segno, seg_type) = if i < 3 {
                (curseg_info.data_segno[i], i as u16)
            } else {
                (curseg_info.node_segno[i - 3], i as u16)
            };

            let valid_blocks = if i < 3 {
                curseg_info.data_blkoff[i]
            } else {
                curseg_info.node_blkoff[i - 3]
            };

            // segno (4 bytes)
            buf[offset..offset + 4].copy_from_slice(&segno.to_le_bytes());
            offset += 4;

            // vblocks (2 bytes)
            let vblocks = valid_blocks | (seg_type << SIT_VBLOCKS_SHIFT);
            buf[offset..offset + 2].copy_from_slice(&vblocks.to_le_bytes());
            offset += 2;

            // valid_map (64 bytes)
            if let Some(sit_entry) = self.sit.get_entry(segno) {
                buf[offset..offset + 64].copy_from_slice(&sit_entry.valid_map);
            }
            offset += 64;

            // mtime (8 bytes)
            offset += 8;
        }

        // Pad SIT journal to 507 bytes total
        offset = 1014; // NAT journal (507) + SIT journal (507)

        // 3. DATA summaries (hot, warm, cold)
        let data_segnos = [hot_data_segno, warm_data_segno, cold_data_segno];
        let data_blkoffs = [
            curseg_info.data_blkoff[0] as usize,
            curseg_info.data_blkoff[1] as usize,
            curseg_info.data_blkoff[2] as usize,
        ];

        for (seg_idx, &segno) in data_segnos.iter().enumerate() {
            let blk_off = data_blkoffs[seg_idx];

            for j in 0..blk_off {
                if offset + SUMMARY_SIZE > F2FS_BLKSIZE - SUM_FOOTER_SIZE {
                    break;
                }

                // Get summary entry from SSA Manager
                if let Some(entry) = self.ssa.get_summary_entry(segno, j) {
                    buf[offset..offset + 4].copy_from_slice(&entry.nid.to_le_bytes());
                    buf[offset + 4] = entry.version;
                    buf[offset + 5..offset + 7].copy_from_slice(&entry.ofs_in_node.to_le_bytes());
                }
                offset += SUMMARY_SIZE;
            }
        }

        Ok(buf)
    }

    // write block
    fn write_block(&mut self, blkaddr: u32, data: &[u8]) -> Result<()> {
        self.writer
            .seek(SeekFrom::Start(blkaddr as u64 * F2FS_BLKSIZE as u64))?;
        self.writer.write_all(data)?;
        Ok(())
    }
}

// Simplified build function
pub fn build_f2fs_image(
    source_dir: &Path,
    output_path: &Path,
    image_size: u64,
    mount_point: &str,
) -> Result<()> {
    let config = F2fsBuilderConfig {
        source_dir: source_dir.to_path_buf(),
        output_path: output_path.to_path_buf(),
        image_size,
        mount_point: mount_point.to_string(),
        features: F2fsFeatures::default(),
        ..Default::default()
    };

    let mut builder = F2fsBuilder::new(config)?;
    builder.build()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;
    use std::fs;
    use std::sync::atomic::{AtomicU32, Ordering};

    static TEST_COUNTER: AtomicU32 = AtomicU32::new(0);

    fn create_temp_dir() -> std::path::PathBuf {
        let counter = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
        let temp_dir = env::temp_dir().join(format!(
            "f2fs_test_{}_{}_{}",
            std::process::id(),
            counter,
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = fs::remove_dir_all(&temp_dir);
        fs::create_dir_all(&temp_dir).unwrap();
        temp_dir
    }

    fn cleanup_temp_dir(path: &Path) {
        let _ = fs::remove_dir_all(path);
    }

    #[test]
    fn test_f2fs_builder_new() {
        let temp_dir = create_temp_dir();
        let output_path = temp_dir.join("test.img");

        let config = F2fsBuilderConfig {
            source_dir: temp_dir.clone(),
            output_path: output_path.clone(),
            image_size: 100 * 1024 * 1024, // 100MB
            mount_point: "/".to_string(),
            features: F2fsFeatures::default(),
            ..Default::default()
        };

        let builder = F2fsBuilder::new(config);
        assert!(builder.is_ok());

        cleanup_temp_dir(&temp_dir);
    }

    #[test]
    fn test_f2fs_builder_build_empty() {
        let temp_dir = create_temp_dir();
        let source_dir = temp_dir.join("source");
        fs::create_dir_all(&source_dir).unwrap();
        let output_path = temp_dir.join("test.img");

        let config = F2fsBuilderConfig {
            source_dir,
            output_path: output_path.clone(),
            image_size: 100 * 1024 * 1024,
            mount_point: "/".to_string(),
            features: F2fsFeatures::default(),
            ..Default::default()
        };

        let mut builder = F2fsBuilder::new(config).unwrap();
        let result = builder.build();
        assert!(result.is_ok(), "Build failed: {:?}", result.err());

        // Verification file created
        assert!(output_path.exists());
        let metadata = fs::metadata(&output_path).unwrap();
        assert_eq!(metadata.len(), 100 * 1024 * 1024);

        cleanup_temp_dir(&temp_dir);
    }
}
