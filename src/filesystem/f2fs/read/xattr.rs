// F2FS xattr reading module

use super::super::error::Result;
use super::super::types::{Inode, Nid, XattrEntry};
use super::volume::F2fsVolume;
use crate::filesystem::f2fs::*;

impl F2fsVolume {
    // Read all xattr of inode
    pub fn read_xattrs(&self, inode: &Inode, nid: Nid) -> Result<Vec<(String, Vec<u8>)>> {
        let mut xattrs = Vec::new();

        // 1. Read the inline xattr (if any)
        if inode.inline & F2FS_INLINE_XATTR != 0 {
            let node_data = self.read_node(nid)?;

            // F2FS inline xattr layout:
            // inline xattr before inode footer
            // Always start at fixed offset: node size - footer(24) - inline_xattr_size
            let inline_xattr_size = DEFAULT_INLINE_XATTR_ADDRS * 4; // 200 bytes
            let xattr_offset = node_data.len() - 24 - inline_xattr_size;

            if node_data.len() >= xattr_offset + inline_xattr_size {
                let xattr_data = &node_data[xattr_offset..xattr_offset + inline_xattr_size];

                // The first 4 bytes of F2FS inline xattr are header (usually 0x00000000)
                // Actual xattr entries start from byte 5
                if xattr_data.len() > 4 {
                    Self::parse_xattr_entries(&xattr_data[4..], &mut xattrs)?;
                }
            }
        }

        // 2. Read the xattr node (if any)
        if inode.xattr_nid != 0 {
            let xattr_node_data = self.read_node(Nid(inode.xattr_nid))?;

            // xattr node layout: 24-byte header + xattr data + 24-byte footer
            if xattr_node_data.len() > 48 {
                let xattr_data = &xattr_node_data[24..xattr_node_data.len() - 24];
                Self::parse_xattr_entries(xattr_data, &mut xattrs)?;
            }
        }

        Ok(xattrs)
    }

    // Parse xattr entries
    fn parse_xattr_entries(data: &[u8], xattrs: &mut Vec<(String, Vec<u8>)>) -> Result<()> {
        let mut offset = 0;

        while offset + 4 <= data.len() {
            // Check if the end is reached (all 0s)
            if data[offset] == 0 && data[offset + 1] == 0 {
                break;
            }

            match XattrEntry::from_bytes(&data[offset..]) {
                Ok((entry, size)) => {
                    let name = entry.full_name();
                    xattrs.push((name, entry.value.clone()));
                    offset += size;
                }
                Err(_) => break, // Stop parsing when an error occurs
            }
        }

        Ok(())
    }
}
