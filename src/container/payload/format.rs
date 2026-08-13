// OTA payload 文件头解析。
//
// payload.bin 布局: 魔数 CrAU + 大端版本号 + 大端 manifest 长度 +
// 大端元数据签名长度 + manifest 协议缓冲 + 元数据签名 + 数据区。

use crate::container::payload::manifest::DeltaArchiveManifest;
use anyhow::{Context, Result, anyhow, bail};
use prost::Message;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};

pub const PAYLOAD_MAGIC: &[u8; 4] = b"CrAU";
pub const PAYLOAD_VERSION: u64 = 2;
// 魔数 4 字节 + 版本 8 字节 + manifest 长度 8 字节 + 元数据签名长度 4 字节
pub const PAYLOAD_HEADER_SIZE: u64 = 24;
pub const DEFAULT_BLOCK_SIZE: u32 = 4096;

// 解析 payload 头部与 manifest, 返回 manifest 及数据区起始偏移
pub fn parse_payload(payload: &mut File, payload_size: u64) -> Result<(DeltaArchiveManifest, u64)> {
    payload.seek(SeekFrom::Start(0))?;

    let mut magic = [0u8; 4];
    payload.read_exact(&mut magic)?;
    if &magic != PAYLOAD_MAGIC {
        bail!("invalid OTA payload magic");
    }

    let version = read_u64_be(payload)?;
    if version != PAYLOAD_VERSION {
        bail!("unsupported OTA payload version: {}", version);
    }

    let manifest_size = read_u64_be(payload)?;
    let metadata_signature_size = u64::from(read_u32_be(payload)?);
    let data_offset = PAYLOAD_HEADER_SIZE
        .checked_add(manifest_size)
        .and_then(|offset| offset.checked_add(metadata_signature_size))
        .ok_or_else(|| anyhow!("OTA payload metadata size overflow"))?;
    if data_offset > payload_size {
        bail!("OTA payload metadata exceeds file size");
    }

    let manifest_size =
        usize::try_from(manifest_size).map_err(|_| anyhow!("OTA payload manifest is too large"))?;
    let mut manifest_bytes = vec![0u8; manifest_size];
    payload.read_exact(&mut manifest_bytes)?;
    let manifest = DeltaArchiveManifest::decode(manifest_bytes.as_slice())
        .context("failed to decode OTA payload manifest")?;

    Ok((manifest, data_offset))
}

// 读取大端 u64
fn read_u64_be(reader: &mut impl Read) -> Result<u64> {
    let mut bytes = [0u8; 8];
    reader.read_exact(&mut bytes)?;
    Ok(u64::from_be_bytes(bytes))
}

// 读取大端 u32
fn read_u32_be(reader: &mut impl Read) -> Result<u32> {
    let mut bytes = [0u8; 4];
    reader.read_exact(&mut bytes)?;
    Ok(u32::from_be_bytes(bytes))
}
