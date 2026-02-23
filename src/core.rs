// core abstraction layer
//
// Define a unified interface for containers and file systems

pub mod compression;
pub mod container;
pub mod directory;
pub mod error;
pub mod file;
pub mod filesystem;
pub mod inode;
pub mod volume;
pub mod xattr;

// Re-export common types

// Container related
pub use container::{Container, ContainerMetadata, PartitionInfo, ReadSeek};

// File system related
pub use filesystem::{FileMetadata, Filesystem, FilesystemMetadata};

// Inode related
pub use inode::{FileType, Inode, InodeAttr, ReadableInode, WritableInode};

// Directory related
pub use directory::{DirEntry, Directory, WritableDirectory};

// File related
pub use file::{CompressedFile, File, WritableFile};

// Extended attributes related
pub use xattr::{WritableXattr, Xattr, XattrEntry, XattrNamespace};

// Volume related
pub use volume::{ReadableVolume, Volume, WritableVolume};

// Compression related
pub use compression::{Codec, CompressionAlgorithm, CompressionOptions, Compressor, Decompressor};

// Error related
pub use error::{CoreError, Result};
