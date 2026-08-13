// 通用压缩与解压缩模块

pub mod deflate;
pub mod lz4;
pub mod lzma;
pub mod zstd;

use std::error::Error;
use std::fmt;

// 压缩错误类型
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
        write!(f, "compression error: {}", self.message)
    }
}

impl Error for CompressionError {}

pub type Result<T> = std::result::Result<T, CompressionError>;

// 压缩算法解压器 trait
pub trait Decompressor: Send + Sync {
    // 解压数据
    // compressed: 压缩后的数据
    // decompressed_size: 解压后的预期大小 (部分算法必须提供)
    fn decompress(&self, compressed: &[u8], decompressed_size: usize) -> Result<Vec<u8>>;

    // 算法名称
    fn name(&self) -> &'static str;
}

// 压缩算法压缩器 trait
pub trait Compressor: Send + Sync {
    // 压缩数据
    // data: 原始数据
    fn compress(&self, data: &[u8]) -> Result<Vec<u8>>;

    // 按指定输出上限压缩数据 (destsize 模式)
    // data: 原始数据
    // max_output_size: 输出的最大字节数
    // 返回: (压缩后的数据, 实际消耗的输入数据大小)
    //
    // 该方法在保证输出不超过 max_output_size 的前提下, 尽可能多地压缩输入数据
    // 若不支持 destsize 模式, 返回 None
    fn compress_destsize(&self, data: &[u8], max_output_size: usize) -> Option<(Vec<u8>, usize)> {
        // 默认实现: 不支持 destsize 模式
        let _ = (data, max_output_size);
        None
    }

    // 算法名称
    fn name(&self) -> &'static str;
}

// 压缩算法枚举
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
    // 获取算法对应的解压器
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

    // 根据 EROFS 算法 ID 获取算法
    pub fn from_erofs_id(id: u8) -> Option<Self> {
        match id {
            0 => Some(Algorithm::Lz4),
            1 => Some(Algorithm::Lz4Hc),
            2 => Some(Algorithm::Lzma),
            _ => None,
        }
    }

    // 根据 F2FS 算法 ID 获取算法
    pub fn from_f2fs_id(id: u8) -> Option<Self> {
        match id {
            1 => Some(Algorithm::Lz4),
            2 => Some(Algorithm::Zstd),
            _ => None,
        }
    }
}
