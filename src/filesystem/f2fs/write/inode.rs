// F2FS inode 构建器
use crate::filesystem::f2fs::consts::*;
//
// 负责构建 F2FS inode 块.

use crate::filesystem::f2fs::Result;
use crate::filesystem::f2fs::types::*;

// inode 块中的地址数量
const ADDRS_PER_INODE: usize = DEF_ADDRS_PER_INODE;

// direct node 块中的地址数量
const ADDRS_PER_BLOCK: usize = DEF_ADDRS_PER_BLOCK;

// indirect node 块中的 nid 数量
const NIDS_PER_BLOCK: usize = (F2FS_BLKSIZE - NODE_FOOTER_SIZE) / 4;

// inode 额外属性区大小
const EXTRA_ISIZE: u16 = 36;

// 默认 inline xattr 大小 (单位: 4 字节)
const DEFAULT_INLINE_XATTR_SIZE: u16 = 50;

// inline xattr 条目
#[derive(Debug, Clone)]
pub struct InlineXattrEntry {
    pub name_index: u8,
    pub name: Vec<u8>,
    pub value: Vec<u8>,
}

impl InlineXattrEntry {
    // 创建 SELinux 上下文 xattr
    pub fn selinux(context: &str) -> Self {
        InlineXattrEntry {
            name_index: F2FS_XATTR_INDEX_SECURITY,
            name: b"selinux".to_vec(),
            value: context.as_bytes().to_vec(),
        }
    }

    // 计算序列化后的大小 (按 4 字节对齐)
    pub fn size(&self) -> usize {
        let raw_size = 4 + self.name.len() + self.value.len();
        (raw_size + 3) & !3
    }

    // 序列化为字节
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut buf = Vec::with_capacity(self.size());
        buf.push(self.name_index);
        buf.push(self.name.len() as u8);
        buf.extend_from_slice(&(self.value.len() as u16).to_le_bytes());
        buf.extend_from_slice(&self.name);
        buf.extend_from_slice(&self.value);
        // 按 4 字节对齐
        while buf.len() % 4 != 0 {
            buf.push(0);
        }
        buf
    }
}

// inode 构建器
#[derive(Debug)]
pub struct InodeBuilder {
    // 基础属性
    mode: u16,
    uid: u32,
    gid: u32,
    links: u32,
    size: u64,
    blocks: u64,

    // 时间戳
    atime: u64,
    atime_nsec: u32,
    ctime: u64,
    ctime_nsec: u32,
    mtime: u64,
    mtime_nsec: u32,
    crtime: u64,
    crtime_nsec: u32,

    // 目录深度 (仅目录使用)
    current_depth: u32,

    // 父 inode 号
    pino: u32,

    // 文件名
    name: Vec<u8>,

    // 目录层级
    dir_level: u8,

    // 标志位
    flags: u32,
    inline_flags: u8,

    // xattr 的 nid
    xattr_nid: u32,

    // 数据块地址
    addrs: Vec<u32>,

    // 间接 node 的 nid
    nids: [u32; 5],

    // 是否启用额外属性
    has_extra_attr: bool,

    // 项目 ID
    projid: u32,

    // inline xattr 条目
    inline_xattrs: Vec<InlineXattrEntry>,

    // 符号链接目标 (inline 数据)
    symlink_target: Option<Vec<u8>>,
}

impl InodeBuilder {
    // 创建新的 inode 构建器
    pub fn new() -> Self {
        InodeBuilder {
            mode: 0,
            uid: 0,
            gid: 0,
            links: 1,
            size: 0,
            blocks: 0,
            atime: 0,
            atime_nsec: 0,
            ctime: 0,
            ctime_nsec: 0,
            mtime: 0,
            mtime_nsec: 0,
            crtime: 0,
            crtime_nsec: 0,
            current_depth: 0,
            pino: 0,
            name: Vec::new(),
            dir_level: 0,
            flags: 0,
            inline_flags: 0,
            xattr_nid: 0,
            addrs: Vec::new(),
            nids: [0; 5],
            has_extra_attr: true,
            projid: 0,
            inline_xattrs: Vec::new(),
            symlink_target: None,
        }
    }

