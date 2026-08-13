// OTA payload 的 manifest 定义与筛选。
//
// 以下结构对应 Android update_engine 的 update_metadata.proto,
// 仅声明全量包提取所需的字段, 其余字段由 prost 自动跳过。

use anyhow::{Result, anyhow, bail};
use prost::{Enumeration, Message};

#[derive(Clone, PartialEq, Message)]
pub struct DeltaArchiveManifest {
    #[prost(uint32, optional, tag = "3")]
    pub block_size: Option<u32>,
    #[prost(uint32, optional, tag = "12")]
    pub minor_version: Option<u32>,
    #[prost(message, repeated, tag = "13")]
    pub partitions: Vec<PartitionUpdate>,
}

#[derive(Clone, PartialEq, Message)]
pub struct PartitionUpdate {
    #[prost(string, required, tag = "1")]
    pub partition_name: String,
    #[prost(message, optional, tag = "7")]
    pub new_partition_info: Option<PartitionInfo>,
    #[prost(message, repeated, tag = "8")]
    pub operations: Vec<InstallOperation>,
}

#[derive(Clone, PartialEq, Message)]
pub struct PartitionInfo {
    #[prost(uint64, optional, tag = "1")]
    pub size: Option<u64>,
    #[prost(bytes = "vec", optional, tag = "2")]
    pub hash: Option<Vec<u8>>,
}

#[derive(Clone, PartialEq, Message)]
pub struct InstallOperation {
    #[prost(enumeration = "InstallOperationType", required, tag = "1")]
    pub r#type: i32,
    #[prost(uint64, optional, tag = "2")]
    pub data_offset: Option<u64>,
    #[prost(uint64, optional, tag = "3")]
    pub data_length: Option<u64>,
    #[prost(message, repeated, tag = "6")]
    pub dst_extents: Vec<Extent>,
    #[prost(bytes = "vec", optional, tag = "8")]
    pub data_sha256_hash: Option<Vec<u8>>,
}

#[derive(Clone, Copy, PartialEq, Message)]
pub struct Extent {
    #[prost(uint64, optional, tag = "1")]
    pub start_block: Option<u64>,
    #[prost(uint64, optional, tag = "2")]
    pub num_blocks: Option<u64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Enumeration)]
#[repr(i32)]
pub enum InstallOperationType {
    Replace = 0,
    ReplaceBz = 1,
    Move = 2,
    Bsdiff = 3,
    SourceCopy = 4,
    SourceBsdiff = 5,
    Zero = 6,
    Discard = 7,
    ReplaceXz = 8,
    Puffdiff = 9,
    BrotliBsdiff = 10,
    Zucchini = 11,
    Lz4diffBsdiff = 12,
    Lz4diffPuffdiff = 13,
    Zstd = 14,
}

// 校验 manifest 是否为受支持的全量包
pub fn validate_manifest(manifest: &DeltaArchiveManifest) -> Result<()> {
    let minor_version = manifest.minor_version.unwrap_or(0);
    if minor_version != 0 {
        bail!(
            "incremental OTA payload is not supported: minor version {} requires source images",
            minor_version
        );
    }
    if manifest.partitions.is_empty() {
        bail!("OTA payload manifest contains no partitions");
    }
    if manifest.block_size == Some(0) {
        bail!("OTA payload block size is zero");
    }

    Ok(())
}

// 按名称筛选待提取分区, 未指定名称时返回全部
pub fn select_partitions<'a>(
    manifest: &'a DeltaArchiveManifest,
    partition_names: &[String],
) -> Result<Vec<&'a PartitionUpdate>> {
    if partition_names.is_empty() {
        return Ok(manifest.partitions.iter().collect());
    }

    let mut selected = Vec::with_capacity(partition_names.len());
    for name in partition_names {
        let partition = manifest
            .partitions
            .iter()
            .find(|partition| &partition.partition_name == name)
            .ok_or_else(|| {
                anyhow!(
                    "partition {} not found in payload, available: {}",
                    name,
                    available_partition_names(manifest)
                )
            })?;

        // 同名分区重复指定时只提取一次
        if !selected
            .iter()
            .any(|existing: &&PartitionUpdate| existing.partition_name == partition.partition_name)
        {
            selected.push(partition);
        }
    }

    Ok(selected)
}

// 拼接可用分区名, 用于错误提示
fn available_partition_names(manifest: &DeltaArchiveManifest) -> String {
    manifest
        .partitions
        .iter()
        .map(|partition| partition.partition_name.as_str())
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_incremental_payload() {
        let manifest = DeltaArchiveManifest {
            block_size: Some(4096),
            minor_version: Some(1),
            partitions: Vec::new(),
        };

        let error = validate_manifest(&manifest).err();
        assert!(error.is_some_and(|err| err.to_string().contains("incremental OTA")));
    }

    #[test]
    fn rejects_unknown_partition_name() {
        let manifest = DeltaArchiveManifest {
            block_size: Some(4096),
            minor_version: Some(0),
            partitions: vec![PartitionUpdate {
                partition_name: "system".to_string(),
                new_partition_info: None,
                operations: Vec::new(),
            }],
        };

        let error = select_partitions(&manifest, &["odm".to_string()]).err();
        assert!(error.is_some_and(|err| err.to_string().contains("available: system")));
    }
}
