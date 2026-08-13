// LP 逻辑分区提取器, 负责解包 Super 分区镜像。

use crate::container::sparse::SparseReader;
use crate::container::super_partition::metadata::{
    LpMetadata, LpMetadataExtent, LpMetadataPartition,
};
use crate::utils::{
    check_windows_case_conflict, is_case_sensitive_directory, progress, sanitize_single_component,
};
use anyhow::{Context, Result, anyhow};
use std::fs::{self, File};
use std::io::{BufReader, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::time::Instant;

// 提取配置
pub struct ExtractConfig {
    // 输入 Super 镜像路径
    pub input_image: String,
    // 输出目录
    pub output_dir: String,
    // 待提取的分区名, 为空表示提取全部分区
    pub partition_names: Vec<String>,
}

// 提取 Super 镜像
pub fn extract_image(config: ExtractConfig) -> Result<()> {
    if let Ok(mut sparse_reader) = SparseReader::new(&config.input_image) {
        return extract_with_reader(&mut sparse_reader, config);
    }

    let file = File::open(&config.input_image)?;
    let mut buf_reader = BufReader::new(file);
    extract_with_reader(&mut buf_reader, config)
}

// 读取 Super 镜像中可提取的分区名
pub fn list_partitions(input_image: &str) -> Result<Vec<String>> {
    let metadata = if let Ok(mut sparse_reader) = SparseReader::new(input_image) {
        LpMetadata::from_reader(&mut sparse_reader)
    } else {
        let mut buf_reader = BufReader::new(File::open(input_image)?);
        LpMetadata::from_reader(&mut buf_reader)
    }
    .context("failed to parse LP metadata")?;

    Ok(extractable_partitions(&metadata)
        .into_iter()
        .map(|partition| partition.name.clone())
        .collect())
}

// 去掉分区名的槽位后缀
fn strip_slot_suffix(name: &str) -> &str {
    name.strip_suffix("_a").unwrap_or(name)
}

// 判断分区名是否匹配用户输入, 允许带或不带 _a 后缀
fn matches_name(selected: &str, partition_name: &str) -> bool {
    selected == partition_name || strip_slot_suffix(selected) == strip_slot_suffix(partition_name)
}

// 使用给定读取器提取镜像
fn extract_with_reader<R: Read + Seek>(reader: &mut R, config: ExtractConfig) -> Result<()> {
    let metadata = LpMetadata::from_reader(reader).context("failed to parse LP metadata")?;

    let output_base = PathBuf::from(&config.output_dir);
    fs::create_dir_all(&output_base)?;
    let case_sensitive = is_case_sensitive_directory(&output_base)?;
    let mut path_by_lowercase = std::collections::HashMap::new();

    let mut partitions_to_extract = extractable_partitions(&metadata);

    if !config.partition_names.is_empty() {
        for name in &config.partition_names {
            if !partitions_to_extract
                .iter()
                .any(|partition| matches_name(name, &partition.name))
            {
                return Err(anyhow!(
                    "partition {} not found in super image, available: {}",
                    name,
                    available_partition_names(&partitions_to_extract)
                ));
            }
        }
        partitions_to_extract.retain(|partition| {
            config
                .partition_names
                .iter()
                .any(|name| matches_name(name, &partition.name))
        });
    }

    let total_partitions = partitions_to_extract.len();
    log::info!("extracting {} partition(s)", total_partitions);

    let start_time = Instant::now();

    for (index, partition) in partitions_to_extract.iter().enumerate() {
        let extents = metadata.get_partition_extents(partition);

        let output_name = strip_slot_suffix(&partition.name);
        let output_name = sanitize_single_component(output_name)
            .with_context(|| format!("invalid partition output name: {}", output_name))?;

        let output_rel = PathBuf::from(format!("{}.img", output_name));
        if !case_sensitive {
            check_windows_case_conflict(&mut path_by_lowercase, &output_base, &output_rel)?;
        }
        let partition_output = output_base.join(&output_rel);

        extract_partition(
            reader,
            &partition_output,
            &extents,
            index + 1,
            total_partitions,
        )?;
    }

    progress::display_completion(start_time.elapsed());
    Ok(())
}

// 筛选出可提取的分区, 仅保留 A 槽且非空的分区
fn extractable_partitions(metadata: &LpMetadata) -> Vec<&LpMetadataPartition> {
    metadata
        .partitions
        .iter()
        .filter(|partition| !partition.name.ends_with("_b"))
        .filter(|partition| {
            metadata
                .get_partition_extents(partition)
                .iter()
                .map(|extent| extent.num_sectors)
                .sum::<u64>()
                > 0
        })
        .collect()
}

// 拼接可用分区名, 用于错误提示
fn available_partition_names(partitions: &[&LpMetadataPartition]) -> String {
    partitions
        .iter()
        .map(|partition| partition.name.as_str())
        .collect::<Vec<_>>()
        .join(", ")
}

// 提取单个分区的数据
fn extract_partition<R: Read + Seek>(
    reader: &mut R,
    output_path: &Path,
    extents: &[&LpMetadataExtent],
    current_partition: usize,
    total_partitions: usize,
) -> Result<()> {
    const LP_TARGET_TYPE_LINEAR: u32 = 0;
    const LP_TARGET_TYPE_ZERO: u32 = 1;

    let mut output = File::create(output_path)
        .with_context(|| format!("failed to create output file: {:?}", output_path))?;

    let filename = output_path
        .file_name()
        .map(|v| v.to_string_lossy().to_string())
        .unwrap_or_else(|| output_path.display().to_string());
    let mut buffer = vec![0u8; 1024 * 1024]; // 1 MB buffer

    for extent in extents {
        let size = extent
            .num_sectors
            .checked_mul(512)
            .context("extent size overflow")?;

        match extent.target_type {
            LP_TARGET_TYPE_LINEAR => {
                let offset = extent
                    .target_data
                    .checked_mul(512)
                    .context("extent offset overflow")?;
                reader.seek(SeekFrom::Start(offset))?;

                let mut remaining = size;
                while remaining > 0 {
                    let to_read = std::cmp::min(remaining, buffer.len() as u64) as usize;
                    reader.read_exact(&mut buffer[..to_read])?;
                    output.write_all(&buffer[..to_read])?;
                    remaining -= to_read as u64;
                }
            }
            LP_TARGET_TYPE_ZERO => {
                let mut remaining = size;
                buffer.fill(0);
                while remaining > 0 {
                    let to_write = std::cmp::min(remaining, buffer.len() as u64) as usize;
                    output.write_all(&buffer[..to_write])?;
                    remaining -= to_write as u64;
                }
            }
            _ => anyhow::bail!("unsupported extent target type: {}", extent.target_type),
        }
    }

    // 显示进度
    progress::display_progress(filename.as_str(), current_partition, total_partitions);

    Ok(())
}
