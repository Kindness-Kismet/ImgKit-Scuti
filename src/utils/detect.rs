// 文件系统检测模块

use crate::container::sparse::{
    CHUNK_HEADER_SIZE, CHUNK_TYPE_DONT_CARE, CHUNK_TYPE_FILL, CHUNK_TYPE_RAW, SPARSE_HEADER_SIZE,
};
use anyhow::{Result, anyhow};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

// magic bytes 条目
struct MagicBytes {
    offset: usize,
    expected: &'static [u8],
    file_type: &'static str,
}

// magic bytes 查找表
const MAGIC_BYTES: &[MagicBytes] = &[
    MagicBytes {
        offset: 0,
        expected: b"CrAU",
        file_type: "payload",
    },
    // sparse 镜像 (优先检测, 可能是 sparse super)
    MagicBytes {
        offset: 0,
        expected: &[0x3a, 0xff, 0x26, 0xed],
        file_type: "sparse",
    },
    // super 分区 (两个可能的偏移)
    MagicBytes {
        offset: 0,
        expected: &[0x67, 0x44, 0x6c, 0x61],
        file_type: "super",
    },
    MagicBytes {
        offset: 4096,
        expected: &[0x67, 0x44, 0x6c, 0x61],
        file_type: "super",
    },
    // EROFS 超级块
    MagicBytes {
        offset: 1024,
        expected: &[0xe2, 0xe1, 0xf5, 0xe0],
        file_type: "erofs",
    },
    // F2FS 超级块
    MagicBytes {
        offset: 1024,
        expected: &[0x10, 0x20, 0xf5, 0xf2],
        file_type: "f2fs",
    },
    // EXT4 超级块
    MagicBytes {
        offset: 1080,
        expected: &[0x53, 0xef],
        file_type: "ext4",
    },
];

// 检测文件的文件系统类型
pub fn detect_filesystem(path: &Path) -> Result<String> {
    let mut file = File::open(path)?;

    // 读取前 5 KB 以覆盖所有 magic bytes 偏移
    let mut buffer = vec![0u8; 5000];
    let read_len = file.read(&mut buffer)?;
    buffer.truncate(read_len);

    // 优先检测 sparse 镜像
    let is_sparse = buffer.len() >= 4 && buffer[0..4] == [0x3a, 0xff, 0x26, 0xed];

    // 若为 sparse, 读取虚拟化数据以检测真实文件系统类型
    if is_sparse {
        return detect_sparse_filesystem(&mut file);
    }

    // 遍历 magic bytes 表
    for m in MAGIC_BYTES {
        if m.offset + m.expected.len() <= buffer.len()
            && buffer[m.offset..m.offset + m.expected.len()] == *m.expected
        {
            return Ok(m.file_type.to_string());
        }
    }

    Err(anyhow!("unrecognized filesystem type"))
}

// sparse 头部字段
struct SparseHeader {
    blk_sz: u32,
    total_chunks: u32,
}

// 读取并解析 sparse 镜像头部
fn read_sparse_header(file: &mut File) -> Result<SparseHeader> {
    file.seek(SeekFrom::Start(0))?;
    let mut header = [0u8; SPARSE_HEADER_SIZE];
    file.read_exact(&mut header)?;

    let blk_sz = u32::from_le_bytes([header[12], header[13], header[14], header[15]]);
    let total_chunks = u32::from_le_bytes([header[20], header[21], header[22], header[23]]);

    if blk_sz == 0 {
        return Err(anyhow!("sparse block size is 0"));
    }
    if total_chunks == 0 {
        return Err(anyhow!("sparse image has no valid chunks"));
    }

    Ok(SparseHeader {
        blk_sz,
        total_chunks,
    })
}

// chunk 头部字段
struct ChunkInfo {
    chunk_type: u16,
    chunk_sz: u32,
    total_sz: u32,
}

// 读取 chunk 头部
fn read_chunk_header(file: &mut File) -> Result<ChunkInfo> {
    let mut chunk_header = [0u8; 12];
    file.read_exact(&mut chunk_header)?;

    Ok(ChunkInfo {
        chunk_type: u16::from_le_bytes([chunk_header[0], chunk_header[1]]),
        chunk_sz: u32::from_le_bytes([
            chunk_header[4],
            chunk_header[5],
            chunk_header[6],
            chunk_header[7],
        ]),
        total_sz: u32::from_le_bytes([
            chunk_header[8],
            chunk_header[9],
            chunk_header[10],
            chunk_header[11],
        ]),
    })
}