    // 创建目录 inode
    pub fn new_dir(mode: u16, uid: u32, gid: u32) -> Self {
        let mut builder = Self::new();
        builder.mode = S_IFDIR | (mode & 0o7777);
        builder.uid = uid;
        builder.gid = gid;
        builder.links = 2; // . 与 ..
        builder.inline_flags = 0; // 不设置 inline 标志, 交由调用方决定
        builder.has_extra_attr = false;
        builder
    }

    // 创建普通文件 inode
    pub fn new_file(mode: u16, uid: u32, gid: u32) -> Self {
        let mut builder = Self::new();
        builder.mode = S_IFREG | (mode & 0o7777);
        builder.uid = uid;
        builder.gid = gid;
        builder.inline_flags = 0;
        builder.has_extra_attr = false;
        builder
    }

    // 创建符号链接 inode
    pub fn new_symlink(uid: u32, gid: u32) -> Self {
        let mut builder = Self::new();
        builder.mode = S_IFLNK | 0o777;
        builder.uid = uid;
        builder.gid = gid;
        // 符号链接使用 inline 数据存放目标路径
        // F2FS_INLINE_DATA: 表示使用 inline 数据
        // F2FS_DATA_EXIST: 表示 inline 数据区存在实际数据
        builder.inline_flags = F2FS_INLINE_DATA | F2FS_DATA_EXIST;
        builder.has_extra_attr = false;
        // inode 自身占用 1 个块存放 inline 数据
        builder.blocks = 1;
        builder
    }

    // 启用 extra_attr 特性
    pub fn with_extra_attr(mut self) -> Self {
        self.has_extra_attr = true;
        self.inline_flags |= F2FS_EXTRA_ATTR;
        self
    }

    // 启用 inline_xattr
    pub fn with_inline_xattr(mut self) -> Self {
        self.inline_flags |= F2FS_INLINE_XATTR;
        self
    }

    // 设置符号链接目标
    pub fn with_symlink_target(mut self, target: &str) -> Self {
        let target_bytes = target.as_bytes().to_vec();
        self.size = target_bytes.len() as u64;
        self.symlink_target = Some(target_bytes);
        self.inline_flags |= F2FS_INLINE_DATA;
        self
    }

    // 设置 mode
    pub fn with_mode(mut self, mode: u16) -> Self {
        self.mode = mode;
        self
    }

    // 设置 UID/GID
    pub fn with_owner(mut self, uid: u32, gid: u32) -> Self {
        self.uid = uid;
        self.gid = gid;
        self
    }

    // 设置链接数
    pub fn with_links(mut self, links: u32) -> Self {
        self.links = links;
        self
    }

    // 设置文件大小
    pub fn with_size(mut self, size: u64) -> Self {
        self.size = size;
        self
    }

    // 设置块数量
    pub fn with_blocks(mut self, blocks: u64) -> Self {
        self.blocks = blocks;
        self
    }

    // 设置时间戳
    pub fn with_timestamp(mut self, time: u64) -> Self {
        self.atime = time;
        self.ctime = time;
        self.mtime = time;
        self.crtime = time;
        self
    }

    // 设置详细时间戳
    pub fn with_times(mut self, atime: u64, ctime: u64, mtime: u64, crtime: u64) -> Self {
        self.atime = atime;
        self.ctime = ctime;
        self.mtime = mtime;
        self.crtime = crtime;
        self
    }

    // 设置父 inode 号
    pub fn with_pino(mut self, pino: u32) -> Self {
        self.pino = pino;
        self
    }

    // 设置文件名
    pub fn with_name(mut self, name: &[u8]) -> Self {
        self.name = name.to_vec();
        self
    }

    // 设置目录深度
    pub fn with_depth(mut self, depth: u32) -> Self {
        self.current_depth = depth;
        self
    }

    // 设置标志位
    pub fn with_flags(mut self, flags: u32) -> Self {
        self.flags = flags;
        self
    }

    // 设置 inline 标志位
    pub fn with_inline_flags(mut self, flags: u8) -> Self {
        self.inline_flags = flags;
        self
    }

