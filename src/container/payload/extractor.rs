// OTA payload 提取器。
//
// 仅支持全量包, 增量包需要旧版本分区镜像作为差分基准。

use crate::container::payload::format::{DEFAULT_BLOCK_SIZE, parse_payload};
use crate::container::payload::hashing::{HashingReader, verify_blob_hash, verify_partition_hash};
use crate::container::payload::manifest::{
    Extent, InstallOperation, InstallOperationType, PartitionUpdate, select_partitions,
    validate_manifest,
};
use crate::utils::{
    check_windows_case_conflict, is_case_sensitive_directory, progress, sanitize_single_component,
};
use anyhow::{Context, Result, anyhow, bail};
use bzip2::read::BzDecoder;
use liblzma::read::XzDecoder;
use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::time::Instant;
use zstd::stream::read::Decoder as ZstdDecoder;

const COPY_BUFFER_SIZE: usize = 1024 * 1024;

// 提取配置
pub struct ExtractConfig {
    // 输入 payload.bin 路径
    pub input_payload: String,
    // 输出目录
    pub output_dir: String,
    // 待提取的分区名, 为空表示提取全部分区
    pub partition_names: Vec<String>,
}

// 校验后的目标区段, 单位为字节
struct ValidatedExtent {
    offset: u64,
    size: u64,
}

// 提取 payload 中的分区镜像
pub fn extract_image(config: ExtractConfig) -> Result<()> {
    let mut payload = File::open(&config.input_payload)?;
    let payload_size = payload.metadata()?.len();
    let (manifest, data_offset) = parse_payload(&mut payload, payload_size)?;
    validate_manifest(&manifest)?;

    let partitions = select_partitions(&manifest, &config.partition_names)?;

    let output_base = PathBuf::from(&config.output_dir);
    fs::create_dir_all(&output_base)?;
    let case_sensitive = is_case_sensitive_directory(&output_base)?;
    let mut path_by_lowercase = HashMap::new();
    let block_size = u64::from(manifest.block_size.unwrap_or(DEFAULT_BLOCK_SIZE));
    let total_partitions = partitions.len();
    let start_time = Instant::now();

    log::info!("extracting {} payload partition(s)", total_partitions);

    for (index, partition) in partitions.iter().enumerate() {
        let output_name =
            sanitize_single_component(&partition.partition_name).with_context(|| {
                format!(
                    "invalid partition output name: {}",
                    partition.partition_name
                )
            })?;
        let output_rel = PathBuf::from(format!("{}.img", output_name));

        if !case_sensitive {
            check_windows_case_conflict(&mut path_by_lowercase, &output_base, &output_rel)?;
        }

        let output_path = output_base.join(&output_rel);
        extract_partition(
            &mut payload,
            payload_size,
            data_offset,
            block_size,
            partition,
            &output_path,
        )?;
        verify_partition_hash(&output_path, partition)?;
        progress::display_progress(
            output_rel.to_string_lossy().as_ref(),
            index + 1,
            total_partitions,
        );
    }

    progress::display_completion(start_time.elapsed());
    Ok(())
}

// 读取 payload 中所有分区名
pub fn list_partitions(input_payload: &str) -> Result<Vec<String>> {
    let mut payload = File::open(input_payload)?;
    let payload_size = payload.metadata()?.len();
    let (manifest, _) = parse_payload(&mut payload, payload_size)?;

    Ok(manifest
        .partitions
        .into_iter()
        .map(|partition| partition.partition_name)
        .collect())
}

