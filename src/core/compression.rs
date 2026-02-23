// Compression abstraction layer
//
// A unified interface for defining compression algorithms

// Compression error type
pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

// Compression algorithm type
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CompressionAlgorithm {
    // No compression
    None,
    // LZ4
    Lz4,
    // LZ4 High Compression
    Lz4Hc,
    // LZMA
    Lzma,
    // MicroLZMA
    MicroLzma,
    // ZSTD
    Zstd,
    // DEFLATE
    Deflate,
    // unknown algorithm
    Unknown(u8),
}

impl CompressionAlgorithm {
    // Get algorithm name
    pub fn name(&self) -> &'static str {
        match self {
            CompressionAlgorithm::None => "none",
            CompressionAlgorithm::Lz4 => "lz4",
            CompressionAlgorithm::Lz4Hc => "lz4hc",
            CompressionAlgorithm::Lzma => "lzma",
            CompressionAlgorithm::MicroLzma => "microlzma",
            CompressionAlgorithm::Zstd => "zstd",
            CompressionAlgorithm::Deflate => "deflate",
            CompressionAlgorithm::Unknown(_) => "unknown",
        }
    }

    // Determine whether compression is needed
    pub fn is_compressed(&self) -> bool {
        !matches!(self, CompressionAlgorithm::None)
    }
}

// Compression options
#[derive(Debug, Clone)]
pub struct CompressionOptions {
    // Compression algorithm
    pub algorithm: CompressionAlgorithm,
    // Compression level (1-9, meaning depends on algorithm)
    pub level: u32,
    // Dictionary size (bytes, for LZMA, etc.)
    pub dict_size: Option<u32>,
}

impl Default for CompressionOptions {
    fn default() -> Self {
        Self {
            algorithm: CompressionAlgorithm::None,
            level: 6,
            dict_size: None,
        }
    }
}

// Compressor interface
pub trait Compressor {
    // Compress data
    fn compress(&self, data: &[u8], options: &CompressionOptions) -> Result<Vec<u8>>;

    // Get supported algorithms
    fn supported_algorithms(&self) -> Vec<CompressionAlgorithm>;

    // Check whether the specified algorithm is supported
    fn supports(&self, algorithm: CompressionAlgorithm) -> bool {
        self.supported_algorithms().contains(&algorithm)
    }
}

// decompressor interface
pub trait Decompressor {
    // Decompress data
    //
    // # Parameters
    // - compressed: compressed data
    // - algorithm: compression algorithm
    // - expected_size: expected decompressed size (if known)
    fn decompress(
        &self,
        compressed: &[u8],
        algorithm: CompressionAlgorithm,
        expected_size: Option<usize>,
    ) -> Result<Vec<u8>>;

    // Get supported algorithms
    fn supported_algorithms(&self) -> Vec<CompressionAlgorithm>;

    // Check whether the specified algorithm is supported
    fn supports(&self, algorithm: CompressionAlgorithm) -> bool {
        self.supported_algorithms().contains(&algorithm)
    }
}

// Compression/decompression unified interface
pub trait Codec: Compressor + Decompressor {
    // codec name
    fn name(&self) -> &str;

    // Compress and return the compression ratio
    fn compress_with_ratio(
        &self,
        data: &[u8],
        options: &CompressionOptions,
    ) -> Result<(Vec<u8>, f64)> {
        let compressed = self.compress(data, options)?;
        let ratio = (compressed.len() as f64) / (data.len() as f64);
        Ok((compressed, ratio))
    }
}
