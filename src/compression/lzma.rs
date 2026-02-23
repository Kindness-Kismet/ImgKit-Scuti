// LZMA decompression implementation

use super::{CompressionError, Compressor, Decompressor, Result};
use std::mem::MaybeUninit;

// LZMA decompressor
pub struct LzmaDecompressor;

impl Decompressor for LzmaDecompressor {
    fn decompress(&self, compressed: &[u8], _decompressed_size: usize) -> Result<Vec<u8>> {
        let mut output = Vec::new();

        lzma_rs::lzma_decompress(&mut &compressed[..], &mut output)
            .map_err(|e| CompressionError::new(format!("LZMA 解压缩失败: {}", e)))?;

        Ok(output)
    }

    fn name(&self) -> &'static str {
        "LZMA"
    }
}

// LZMA compressor
pub struct LzmaCompressor {
    pub level: u32,
}

impl LzmaCompressor {
    pub fn new(level: u32) -> Self {
        Self { level }
    }
}

impl Compressor for LzmaCompressor {
    fn compress(&self, data: &[u8]) -> Result<Vec<u8>> {
        let mut output = Vec::new();

        lzma_rs::lzma_compress(&mut &data[..], &mut output)
            .map_err(|e| CompressionError::new(format!("LZMA 压缩失败: {}", e)))?;

        Ok(output)
    }

    fn name(&self) -> &'static str {
        "LZMA"
    }
}

// MicroLZMA decompressor (EROFS-specific format)
//
// MicroLZMA is a stripped-down variant of LZMA, supported natively by liblzma.
// This implementation uses the lzma_microlzma_decoder of liblzma-sys for decompression.
pub struct MicroLzmaDecompressor;

impl Decompressor for MicroLzmaDecompressor {
    fn decompress(&self, compressed: &[u8], decompressed_size: usize) -> Result<Vec<u8>> {
        use crate::filesystem::erofs::Z_EROFS_LZMA_MAX_DICT_SIZE;

        if compressed.is_empty() {
            return Err(CompressionError::new("MicroLZMA 压缩数据为空".to_string()));
        }

        // MicroLZMA decoder using liblzma-sys
        unsafe {
            // initialize lzma_stream
            let mut strm: MaybeUninit<liblzma_sys::lzma_stream> = MaybeUninit::zeroed();
            let strm_ptr = strm.as_mut_ptr();

            // Initialize MicroLZMA decoder
            let ret = liblzma_sys::lzma_microlzma_decoder(
                strm_ptr,
                compressed.len() as u64,
                decompressed_size as u64,
                1, // uncomp_size_is_exact = true
                Z_EROFS_LZMA_MAX_DICT_SIZE,
            );

            if ret != liblzma_sys::lzma_ret_LZMA_OK {
                return Err(CompressionError::new(format!(
                    "lzma_microlzma_decoder 初始化失败: ret={}",
                    ret
                )));
            }

            // Allocate output buffer
            let mut output = vec![0u8; decompressed_size];

            // Set input and output buffers
            (*strm_ptr).next_in = compressed.as_ptr();
            (*strm_ptr).avail_in = compressed.len();
            (*strm_ptr).next_out = output.as_mut_ptr();
            (*strm_ptr).avail_out = decompressed_size;

            // Execute decompression
            let ret = liblzma_sys::lzma_code(strm_ptr, liblzma_sys::lzma_action_LZMA_FINISH);
            let total_out = (*strm_ptr).total_out as usize;

            // clean up
            liblzma_sys::lzma_end(strm_ptr);

            // Check results
            if ret != liblzma_sys::lzma_ret_LZMA_STREAM_END {
                return Err(CompressionError::new(format!(
                    "MicroLZMA 解压失败: ret={}, 压缩数据: {} 字节, 预期大小: {} 字节, 实际输出: {} 字节",
                    ret,
                    compressed.len(),
                    decompressed_size,
                    total_out
                )));
            }

            Ok(output)
        }
    }

    fn name(&self) -> &'static str {
        "MicroLZMA"
    }
}

// MicroLZMA compressor (EROFS proprietary format)
pub struct MicroLzmaCompressor {
    pub level: u32,
}

impl MicroLzmaCompressor {
    pub fn new(level: u32) -> Self {
        Self { level }
    }
}