// 依次回放分区的安装操作, 还原出完整分区镜像
fn extract_partition(
    payload: &mut File,
    payload_size: u64,
    data_offset: u64,
    block_size: u64,
    partition: &PartitionUpdate,
    output_path: &Path,
) -> Result<()> {
    let partition_size = partition
        .new_partition_info
        .as_ref()
        .and_then(|info| info.size)
        .ok_or_else(|| {
            anyhow!(
                "partition {} is missing its output size",
                partition.partition_name
            )
        })?;
    let mut output = File::create(output_path)?;
    output.set_len(partition_size)?;
    let zero_buffer = vec![0u8; COPY_BUFFER_SIZE];

    for (index, operation) in partition.operations.iter().enumerate() {
        let operation_type = InstallOperationType::try_from(operation.r#type).map_err(|_| {
            anyhow!(
                "partition {} operation {} has unknown type {}",
                partition.partition_name,
                index,
                operation.r#type
            )
        })?;
        let extents = validate_extents(
            partition_size,
            block_size,
            &partition.partition_name,
            index,
            &operation.dst_extents,
        )?;

        match operation_type {
            InstallOperationType::Replace
            | InstallOperationType::ReplaceBz
            | InstallOperationType::ReplaceXz
            | InstallOperationType::Zstd => extract_data_operation(
                payload,
                payload_size,
                data_offset,
                partition,
                index,
                operation,
                operation_type,
                &extents,
                &zero_buffer,
                &mut output,
            )?,
            InstallOperationType::Zero | InstallOperationType::Discard => {
                if operation.data_length.unwrap_or(0) != 0 {
                    bail!(
                        "partition {} operation {} unexpectedly contains data",
                        partition.partition_name,
                        index
                    );
                }
                write_zero_extents(&mut output, &extents, &zero_buffer)?;
            }
            _ => {
                bail!(
                    "partition {} operation {} uses incremental OTA type {}",
                    partition.partition_name,
                    index,
                    format!("{:?}", operation_type).to_uppercase()
                );
            }
        }
    }

    output.flush()?;
    Ok(())
}

// 处理携带数据的操作, 按压缩类型解码后写入目标区段
#[allow(clippy::too_many_arguments)]
fn extract_data_operation(
    payload: &mut File,
    payload_size: u64,
    data_offset: u64,
    partition: &PartitionUpdate,
    operation_index: usize,
    operation: &InstallOperation,
    operation_type: InstallOperationType,
    extents: &[ValidatedExtent],
    zero_buffer: &[u8],
    output: &mut File,
) -> Result<()> {
    let blob_offset = data_offset
        .checked_add(operation.data_offset.unwrap_or(0))
        .ok_or_else(|| anyhow!("payload data offset overflow"))?;
    let blob_size = operation.data_length.unwrap_or(0);
    let blob_end = blob_offset
        .checked_add(blob_size)
        .ok_or_else(|| anyhow!("payload data size overflow"))?;
    if blob_end > payload_size {
        bail!(
            "partition {} operation {} data exceeds payload size",
            partition.partition_name,
            operation_index
        );
    }

    payload.seek(SeekFrom::Start(blob_offset))?;
    let blob_reader = HashingReader::new(payload.take(blob_size));

    let blob_reader = match operation_type {
        InstallOperationType::Replace => {
            let mut reader = blob_reader;
            write_data_extents(&mut reader, output, extents, zero_buffer)?;
            reader
        }
        InstallOperationType::ReplaceBz => {
            let mut decoder = BzDecoder::new(blob_reader);
            write_data_extents(&mut decoder, output, extents, zero_buffer)?;
            decoder.into_inner()
        }
        InstallOperationType::ReplaceXz => {
            let mut decoder = XzDecoder::new(blob_reader);
            write_data_extents(&mut decoder, output, extents, zero_buffer)?;
            decoder.into_inner()
        }
        InstallOperationType::Zstd => {
            let mut decoder = ZstdDecoder::new(blob_reader)?;
            write_data_extents(&mut decoder, output, extents, zero_buffer)?;
            decoder.finish().into_inner()
        }
        _ => return Err(anyhow!("unsupported payload data operation")),
    };

    verify_blob_hash(
        blob_reader,
        blob_size,
        operation.data_sha256_hash.as_deref(),
        &partition.partition_name,
        operation_index,
    )
}

