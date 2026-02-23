// LZ4 decompression implementation

use super::{CompressionError, Compressor, Decompressor, Result};
use std::os::raw::{c_char, c_int};

// FFI binding: LZ4_compress_destSize
// This function is not exported in lz4-sys and needs to be manually bound.
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

// LZ4 standard decompressor
pub struct Lz4Decompressor;

impl Decompressor for Lz4Decompressor {
    fn decompress(&self, compressed: &[u8], decompressed_size: usize) -> Result<Vec<u8>> {
        // Try using lz4 official library
        if let Ok(decompressed) = lz4::block::decompress(compressed, Some(decompressed_size as i32))
        {
            return Ok(decompressed);
        }

        // Fallback to lz4_flex
        lz4_flex::decompress(compressed, decompressed_size)
            .map_err(|e| CompressionError::new(format!("LZ4 解压缩失败: {}", e)))
    }

    fn name(&self) -> &'static str {
        "LZ4"
    }
}

// LZ4HC decompressor (the same decompression process as LZ4, but the compression algorithm is different)
pub struct Lz4HcDecompressor;

impl Decompressor for Lz4HcDecompressor {
    fn decompress(&self, compressed: &[u8], decompressed_size: usize) -> Result<Vec<u8>> {
        // LZ4HC decompression is the same as LZ4
        Lz4Decompressor.decompress(compressed, decompressed_size)
    }

    fn name(&self) -> &'static str {
        "LZ4HC"
    }
}

// LZ4 decompressor supporting ZERO_PADDING feature (for EROFS)
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

        // Try lz4 official library
        if let Ok(decompressed) =
            lz4::block::decompress(&compressed[start..], Some(decompressed_size as i32))
        {
            return Ok(decompressed);
        }

        // Fallback to lz4_flex
        lz4_flex::decompress(&compressed[start..], decompressed_size)
            .map_err(|e| CompressionError::new(format!("LZ4 解压缩失败: {}", e)))
    }

    fn name(&self) -> &'static str {
        "LZ4 (with ZERO_PADDING support)"
    }
}

// LZ4 compressor
pub struct Lz4Compressor;

impl Compressor for Lz4Compressor {
    fn compress(&self, data: &[u8]) -> Result<Vec<u8>> {
        lz4::block::compress(data, None, false)
            .map_err(|e| CompressionError::new(format!("LZ4 压缩失败: {}", e)))
    }

    fn compress_destsize(&self, data: &[u8], max_output_size: usize) -> Option<(Vec<u8>, usize)> {
        if data.is_empty() || max_output_size == 0 {
            return None;
        }

        // Use native LZ4_compress_destSize FFI
        let mut src_size = data.len() as c_int;
        let mut dst = vec![0u8; max_output_size];

        let compressed_size = unsafe {
            LZ4_compress_destSize(
                data.as_ptr() as *const c_char,
                dst.as_mut_ptr() as *mut c_char,
                &mut src_size,
                max_output_size as c_int,
                1, // acceleration = 1 (default)
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

// LZ4HC compressor (high compression ratio)
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
        .map_err(|e| CompressionError::new(format!("LZ4HC 压缩失败: {}", e)))
    }

    fn compress_destsize(&self, data: &[u8], max_output_size: usize) -> Option<(Vec<u8>, usize)> {
        if data.is_empty() || max_output_size == 0 {
            return None;
        }

        // Use native LZ4_compress_HC_destSize FFI
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
