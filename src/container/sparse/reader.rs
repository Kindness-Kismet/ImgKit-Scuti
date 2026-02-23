// Android Sparse Image Reader
//
// Provides virtual reading of sparse image formats without converting to a full image

use crate::container::sparse::format::*;
use anyhow::{Context, Result, anyhow};
use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::Path;

// Chunk metadata
#[derive(Debug, Clone)]
struct ChunkMeta {
    chunk_type: u16,
    output_offset: u64,
    output_size: u64,
    file_offset: u64,
    fill_value: u32,
}

// Sparse Image Virtual Reader
pub struct SparseReader {
    file: File,
    #[allow(dead_code)]
    header: SparseHeader,
    chunks: Vec<ChunkMeta>,
    total_size: u64,
    position: u64,
}

impl SparseReader {
    // Open sparse image file
    pub fn new<P: AsRef<Path>>(path: P) -> Result<Self> {
        let mut file = File::open(path.as_ref())
            .with_context(|| format!("无法打开文件: {:?}", path.as_ref()))?;

        let mut buf = [0u8; SPARSE_HEADER_SIZE];
        file.read_exact(&mut buf)?;

        let header =
            SparseHeader::from_bytes(&buf).ok_or_else(|| anyhow::anyhow!("非稀疏镜像格式"))?;

        // Parse all chunks
        let mut chunks = Vec::new();
        let mut output_offset = 0u64;

        for _ in 0..header.total_chunks {
            let mut chunk_buf = [0u8; 12];
            file.read_exact(&mut chunk_buf)?;
            let chunk_header = ChunkHeader::from_bytes(&chunk_buf)
                .ok_or_else(|| anyhow::anyhow!("无效的 chunk 头"))?;

            let file_offset = file.stream_position()?;
            let output_size = (chunk_header.chunk_sz as u64)
                .checked_mul(header.blk_sz as u64)
                .ok_or_else(|| anyhow!("chunk 输出大小溢出"))?;
            let data_sz = chunk_data_size(chunk_header.total_sz)?;

            let meta = match chunk_header.chunk_type {
                CHUNK_TYPE_RAW => {
                    if data_sz < output_size {
                        return Err(anyhow!(
                            "RAW chunk 数据长度过小: data_sz={}, output_size={}",
                            data_sz,
                            output_size
                        ));
                    }
                    seek_forward(&mut file, data_sz)?;
                    ChunkMeta {
                        chunk_type: CHUNK_TYPE_RAW,
                        output_offset,
                        output_size,
                        file_offset,
                        fill_value: 0,
                    }
                }
                CHUNK_TYPE_FILL => {
                    // Fill data, read 4-byte filling value
                    if data_sz < 4 {
                        return Err(anyhow!("FILL chunk 数据长度不足 4 字节"));
                    }
                    let mut fill_buf = [0u8; 4];
                    file.read_exact(&mut fill_buf)?;
                    let fill_value = u32::from_le_bytes(fill_buf);

                    // Skip possible remaining data
                    if data_sz > 4 {
                        seek_forward(&mut file, data_sz - 4)?;
                    }

                    ChunkMeta {
                        chunk_type: CHUNK_TYPE_FILL,
                        output_offset,
                        output_size,
                        file_offset,
                        fill_value,
                    }
                }
                CHUNK_TYPE_DONT_CARE => {
                    if data_sz > 0 {
                        seek_forward(&mut file, data_sz)?;
                    }
                    ChunkMeta {
                        chunk_type: CHUNK_TYPE_DONT_CARE,
                        output_offset,
                        output_size,
                        file_offset,
                        fill_value: 0,
                    }
                }
                _ => {
                    // Skip chunks of unknown type
                    seek_forward(&mut file, data_sz)?;
                    ChunkMeta {
                        chunk_type: chunk_header.chunk_type,
                        output_offset,
                        output_size,
                        file_offset,
                        fill_value: 0,
                    }
                }
            };

            chunks.push(meta);
            output_offset = output_offset
                .checked_add(output_size)
                .ok_or_else(|| anyhow!("sparse 输出偏移溢出"))?;
        }

        let total_size = (header.total_blks as u64)
            .checked_mul(header.blk_sz as u64)
            .ok_or_else(|| anyhow!("sparse 总大小溢出"))?;

        Ok(Self {
            file,
            header,
            chunks,
            total_size,
            position: 0,
        })
    }

