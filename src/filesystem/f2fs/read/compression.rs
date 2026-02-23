// F2FS compression algorithm support

use crate::compression::{Algorithm, Decompressor as CommonDecompressor};
use crate::filesystem::f2fs::{F2fsError, Result};

// Compression algorithm decompressor trait (maintains F2FS interface compatibility)
pub trait Decompressor {
    fn decompress(&self, compressed: &[u8], decompressed_size: usize) -> Result<Vec<u8>>;
}

// Universal Decompressor Adapter: Adapt universal traits to F2FS error types
struct DecompressorAdapter {
    inner: Box<dyn CommonDecompressor>,
}

impl Decompressor for DecompressorAdapter {
    fn decompress(&self, compressed: &[u8], decompressed_size: usize) -> Result<Vec<u8>> {
        self.inner
            .decompress(compressed, decompressed_size)
            .map_err(|e| F2fsError::Decompression(e.to_string()))
    }
}

// Get decompressor based on algorithm ID
pub fn get_decompressor(algorithm: u8) -> Option<Box<dyn Decompressor>> {
    let common_algo = Algorithm::from_f2fs_id(algorithm)?;
    Some(Box::new(DecompressorAdapter {
        inner: common_algo.decompressor(),
    }))
}
