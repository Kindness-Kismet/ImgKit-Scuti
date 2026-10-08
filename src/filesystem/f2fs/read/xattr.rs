// F2FS xattr 读取模块

use super::super::error::Result;
use super::super::types::{Inode, Nid, XattrEntry};
use super::volume::F2fsVolume;
use crate::filesystem::f2fs::*;
use std::io::{Read, Seek};

impl<R: Read + Seek + Send> F2fsVolume<R> {
    // 读取 inode 的全部 xattr
    pub fn read_xattrs(&self, inode: &Inode, nid: Nid) -> Result<Vec<(String, Vec<u8>)>> {
        let mut xattrs = Vec::new();
        let mut data = Vec::new();
        if inode.inline & F2FS_INLINE_XATTR != 0 {
            let node_data = self.read_node(nid)?;
            let inline_size = if self.superblock.features & F2FS_FEATURE_FLEXIBLE_INLINE_XATTR != 0
                && inode.inline & F2FS_EXTRA_ATTR != 0
            {
                usize::from(u16::from_le_bytes([node_data[362], node_data[363]])) * 4
            } else {
                DEFAULT_INLINE_XATTR_ADDRS * 4
            };
            let nid_offset = 360 + DEF_ADDRS_PER_INODE * 4;
            let extra_size = if inode.inline & F2FS_EXTRA_ATTR != 0 {
                usize::from(inode.extra_isize)
            } else {
                0
            };
            if inline_size + extra_size > DEF_ADDRS_PER_INODE * 4 {
                return Err(F2fsError::InvalidData("内联扩展属性长度越界".into()));
            }
            data.extend_from_slice(&node_data[nid_offset - inline_size..nid_offset]);
        }
        if inode.xattr_nid != 0 {
            let node_data = self.read_node(Nid(inode.xattr_nid))?;
            data.extend_from_slice(&node_data[..node_data.len() - 24]);
        }
        if data.is_empty() || data.iter().all(|byte| *byte == 0) {
            return Ok(xattrs);
        }
        if data.len() < 28 || data[..4] != 0xF2F52011u32.to_le_bytes() {
            return Err(F2fsError::InvalidData("扩展属性头无效".into()));
        }
        Self::parse_xattr_entries(&data[24..], &mut xattrs)?;
        Ok(xattrs)
    }

    // 解析 xattr 条目
    fn parse_xattr_entries(data: &[u8], xattrs: &mut Vec<(String, Vec<u8>)>) -> Result<()> {
        let mut offset = 0;

        while offset + 4 <= data.len() {
            // 检查是否已到达末尾 (全为 0)
            if data[offset] == 0 && data[offset + 1] == 0 {
                break;
            }

            let (entry, size) = XattrEntry::from_bytes(&data[offset..])
                .map_err(|err| F2fsError::InvalidData(err.to_string()))?;
            xattrs.push((entry.full_name(), entry.value));
            offset += size;
        }

        Ok(())
    }
}
