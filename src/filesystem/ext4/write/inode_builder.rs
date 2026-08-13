// EXT4 inode 构建器

use crate::filesystem::ext4::Result;
use crate::filesystem::ext4::types::*;
use crate::filesystem::ext4::write::extent::ExtentBuilder;
use crate::filesystem::ext4::write::xattr::*;
use std::time::{SystemTime, UNIX_EPOCH};

// inode 构建器
pub struct InodeBuilder {
    mode: u16,
    uid: u32,
    gid: u32,
    size: u64,
    atime: u32,
    ctime: u32,
    mtime: u32,
    dtime: u32,
    links_count: u16,
    blocks: u32,
    flags: u32,
    i_block: [u8; 60],
    xattrs: Vec<XattrEntry>,
}

impl InodeBuilder {
    // 创建新的 inode 构建器
    pub fn new() -> Self {
        // 系统时钟早于 epoch 时退化为 0, 不影响 inode 有效性
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as u32;

        InodeBuilder {
            mode: 0,
            uid: 0,
            gid: 0,
            size: 0,
            atime: now,
            ctime: now,
            mtime: now,
            dtime: 0,
            links_count: 1,
            blocks: 0,
            flags: inode_mode::EXT4_EXTENTS_FL,
            i_block: [0; 60],
            xattrs: Vec::new(),
        }
    }

    // 创建目录 inode
    pub fn new_dir(mode: u16, uid: u32, gid: u32) -> Self {
        let mut builder = Self::new();
        builder.mode = inode_mode::S_IFDIR | (mode & 0o7777);
        builder.uid = uid;
        builder.gid = gid;
        builder.links_count = 2;
        builder
    }

    // 创建普通文件 inode
    pub fn new_file(mode: u16, uid: u32, gid: u32) -> Self {
        let mut builder = Self::new();
        builder.mode = inode_mode::S_IFREG | (mode & 0o7777);
        builder.uid = uid;
        builder.gid = gid;
        builder
    }

    // 创建符号链接 inode
    pub fn new_symlink(uid: u32, gid: u32) -> Self {
        let mut builder = Self::new();
        builder.mode = inode_mode::S_IFLNK | 0o777;
        builder.uid = uid;
        builder.gid = gid;
        builder.flags = 0; // 符号链接不使用 extent
        builder
    }

    // 设置文件大小
    pub fn with_size(mut self, size: u64) -> Self {
        self.size = size;
        self
    }

    // 设置块数
    pub fn with_blocks(mut self, blocks: u32) -> Self {
        self.blocks = blocks;
        self
    }

    // 设置链接数
    pub fn with_links(mut self, links: u16) -> Self {
        self.links_count = links;
        self
    }

    // 设置 extent 数据
    pub fn with_extents(mut self, extents: &ExtentBuilder) -> Self {
        self.i_block = extents.build_inline();
        self
    }

    // 设置 extent 标志
    pub fn with_extent_flag(mut self) -> Self {
        self.flags |= inode_mode::EXT4_EXTENTS_FL;
        self
    }

    // 设置符号链接目标
    pub fn with_symlink_target(mut self, target: &str) -> Self {
        let target_bytes = target.as_bytes();
        let copy_len = target_bytes.len().min(60);
        self.i_block[..copy_len].copy_from_slice(&target_bytes[..copy_len]);
        self.size = target_bytes.len() as u64;
        self
    }

    // 添加 xattr
    pub fn add_xattr(&mut self, entry: XattrEntry) {
        self.xattrs.push(entry);
    }

    // 设置 SELinux 安全上下文
    pub fn with_selinux_context(mut self, context: &str) -> Self {
        self.xattrs.push(XattrEntry::selinux(context));
        self
    }

    // 设置时间戳
    pub fn with_times(mut self, atime: u32, ctime: u32, mtime: u32) -> Self {
        self.atime = atime;
        self.ctime = ctime;
        self.mtime = mtime;
        self
    }

