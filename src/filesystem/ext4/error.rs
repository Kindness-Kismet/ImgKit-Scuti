// EXT4 error type definitions

use std::path::PathBuf;
use thiserror::Error;

#[derive(Error, Debug)]
#[allow(dead_code)]
pub enum Ext4Error {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("invalid magic: expected {expected}, found {found}")]
    Magic { expected: u16, found: u16 },
    #[error("incompatible filesystem feature: {0}")]
    FeatureIncompat(&'static str),
    #[error("inode not found: {0}")]
    InodeNotFound(u32),
    #[error("path not found: {0}")]
    PathNotFound(PathBuf),
    #[error("not a directory: {0}")]
    NotADirectory(PathBuf),
    #[error("invalid extent header")]
    InvalidExtentHeader,
    #[error("invalid extent")]
    InvalidExtent,
    #[error("invalid inode size: {size} exceeds maximum {max}")]
    InvalidInodeSize { size: u64, max: u64 },
    #[error("extent tree is too deep: depth {depth}")]
    ExtentTreeTooDeep { depth: u8 },
    #[error("detected extent tree cycle at block {block}")]
    ExtentCycleDetected { block: u64 },
}

pub type Result<T> = std::result::Result<T, Ext4Error>;
