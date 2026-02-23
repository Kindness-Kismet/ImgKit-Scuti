// EROFS error type definitions

use std::io;
use std::path::PathBuf;
use thiserror::Error;

// EROFS operation errors
#[derive(Error, Debug)]
pub enum ErofsError {
    #[error("I/O error: {0}")]
    Io(#[from] io::Error),

    #[error("invalid magic: expected {expected:#x}, found {found:#x}")]
    InvalidMagic { expected: u32, found: u32 },

    #[error("unsupported feature: {0}")]
    UnsupportedFeature(String),

    #[error("inode not found: {0}")]
    InodeNotFound(u64),

    #[error("invalid data layout: {0}")]
    InvalidDataLayout(u16),

    #[error("path not found: {0}")]
    PathNotFound(PathBuf),
}

pub type Result<T> = std::result::Result<T, ErofsError>;
