// DEFLATE decompression implementation

use super::{CompressionError, Compressor, Decompressor, Result};
use std::io::{Read, Write};

// DEFLATE decompressor
pub struct DeflateDecompressor;

impl Decompressor for DeflateDecompressor {
    fn decompress(&self, compressed: &[u8], decompressed_size: usize) -> Result<Vec<u8>> {
        use flate2::bufread::DeflateDecoder;

        // Use BufReader wrapper to improve performance
        let buf_reader = std::io::BufReader::with_capacity(8192, compressed);
        let mut decoder = DeflateDecoder::new(buf_reader);

        // Pre-allocate a buffer large enough to avoid multiple reallocations
        let mut output = vec![0u8; decompressed_size];
        let mut total_read = 0;

        loop {
            match decoder.read(&mut output[total_read..]) {
                Ok(0) => break, // end of file reached
                Ok(n) => {
                    total_read += n;
                    if total_read >= decompressed_size {
                        break;
                    }
                    // If the buffer is not enough, extend it
                    if total_read == output.len() {
                        output.resize(output.len() * 2, 0);
                    }
                }
                Err(e) => {
                    return Err(CompressionError::new(format!("DEFLATE 解压缩失败: {}", e)));
                }
            }
        }

        output.truncate(total_read);
        Ok(output)
    }

    fn name(&self) -> &'static str {
        "DEFLATE"
    }
}

// DEFLATE compressor
pub struct DeflateCompressor {
    pub level: u32,
}

impl DeflateCompressor {
    pub fn new(level: u32) -> Self {
        Self { level }
    }
}

impl Compressor for DeflateCompressor {
    fn compress(&self, data: &[u8]) -> Result<Vec<u8>> {
        use flate2::Compression;
        use flate2::write::DeflateEncoder;

        let mut encoder = DeflateEncoder::new(Vec::new(), Compression::new(self.level));
        encoder
            .write_all(data)
            .map_err(|e| CompressionError::new(format!("DEFLATE 压缩失败: {}", e)))?;

        encoder
            .finish()
            .map_err(|e| CompressionError::new(format!("DEFLATE 压缩完成失败: {}", e)))
    }

    fn compress_destsize(&self, data: &[u8], max_output_size: usize) -> Option<(Vec<u8>, usize)> {
        if data.is_empty() || max_output_size == 0 {
            return None;
        }

        // Binary search + heuristic estimation (refer to erofs-utils implementation)
        let mut l = 0usize;
        let mut l_csize = 0usize;
        let mut l_compressed: Vec<u8> = Vec::new();
        let mut r = data.len() + 1;
        let mut m = max_output_size * 4;

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
                        l = m;
                        l_csize = csize;
                        l_compressed = compressed;

                        if r <= l + 1 || csize + (22 - 2 * self.level as usize) >= max_output_size {
                            break;
                        }
                        m = (max_output_size * m) / csize;
                    } else {
                        r = m;
                        m = (l + r) / 2;
                    }
                }
                Err(_) => {
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
        "DEFLATE"
    }
}
