// Unified error types for the core layer

use thiserror::Error;

// Core error variants
#[derive(Error, Debug)]
pub enum CoreError {
    // I/O error
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    // Invalid magic number
    #[error("invalid magic: expected {expected:#x}, found {found:#x}")]
    InvalidMagic { expected: u32, found: u32 },

    // Unsupported version
    #[error("unsupported version: {0}")]
    UnsupportedVersion(u32),

    // Invalid data format
    #[error("invalid format: {0}")]
    InvalidFormat(String),

    // Filesystem corruption
    #[error("filesystem corrupted: {0}")]
    Corrupted(String),

    // File or directory not found
    #[error("not found: {0}")]
    NotFound(String),

    // Insufficient permissions
    #[error("permission denied: {0}")]
    PermissionDenied(String),

    // Unsupported operation
    #[error("unsupported operation: {0}")]
    Unsupported(String),

    // Compression error
    #[error("compression error: {0}")]
    CompressionError(String),

    // Decompression error
    #[error("decompression error: {0}")]
    DecompressionError(String),

    // Value out of range
    #[error("out of range: {0}")]
    OutOfRange(String),

    // Insufficient capacity
    #[error("insufficient capacity: required {required} bytes, available {available} bytes")]
    InsufficientCapacity { required: u64, available: u64 },

    // Generic error
    #[error("{0}")]
    Other(String),
}

// Converts String into CoreError::Other
impl From<String> for CoreError {
    fn from(msg: String) -> Self {
        CoreError::Other(msg)
    }
}

// Converts &str into CoreError::Other
impl From<&str> for CoreError {
    fn from(msg: &str) -> Self {
        CoreError::Other(msg.to_string())
    }
}

// Result type alias for core operations
pub type Result<T> = std::result::Result<T, CoreError>;
