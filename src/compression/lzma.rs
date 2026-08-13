// LZMA 解压缩实现

use super::{CompressionError, Compressor, Decompressor, Result};
use std::mem::MaybeUninit;

// LZMA 解压器
pub struct LzmaDecompressor;

impl Decompressor for LzmaDecompressor {
    fn decompress(&self, compressed: &[u8], _decompressed_size: usize) -> Result<Vec<u8>> {
        let mut output = Vec::new();

        lzma_rs::lzma_decompress(&mut &compressed[..], &mut output)
            .map_err(|e| CompressionError::new(format!("LZMA decompression failed: {}", e)))?;

        Ok(output)
    }

    fn name(&self) -> &'static str {
        "LZMA"
    }
}

// LZMA 压缩器
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
            .map_err(|e| CompressionError::new(format!("LZMA compression failed: {}", e)))?;

        Ok(output)
    }

    fn name(&self) -> &'static str {
        "LZMA"
    }
}

// MicroLZMA 解压器 (EROFS 专用格式)
//
// MicroLZMA 是 LZMA 的精简变体, liblzma 原生支持该格式
// 此处使用 liblzma-sys 的 lzma_microlzma_decoder 完成解压
pub struct MicroLzmaDecompressor;

impl Decompressor for MicroLzmaDecompressor {
    fn decompress(&self, compressed: &[u8], decompressed_size: usize) -> Result<Vec<u8>> {
        use crate::filesystem::erofs::Z_EROFS_LZMA_MAX_DICT_SIZE;

        if compressed.is_empty() {
            return Err(CompressionError::new("MicroLZMA data is empty".to_string()));
        }

        // 通过 liblzma-sys 使用 MicroLZMA 解码器
        unsafe {
            // 初始化 lzma_stream
            let mut strm: MaybeUninit<liblzma_sys::lzma_stream> = MaybeUninit::zeroed();
            let strm_ptr = strm.as_mut_ptr();

            // 初始化 MicroLZMA 解码器
            let ret = liblzma_sys::lzma_microlzma_decoder(
                strm_ptr,
                compressed.len() as u64,
                decompressed_size as u64,
                1, // uncomp_size_is_exact = true
                Z_EROFS_LZMA_MAX_DICT_SIZE,
            );

            if ret != liblzma_sys::lzma_ret_LZMA_OK {
                return Err(CompressionError::new(format!(
                    "lzma_microlzma_decoder init failed: ret={}",
                    ret
                )));
            }

            // 分配输出缓冲区
            let mut output = vec![0u8; decompressed_size];

            // 设置输入与输出缓冲区
            (*strm_ptr).next_in = compressed.as_ptr();
            (*strm_ptr).avail_in = compressed.len();
            (*strm_ptr).next_out = output.as_mut_ptr();
            (*strm_ptr).avail_out = decompressed_size;

            // 执行解压
            let ret = liblzma_sys::lzma_code(strm_ptr, liblzma_sys::lzma_action_LZMA_FINISH);
            let total_out = (*strm_ptr).total_out as usize;

            // 清理资源
            liblzma_sys::lzma_end(strm_ptr);

            // 检查结果
            if ret != liblzma_sys::lzma_ret_LZMA_STREAM_END {
                return Err(CompressionError::new(format!(
                    "MicroLZMA decompression failed: ret={}, compressed: {} bytes, expected size: {} bytes, actual output: {} bytes",
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

// MicroLZMA 压缩器 (EROFS 专用格式)
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

        // 通过 liblzma-sys 使用 MicroLZMA 编码器
        unsafe {
            // 初始化 lzma_stream
            let mut strm: MaybeUninit<liblzma_sys::lzma_stream> = MaybeUninit::zeroed();
            let strm_ptr = strm.as_mut_ptr();

            // 配置 LZMA 选项
            let mut options: MaybeUninit<liblzma_sys::lzma_options_lzma> = MaybeUninit::zeroed();
            let options_ptr = options.as_mut_ptr();

            // 使用 preset 压缩等级初始化选项
            let preset = self.level.min(9);
            let ret = liblzma_sys::lzma_lzma_preset(options_ptr, preset);
            if ret != 0 {
                return Err(CompressionError::new(format!(
                    "lzma_lzma_preset failed: preset={}",
                    preset
                )));
            }

            // 设置 EROFS 使用的 dictionary 大小
            (*options_ptr).dict_size = Z_EROFS_LZMA_MAX_DICT_SIZE;

            // 初始化 MicroLZMA 编码器
            let ret = liblzma_sys::lzma_microlzma_encoder(strm_ptr, options_ptr);
            if ret != liblzma_sys::lzma_ret_LZMA_OK {
                return Err(CompressionError::new(format!(
                    "lzma_microlzma_encoder init failed: ret={}",
                    ret
                )));
            }

            // 分配输出缓冲区
            let out_size = data.len() + data.len() / 8 + 256;
            let mut output = vec![0u8; out_size];

            // 设置输入与输出缓冲区
            (*strm_ptr).next_in = data.as_ptr();
            (*strm_ptr).avail_in = data.len();
            (*strm_ptr).next_out = output.as_mut_ptr();
            (*strm_ptr).avail_out = out_size;

            // 执行压缩
            let ret = liblzma_sys::lzma_code(strm_ptr, liblzma_sys::lzma_action_LZMA_FINISH);
            let total_out = (*strm_ptr).total_out as usize;

            // 清理资源
            liblzma_sys::lzma_end(strm_ptr);

            // 检查结果
            if ret != liblzma_sys::lzma_ret_LZMA_STREAM_END {
                return Err(CompressionError::new(format!(
                    "MicroLZMA compression failed: ret={}, total_out={}",
                    ret, total_out
                )));
            }

            // 将输出截断到实际大小
            output.truncate(total_out);

            Ok(output)
        }
    }

    fn compress_destsize(&self, data: &[u8], max_output_size: usize) -> Option<(Vec<u8>, usize)> {
        use crate::filesystem::erofs::Z_EROFS_LZMA_MAX_DICT_SIZE;

        if data.is_empty() || max_output_size == 0 {
            return None;
        }

        // MicroLZMA 编码器原生支持 destsize 模式
        // 输出缓冲区写满后会自动停止, 并返回已消耗的输入大小
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

            // LZMA_STREAM_END 表示压缩正常结束
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