// 将块号区段换算为字节偏移, 同时确保不越界写入
fn validate_extents(
    partition_size: u64,
    block_size: u64,
    partition_name: &str,
    operation_index: usize,
    extents: &[Extent],
) -> Result<Vec<ValidatedExtent>> {
    if extents.is_empty() {
        bail!(
            "partition {} operation {} has no destination extents",
            partition_name,
            operation_index
        );
    }

    extents
        .iter()
        .map(|extent| {
            let start_block = extent.start_block.ok_or_else(|| {
                anyhow!(
                    "partition {} operation {} has an extent without start block",
                    partition_name,
                    operation_index
                )
            })?;
            let block_count = extent.num_blocks.ok_or_else(|| {
                anyhow!(
                    "partition {} operation {} has an extent without block count",
                    partition_name,
                    operation_index
                )
            })?;
            let offset = start_block
                .checked_mul(block_size)
                .ok_or_else(|| anyhow!("partition {} extent offset overflow", partition_name))?;
            let size = block_count
                .checked_mul(block_size)
                .ok_or_else(|| anyhow!("partition {} extent size overflow", partition_name))?;
            let end = offset
                .checked_add(size)
                .ok_or_else(|| anyhow!("partition {} extent end overflow", partition_name))?;
            if end > partition_size {
                bail!(
                    "partition {} operation {} writes beyond partition size",
                    partition_name,
                    operation_index
                );
            }

            Ok(ValidatedExtent { offset, size })
        })
        .collect()
}

// 将解码流按顺序写入各目标区段, 数据不足的部分补零
fn write_data_extents<R: Read>(
    reader: &mut R,
    output: &mut File,
    extents: &[ValidatedExtent],
    zero_buffer: &[u8],
) -> Result<()> {
    for extent in extents {
        output.seek(SeekFrom::Start(extent.offset))?;
        let mut limited = reader.by_ref().take(extent.size);
        let copied = io::copy(&mut limited, output)?;
        if copied < extent.size {
            write_zeros(output, extent.size - copied, zero_buffer)?;
        }
    }

    let mut extra = [0u8; 1];
    if reader.read(&mut extra)? != 0 {
        bail!("payload operation output exceeds destination extents");
    }

    Ok(())
}

// 将目标区段整体置零, 对应 ZERO 与 DISCARD 操作
fn write_zero_extents(
    output: &mut File,
    extents: &[ValidatedExtent],
    zero_buffer: &[u8],
) -> Result<()> {
    for extent in extents {
        output.seek(SeekFrom::Start(extent.offset))?;
        write_zeros(output, extent.size, zero_buffer)?;
    }

    Ok(())
}

