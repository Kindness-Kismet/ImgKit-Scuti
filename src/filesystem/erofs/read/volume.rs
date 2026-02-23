// EROFS volume operations module

use crate::filesystem::erofs::*;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use zerocopy::TryFromBytes;

pub struct ErofsVolume {
    pub(crate) file: File,
    pub superblock: ErofsSuperBlock,
    pub block_size: u32,
}

impl ErofsVolume {
    // Create a new ErofsVolume instance by reading and validating the superblock.
    pub fn new(mut file: File) -> Result<Self> {
        file.seek(SeekFrom::Start(EROFS_SUPER_OFFSET))?;
        let mut sb_bytes = vec![0u8; std::mem::size_of::<ErofsSuperBlock>()];
        file.read_exact(&mut sb_bytes)?;

        let superblock = ErofsSuperBlock::try_read_from_bytes(&sb_bytes[..]).map_err(|_| {
            ErofsError::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "failed to parse superblock",
            ))
        })?;

        if superblock.magic != EROFS_SUPER_MAGIC_V1 {
            return Err(ErofsError::InvalidMagic {
                expected: EROFS_SUPER_MAGIC_V1,
                found: superblock.magic,
            });
        }

        let block_size = superblock.block_size();

        // Copy packed struct fields to avoid alignment issues.
        let blkszbits = superblock.blkszbits;
        let meta_blkaddr = superblock.meta_blkaddr;
        let root_nid = superblock.root_nid;

        log::debug!(
            "EROFS Superblock: blkszbits={}, block_size={}, meta_blkaddr={}, root_nid={}",
            blkszbits,
            block_size,
            meta_blkaddr,
            root_nid
        );

        Ok(Self {
            file,
            superblock,
            block_size,
        })
    }

    // Read and decode an inode by its NID.
    pub fn read_inode(&mut self, nid: u64) -> Result<InodeInfo> {
        log::debug!("read inode: nid={}", nid);
        let inode_offset = self.nid_to_offset(nid);
        log::debug!("  inode_offset={}", inode_offset);
        self.file.seek(SeekFrom::Start(inode_offset))?;

        // Peek at i_format to determine compact vs extended layout.
        let mut peek_buf = [0u8; 2];
        self.file.read_exact(&mut peek_buf)?;
        let i_format = u16::from_le_bytes(peek_buf);

        self.file.seek(SeekFrom::Start(inode_offset))?;

        let layout = i_format & EROFS_I_VERSION_MASK;

        if layout == EROFS_INODE_LAYOUT_COMPACT {
            let mut inode_bytes = vec![0u8; std::mem::size_of::<ErofsInodeCompact>()];
            self.file.read_exact(&mut inode_bytes)?;
            let inode = ErofsInodeCompact::try_read_from_bytes(&inode_bytes[..]).map_err(|_| {
                ErofsError::Io(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "failed to parse compact inode",
                ))
            })?;

            let info = InodeInfo {
                nid,
                mode: inode.i_mode,
                uid: inode.i_uid as u32,
                gid: inode.i_gid as u32,
                // When bit 4 of i_format is clear, nlink is stored in i_nb union.
                nlink: if (inode.i_format & (1 << 4)) == 0 {
                    unsafe { inode.i_nb.nlink as u32 }
                } else {
                    1
                },
                size: inode.i_size as u64,
                format: inode.i_format,
                xattr_icount: inode.i_xattr_icount,
                raw_blkaddr: inode.raw_blkaddr(),
                is_compact: true,
            };
            log::debug!(
                "  Compact inode: mode=0x{:04X}, size={}, format=0x{:04X}, raw_blkaddr={}",
                info.mode,
                info.size,
                info.format,
                info.raw_blkaddr
            );
            Ok(info)
        } else {
            let mut inode_bytes = vec![0u8; std::mem::size_of::<ErofsInodeExtended>()];
            self.file.read_exact(&mut inode_bytes)?;
            let inode =
                ErofsInodeExtended::try_read_from_bytes(&inode_bytes[..]).map_err(|_| {
                    ErofsError::Io(std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        "failed to parse extended inode",
                    ))
                })?;

            let info = InodeInfo {
                nid,
                mode: inode.i_mode,
                uid: inode.i_uid,
                gid: inode.i_gid,
                nlink: inode.i_nlink,
                size: inode.i_size,
                format: inode.i_format,
                xattr_icount: inode.i_xattr_icount,
                raw_blkaddr: inode.raw_blkaddr(),
                is_compact: false,
            };
            log::debug!(
                "  Extended inode: mode=0x{:04X}, size={}, format=0x{:04X}, raw_blkaddr={}",
                info.mode,
                info.size,
                info.format,
                info.raw_blkaddr
            );
            Ok(info)
        }
    }

    // Convert a NID to its absolute byte offset in the image.
    pub(crate) fn nid_to_offset(&self, nid: u64) -> u64 {
        let meta_blkaddr = self.superblock.meta_blkaddr as u64;
        meta_blkaddr
            .saturating_mul(self.block_size as u64)
            .saturating_add(nid.saturating_mul(32))
    }

    // Compute the xattr ibody size from erofs_fs.h erofs_xattr_ibody_size():
    // sizeof(erofs_xattr_ibody_header) + sizeof(__u32) * (i_xattr_icount - 1)
    pub(crate) fn xattr_ibody_size(&self, i_xattr_icount: u16) -> usize {
        if i_xattr_icount == 0 {
            0
        } else {
            std::mem::size_of::<ErofsXattrIbodyHeader>() + (i_xattr_icount as usize - 1) * 4
        }
    }

    // Return the root directory NID from the superblock.
    pub fn root_nid(&self) -> u64 {
        self.superblock.root_nid as u64
    }
}
