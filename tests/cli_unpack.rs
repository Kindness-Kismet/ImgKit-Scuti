// unpack 子命令的端到端测试, 直接调用编译产物验证参数行为。

use anyhow::Result;
use prost::Message;
use sha2::{Digest, Sha256};
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

use imgkit_scuti::container::payload::manifest::{
    DeltaArchiveManifest, Extent, InstallOperation, InstallOperationType, PartitionInfo,
    PartitionUpdate,
};

const BINARY: &str = env!("CARGO_BIN_EXE_imgkit_scuti");
const BLOCK_SIZE: u32 = 4096;

static TEST_ID: AtomicU64 = AtomicU64::new(0);

// 测试结束后自动清理的临时目录
struct TestDir {
    path: PathBuf,
}

impl TestDir {
    fn new() -> Result<Self> {
        let path = std::env::temp_dir().join(format!(
            "imgkit_cli_{}_{}",
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

fn run_unpack(args: &[&str]) -> Result<Output> {
    Ok(Command::new(BINARY).arg("unpack").args(args).output()?)
}

// 生成含 system 与 vendor 两个分区的 payload
fn write_sample_payload(path: &Path) -> Result<()> {
    let block_size = BLOCK_SIZE as usize;
    let system = {
        let mut data = b"SYSTEM".to_vec();
        data.resize(block_size, 0);
        data
    };
    let vendor = {
        let mut data = b"VENDOR".to_vec();
        data.resize(block_size, 0);
        data
    };

    let mut blobs = Vec::new();
    blobs.extend_from_slice(&system);
    let vendor_offset = blobs.len() as u64;
    blobs.extend_from_slice(&vendor);

    let make = |name: &str, content: &[u8], offset: u64| PartitionUpdate {
        partition_name: name.to_string(),
        new_partition_info: Some(PartitionInfo {
            size: Some(content.len() as u64),
            hash: Some(Sha256::digest(content).to_vec()),
        }),
        operations: vec![InstallOperation {
            r#type: InstallOperationType::Replace as i32,
            data_offset: Some(offset),
            data_length: Some(content.len() as u64),
            dst_extents: vec![Extent {
                start_block: Some(0),
                num_blocks: Some(1),
            }],
            data_sha256_hash: Some(Sha256::digest(content).to_vec()),
        }],
    };

    let manifest = DeltaArchiveManifest {
        block_size: Some(BLOCK_SIZE),
        minor_version: Some(0),
        partitions: vec![
            make("system", &system, 0),
            make("vendor", &vendor, vendor_offset),
        ],
    };
    let manifest_bytes = manifest.encode_to_vec();

    let mut output = File::create(path)?;
    output.write_all(b"CrAU")?;
    output.write_all(&2u64.to_be_bytes())?;
    output.write_all(&(manifest_bytes.len() as u64).to_be_bytes())?;
    output.write_all(&0u32.to_be_bytes())?;
    output.write_all(&manifest_bytes)?;
    output.write_all(&blobs)?;
    Ok(())
}

#[test]
fn list_prints_partition_names_without_output_dir() -> Result<()> {
    let dir = TestDir::new()?;
    let input = dir.join("payload.bin");
    write_sample_payload(&input)?;

    let output = run_unpack(&["-i", input.to_string_lossy().as_ref(), "--list", "-l", "0"])?;

    assert!(output.status.success(), "unpack --list failed");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let names: Vec<_> = stdout.split_whitespace().collect();
    assert_eq!(names, vec!["system", "vendor"]);
    Ok(())
}

#[test]
fn partition_flag_extracts_single_partition() -> Result<()> {
    let dir = TestDir::new()?;
    let input = dir.join("payload.bin");
    let out_dir = dir.join("out");
    write_sample_payload(&input)?;

    let output = run_unpack(&[
        "-i",
        input.to_string_lossy().as_ref(),
        "-o",
        out_dir.to_string_lossy().as_ref(),
        "-p",
        "vendor",
        "-l",
        "0",
    ])?;

    assert!(output.status.success(), "unpack -p vendor failed");
    assert!(out_dir.join("vendor.img").exists());
    assert!(!out_dir.join("system.img").exists());
    Ok(())
}

#[test]
fn missing_output_dir_is_rejected_without_list() -> Result<()> {
    let dir = TestDir::new()?;
    let input = dir.join("payload.bin");
    write_sample_payload(&input)?;

    let output = run_unpack(&["-i", input.to_string_lossy().as_ref(), "-l", "0"])?;

    assert!(!output.status.success(), "missing --output must fail");
    Ok(())
}

#[test]
fn unknown_partition_name_is_rejected() -> Result<()> {
    let dir = TestDir::new()?;
    let input = dir.join("payload.bin");
    let out_dir = dir.join("out");
    write_sample_payload(&input)?;

    let output = run_unpack(&[
        "-i",
        input.to_string_lossy().as_ref(),
        "-o",
        out_dir.to_string_lossy().as_ref(),
        "-p",
        "odm",
        "-l",
        "0",
    ])?;

    assert!(!output.status.success(), "unknown partition must fail");
    Ok(())
}