impl Compressor for MicroLzmaCompressor {
    fn compress(&self, data: &[u8]) -> Result<Vec<u8>> {
        use crate::filesystem::erofs::Z_EROFS_LZMA_MAX_DICT_SIZE;

        // MicroLZMA encoder using liblzma-sys
        unsafe {
            // initialize lzma_stream
            let mut strm: MaybeUninit<liblzma_sys::lzma_stream> = MaybeUninit::zeroed();
            let strm_ptr = strm.as_mut_ptr();

            // Configure LZMA options
            let mut options: MaybeUninit<liblzma_sys::lzma_options_lzma> = MaybeUninit::zeroed();
            let options_ptr = options.as_mut_ptr();

            // Use preset level initialization options
            let preset = self.level.min(9);
            let ret = liblzma_sys::lzma_lzma_preset(options_ptr, preset);
            if ret != 0 {
                return Err(CompressionError::new(format!(
                    "lzma_lzma_preset 失败: preset={}",
                    preset
                )));
            }

            // Set the dictionary size used by EROFS
            (*options_ptr).dict_size = Z_EROFS_LZMA_MAX_DICT_SIZE;

            // Initialize the MicroLZMA encoder
            let ret = liblzma_sys::lzma_microlzma_encoder(strm_ptr, options_ptr);
            if ret != liblzma_sys::lzma_ret_LZMA_OK {
                return Err(CompressionError::new(format!(
                    "lzma_microlzma_encoder 初始化失败: ret={}",
                    ret
                )));
            }

            // Allocate output buffer
            let out_size = data.len() + data.len() / 8 + 256;
            let mut output = vec![0u8; out_size];

            // Set input and output buffers
            (*strm_ptr).next_in = data.as_ptr();
            (*strm_ptr).avail_in = data.len();
            (*strm_ptr).next_out = output.as_mut_ptr();
            (*strm_ptr).avail_out = out_size;

            // Perform compression
            let ret = liblzma_sys::lzma_code(strm_ptr, liblzma_sys::lzma_action_LZMA_FINISH);
            let total_out = (*strm_ptr).total_out as usize;

            // clean up
            liblzma_sys::lzma_end(strm_ptr);

            // Check results
            if ret != liblzma_sys::lzma_ret_LZMA_STREAM_END {
                return Err(CompressionError::new(format!(
                    "MicroLZMA 压缩失败: ret={}, total_out={}",
                    ret, total_out
                )));
            }

            // Truncate output to actual size
            output.truncate(total_out);

            Ok(output)
        }
    }

    fn compress_destsize(&self, data: &[u8], max_output_size: usize) -> Option<(Vec<u8>, usize)> {
        use crate::filesystem::erofs::Z_EROFS_LZMA_MAX_DICT_SIZE;

        if data.is_empty() || max_output_size == 0 {
            return None;
        }

        // MicroLZMA encoder natively supports destsize mode
        // When the output buffer is full it automatically stops and returns the consumed input size
        unsafe {
            let mut strm: MaybeUninit<liblzma_sys::lzma_stream> = MaybeUninit::zeroed();
            let strm_ptr = strm.as_mut_ptr();

            let mut options: MaybeUninit<liblzma_sys::lzma_options_lzma> = MaybeUninit::zeroed();
            let options_ptr = options.as_mut_ptr();

            let preset = self.level.min(9);
            if liblzma_sys::lzma_lzma_preset(options_ptr, preset) != 0 {
                return None;
            }

            (*options_ptr).dict_size = Z_EROFS_LZMA_MAX_DICT_SIZE;

            if liblzma_sys::lzma_microlzma_encoder(strm_ptr, options_ptr)
                != liblzma_sys::lzma_ret_LZMA_OK
            {
                return None;
            }

            let mut output = vec![0u8; max_output_size];

            (*strm_ptr).next_in = data.as_ptr();
            (*strm_ptr).avail_in = data.len();
            (*strm_ptr).next_out = output.as_mut_ptr();
            (*strm_ptr).avail_out = max_output_size;

            let ret = liblzma_sys::lzma_code(strm_ptr, liblzma_sys::lzma_action_LZMA_FINISH);
            let total_in = (*strm_ptr).total_in as usize;
            let total_out = (*strm_ptr).total_out as usize;

            liblzma_sys::lzma_end(strm_ptr);

            // LZMA_STREAM_END indicates successful completion
            if ret == liblzma_sys::lzma_ret_LZMA_STREAM_END && total_in > 0 && total_out > 0 {
                output.truncate(total_out);
                Some((output, total_in))
            } else {
                None
            }
        }
    }

    fn name(&self) -> &'static str {
        "MicroLZMA"
    }
}
