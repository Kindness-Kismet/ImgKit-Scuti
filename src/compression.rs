// Universal compression and decompression module

pub mod deflate;
pub mod lz4;
pub mod lzma;
pub mod zstd;

use std::error::Error;
use std::fmt;

// Compression error type
#[derive(Debug)]
pub struct CompressionError {
    message: String,
}

impl CompressionError {
    pub fn new(message: String) -> Self {
        Self { message }
    }
}

impl fmt::Display for CompressionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "压缩错误: {}", self.message)
    }
}

impl Error for CompressionError {}

pub type Result<T> = std::result::Result<T, CompressionError>;

// Compression algorithm decompressor trait
pub trait Decompressor: Send + Sync {
    // Decompress data
    // compressed: compressed data
    // decompressed_size: expected size after decompression (required by some algorithms)
    fn decompress(&self, compressed: &[u8], decompressed_size: usize) -> Result<Vec<u8>>;

    // Algorithm name
    fn name(&self) -> &'static str;
}

// Compression algorithm compressor trait
pub trait Compressor: Send + Sync {
    // Compress data
    // data: original data
    fn compress(&self, data: &[u8]) -> Result<Vec<u8>>;

    // Compress data to specified size (destsize mode)
    // data: original data
    // max_output_size: maximum output size
    // Return: (compressed data, actual input data size used)
    //
    // This method will try to compress as much of the input data as possible while ensuring that the output does not exceed max_output_size
    // If destsize mode is not supported, return None
    fn compress_destsize(&self, data: &[u8], max_output_size: usize) -> Option<(Vec<u8>, usize)> {
        // Default implementation: destsize mode is not supported
        let _ = (data, max_output_size);
        None
    }

    // Algorithm name
    fn name(&self) -> &'static str;
}

// Compression algorithm enumeration
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Algorithm {
    Lz4,
    Lz4Hc,
    Lzma,
    MicroLzma,
    Deflate,
    Zstd,
}

impl Algorithm {
    // Get the decompressor corresponding to the algorithm
    pub fn decompressor(&self) -> Box<dyn Decompressor> {
        match self {
            Algorithm::Lz4 => Box::new(lz4::Lz4Decompressor),
            Algorithm::Lz4Hc => Box::new(lz4::Lz4HcDecompressor),
            Algorithm::Lzma => Box::new(lzma::LzmaDecompressor),
            Algorithm::MicroLzma => Box::new(lzma::MicroLzmaDecompressor),
            Algorithm::Deflate => Box::new(deflate::DeflateDecompressor),
            Algorithm::Zstd => Box::new(zstd::ZstdDecompressor),
        }
    }

    // Get algorithm from EROFS algorithm ID
    pub fn from_erofs_id(id: u8) -> Option<Self> {
        match id {
            0 => Some(Algorithm::Lz4),
            1 => Some(Algorithm::Lz4Hc),
            2 => Some(Algorithm::Lzma),
            _ => None,
        }
    }

    // Get algorithm from F2FS algorithm ID
    pub fn from_f2fs_id(id: u8) -> Option<Self> {
        match id {
            1 => Some(Algorithm::Lz4),
            2 => Some(Algorithm::Zstd),
            _ => None,
        }
    }
}