    // Get the total size of the image
    pub fn total_size(&self) -> u64 {
        self.total_size
    }

    // Find the chunk at the specified location (binary search)
    fn find_chunk(&self, pos: u64) -> Option<usize> {
        if self.chunks.is_empty() {
            return None;
        }

        self.chunks
            .binary_search_by(|chunk| {
                if pos < chunk.output_offset {
                    std::cmp::Ordering::Greater
                } else if pos >= chunk.output_offset + chunk.output_size {
                    std::cmp::Ordering::Less
                } else {
                    std::cmp::Ordering::Equal
                }
            })
            .ok()
    }
}

fn chunk_data_size(total_sz: u32) -> Result<u64> {
    total_sz
        .checked_sub(CHUNK_HEADER_SIZE)
        .map(u64::from)
        .ok_or_else(|| anyhow!("chunk total_sz 小于头部大小"))
}

fn seek_forward(file: &mut File, amount: u64) -> Result<()> {
    let offset = i64::try_from(amount).map_err(|_| anyhow!("偏移量过大: {}", amount))?;
    file.seek(SeekFrom::Current(offset))?;
    Ok(())
}

impl Read for SparseReader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.position >= self.total_size {
            return Ok(0);
        }

        let to_read = std::cmp::min(buf.len(), (self.total_size - self.position) as usize);
        let mut total_read = 0;

        while total_read < to_read {
            let chunk_idx = match self.find_chunk(self.position) {
                Some(idx) => idx,
                None => break,
            };

            let chunk = &self.chunks[chunk_idx];
            let offset_in_chunk = self.position - chunk.output_offset;
            let remaining_in_chunk = chunk.output_size - offset_in_chunk;
            let to_read_now =
                std::cmp::min((to_read - total_read) as u64, remaining_in_chunk) as usize;

            match chunk.chunk_type {
                CHUNK_TYPE_RAW => {
                    self.file
                        .seek(SeekFrom::Start(chunk.file_offset + offset_in_chunk))?;
                    self.file
                        .read_exact(&mut buf[total_read..total_read + to_read_now])?;
                }
                CHUNK_TYPE_FILL => {
                    let fill_byte = (chunk.fill_value & 0xFF) as u8;
                    buf[total_read..total_read + to_read_now].fill(fill_byte);
                }
                CHUNK_TYPE_DONT_CARE => {
                    buf[total_read..total_read + to_read_now].fill(0);
                }
                _ => {
                    buf[total_read..total_read + to_read_now].fill(0);
                }
            }

            total_read += to_read_now;
            self.position += to_read_now as u64;
        }

        Ok(total_read)
    }
}

impl Seek for SparseReader {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        let new_pos = match pos {
            SeekFrom::Start(offset) => offset as i64,
            SeekFrom::End(offset) => self.total_size as i64 + offset,
            SeekFrom::Current(offset) => self.position as i64 + offset,
        };

        if new_pos < 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "无效的 seek 位置",
            ));
        }

        self.position = new_pos as u64;
        Ok(self.position)
    }
}

// Check if a file is in sparse format
pub fn is_sparse_image<P: AsRef<Path>>(path: P) -> Result<bool> {
    let mut file = File::open(path.as_ref())?;
    let mut buf = [0u8; 4];
    if file.read_exact(&mut buf).is_err() {
        return Ok(false);
    }
    let magic = u32::from_le_bytes(buf);
    Ok(magic == SPARSE_HEADER_MAGIC)
}