    // 添加数据块地址
    pub fn add_addr(&mut self, addr: u32) {
        self.addrs.push(addr);
    }

    // 设置数据块地址列表
    pub fn with_addrs(mut self, addrs: Vec<u32>) -> Self {
        self.addrs = addrs;
        self
    }

    // 设置间接 node 的 nid
    pub fn set_nid(&mut self, idx: usize, nid: u32) {
        if idx < 5 {
            self.nids[idx] = nid;
        }
    }

    // 设置全部间接 node 的 nid
    pub fn with_nids(mut self, nids: [u32; 5]) -> Self {
        self.nids = nids;
        self
    }

    // 设置 xattr nid
    pub fn with_xattr_nid(mut self, nid: u32) -> Self {
        self.xattr_nid = nid;
        self
    }

    // 设置项目 ID
    pub fn with_projid(mut self, projid: u32) -> Self {
        self.projid = projid;
        self
    }

    // 设置 SELinux 上下文
    pub fn with_selinux_context(mut self, context: &str) -> Self {
        // Inline xattr 依赖 extra_attr 区域, 缺失时不会真正写入 xattr 数据
        self.has_extra_attr = true;
        self.inline_flags |= F2FS_EXTRA_ATTR;
        self.inline_xattrs.push(InlineXattrEntry::selinux(context));
        self.inline_flags |= F2FS_INLINE_XATTR;
        self
    }

    // 添加 inline xattr
    pub fn add_inline_xattr(&mut self, entry: InlineXattrEntry) {
        self.inline_xattrs.push(entry);
        self.inline_flags |= F2FS_INLINE_XATTR;
    }

    // 获取文件类型
    pub fn file_type(&self) -> FileType {
        FileType::from(self.mode)
    }

    // 计算实际可用的地址数量
    fn addrs_per_inode(&self) -> usize {
        if self.has_extra_attr {
            ADDRS_PER_INODE - (EXTRA_ISIZE as usize / 4) - DEFAULT_INLINE_XATTR_SIZE as usize
        } else {
            ADDRS_PER_INODE
        }
    }