// 检测 sparse 镜像内部的文件系统类型
fn detect_sparse_filesystem(file: &mut File) -> Result<String> {
    if is_sparse_super(file)? {
        return Ok("sparse_super".to_string());
    }

    let header = read_sparse_header(file)?;

    // 读取第一个有效数据 chunk 以检测 ext4 magic bytes
    let mut virtual_data = vec![0u8; 2048.min(header.blk_sz as usize)];
    let mut data_read = 0usize;

    for _ in 0..header.total_chunks.min(5) {
        let chunk = match read_chunk_header(file) {
            Ok(c) => c,
            Err(_) => break,
        };
        let chunk_output_size = (chunk.chunk_sz as u64)
            .checked_mul(header.blk_sz as u64)
            .ok_or_else(|| anyhow!("sparse chunk output size overflow"))?;
        let chunk_data_size = chunk
            .total_sz
            .checked_sub(CHUNK_HEADER_SIZE)
            .map(u64::from)
            .ok_or_else(|| anyhow!("sparse chunk total_sz is smaller than header size"))?;

        match chunk.chunk_type {
            CHUNK_TYPE_RAW => {
                let to_read = (chunk_output_size as usize).min(virtual_data.len() - data_read);
                if file
                    .read_exact(&mut virtual_data[data_read..data_read + to_read])
                    .is_ok()
                {
                    data_read += to_read;
                    if data_read >= virtual_data.len() {
                        break;
                    }
                }
                if chunk_data_size < to_read as u64 {
                    return Err(anyhow!("RAW chunk data is truncated"));
                }
                let remaining = chunk_data_size - to_read as u64;
                if remaining > 0 {
                    let _ = file.seek(SeekFrom::Current(remaining as i64));
                }
            }
            CHUNK_TYPE_FILL => {
                let mut fill_buf = [0u8; 4];
                if file.read_exact(&mut fill_buf).is_ok() {
                    let fill_byte = fill_buf[0];
                    let to_fill = (chunk_output_size as usize).min(virtual_data.len() - data_read);
                    virtual_data[data_read..data_read + to_fill].fill(fill_byte);
                    data_read += to_fill;
                }
                if chunk_data_size > 4 {
                    let _ = file.seek(SeekFrom::Current((chunk_data_size - 4) as i64));
                }
            }
            CHUNK_TYPE_DONT_CARE => {
                let to_fill = (chunk_output_size as usize).min(virtual_data.len() - data_read);
                virtual_data[data_read..data_read + to_fill].fill(0);
                data_read += to_fill;
                if chunk_data_size > 0 {
                    let _ = file.seek(SeekFrom::Current(chunk_data_size as i64));
                }
            }
            _ => {
                let _ = file.seek(SeekFrom::Current(chunk_data_size as i64));
            }
        }

        if data_read >= virtual_data.len() {
            break;
        }
    }

    if virtual_data.len() >= 1082 && virtual_data[1080..1082] == [0x53, 0xef] {
        return Ok("sparse_ext4".to_string());
    }

    // F2FS superblock magic bytes 位于虚拟偏移 1024
    if virtual_data.len() >= 1028 && virtual_data[1024..1028] == [0x10, 0x20, 0xf5, 0xf2] {
        return Ok("sparse_f2fs".to_string());
    }

    Err(anyhow!("unrecognized filesystem type in sparse image"))
}

// 检测 sparse 镜像是否包含 super 分区
fn is_sparse_super(file: &mut File) -> Result<bool> {
    let header = read_sparse_header(file)?;

    for _ in 0..header.total_chunks.min(5) {
        let chunk = match read_chunk_header(file) {
            Ok(c) => c,
            Err(_) => break,
        };
        let chunk_output_size = (chunk.chunk_sz as u64)
            .checked_mul(header.blk_sz as u64)
            .ok_or_else(|| anyhow!("sparse chunk output size overflow"))?;
        let chunk_data_size = chunk
            .total_sz
            .checked_sub(CHUNK_HEADER_SIZE)
            .map(u64::from)
            .ok_or_else(|| anyhow!("sparse chunk total_sz is smaller than header size"))?;

        match chunk.chunk_type {
            CHUNK_TYPE_RAW => {
                let to_read = chunk_output_size.min(16384) as usize;
                let mut data = vec![0u8; to_read];

                if file.read_exact(&mut data).is_ok() {
                    for i in 0..data.len().saturating_sub(4) {
                        if data[i..i + 4] == [0x67, 0x44, 0x6c, 0x61] {
                            return Ok(true);
                        }
                    }
                }

                if chunk_data_size < to_read as u64 {
                    return Err(anyhow!("RAW chunk data is truncated"));
                }
                let remaining = chunk_data_size - to_read as u64;
                if remaining > 0 {
                    let _ = file.seek(SeekFrom::Current(remaining as i64));
                }
            }
            CHUNK_TYPE_FILL => {
                let _ = file.seek(SeekFrom::Current(chunk_data_size as i64));
            }
            CHUNK_TYPE_DONT_CARE => {
                if chunk_data_size > 0 {
                    let _ = file.seek(SeekFrom::Current(chunk_data_size as i64));
                }
            }
            _ => {
                let _ = file.seek(SeekFrom::Current(chunk_data_size as i64));
            }
        }
    }

    Ok(false)
}
