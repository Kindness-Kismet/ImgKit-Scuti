// F2FS 压缩算法支持

use crate::compression::{Algorithm, Decompressor as CommonDecompressor};
use crate::filesystem::f2fs::{F2fsError, Result};

// 压缩算法解压器 trait (保持 F2FS 接口兼容)
pub trait Decompressor {
    fn decompress(&self, compressed: &[u8], decompressed_size: usize) -> Result<Vec<u8>>;
}

// 通用解压器适配器: 将通用 trait 适配到 F2FS 错误类型
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

// 根据算法 ID 获取解压器
pub fn get_decompressor(algorithm: u8) -> Option<Box<dyn Decompressor>> {
    let common_algo = Algorithm::from_f2fs_id(algorithm)?;
    Some(Box::new(DecompressorAdapter {
        inner: common_algo.decompressor(),
    }))
}