    // 构建 inode node 块
    pub fn build(&self, nid: u32, ino: u32, cp_ver: u64) -> Result<[u8; F2FS_BLKSIZE]> {
        let mut buf = [0u8; F2FS_BLKSIZE];

        // i_mode (偏移 0)
        buf[0..2].copy_from_slice(&self.mode.to_le_bytes());

        // i_advise (偏移 2)
        buf[2] = 0;

        // i_inline (偏移 3)
        buf[3] = self.inline_flags;

        // i_uid (偏移 4)
        buf[4..8].copy_from_slice(&self.uid.to_le_bytes());

        // i_gid (偏移 8)
        buf[8..12].copy_from_slice(&self.gid.to_le_bytes());

        // i_links (偏移 12)
        buf[12..16].copy_from_slice(&self.links.to_le_bytes());

        // i_size (偏移 16)
        buf[16..24].copy_from_slice(&self.size.to_le_bytes());

        // i_blocks (偏移 24)
        buf[24..32].copy_from_slice(&self.blocks.to_le_bytes());

        // i_atime (偏移 32)
        buf[32..40].copy_from_slice(&self.atime.to_le_bytes());

        // i_ctime (偏移 40)
        buf[40..48].copy_from_slice(&self.ctime.to_le_bytes());

        // i_mtime (偏移 48)
        buf[48..56].copy_from_slice(&self.mtime.to_le_bytes());

        // i_atime_nsec (偏移 56)
        buf[56..60].copy_from_slice(&self.atime_nsec.to_le_bytes());

        // i_ctime_nsec (偏移 60)
        buf[60..64].copy_from_slice(&self.ctime_nsec.to_le_bytes());

        // i_mtime_nsec (偏移 64)
        buf[64..68].copy_from_slice(&self.mtime_nsec.to_le_bytes());

        // i_generation (偏移 68)
        buf[68..72].copy_from_slice(&0u32.to_le_bytes());

        // i_current_depth (偏移 72)
        buf[72..76].copy_from_slice(&self.current_depth.to_le_bytes());

        // i_xattr_nid (偏移 76)
        buf[76..80].copy_from_slice(&self.xattr_nid.to_le_bytes());

        // i_flags (偏移 80)
        buf[80..84].copy_from_slice(&self.flags.to_le_bytes());

        // i_pino (偏移 84)
        buf[84..88].copy_from_slice(&self.pino.to_le_bytes());

        // i_namelen (偏移 88)
        let namelen = self.name.len().min(F2FS_NAME_LEN) as u32;
        buf[88..92].copy_from_slice(&namelen.to_le_bytes());

        // i_name (偏移 92, 255 字节)
        let name_end = 92 + namelen as usize;
        buf[92..name_end].copy_from_slice(&self.name[..namelen as usize]);

        // i_dir_level (偏移 347)
        buf[347] = self.dir_level;

        // i_ext (偏移 348, 12 字节) - extent 缓存, 初始化为 0
        // 保持全零

        // 额外属性区域 (偏移 360)
        if self.has_extra_attr {
            // i_extra_isize (偏移 360)
            buf[360..362].copy_from_slice(&EXTRA_ISIZE.to_le_bytes());

            // i_inline_xattr_size (偏移 362)
            buf[362..364].copy_from_slice(&DEFAULT_INLINE_XATTR_SIZE.to_le_bytes());

            // i_projid (偏移 364)
            buf[364..368].copy_from_slice(&self.projid.to_le_bytes());

            // i_inode_checksum (偏移 368) - 稍后计算
            buf[368..372].copy_from_slice(&0u32.to_le_bytes());

            // i_crtime (偏移 372)
            buf[372..380].copy_from_slice(&self.crtime.to_le_bytes());

            // i_crtime_nsec (偏移 380)
            buf[380..384].copy_from_slice(&self.crtime_nsec.to_le_bytes());

            // i_compr_blocks (偏移 384)
            buf[384..392].copy_from_slice(&0u64.to_le_bytes());

            // i_compress_algorithm (偏移 392)
            buf[392] = 0;

            // i_log_cluster_size (偏移 393)
            buf[393] = 0;

            // i_compress_flag (偏移 394)
            buf[394..396].copy_from_slice(&0u16.to_le_bytes());
        }

        // 数据块地址 (有额外属性时为偏移 396, 否则为 360)
        // 或 inline 数据 (符号链接目标)
        let addr_offset = if self.has_extra_attr { 396 } else { 360 };

        if let Some(ref target) = self.symlink_target {
            // 将符号链接目标作为 inline 数据写入
            // F2FS inline 数据结构:
            // - i_addr[0..extra_isize] 为额外属性区域 (存在 F2FS_EXTRA_ATTR 时)
            // - i_addr[extra_isize] 为保留槽位 (DEF_INLINE_RESERVED_SIZE = 1), 必须为 0
            // - i_addr[extra_isize + 1] 起为实际的 inline 数据
            //
            // addr_offset 已是 i_addr[extra_isize] 的位置 (已跳过额外属性)
            // 无 extra_attr: addr_offset=360, i_addr[0] = 0 (保留), i_addr[1] 起为数据 (偏移 364)
            // 有 extra_attr: addr_offset=396, i_addr[9] = 0 (保留), i_addr[10] 起为数据 (偏移 400)
            let reserved_offset = addr_offset;
            let inline_data_offset = reserved_offset + 4; // 跳过保留槽位

            // 保留槽位置 0
            buf[reserved_offset..reserved_offset + 4].copy_from_slice(&0u32.to_le_bytes());

            // 写入 inline 数据
            let max_inline_size = F2FS_BLKSIZE - inline_data_offset - NODE_FOOTER_SIZE;
            let write_len = target.len().min(max_inline_size);
            buf[inline_data_offset..inline_data_offset + write_len]
                .copy_from_slice(&target[..write_len]);
        } else {
            // 写入数据块地址
            let max_addrs = self.addrs_per_inode();
            for (i, &addr) in self.addrs.iter().take(max_addrs).enumerate() {
                let offset = addr_offset + i * 4;
                buf[offset..offset + 4].copy_from_slice(&addr.to_le_bytes());
            }
        }

        // 间接 node 的 nid (位于地址数组之后)
        // nids[5] 的位置: 360 + DEF_ADDRS_PER_INODE * 4
        let nid_offset = 360 + ADDRS_PER_INODE * 4;
        for (i, &n) in self.nids.iter().enumerate() {
            let offset = nid_offset + i * 4;
            buf[offset..offset + 4].copy_from_slice(&n.to_le_bytes());
        }

        // 写入 inline xattr (位于 footer 之前)
        // inline xattr 区域从 inode 末尾向前推算
        // 位置: F2FS_BLKSIZE - NODE_FOOTER_SIZE - inline_xattr_size * 4
        if !self.inline_xattrs.is_empty() && self.has_extra_attr {
            let inline_xattr_bytes = DEFAULT_INLINE_XATTR_SIZE as usize * 4; // 200 字节
            let xattr_start = F2FS_BLKSIZE - NODE_FOOTER_SIZE - inline_xattr_bytes;

            // 序列化所有 xattr 条目
            let mut xattr_data = Vec::new();
            // xattr header: magic (4 字节)
            xattr_data.extend_from_slice(&0xF2F52011u32.to_le_bytes());

            for entry in &self.inline_xattrs {
                xattr_data.extend_from_slice(&entry.to_bytes());
            }

            // 追加终止标记 (全零条目)
            xattr_data.extend_from_slice(&[0u8; 4]);

            // 写入 xattr 数据
            let write_len = xattr_data.len().min(inline_xattr_bytes);
            buf[xattr_start..xattr_start + write_len].copy_from_slice(&xattr_data[..write_len]);
        }

        // node 尾部 (最后 24 字节)
        let footer = NodeFooter {
            nid,
            ino,
            flag: 0,
            cp_ver,
            next_blkaddr: 0,
        };
        let footer_bytes = footer.to_bytes();
        buf[F2FS_BLKSIZE - NODE_FOOTER_SIZE..].copy_from_slice(&footer_bytes);

        // 计算并写入 inode 校验和 (启用 extra_attr 时)
        if self.has_extra_attr {
            let checksum = calculate_inode_checksum(ino, &buf);
            buf[368..372].copy_from_slice(&checksum.to_le_bytes());
        }

        Ok(buf)
    }
}

