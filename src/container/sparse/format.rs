// Android Sparse Image format definition
//
// Reference Android source code libsparse

// Sparse file magic number
pub const SPARSE_HEADER_MAGIC: u32 = 0xED26FF3A;

// head size
pub const SPARSE_HEADER_SIZE: usize = 28;

// Chunk head size
pub const CHUNK_HEADER_SIZE: u32 = 12;

// Chunk type
pub const CHUNK_TYPE_RAW: u16 = 0xCAC1;
pub const CHUNK_TYPE_FILL: u16 = 0xCAC2;
pub const CHUNK_TYPE_DONT_CARE: u16 = 0xCAC3;
pub const CHUNK_TYPE_CRC32: u16 = 0xCAC4;

// Sparse file header
#[derive(Debug, Clone)]
pub struct SparseHeader {
    pub magic: u32,
    pub major_version: u16,
    pub minor_version: u16,
    pub file_hdr_sz: u16,
    pub chunk_hdr_sz: u16,
    pub blk_sz: u32,
    pub total_blks: u32,
    pub total_chunks: u32,
    pub image_checksum: u32,
}

impl SparseHeader {
    pub fn new(block_size: u32, total_blocks: u32, total_chunks: u32) -> Self {
        Self {
            magic: SPARSE_HEADER_MAGIC,
            major_version: 1,
            minor_version: 0,
            file_hdr_sz: 28,
            chunk_hdr_sz: 12,
            blk_sz: block_size,
            total_blks: total_blocks,
            total_chunks,
            image_checksum: 0,
        }
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        let mut buf = vec![0u8; 28];
        buf[0..4].copy_from_slice(&self.magic.to_le_bytes());
        buf[4..6].copy_from_slice(&self.major_version.to_le_bytes());
        buf[6..8].copy_from_slice(&self.minor_version.to_le_bytes());
        buf[8..10].copy_from_slice(&self.file_hdr_sz.to_le_bytes());
        buf[10..12].copy_from_slice(&self.chunk_hdr_sz.to_le_bytes());
        buf[12..16].copy_from_slice(&self.blk_sz.to_le_bytes());
        buf[16..20].copy_from_slice(&self.total_blks.to_le_bytes());
        buf[20..24].copy_from_slice(&self.total_chunks.to_le_bytes());
        buf[24..28].copy_from_slice(&self.image_checksum.to_le_bytes());
        buf
    }

    pub fn from_bytes(buf: &[u8]) -> Option<Self> {
        if buf.len() < SPARSE_HEADER_SIZE {
            return None;
        }
        let magic = u32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]]);
        if magic != SPARSE_HEADER_MAGIC {
            return None;
        }
        Some(Self {
            magic,
            major_version: u16::from_le_bytes([buf[4], buf[5]]),
            minor_version: u16::from_le_bytes([buf[6], buf[7]]),
            file_hdr_sz: u16::from_le_bytes([buf[8], buf[9]]),
            chunk_hdr_sz: u16::from_le_bytes([buf[10], buf[11]]),
            blk_sz: u32::from_le_bytes([buf[12], buf[13], buf[14], buf[15]]),
            total_blks: u32::from_le_bytes([buf[16], buf[17], buf[18], buf[19]]),
            total_chunks: u32::from_le_bytes([buf[20], buf[21], buf[22], buf[23]]),
            image_checksum: u32::from_le_bytes([buf[24], buf[25], buf[26], buf[27]]),
        })
    }
}

// Chunk head
#[derive(Debug, Clone)]
pub struct ChunkHeader {
    pub chunk_type: u16,
    pub reserved: u16,
    pub chunk_sz: u32,
    pub total_sz: u32,
}

impl ChunkHeader {
    pub fn new_raw(chunk_blocks: u32, data_size: u32) -> Self {
        Self {
            chunk_type: CHUNK_TYPE_RAW,
            reserved: 0,
            chunk_sz: chunk_blocks,
            total_sz: CHUNK_HEADER_SIZE + data_size,
        }
    }

    pub fn new_fill(chunk_blocks: u32) -> Self {
        Self {
            chunk_type: CHUNK_TYPE_FILL,
            reserved: 0,
            chunk_sz: chunk_blocks,
            total_sz: CHUNK_HEADER_SIZE + 4,
        }
    }

    pub fn new_dont_care(chunk_blocks: u32) -> Self {
        Self {
            chunk_type: CHUNK_TYPE_DONT_CARE,
            reserved: 0,
            chunk_sz: chunk_blocks,
            total_sz: CHUNK_HEADER_SIZE,
        }
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        let mut buf = vec![0u8; 12];
        buf[0..2].copy_from_slice(&self.chunk_type.to_le_bytes());
        buf[2..4].copy_from_slice(&self.reserved.to_le_bytes());
        buf[4..8].copy_from_slice(&self.chunk_sz.to_le_bytes());
        buf[8..12].copy_from_slice(&self.total_sz.to_le_bytes());
        buf
    }

    pub fn from_bytes(buf: &[u8]) -> Option<Self> {
        if buf.len() < CHUNK_HEADER_SIZE as usize {
            return None;
        }
        Some(Self {
            chunk_type: u16::from_le_bytes([buf[0], buf[1]]),
            reserved: u16::from_le_bytes([buf[2], buf[3]]),
            chunk_sz: u32::from_le_bytes([buf[4], buf[5], buf[6], buf[7]]),
            total_sz: u32::from_le_bytes([buf[8], buf[9], buf[10], buf[11]]),
        })
    }
}
