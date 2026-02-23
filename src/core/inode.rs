// Inode abstraction layer
//
// Define a unified interface for file system inodes

use std::path::PathBuf;

// Inode error type
pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

// file type enum
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileType {
    // Ordinary document
    RegularFile,
    // Table of contents
    Directory,
    // symbolic link
    Symlink,
    // character device
    CharDevice,
    // block device
    BlockDevice,
    // named pipe
    Fifo,
    // Unix socket
    Socket,
}

// Inode permissions and properties
#[derive(Debug, Clone, Copy)]
pub struct InodeAttr {
    // File mode (permission bits)
    pub mode: u16,
    // User ID
    pub uid: u32,
    // Group ID
    pub gid: u32,
    // Modification time (Unix timestamp)
    pub mtime: u64,
    // access time
    pub atime: u64,
    // creation time
    pub ctime: u64,
}

// Unified interface for Inode
pub trait Inode {
    // Inode number
    fn ino(&self) -> u64;

    // File type
    fn file_type(&self) -> FileType;

    // File size (bytes)
    fn size(&self) -> u64;

    // Get properties
    fn attr(&self) -> InodeAttr;

    // Is it a directory?
    fn is_dir(&self) -> bool {
        self.file_type() == FileType::Directory
    }

    // Is it an ordinary file?
    fn is_file(&self) -> bool {
        self.file_type() == FileType::RegularFile
    }

    // Whether it is a symbolic link
    fn is_symlink(&self) -> bool {
        self.file_type() == FileType::Symlink
    }

    // Get the number of hard links
    fn nlink(&self) -> u32 {
        1
    }
}

// Readable Inode extension interface
pub trait ReadableInode: Inode {
    // Read file data
    fn read_data(&self) -> Result<Vec<u8>>;

    // Read symbolic link target
    fn read_link(&self) -> Result<PathBuf> {
        if !self.is_symlink() {
            return Err("不是符号链接".into());
        }
        Err("未实现".into())
    }
}

// Extension interface for writable Inode
pub trait WritableInode: Inode {
    // Write file data
    fn write_data(&mut self, data: &[u8]) -> Result<()>;

    // Set properties
    fn set_attr(&mut self, attr: InodeAttr) -> Result<()>;

    // Truncate file to specified size
    fn truncate(&mut self, size: u64) -> Result<()>;
}
