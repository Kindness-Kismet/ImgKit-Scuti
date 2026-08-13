// OTA payload 提取的集成测试。
//
// 直接构造合成 payload.bin, 覆盖多区段回放、分区筛选、分区列举
// 以及增量包与未知分区名的拒绝路径。

use anyhow::Result;
use imgkit_scuti::container::payload::extractor::{ExtractConfig, extract_image, list_partitions};
use imgkit_scuti::container::payload::manifest::{
    DeltaArchiveManifest, Extent, InstallOperation, InstallOperationType, PartitionInfo,
    PartitionUpdate,
};
use prost::Message;
use sha2::{Digest, Sha256};
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

const PAYLOAD_MAGIC: &[u8; 4] = b"CrAU";
const PAYLOAD_VERSION: u64 = 2;
const BLOCK_SIZE: u32 = 4096;

static TEST_ID: AtomicU64 = AtomicU64::new(0);

// 测试结束后自动清理的临时目录
struct TestDir {
    path: PathBuf,
}

impl TestDir {
    fn new() -> Result<Self> {
        let path = std::env::temp_dir().join(format!(
            "imgkit_it_{}_{}",
            std::process::id(),
            TEST_ID.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&path)?;
        Ok(Self { path })
    }

    fn join(&self, name: &str) -> PathBuf {
        self.path.join(name)
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

// 把内容补齐到块边界
fn pad_to_block(data: &[u8]) -> Vec<u8> {
    let mut padded = data.to_vec();
    let block_size = BLOCK_SIZE as usize;
    let remainder = padded.len() % block_size;
    if remainder != 0 {
        padded.resize(padded.len() + block_size - remainder, 0);
    }
    padded
}

// 构造一个把整块数据一次性写入的分区
fn make_partition(
    name: &str,
    content: &[u8],
    blob: &[u8],
    operation_type: InstallOperationType,
    data_offset: u64,
) -> PartitionUpdate {
    let block_count = content.len() as u64 / u64::from(BLOCK_SIZE);
    PartitionUpdate {
        partition_name: name.to_string(),
        new_partition_info: Some(PartitionInfo {
            size: Some(content.len() as u64),
            hash: Some(Sha256::digest(content).to_vec()),
        }),
        operations: vec![InstallOperation {
            r#type: operation_type as i32,
            data_offset: Some(data_offset),
            data_length: Some(blob.len() as u64),
            dst_extents: vec![Extent {
                start_block: Some(0),
                num_blocks: Some(block_count),
            }],
            data_sha256_hash: Some(Sha256::digest(blob).to_vec()),
        }],
    }
}

// 按 payload 布局写出文件
fn write_payload(
    path: &Path,
    partitions: Vec<PartitionUpdate>,
    blobs: &[u8],
    minor_version: u32,
) -> Result<()> {
    let manifest = DeltaArchiveManifest {
        block_size: Some(BLOCK_SIZE),
        minor_version: Some(minor_version),
        partitions,
    };
    let manifest_bytes = manifest.encode_to_vec();

    let mut output = File::create(path)?;
    output.write_all(PAYLOAD_MAGIC)?;
    output.write_all(&PAYLOAD_VERSION.to_be_bytes())?;
    output.write_all(&(manifest_bytes.len() as u64).to_be_bytes())?;
    output.write_all(&0u32.to_be_bytes())?;
    output.write_all(&manifest_bytes)?;
    output.write_all(blobs)?;
    Ok(())
}

// 构造包含 system、vendor、cache 三个分区的样本
fn build_sample(path: &Path) -> Result<(Vec<u8>, Vec<u8>, Vec<u8>)> {
    let system = pad_to_block(b"SYSTEM CONTENT");
    let vendor = pad_to_block(b"VENDOR CONTENT");
    let cache = vec![0u8; BLOCK_SIZE as usize];

    let mut blobs = Vec::new();
    blobs.extend_from_slice(&system);
    let vendor_offset = blobs.len() as u64;
    blobs.extend_from_slice(&vendor);

    let partitions = vec![
        make_partition("system", &system, &system, InstallOperationType::Replace, 0),
        make_partition(
            "vendor",
            &vendor,
            &vendor,
            InstallOperationType::Replace,
            vendor_offset,
        ),
        PartitionUpdate {
            partition_name: "cache".to_string(),
            new_partition_info: Some(PartitionInfo {
                size: Some(cache.len() as u64),
                hash: Some(Sha256::digest(&cache).to_vec()),
            }),
            operations: vec![InstallOperation {
                r#type: InstallOperationType::Zero as i32,
                data_offset: None,
                data_length: None,
                dst_extents: vec![Extent {
                    start_block: Some(0),
                    num_blocks: Some(1),
                }],
                data_sha256_hash: None,
            }],
        },
    ];

    write_payload(path, partitions, &blobs, 0)?;
    Ok((system, vendor, cache))
}

#[test]
fn extracts_all_partitions() -> Result<()> {
    let dir = TestDir::new()?;
    let input = dir.join("payload.bin");
    let output = dir.join("out");
    let (system, vendor, cache) = build_sample(&input)?;

    extract_image(ExtractConfig {
        input_payload: input.to_string_lossy().into_owned(),
        output_dir: output.to_string_lossy().into_owned(),
        partition_names: Vec::new(),
    })?;

    assert_eq!(fs::read(output.join("system.img"))?, system);
    assert_eq!(fs::read(output.join("vendor.img"))?, vendor);
    assert_eq!(fs::read(output.join("cache.img"))?, cache);
    Ok(())
}

#[test]
fn extracts_selected_partitions_only() -> Result<()> {
    let dir = TestDir::new()?;
    let input = dir.join("payload.bin");
    let output = dir.join("out");
    let (_, vendor, _) = build_sample(&input)?;

    extract_image(ExtractConfig {
        input_payload: input.to_string_lossy().into_owned(),
        output_dir: output.to_string_lossy().into_owned(),
        partition_names: vec!["vendor".to_string()],
    })?;

    assert_eq!(fs::read(output.join("vendor.img"))?, vendor);
    assert!(!output.join("system.img").exists());
    assert!(!output.join("cache.img").exists());
    Ok(())
}

#[test]
fn lists_partition_names() -> Result<()> {
    let dir = TestDir::new()?;
    let input = dir.join("payload.bin");
    build_sample(&input)?;

    let names = list_partitions(input.to_string_lossy().as_ref())?;
    assert_eq!(names, vec!["system", "vendor", "cache"]);
    Ok(())
}

#[test]
fn rejects_unknown_partition_name() -> Result<()> {
    let dir = TestDir::new()?;
    let input = dir.join("payload.bin");
    let output = dir.join("out");
    build_sample(&input)?;

    let error = extract_image(ExtractConfig {
        input_payload: input.to_string_lossy().into_owned(),
        output_dir: output.to_string_lossy().into_owned(),
        partition_names: vec!["odm".to_string()],
    })
    .expect_err("unknown partition name must be rejected");

    let message = error.to_string();
    assert!(message.contains("odm"), "unexpected message: {}", message);
    assert!(
        message.contains("system, vendor, cache"),
        "unexpected message: {}",
        message
    );
    Ok(())
}

#[test]
fn rejects_incremental_payload() -> Result<()> {
    let dir = TestDir::new()?;
    let input = dir.join("payload.bin");
    let output = dir.join("out");

    let content = pad_to_block(b"DELTA");
    write_payload(
        &input,
        vec![make_partition(
            "system",
            &content,
            &content,
            InstallOperationType::Replace,
            0,
        )],
        &content,
        1,
    )?;

    let error = extract_image(ExtractConfig {
        input_payload: input.to_string_lossy().into_owned(),
        output_dir: output.to_string_lossy().into_owned(),
        partition_names: Vec::new(),
    })
    .expect_err("incremental payload must be rejected");

    assert!(
        error.to_string().contains("incremental OTA"),
        "unexpected message: {}",
        error
    );
    Ok(())
}

#[test]
fn detects_corrupted_partition_data() -> Result<()> {
    let dir = TestDir::new()?;
    let input = dir.join("payload.bin");
    let output = dir.join("out");

    let content = pad_to_block(b"ORIGINAL");
    let mut corrupted = content.clone();
    corrupted[0] ^= 0xff;

    // manifest 中记录原始内容的摘要, 但数据区写入被篡改的内容
    write_payload(
        &input,
        vec![make_partition(
            "system",
            &content,
            &content,
            InstallOperationType::Replace,
            0,
        )],
        &corrupted,
        0,
    )?;

    let error = extract_image(ExtractConfig {
        input_payload: input.to_string_lossy().into_owned(),
        output_dir: output.to_string_lossy().into_owned(),
        partition_names: Vec::new(),
    })
    .expect_err("corrupted data must fail hash verification");

    assert!(
        error.to_string().contains("SHA-256"),
        "unexpected message: {}",
        error
    );
    Ok(())
}