// 从当前位置写入指定长度的零字节
fn write_zeros(output: &mut File, mut size: u64, zero_buffer: &[u8]) -> Result<()> {
    while size > 0 {
        let write_len = size.min(zero_buffer.len() as u64) as usize;
        output.write_all(&zero_buffer[..write_len])?;
        size -= write_len as u64;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::container::payload::format::{PAYLOAD_MAGIC, PAYLOAD_VERSION};
    use crate::container::payload::manifest::{DeltaArchiveManifest, PartitionInfo};
    use prost::Message;
    use sha2::{Digest, Sha256};
    use std::sync::atomic::{AtomicU64, Ordering};

    static TEST_ID: AtomicU64 = AtomicU64::new(0);
    const TEST_BLOCK_SIZE: u32 = 4;

    // 为单个测试分配独立的临时目录
    fn make_test_root() -> PathBuf {
        std::env::temp_dir().join(format!(
            "imgkit_payload_test_{}_{}",
            std::process::id(),
            TEST_ID.fetch_add(1, Ordering::Relaxed)
        ))
    }

    // 构造一个把整块数据写入指定区段的 REPLACE 类操作
    fn make_replace_partition(
        name: &str,
        partition_data: &[u8],
        blob: &[u8],
        operation_type: InstallOperationType,
        dst_extents: Vec<Extent>,
        data_offset: u64,
    ) -> PartitionUpdate {
        PartitionUpdate {
            partition_name: name.to_string(),
            new_partition_info: Some(PartitionInfo {
                size: Some(partition_data.len() as u64),
                hash: Some(Sha256::digest(partition_data).to_vec()),
            }),
            operations: vec![InstallOperation {
                r#type: operation_type as i32,
                data_offset: Some(data_offset),
                data_length: Some(blob.len() as u64),
                dst_extents,
                data_sha256_hash: Some(Sha256::digest(blob).to_vec()),
            }],
        }
    }

    // 单区段的常见情形
    fn make_simple_partition(
        name: &str,
        partition_data: &[u8],
        blob: &[u8],
        operation_type: InstallOperationType,
        data_offset: u64,
    ) -> PartitionUpdate {
        make_replace_partition(
            name,
            partition_data,
            blob,
            operation_type,
            vec![Extent {
                start_block: Some(0),
                num_blocks: Some(partition_data.len().div_ceil(TEST_BLOCK_SIZE as usize) as u64),
            }],
            data_offset,
        )
    }

    // 按 payload 布局写出测试文件
    fn write_payload(path: &Path, partitions: Vec<PartitionUpdate>, blobs: &[u8]) -> Result<()> {
        let manifest = DeltaArchiveManifest {
            block_size: Some(TEST_BLOCK_SIZE),
            minor_version: Some(0),
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

    #[test]
    fn extracts_multiple_destination_extents() -> Result<()> {
        let test_root = make_test_root();
        let input_path = test_root.join("payload.bin");
        let output_dir = test_root.join("output");
        fs::create_dir_all(&test_root)?;

        let blob = b"ABCDWX";
        let partition_data = b"WX\0\0ABCD";
        let partition = make_replace_partition(
            "system",
            partition_data,
            blob,
            InstallOperationType::Replace,
            vec![
                Extent {
                    start_block: Some(1),
                    num_blocks: Some(1),
                },
                Extent {
                    start_block: Some(0),
                    num_blocks: Some(1),
                },
            ],
            0,
        );
        write_payload(&input_path, vec![partition], blob)?;

        extract_image(ExtractConfig {
            input_payload: input_path.to_string_lossy().into_owned(),
            output_dir: output_dir.to_string_lossy().into_owned(),
            partition_names: Vec::new(),
        })?;

        assert_eq!(fs::read(output_dir.join("system.img"))?, partition_data);
        fs::remove_dir_all(test_root)?;
        Ok(())
    }

    #[test]
    fn extracts_zstd_operation() -> Result<()> {
        let test_root = make_test_root();
        let input_path = test_root.join("payload.bin");
        let output_dir = test_root.join("output");
        fs::create_dir_all(&test_root)?;

        let partition_data = b"OEM!";
        let blob = zstd::stream::encode_all(partition_data.as_slice(), 1)?;
        let partition = make_simple_partition(
            "vendor",
            partition_data,
            &blob,
            InstallOperationType::Zstd,
            0,
        );
        write_payload(&input_path, vec![partition], &blob)?;

        extract_image(ExtractConfig {
            input_payload: input_path.to_string_lossy().into_owned(),
            output_dir: output_dir.to_string_lossy().into_owned(),
            partition_names: Vec::new(),
        })?;

        assert_eq!(fs::read(output_dir.join("vendor.img"))?, partition_data);
        fs::remove_dir_all(test_root)?;
        Ok(())
    }

    #[test]
    fn extracts_only_requested_partition() -> Result<()> {
        let test_root = make_test_root();
        let input_path = test_root.join("payload.bin");
        let output_dir = test_root.join("output");
        fs::create_dir_all(&test_root)?;

        let system_data = b"SYS!";
        let vendor_data = b"VEN!";
        let mut blobs = Vec::new();
        blobs.extend_from_slice(system_data);
        blobs.extend_from_slice(vendor_data);

        let partitions = vec![
            make_simple_partition(
                "system",
                system_data,
                system_data,
                InstallOperationType::Replace,
                0,
            ),
            make_simple_partition(
                "vendor",
                vendor_data,
                vendor_data,
                InstallOperationType::Replace,
                system_data.len() as u64,
            ),
        ];
        write_payload(&input_path, partitions, &blobs)?;

        extract_image(ExtractConfig {
            input_payload: input_path.to_string_lossy().into_owned(),
            output_dir: output_dir.to_string_lossy().into_owned(),
            partition_names: vec!["vendor".to_string()],
        })?;

        assert_eq!(fs::read(output_dir.join("vendor.img"))?, vendor_data);
        assert!(!output_dir.join("system.img").exists());

        let names = list_partitions(input_path.to_string_lossy().as_ref())?;
        assert_eq!(names, vec!["system".to_string(), "vendor".to_string()]);

        fs::remove_dir_all(test_root)?;
        Ok(())
    }
}