// 计算 inode 校验和
// F2FS 采用 crc32(ino, inode_data) 的方式计算
fn calculate_inode_checksum(ino: u32, inode_data: &[u8]) -> u32 {
    let mut crc = F2FS_MAGIC;

    // 先计算 ino 的 CRC
    for &byte in &ino.to_le_bytes() {
        crc ^= byte as u32;
        for _ in 0..8 {
            if crc & 1 != 0 {
                crc = (crc >> 1) ^ 0xEDB88320;
            } else {
                crc >>= 1;
            }
        }
    }

    // 再计算 inode 数据的 CRC (跳过校验和字段)
    for (i, &byte) in inode_data.iter().enumerate() {
        // 跳过校验和字段 (偏移 368-371)
        if (368..372).contains(&i) {
            continue;
        }
        crc ^= byte as u32;
        for _ in 0..8 {
            if crc & 1 != 0 {
                crc = (crc >> 1) ^ 0xEDB88320;
            } else {
                crc >>= 1;
            }
        }
    }

    crc
}

impl Default for InodeBuilder {
    fn default() -> Self {
        Self::new()
    }
}

// direct node 块构建器
#[derive(Debug)]
pub struct DirectNodeBuilder {
    addrs: Vec<u32>,
}

impl DirectNodeBuilder {
    pub fn new() -> Self {
        DirectNodeBuilder { addrs: Vec::new() }
    }

    pub fn add_addr(&mut self, addr: u32) {
        if self.addrs.len() < ADDRS_PER_BLOCK {
            self.addrs.push(addr);
        }
    }

    pub fn with_addrs(mut self, addrs: Vec<u32>) -> Self {
        self.addrs = addrs;
        self
    }