    // 构建 inode
    pub fn build(&self, inode_size: u16) -> Result<Vec<u8>> {
        let mut data = vec![0u8; inode_size as usize];

        // 基础字段
        data[0..2].copy_from_slice(&self.mode.to_le_bytes());
        data[2..4].copy_from_slice(&(self.uid as u16).to_le_bytes());
        data[4..8].copy_from_slice(&(self.size as u32).to_le_bytes());
        data[8..12].copy_from_slice(&self.atime.to_le_bytes());
        data[12..16].copy_from_slice(&self.ctime.to_le_bytes());
        data[16..20].copy_from_slice(&self.mtime.to_le_bytes());
        data[20..24].copy_from_slice(&self.dtime.to_le_bytes());
        data[24..26].copy_from_slice(&(self.gid as u16).to_le_bytes());
        data[26..28].copy_from_slice(&self.links_count.to_le_bytes());
        data[28..32].copy_from_slice(&self.blocks.to_le_bytes());
        data[32..36].copy_from_slice(&self.flags.to_le_bytes());

        // osd1 操作系统相关字段
        data[36..40].copy_from_slice(&0u32.to_le_bytes());

        // i_block (存放 extent 或符号链接目标)
        data[40..100].copy_from_slice(&self.i_block);

        // i_generation 版本号
        data[100..104].copy_from_slice(&0u32.to_le_bytes());

        // i_file_acl_lo ACL 块低 32 位
        data[104..108].copy_from_slice(&0u32.to_le_bytes());

        // i_size_hi 文件大小高 32 位
        data[108..112].copy_from_slice(&((self.size >> 32) as u32).to_le_bytes());

        // osd2 (12 字节)
        data[112..116].copy_from_slice(&0u32.to_le_bytes()); // blocks_high
        data[116..118].copy_from_slice(&0u16.to_le_bytes()); // file_acl_hi
        data[118..120].copy_from_slice(&((self.uid >> 16) as u16).to_le_bytes());
        data[120..122].copy_from_slice(&((self.gid >> 16) as u16).to_le_bytes());
        data[122..124].copy_from_slice(&0u16.to_le_bytes()); // checksum_lo
        data[124..126].copy_from_slice(&0u16.to_le_bytes()); // 保留字段

        // 额外字段 (inode_size 大于 128 时存在)
        if inode_size > 128 {
            // i_extra_isize 额外字段长度
            data[128..130].copy_from_slice(&32u16.to_le_bytes());
            // i_checksum_hi 校验和高位
            data[130..132].copy_from_slice(&0u16.to_le_bytes());
            // i_ctime_extra 变更时间高精度位
            data[132..136].copy_from_slice(&0u32.to_le_bytes());
            // i_mtime_extra 修改时间高精度位
            data[136..140].copy_from_slice(&0u32.to_le_bytes());
            // i_atime_extra 访问时间高精度位
            data[140..144].copy_from_slice(&0u32.to_le_bytes());
            // i_crtime 创建时间
            data[144..148].copy_from_slice(&self.ctime.to_le_bytes());
            // i_crtime_extra 创建时间高精度位
            data[148..152].copy_from_slice(&0u32.to_le_bytes());
            // i_version_hi 版本号高位
            data[152..156].copy_from_slice(&0u32.to_le_bytes());
            // i_projid 项目 ID
            data[156..160].copy_from_slice(&0u32.to_le_bytes());

            // inline xattr (存在时写入)
            if !self.xattrs.is_empty() && inode_size >= 256 {
                let xattr_start = 160;
                let xattr_size = (inode_size as usize) - xattr_start;

                let mut xattr_builder = InlineXattrBuilder::new();
                for xattr in &self.xattrs {
                    xattr_builder.add_entry(xattr.clone());
                }

                let xattr_data = xattr_builder.build(xattr_size)?;
                data[xattr_start..xattr_start + xattr_data.len()].copy_from_slice(&xattr_data);
            }
        }

        Ok(data)
    }
}

impl Default for InodeBuilder {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_inode_builder_dir() {
        let builder = InodeBuilder::new_dir(0o755, 0, 0);
        let data = builder.build(256).unwrap();

        assert_eq!(data.len(), 256);

        // 校验 mode
        let mode = u16::from_le_bytes([data[0], data[1]]);
        assert_eq!(mode & inode_mode::S_IFMT, inode_mode::S_IFDIR);
    }

    #[test]
    fn test_inode_builder_file() {
        let builder = InodeBuilder::new_file(0o644, 1000, 1000);
        let data = builder.build(256).unwrap();

        let mode = u16::from_le_bytes([data[0], data[1]]);
        assert_eq!(mode & inode_mode::S_IFMT, inode_mode::S_IFREG);
    }
}
