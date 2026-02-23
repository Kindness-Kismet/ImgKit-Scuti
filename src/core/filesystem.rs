// file system abstraction layer
//
// Define a unified interface for file systems, such as erofs, f2fs, ext4, etc.

use std::io::{Read, Seek};
use std::path::{Path, PathBuf};

// File system error type
pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

// File type
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileType {
    RegularFile,
    Directory,
    SymbolicLink,
    BlockDevice,
    CharDevice,
    Fifo,
    Socket,
}

// File metadata
#[derive(Debug, Clone)]
pub struct FileMetadata {
    // file path
    pub path: PathBuf,
    // File type
    pub file_type: FileType,
    // file size
    pub size: u64,
    // permission mode
    pub mode: u32,
    // User ID
    pub uid: u32,
    // Group ID
    pub gid: u32,
    // Modification time (Unix timestamp)
    pub mtime: i64,
    // Symbolic link target (valid for symbolic links only)
    pub symlink_target: Option<String>,
}

// File system metadata
#[derive(Debug, Clone)]
pub struct FilesystemMetadata {
    // File system type (such as "erofs", "f2fs", "ext4")
    pub fs_type: String,
    // File system version
    pub version: String,
    // block size
    pub block_size: u32,
    // total number of blocks
    pub total_blocks: u64,
    // Total number of inodes
    pub total_inodes: u64,
    // volume label
    pub label: Option<String>,
}

// filesystem traits
//
// Define a unified interface for file systems
pub trait Filesystem {
    // Open file system
    fn open<R: Read + Seek + 'static>(reader: R) -> Result<Self>
    where
        Self: Sized;

    // Get file system metadata
    fn metadata(&self) -> &FilesystemMetadata;

    // List all files in the specified directory
    fn list_dir(&mut self, path: &Path) -> Result<Vec<FileMetadata>>;

    // Read file contents
    fn read_file(&mut self, path: &Path) -> Result<Vec<u8>>;

    // Check if the file exists
    fn exists(&mut self, path: &Path) -> bool;

    // Get file metadata
    fn get_metadata(&mut self, path: &Path) -> Result<FileMetadata>;

    // Extract the entire file system to a specified directory
    fn extract_all(&mut self, output_dir: &Path) -> Result<()>;

    // Extract the specified file to the output path
    fn extract_file(&mut self, file_path: &Path, output_path: &Path) -> Result<()>;
}