    pub fn build(&self, nid: u32, ino: u32, cp_ver: u64) -> [u8; F2FS_BLKSIZE] {
        let mut buf = [0u8; F2FS_BLKSIZE];

        // 写入地址
        for (i, &addr) in self.addrs.iter().take(ADDRS_PER_BLOCK).enumerate() {
            let offset = i * 4;
            buf[offset..offset + 4].copy_from_slice(&addr.to_le_bytes());
        }

        // node 尾部
        let footer = NodeFooter {
            nid,
            ino,
            flag: 0,
            cp_ver,
            next_blkaddr: 0,
        };
        let footer_bytes = footer.to_bytes();
        buf[F2FS_BLKSIZE - NODE_FOOTER_SIZE..].copy_from_slice(&footer_bytes);

        buf
    }
}

impl Default for DirectNodeBuilder {
    fn default() -> Self {
        Self::new()
    }
}

// indirect node 块构建器
#[derive(Debug)]
pub struct IndirectNodeBuilder {
    nids: Vec<u32>,
}

impl IndirectNodeBuilder {
    pub fn new() -> Self {
        IndirectNodeBuilder { nids: Vec::new() }
    }

    pub fn add_nid(&mut self, nid: u32) {
        if self.nids.len() < NIDS_PER_BLOCK {
            self.nids.push(nid);
        }
    }

    pub fn len(&self) -> usize {
        self.nids.len()
    }

    pub fn is_empty(&self) -> bool {
        self.nids.is_empty()
    }

    pub fn build(&self, nid: u32, ino: u32, cp_ver: u64) -> [u8; F2FS_BLKSIZE] {
        let mut buf = [0u8; F2FS_BLKSIZE];

        // 写入 nid
        for (i, &n) in self.nids.iter().take(NIDS_PER_BLOCK).enumerate() {
            let offset = i * 4;
            buf[offset..offset + 4].copy_from_slice(&n.to_le_bytes());
        }

        // node 尾部
        let footer = NodeFooter {
            nid,
            ino,
            flag: 0,
            cp_ver,
            next_blkaddr: 0,
        };
        let footer_bytes = footer.to_bytes();
        buf[F2FS_BLKSIZE - NODE_FOOTER_SIZE..].copy_from_slice(&footer_bytes);

        buf
    }
}

impl Default for IndirectNodeBuilder {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_inode_builder_new_dir() {
        let builder = InodeBuilder::new_dir(0o755, 1000, 1000);
        assert_eq!(builder.mode, S_IFDIR | 0o755);
        assert_eq!(builder.uid, 1000);
        assert_eq!(builder.gid, 1000);
        assert_eq!(builder.links, 2);
    }

    #[test]
    fn test_inode_builder_new_file() {
        let builder = InodeBuilder::new_file(0o644, 1000, 1000);
        assert_eq!(builder.mode, S_IFREG | 0o644);
        assert_eq!(builder.links, 1);
    }

    #[test]
    fn test_inode_build() {
        let builder = InodeBuilder::new_dir(0o755, 0, 0)
            .with_timestamp(1234567890)
            .with_pino(3)
            .with_name(b"test");

        let data = builder.build(4, 4, 1).unwrap();
        assert_eq!(data.len(), F2FS_BLKSIZE);

        // 校验 mode
        let mode = u16::from_le_bytes([data[0], data[1]]);
        assert_eq!(mode, S_IFDIR | 0o755);

        // 校验 footer
        let footer_offset = F2FS_BLKSIZE - NODE_FOOTER_SIZE;
        let nid = u32::from_le_bytes([
            data[footer_offset],
            data[footer_offset + 1],
            data[footer_offset + 2],
            data[footer_offset + 3],
        ]);
        assert_eq!(nid, 4);
    }

    #[test]
    fn test_direct_node_builder() {
        let mut builder = DirectNodeBuilder::new();
        builder.add_addr(100);
        builder.add_addr(101);

        let data = builder.build(5, 4, 1);
        assert_eq!(data.len(), F2FS_BLKSIZE);

        // 校验第一个地址
        let addr = u32::from_le_bytes([data[0], data[1], data[2], data[3]]);
        assert_eq!(addr, 100);
    }
}
