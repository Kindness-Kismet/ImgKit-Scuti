// zstd 解压缩实现

use super::{CompressionError, Compressor, Decompressor, Result};
use std::io::Read;

// zstd 解压器
pub struct ZstdDecompressor;

impl Decompressor for ZstdDecompressor {
    fn decompress(&self, compressed: &[u8], decompressed_size: usize) -> Result<Vec<u8>> {
        use ruzstd::decoding::StreamingDecoder;

        let mut decoder = StreamingDecoder::new(compressed)
            .map_err(|e| CompressionError::new(format!("ZSTD decoder init failed: {}", e)))?;

        let mut output = Vec::with_capacity(decompressed_size);
        decoder
            .read_to_end(&mut output)
            .map_err(|e| CompressionError::new(format!("ZSTD decompression failed: {}", e)))?;

        Ok(output)
    }

    fn name(&self) -> &'static str {
        "ZSTD"
    }
}

// zstd 压缩器
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
        // 使用 bulk::Compressor 并开启 include_contentsize
        // EROFS 的 zstd 解压要求 frame header 中携带 content size 信息
        let mut compressor = zstd::bulk::Compressor::new(self.level)
            .map_err(|e| CompressionError::new(format!("ZSTD compressor init failed: {}", e)))?;

        // 开启 content size 记录, EROFS 解压时必须依赖该字段
        compressor
            .include_contentsize(true)
            .map_err(|e| CompressionError::new(format!("ZSTD set contentsize failed: {}", e)))?;

        compressor
            .compress(data)
            .map_err(|e| CompressionError::new(format!("ZSTD compression failed: {}", e)))
    }

    fn compress_destsize(&self, data: &[u8], max_output_size: usize) -> Option<(Vec<u8>, usize)> {
        if data.is_empty() || max_output_size == 0 {
            return None;
        }

        // 二分查找加启发式估算 (参考 erofs-utils 的实现)
        let mut l = 0usize; // 可以放得下的最大输入大小
        let mut l_csize = 0usize;
        let mut l_compressed: Vec<u8> = Vec::new();
        let mut r = data.len() + 1; // 放不下的最小输入大小
        let mut m = max_output_size * 4; // 初始猜测值

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
                        // 成功放下
                        l = m;
                        l_csize = csize;
                        l_compressed = compressed;

                        if r <= l + 1 || csize + 1 >= max_output_size {
                            break;
                        }
                        // 依据压缩率估算下一次尝试的大小
                        m = (max_output_size * m) / csize;
                    } else {
                        // 压缩后仍然过大
                        r = m;
                        m = (l + r) / 2;
                    }
                }
                Err(_) => {
                    // 压缩失败
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
