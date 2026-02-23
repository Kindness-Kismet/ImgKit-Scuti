// ZSTD decompression implementation

use super::{CompressionError, Compressor, Decompressor, Result};
use std::io::Read;

// ZSTD decompressor
pub struct ZstdDecompressor;

impl Decompressor for ZstdDecompressor {
    fn decompress(&self, compressed: &[u8], decompressed_size: usize) -> Result<Vec<u8>> {
        use ruzstd::decoding::StreamingDecoder;

        let mut decoder = StreamingDecoder::new(compressed)
            .map_err(|e| CompressionError::new(format!("ZSTD 解压缩初始化失败: {}", e)))?;

        let mut output = Vec::with_capacity(decompressed_size);
        decoder
            .read_to_end(&mut output)
            .map_err(|e| CompressionError::new(format!("ZSTD 解压缩失败: {}", e)))?;

        Ok(output)
    }

    fn name(&self) -> &'static str {
        "ZSTD"
    }
}

// ZSTD compressor
pub struct ZstdCompressor {
    pub level: i32,
}

impl ZstdCompressor {
    pub fn new(level: i32) -> Self {
        Self { level }
    }
}

impl Compressor for ZstdCompressor {
    fn compress(&self, data: &[u8]) -> Result<Vec<u8>> {
        // Use bulk::Compressor and enable include_contentsize
        // EROFS's ZSTD decompression requires content size information in the frame header
        let mut compressor = zstd::bulk::Compressor::new(self.level)
            .map_err(|e| CompressionError::new(format!("ZSTD 压缩器初始化失败: {}", e)))?;

        // Enable content size storage, required for EROFS decompression
        compressor
            .include_contentsize(true)
            .map_err(|e| CompressionError::new(format!("ZSTD 设置 contentsize 失败: {}", e)))?;

        compressor
            .compress(data)
            .map_err(|e| CompressionError::new(format!("ZSTD 压缩失败: {}", e)))
    }

    fn compress_destsize(&self, data: &[u8], max_output_size: usize) -> Option<(Vec<u8>, usize)> {
        if data.is_empty() || max_output_size == 0 {
            return None;
        }

        // Binary search + heuristic estimation (refer to erofs-utils implementation)
        let mut l = 0usize; // Maximum input size that can be placed
        let mut l_csize = 0usize;
        let mut l_compressed: Vec<u8> = Vec::new();
        let mut r = data.len() + 1; // Minimum input size that cannot fit
        let mut m = max_output_size * 4; // initial guess

        loop {
            m = m.max(l + 1);
            m = m.min(r - 1);

            if m <= l || m >= r {
                break;
            }

            match self.compress(&data[..m]) {
                Ok(compressed) => {
                    let csize = compressed.len();
                    if csize > 0 && csize <= max_output_size {
                        // successfully placed
                        l = m;
                        l_csize = csize;
                        l_compressed = compressed;

                        if r <= l + 1 || csize + 1 >= max_output_size {
                            break;
                        }
                        // Estimate next try size based on compression ratio
                        m = (max_output_size * m) / csize;
                    } else {
                        // Too big after compression
                        r = m;
                        m = (l + r) / 2;
                    }
                }
                Err(_) => {
                    // Compression failed
                    r = m;
                    m = (l + r) / 2;
                }
            }
        }

        if l > 0 && l_csize > 0 {
            Some((l_compressed, l))
        } else {
            None
        }
    }

    fn name(&self) -> &'static str {
        "ZSTD"
    }
}
