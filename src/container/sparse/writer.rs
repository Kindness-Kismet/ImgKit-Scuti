// Android Sparse Image Writer
//
// Provides writing function for sparse image format

use crate::container::sparse::format::*;
use anyhow::{Context, Result};
use std::fs::File;
use std::io::{Read, Write};
use std::path::Path;

// Data block type
pub enum DataChunk {
    // raw data
    Raw(Vec<u8>),
    // Reading data from file (with zero region detection)
    File { path: String, size: u64 },
    // fill value
    Fill(u32),
    // Don't care (all zeros)
    DontCare,
}

// Internal chunk type (for writing)
enum InternalChunk {
    Raw(Vec<u8>),
    Fill(u32, u32), // (value, blocks)
    DontCare(u32),  // blocks
}

// sparse image writer
pub struct SparseWriter {
    path: std::path::PathBuf,
    block_size: u32,
    total_blocks: u32,
    chunks: Vec<(u32, DataChunk)>, // (number of blocks, data type)
}

impl SparseWriter {
    // Create a new sparse image writer
    pub fn new<P: AsRef<Path>>(path: P, block_size: u32, total_blocks: u32) -> Result<Self> {
        Ok(Self {
            path: path.as_ref().to_path_buf(),
            block_size,
            total_blocks,
            chunks: Vec::new(),
        })
    }

    // Add raw data block
    pub fn add_raw_chunk(&mut self, data: Vec<u8>) {
        let blocks = (data.len() as u64).div_ceil(self.block_size as u64) as u32;
        self.chunks.push((blocks, DataChunk::Raw(data)));
    }

    // Add file data block (zero area will be automatically detected)
    pub fn add_file_chunk(&mut self, path: &str, size: u64) {
        let blocks = size.div_ceil(self.block_size as u64) as u32;
        self.chunks.push((
            blocks,
            DataChunk::File {
                path: path.to_string(),
                size,
            },
        ));
    }

    // Add padding blocks
    pub fn add_fill_chunk(&mut self, blocks: u32, value: u32) {
        self.chunks.push((blocks, DataChunk::Fill(value)));
    }

    // Add DONT_CARE block
    pub fn add_dont_care_chunk(&mut self, blocks: u32) {
        self.chunks.push((blocks, DataChunk::DontCare));
    }

    // Check if the data block is all zeros
    fn is_zero_block(data: &[u8]) -> bool {
        // Check using SIMD friendly way
        data.iter().all(|&b| b == 0)
    }

    // Check if data block is filled value
    fn get_fill_value(data: &[u8], block_size: u32) -> Option<u32> {
        if data.len() < 4 || data.len() != block_size as usize {
            return None;
        }

        // Get the first 4 bytes as padding value
        let fill_value = u32::from_le_bytes([data[0], data[1], data[2], data[3]]);

        // Check whether the entire block is a repetition of this 4-byte pattern
        let pattern = fill_value.to_le_bytes();
        for chunk in data.chunks(4) {
            if chunk.len() == 4 && chunk != pattern {
                return None;
            }
        }

        Some(fill_value)
    }

    // Process file data and generate optimized chunk list
    fn process_file_data(
        &self,
        path: &str,
        size: u64,
        expected_blocks: u32,
    ) -> Result<Vec<InternalChunk>> {
        let mut file = File::open(path).with_context(|| format!("打开文件失败: {}", path))?;

        let block_size = self.block_size as usize;
        let mut buffer = vec![0u8; block_size];
        let mut chunks: Vec<InternalChunk> = Vec::new();

        let mut current_raw_data: Vec<u8> = Vec::new();
        let mut current_fill_value: Option<u32> = None;
        let mut current_fill_blocks: u32 = 0;
        let mut current_zero_blocks: u32 = 0;

        let mut total_read = 0u64;
        let mut blocks_processed = 0u32;

        while total_read < size && blocks_processed < expected_blocks {
            // read a block
            let to_read = std::cmp::min(block_size as u64, size - total_read) as usize;
            buffer.fill(0); // Clear buffer
            let bytes_read = file.read(&mut buffer[..to_read])?;

            if bytes_read == 0 {
                break;
            }

            total_read += bytes_read as u64;
            blocks_processed += 1;

            // Analyze the current block (only the data actually read)
            let block_data = if bytes_read < block_size {
                // The last incomplete block needs to be padded to the block boundary
                &buffer[..block_size]
            } else {
                &buffer[..bytes_read]
            };

            if Self::is_zero_block(block_data) {
                // all zero blocks
                Self::flush_raw_data(&mut chunks, &mut current_raw_data, self.block_size);
                Self::flush_fill_data(
                    &mut chunks,
                    &mut current_fill_value,
                    &mut current_fill_blocks,
                );
                current_zero_blocks += 1;
            } else if let Some(fill_val) = Self::get_fill_value(block_data, self.block_size) {
                // fill block
                Self::flush_raw_data(&mut chunks, &mut current_raw_data, self.block_size);
                Self::flush_zero_data(&mut chunks, &mut current_zero_blocks);

                if current_fill_value == Some(fill_val) {
                    current_fill_blocks += 1;
                } else {
                    Self::flush_fill_data(
                        &mut chunks,
                        &mut current_fill_value,
                        &mut current_fill_blocks,
                    );
                    current_fill_value = Some(fill_val);
                    current_fill_blocks = 1;
                }
            } else {
                // Ordinary data block
                Self::flush_zero_data(&mut chunks, &mut current_zero_blocks);
                Self::flush_fill_data(
                    &mut chunks,
                    &mut current_fill_value,
                    &mut current_fill_blocks,
                );
                current_raw_data.extend_from_slice(block_data);
            }
        }

        // Refresh remaining data
        Self::flush_raw_data(&mut chunks, &mut current_raw_data, self.block_size);
        Self::flush_fill_data(
            &mut chunks,
            &mut current_fill_value,
            &mut current_fill_blocks,
        );
        Self::flush_zero_data(&mut chunks, &mut current_zero_blocks);

        // If the file is smaller than expected, fill the remainder with DONT_CARE
        if blocks_processed < expected_blocks {
            let remaining = expected_blocks - blocks_processed;
            chunks.push(InternalChunk::DontCare(remaining));
        }

        Ok(chunks)
    }

