// Android sparse image 写入器
//
// 提供 sparse image 格式的写入能力

use crate::container::sparse::format::*;
use anyhow::{Context, Result};
use std::fs::File;
use std::io::{Read, Write};
use std::path::Path;

// 数据块类型
pub enum DataChunk {
    // 原始数据
    Raw(Vec<u8>),
    // 从文件读取数据 (自动进行全零区域检测)
    File { path: String, size: u64 },
    // 填充值
    Fill(u32),
    // dont-care chunk (视为全零)
    DontCare,
}

// 内部 chunk 类型 (仅用于写入)
enum InternalChunk {
    Raw(Vec<u8>),
    Fill(u32, u32), // (填充值, 块数)
    DontCare(u32),  // 块数
}

// sparse image 写入器
pub struct SparseWriter {
    path: std::path::PathBuf,
    block_size: u32,
    total_blocks: u32,
    chunks: Vec<(u32, DataChunk)>, // (块数, 数据类型)
}

impl SparseWriter {
    // 创建新的 sparse image 写入器
    pub fn new<P: AsRef<Path>>(path: P, block_size: u32, total_blocks: u32) -> Result<Self> {
        Ok(Self {
            path: path.as_ref().to_path_buf(),
            block_size,
            total_blocks,
            chunks: Vec::new(),
        })
    }

    // 添加 raw chunk 数据块
    pub fn add_raw_chunk(&mut self, data: Vec<u8>) {
        let blocks = (data.len() as u64).div_ceil(self.block_size as u64) as u32;
        self.chunks.push((blocks, DataChunk::Raw(data)));
    }

    // 添加文件数据块 (自动检测全零区域)
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

    // 添加 fill chunk 数据块
    pub fn add_fill_chunk(&mut self, blocks: u32, value: u32) {
        self.chunks.push((blocks, DataChunk::Fill(value)));
    }

    // 添加 dont-care chunk 数据块
    pub fn add_dont_care_chunk(&mut self, blocks: u32) {
        self.chunks.push((blocks, DataChunk::DontCare));
    }

    // 判断数据块是否全为零
    fn is_zero_block(data: &[u8]) -> bool {
        // 采用对 SIMD 友好的方式检测
        data.iter().all(|&b| b == 0)
    }

    // 判断数据块是否为固定填充值
    fn get_fill_value(data: &[u8], block_size: u32) -> Option<u32> {
        if data.len() < 4 || data.len() != block_size as usize {
            return None;
        }

        // 取前 4 字节作为填充值
        let fill_value = u32::from_le_bytes([data[0], data[1], data[2], data[3]]);

        // 检查整个块是否都是该 4 字节模式的重复
        let pattern = fill_value.to_le_bytes();
        for chunk in data.chunks(4) {
            if chunk.len() == 4 && chunk != pattern {
                return None;
            }
        }

        Some(fill_value)
    }

    // 处理文件数据并生成优化后的 chunk 列表
    fn process_file_data(
        &self,
        path: &str,
        size: u64,
        expected_blocks: u32,
    ) -> Result<Vec<InternalChunk>> {
        let mut file =
            File::open(path).with_context(|| format!("failed to open file: {}", path))?;

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
            // 读取一个块
            let to_read = std::cmp::min(block_size as u64, size - total_read) as usize;
            buffer.fill(0); // 清空缓冲区
            let bytes_read = file.read(&mut buffer[..to_read])?;

            if bytes_read == 0 {
                break;
            }

            total_read += bytes_read as u64;
            blocks_processed += 1;

            // 分析当前块 (仅针对实际读取到的数据)
            let block_data = if bytes_read < block_size {
                // 最后一个不完整的块需要补齐到块边界
                &buffer[..block_size]
            } else {
                &buffer[..bytes_read]
            };

            if Self::is_zero_block(block_data) {
                // 全零块
                Self::flush_raw_data(&mut chunks, &mut current_raw_data, self.block_size);
                Self::flush_fill_data(
                    &mut chunks,
                    &mut current_fill_value,
                    &mut current_fill_blocks,
                );
                current_zero_blocks += 1;
            } else if let Some(fill_val) = Self::get_fill_value(block_data, self.block_size) {
                // fill chunk 块
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
                // 普通数据块
                Self::flush_zero_data(&mut chunks, &mut current_zero_blocks);
                Self::flush_fill_data(
                    &mut chunks,
                    &mut current_fill_value,
                    &mut current_fill_blocks,
                );
                current_raw_data.extend_from_slice(block_data);
            }
        }

        // 刷出剩余数据
        Self::flush_raw_data(&mut chunks, &mut current_raw_data, self.block_size);
        Self::flush_fill_data(
            &mut chunks,
            &mut current_fill_value,
            &mut current_fill_blocks,
        );
        Self::flush_zero_data(&mut chunks, &mut current_zero_blocks);

        // 文件小于预期时, 剩余部分使用 dont-care chunk 补齐
        if blocks_processed < expected_blocks {
            let remaining = expected_blocks - blocks_processed;
            chunks.push(InternalChunk::DontCare(remaining));
        }

        Ok(chunks)
    }

    fn flush_raw_data(chunks: &mut Vec<InternalChunk>, raw_data: &mut Vec<u8>, block_size: u32) {
        if !raw_data.is_empty() {
            // 补齐到块边界
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
                // 填充值为零时改用 dont-care chunk
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

    // 写出 sparse image
    pub fn write(self) -> Result<()> {
        // 先处理全部 chunk, 生成内部 chunk 列表
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

        // 合并相邻的同类型 chunk
        let merged_chunks = Self::merge_chunks(internal_chunks);

        // 统计 chunk 总数
        let total_chunks = merged_chunks.len() as u32;

        // 创建输出文件
        let mut file = File::create(&self.path)
            .with_context(|| format!("failed to create output file: {:?}", self.path))?;

        // 写入文件头
        let header = SparseHeader::new(self.block_size, self.total_blocks, total_chunks);
        file.write_all(&header.to_bytes())?;

        // 逐个写入 chunk
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

    // 合并相邻的同类型 chunk
    fn merge_chunks(chunks: Vec<InternalChunk>) -> Vec<InternalChunk> {
        let mut result: Vec<InternalChunk> = Vec::new();

        for chunk in chunks {
            if result.is_empty() {
                result.push(chunk);
                continue;
            }

            // 上面已确保 result 非空
            let last_index = result.len() - 1;
            match (&mut result[last_index], &chunk) {
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

// 将普通镜像转换为 sparse image
pub fn convert_to_sparse<P: AsRef<Path>>(input: P, output: P, block_size: u32) -> Result<()> {
    let input_size = std::fs::metadata(input.as_ref())?.len();
    let total_blocks = input_size.div_ceil(block_size as u64) as u32;

    let mut writer = SparseWriter::new(output, block_size, total_blocks)?;
    writer.add_file_chunk(
        input
            .as_ref()
            .to_str()
            .ok_or_else(|| anyhow::anyhow!("invalid path"))?,
        input_size,
    );
    writer.write()
}
