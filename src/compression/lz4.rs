// LZ4 解压缩实现

use super::{CompressionError, Compressor, Decompressor, Result};
use std::os::raw::{c_char, c_int};

// FFI 绑定: LZ4_compress_destSize
// 该函数未在 lz4-sys 中导出, 需要手动绑定
unsafe extern "C" {
    // int LZ4_compress_destSize(const char* src, char* dst, int* srcSizePtr, int targetDstSize, int acceleration);
    fn LZ4_compress_destSize(
        src: *const c_char,
        dst: *mut c_char,
        src_size_ptr: *mut c_int,
        target_dst_size: c_int,
        acceleration: c_int,
    ) -> c_int;

    // int LZ4_compress_HC_destSize(void* stateHC, const char* src, char* dst, int* srcSizePtr, int targetDstSize, int compressionLevel);
    fn LZ4_compress_HC_destSize(
        state_hc: *mut std::ffi::c_void,
        src: *const c_char,
        dst: *mut c_char,
        src_size_ptr: *mut c_int,
        target_dst_size: c_int,
        compression_level: c_int,
    ) -> c_int;

    // int LZ4_sizeofStateHC(void);
    fn LZ4_sizeofStateHC() -> c_int;
}

// LZ4 标准解压器
pub struct Lz4Decompressor;

impl Decompressor for Lz4Decompressor {
    fn decompress(&self, compressed: &[u8], decompressed_size: usize) -> Result<Vec<u8>> {
        // 优先尝试 lz4 官方库
        if let Ok(decompressed) = lz4::block::decompress(compressed, Some(decompressed_size as i32))
        {
            return Ok(decompressed);
        }

        // 回退到 lz4_flex
        lz4_flex::decompress(compressed, decompressed_size)
            .map_err(|e| CompressionError::new(format!("LZ4 decompression failed: {}", e)))
    }

    fn name(&self) -> &'static str {
        "LZ4"
    }
}

// LZ4HC 解压器 (解压流程与 LZ4 相同, 仅压缩算法不同)
pub struct Lz4HcDecompressor;

impl Decompressor for Lz4HcDecompressor {
    fn decompress(&self, compressed: &[u8], decompressed_size: usize) -> Result<Vec<u8>> {
        // LZ4HC 的解压过程与 LZ4 完全一致
        Lz4Decompressor.decompress(compressed, decompressed_size)
    }

    fn name(&self) -> &'static str {
        "LZ4HC"
    }
}

// 支持 ZERO_PADDING 特性的 LZ4 解压器 (用于 EROFS)
pub struct Lz4ZeroPaddingDecompressor {
    pub skip_zero_padding: bool,
}

impl Lz4ZeroPaddingDecompressor {
    pub fn new(skip_zero_padding: bool) -> Self {
        Self { skip_zero_padding }
    }

    fn find_data_start(&self, data: &[u8]) -> usize {
        if !self.skip_zero_padding {
            return 0;
        }

        let mut start = 0;
        while start < data.len() && data[start] == 0 {
            start += 1;
        }

        if start >= data.len() {
            return 0;
        }

        start
    }
}

impl Decompressor for Lz4ZeroPaddingDecompressor {
    fn decompress(&self, compressed: &[u8], decompressed_size: usize) -> Result<Vec<u8>> {
        let start = self.find_data_start(compressed);

        // 优先尝试 lz4 官方库
        if let Ok(decompressed) =
            lz4::block::decompress(&compressed[start..], Some(decompressed_size as i32))
        {
            return Ok(decompressed);
        }

        // 回退到 lz4_flex
        lz4_flex::decompress(&compressed[start..], decompressed_size)
            .map_err(|e| CompressionError::new(format!("LZ4 decompression failed: {}", e)))
    }

    fn name(&self) -> &'static str {
        "LZ4 (with ZERO_PADDING support)"
    }
}

// LZ4 压缩器
pub struct Lz4Compressor;

impl Compressor for Lz4Compressor {
    fn compress(&self, data: &[u8]) -> Result<Vec<u8>> {
        lz4::block::compress(data, None, false)
            .map_err(|e| CompressionError::new(format!("LZ4 compression failed: {}", e)))
    }

    fn compress_destsize(&self, data: &[u8], max_output_size: usize) -> Option<(Vec<u8>, usize)> {
        if data.is_empty() || max_output_size == 0 {
            return None;
        }

        // 直接使用原生的 LZ4_compress_destSize FFI
        let mut src_size = data.len() as c_int;
        let mut dst = vec![0u8; max_output_size];

        let compressed_size = unsafe {
            LZ4_compress_destSize(
                data.as_ptr() as *const c_char,
                dst.as_mut_ptr() as *mut c_char,
                &mut src_size,
                max_output_size as c_int,
                1, // acceleration = 1 (默认值)
            )
        };

        if compressed_size > 0 && src_size > 0 {
            dst.truncate(compressed_size as usize);
            Some((dst, src_size as usize))
        } else {
            None
        }
    }

    fn name(&self) -> &'static str {
        "LZ4"
    }
}

// LZ4HC 压缩器 (高压缩率)
pub struct Lz4HcCompressor {
    pub level: i32,
}

impl Lz4HcCompressor {
    pub fn new(level: i32) -> Self {
        Self { level }
    }
}

impl Compressor for Lz4HcCompressor {
    fn compress(&self, data: &[u8]) -> Result<Vec<u8>> {
        lz4::block::compress(
            data,
            Some(lz4::block::CompressionMode::HIGHCOMPRESSION(self.level)),
            false,
        )
        .map_err(|e| CompressionError::new(format!("LZ4HC compression failed: {}", e)))
    }

    fn compress_destsize(&self, data: &[u8], max_output_size: usize) -> Option<(Vec<u8>, usize)> {
        if data.is_empty() || max_output_size == 0 {
            return None;
        }

        // 直接使用原生的 LZ4_compress_HC_destSize FFI
        let state_size = unsafe { LZ4_sizeofStateHC() } as usize;
        let mut state = vec![0u8; state_size];

        let mut src_size = data.len() as c_int;
        let mut dst = vec![0u8; max_output_size];

        let compressed_size = unsafe {
            LZ4_compress_HC_destSize(
                state.as_mut_ptr() as *mut std::ffi::c_void,
                data.as_ptr() as *const c_char,
                dst.as_mut_ptr() as *mut c_char,
                &mut src_size,
                max_output_size as c_int,
                self.level,
            )
        };

        if compressed_size > 0 && src_size > 0 {
            dst.truncate(compressed_size as usize);
            Some((dst, src_size as usize))
        } else {
            None
        }
    }

    fn name(&self) -> &'static str {
        "LZ4HC"
    }
}
