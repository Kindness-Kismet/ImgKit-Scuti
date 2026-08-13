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

        // 1. 读取 inline xattr (如果存在)
        if inode.inline & F2FS_INLINE_XATTR != 0 {
            let node_data = self.read_node(nid)?;

            // F2FS inline xattr 布局:
            // inline xattr 位于 inode footer 之前
            // 起始偏移固定为: node 大小 - footer(24) - inline_xattr_size
            let inline_xattr_size = DEFAULT_INLINE_XATTR_ADDRS * 4; // 200 字节
            let xattr_offset = node_data.len() - 24 - inline_xattr_size;

            if node_data.len() >= xattr_offset + inline_xattr_size {
                let xattr_data = &node_data[xattr_offset..xattr_offset + inline_xattr_size];

                // F2FS inline xattr 的前 4 字节为头部 (通常为 0x00000000)
                // 实际的 xattr 条目从第 5 字节开始
                if xattr_data.len() > 4 {
                    Self::parse_xattr_entries(&xattr_data[4..], &mut xattrs)?;
                }
            }
        }

        // 2. 读取 xattr node (如果存在)
        if inode.xattr_nid != 0 {
            let xattr_node_data = self.read_node(Nid(inode.xattr_nid))?;

            // xattr node 布局: 24 字节头部 + xattr 数据 + 24 字节 footer
            if xattr_node_data.len() > 48 {
                let xattr_data = &xattr_node_data[24..xattr_node_data.len() - 24];
                Self::parse_xattr_entries(xattr_data, &mut xattrs)?;
            }
        }

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

            match XattrEntry::from_bytes(&data[offset..]) {
                Ok((entry, size)) => {
                    let name = entry.full_name();
                    xattrs.push((name, entry.value.clone()));
                    offset += size;
                }
                Err(_) => break, // 出错时停止解析
            }
        }

        Ok(())
    }
}