    fn flush_raw_data(chunks: &mut Vec<InternalChunk>, raw_data: &mut Vec<u8>, block_size: u32) {
        if !raw_data.is_empty() {
            // Pad to block boundaries
            let padded_size = raw_data.len().div_ceil(block_size as usize) * block_size as usize;
            raw_data.resize(padded_size, 0);
            chunks.push(InternalChunk::Raw(std::mem::take(raw_data)));
        }
    }

    fn flush_fill_data(
        chunks: &mut Vec<InternalChunk>,
        fill_value: &mut Option<u32>,
        fill_blocks: &mut u32,
    ) {
        if let Some(value) = fill_value.take()
            && *fill_blocks > 0
        {
            if value == 0 {
                // Zero padding uses DONT_CARE
                chunks.push(InternalChunk::DontCare(*fill_blocks));
            } else {
                chunks.push(InternalChunk::Fill(value, *fill_blocks));
            }
        }
        *fill_blocks = 0;
    }

    fn flush_zero_data(chunks: &mut Vec<InternalChunk>, zero_blocks: &mut u32) {
        if *zero_blocks > 0 {
            chunks.push(InternalChunk::DontCare(*zero_blocks));
            *zero_blocks = 0;
        }
    }

    // Write to sparse image
    pub fn write(self) -> Result<()> {
        // Process all chunks first and generate internal chunk list
        let mut internal_chunks: Vec<InternalChunk> = Vec::new();

        for (blocks, chunk) in &self.chunks {
            match chunk {
                DataChunk::Raw(data) => {
                    let mut padded_data = data.clone();
                    let padded_size = (*blocks as u64 * self.block_size as u64) as usize;
                    padded_data.resize(padded_size, 0);
                    internal_chunks.push(InternalChunk::Raw(padded_data));
                }
                DataChunk::File { path, size } => {
                    let file_chunks = self.process_file_data(path, *size, *blocks)?;
                    internal_chunks.extend(file_chunks);
                }
                DataChunk::Fill(value) => {
                    internal_chunks.push(InternalChunk::Fill(*value, *blocks));
                }
                DataChunk::DontCare => {
                    internal_chunks.push(InternalChunk::DontCare(*blocks));
                }
            }
        }

        // Merge adjacent chunks of the same type
        let merged_chunks = Self::merge_chunks(internal_chunks);

        // Calculate the total number of chunks
        let total_chunks = merged_chunks.len() as u32;

        // Create output file
        let mut file = File::create(&self.path)
            .with_context(|| format!("创建输出文件失败: {:?}", self.path))?;

        // Write header
        let header = SparseHeader::new(self.block_size, self.total_blocks, total_chunks);
        file.write_all(&header.to_bytes())?;

        // Write to each chunk
        for chunk in merged_chunks {
            match chunk {
                InternalChunk::Raw(data) => {
                    let blocks = (data.len() / self.block_size as usize) as u32;
                    let chunk_header = ChunkHeader::new_raw(blocks, data.len() as u32);
                    file.write_all(&chunk_header.to_bytes())?;
                    file.write_all(&data)?;
                }
                InternalChunk::Fill(value, blocks) => {
                    let chunk_header = ChunkHeader::new_fill(blocks);
                    file.write_all(&chunk_header.to_bytes())?;
                    file.write_all(&value.to_le_bytes())?;
                }
                InternalChunk::DontCare(blocks) => {
                    let chunk_header = ChunkHeader::new_dont_care(blocks);
                    file.write_all(&chunk_header.to_bytes())?;
                }
            }
        }

        Ok(())
    }

    // Merge adjacent chunks of the same type
    fn merge_chunks(chunks: Vec<InternalChunk>) -> Vec<InternalChunk> {
        let mut result: Vec<InternalChunk> = Vec::new();

        for chunk in chunks {
            if result.is_empty() {
                result.push(chunk);
                continue;
            }

            let last = result.last_mut().unwrap();
            match (&mut *last, &chunk) {
                (InternalChunk::DontCare(a), InternalChunk::DontCare(b)) => {
                    *a += b;
                }
                (InternalChunk::Fill(val_a, blocks_a), InternalChunk::Fill(val_b, blocks_b))
                    if *val_a == *val_b =>
                {
                    *blocks_a += blocks_b;
                }
                (InternalChunk::Raw(data_a), InternalChunk::Raw(data_b)) => {
                    data_a.extend_from_slice(data_b);
                }
                _ => {
                    result.push(chunk);
                }
            }
        }

        result
    }
}

// Convert normal image to sparse image
pub fn convert_to_sparse<P: AsRef<Path>>(input: P, output: P, block_size: u32) -> Result<()> {
    let input_size = std::fs::metadata(input.as_ref())?.len();
    let total_blocks = input_size.div_ceil(block_size as u64) as u32;

    let mut writer = SparseWriter::new(output, block_size, total_blocks)?;
    writer.add_file_chunk(
        input
            .as_ref()
            .to_str()
            .ok_or_else(|| anyhow::anyhow!("无效路径"))?,
        input_size,
    );
    writer.write()
}
