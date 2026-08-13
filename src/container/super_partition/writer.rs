// LP (Logical Partition) 元数据写入器
//
// 参考 Android 源码 liblp/writer.cpp

use crate::container::sparse::SparseWriter;
use crate::container::super_partition::format::*;
use anyhow::{Context, Result};
use sha2::{Digest, Sha256};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;

// 计算 SHA256 校验和
pub fn sha256(data: &[u8]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(data);
    let result = hasher.finalize();
    let mut checksum = [0u8; 32];
    checksum.copy_from_slice(&result);
    checksum
}

// 序列化 geometry 信息
pub fn serialize_geometry(geometry: &LpMetadataGeometry) -> Vec<u8> {
    let mut geo = geometry.clone();
    geo.checksum = [0u8; 32];
    let data = geo.to_bytes();
    geo.checksum = sha256(&data[..LP_METADATA_GEOMETRY_STRUCT_SIZE as usize]);
    geo.to_bytes()
}

// 序列化 metadata
pub fn serialize_metadata(metadata: &LpMetadata) -> Vec<u8> {
    // 分别序列化各张表
    let partitions_data: Vec<u8> = metadata
        .partitions
        .iter()
        .flat_map(|p| p.to_bytes())
        .collect();
    let extents_data: Vec<u8> = metadata.extents.iter().flat_map(|e| e.to_bytes()).collect();
    let groups_data: Vec<u8> = metadata.groups.iter().flat_map(|g| g.to_bytes()).collect();
    let block_devices_data: Vec<u8> = metadata
        .block_devices
        .iter()
        .flat_map(|b| b.to_bytes())
        .collect();

    // 计算各张表的偏移
    let partitions_offset = 0u32;
    let extents_offset = partitions_offset + partitions_data.len() as u32;
    let groups_offset = extents_offset + extents_data.len() as u32;
    let block_devices_offset = groups_offset + groups_data.len() as u32;
    let tables_size = block_devices_offset + block_devices_data.len() as u32;

    // 合并表数据
    let mut tables_data = Vec::new();
    tables_data.extend(&partitions_data);
    tables_data.extend(&extents_data);
    tables_data.extend(&groups_data);
    tables_data.extend(&block_devices_data);

    // 计算表数据的校验和
    let tables_checksum = sha256(&tables_data);

    // 构建 metadata header
    let mut header = metadata.header.clone();
    header.partitions.offset = partitions_offset;
    header.partitions.num_entries = metadata.partitions.len() as u32;
    header.partitions.entry_size = LP_METADATA_PARTITION_SIZE;
    header.extents.offset = extents_offset;
    header.extents.num_entries = metadata.extents.len() as u32;
    header.extents.entry_size = LP_METADATA_EXTENT_SIZE;
    header.groups.offset = groups_offset;
    header.groups.num_entries = metadata.groups.len() as u32;
    header.groups.entry_size = LP_METADATA_GROUP_SIZE;
    header.block_devices.offset = block_devices_offset;
    header.block_devices.num_entries = metadata.block_devices.len() as u32;
    header.block_devices.entry_size = LP_METADATA_BLOCK_DEVICE_SIZE;
    header.tables_size = tables_size;
    header.tables_checksum = tables_checksum;

    // 计算 header 校验和
    header.header_checksum = [0u8; 32];
    let header_bytes = header.to_bytes();
    header.header_checksum = sha256(&header_bytes);

    // 返回 header 加表数据
    let mut result = header.to_bytes();
    result.extend(tables_data);
    result
}

// 获取主 geometry 的偏移
pub fn get_primary_geometry_offset() -> u64 {
    LP_PARTITION_RESERVED_BYTES
}

// 获取备份 geometry 的偏移
pub fn get_backup_geometry_offset() -> u64 {
    get_primary_geometry_offset() + LP_METADATA_GEOMETRY_SIZE
}

// 获取主 metadata 的偏移
pub fn get_primary_metadata_offset(geometry: &LpMetadataGeometry, slot_number: u32) -> u64 {
    LP_PARTITION_RESERVED_BYTES
        + LP_METADATA_GEOMETRY_SIZE * 2
        + geometry.metadata_max_size as u64 * slot_number as u64
}

// 获取备份 metadata 的偏移
pub fn get_backup_metadata_offset(geometry: &LpMetadataGeometry, slot_number: u32) -> u64 {
    let start = LP_PARTITION_RESERVED_BYTES
        + LP_METADATA_GEOMETRY_SIZE * 2
        + geometry.metadata_max_size as u64 * geometry.metadata_slot_count as u64;
    start + geometry.metadata_max_size as u64 * slot_number as u64
}

// 获取 metadata 区域的总大小
pub fn get_total_metadata_size(metadata_max_size: u32, max_slots: u32) -> u64 {
    LP_PARTITION_RESERVED_BYTES
        + (LP_METADATA_GEOMETRY_SIZE + metadata_max_size as u64 * max_slots as u64) * 2
}

// 写出空镜像文件 (仅含 metadata, 与 lpmake 输出格式兼容)
pub fn write_empty_image(path: &Path, metadata: &LpMetadata) -> Result<()> {
    let mut file = File::create(path).with_context(|| format!("failed to create: {:?}", path))?;

    // 空镜像格式: geometry 信息加 metadata (不含保留区)
    let geometry_data = serialize_geometry(&metadata.geometry);
    file.write_all(&geometry_data)?;

    // 写入 metadata
    let metadata_blob = serialize_metadata(metadata);
    file.write_all(&metadata_blob)?;

    Ok(())
}

// 写出完整镜像文件 (仅含 metadata, 用于刷入设备)
pub fn write_to_image_file(path: &Path, metadata: &LpMetadata) -> Result<()> {
    let mut file = File::create(path).with_context(|| format!("failed to create: {:?}", path))?;

    // 写入保留区 (全零)
    let reserved = vec![0u8; LP_PARTITION_RESERVED_BYTES as usize];
    file.write_all(&reserved)?;

    // 写入主 geometry 信息
    let geometry_data = serialize_geometry(&metadata.geometry);
    file.write_all(&geometry_data)?;

    // 写入备份 geometry 信息
    file.write_all(&geometry_data)?;

    // 写入 metadata
    let metadata_blob = serialize_metadata(metadata);
    let mut padded_metadata = metadata_blob.clone();
    padded_metadata.resize(metadata.geometry.metadata_max_size as usize, 0);

    // 逐个 slot 写入主 metadata
    for _ in 0..metadata.geometry.metadata_slot_count {
        file.write_all(&padded_metadata)?;
    }

    // 写入备份 metadata
    for _ in 0..metadata.geometry.metadata_slot_count {
        file.write_all(&padded_metadata)?;
    }

    Ok(())
}

// 写出完整镜像文件 (包含分区数据)
pub fn write_to_image_file_with_data(
    path: &Path,
    metadata: &LpMetadata,
    images: &std::collections::HashMap<String, String>,
) -> Result<()> {
    if metadata.block_devices.is_empty() {
        anyhow::bail!("no block devices");
    }

    let device_size = metadata.block_devices[0].size;
    let mut file = File::create(path).with_context(|| format!("failed to create: {:?}", path))?;

    // 预分配文件大小
    file.set_len(device_size)?;

    // 写入保留区 (全零)
    file.seek(SeekFrom::Start(0))?;
    let reserved = vec![0u8; LP_PARTITION_RESERVED_BYTES as usize];
    file.write_all(&reserved)?;

    // 写入主 geometry 信息
    let geometry_data = serialize_geometry(&metadata.geometry);
    file.write_all(&geometry_data)?;

    // 写入备份 geometry 信息
    file.write_all(&geometry_data)?;

    // 写入 metadata
    let metadata_blob = serialize_metadata(metadata);
    let mut padded_metadata = metadata_blob.clone();
    padded_metadata.resize(metadata.geometry.metadata_max_size as usize, 0);

    for _ in 0..metadata.geometry.metadata_slot_count {
        file.write_all(&padded_metadata)?;
    }

    for _ in 0..metadata.geometry.metadata_slot_count {
        file.write_all(&padded_metadata)?;
    }

    // 写入分区数据
    for partition in &metadata.partitions {
        let partition_name = partition.get_name();
        if let Some(image_path) = images.get(&partition_name) {
            for i in 0..partition.num_extents {
                let extent_idx = (partition.first_extent_index + i) as usize;
                if extent_idx >= metadata.extents.len() {
                    continue;
                }
                let extent = &metadata.extents[extent_idx];
                if extent.target_type != LP_TARGET_TYPE_LINEAR {
                    continue;
                }

                let offset = extent.target_data * LP_SECTOR_SIZE;
                let extent_size = extent.num_sectors * LP_SECTOR_SIZE;

                let mut src = File::open(image_path)
                    .with_context(|| format!("failed to open image file: {}", image_path))?;

                file.seek(SeekFrom::Start(offset))?;

                let mut buffer = vec![0u8; 1024 * 1024];
                let mut remaining = extent_size;
                while remaining > 0 {
                    let to_read = std::cmp::min(buffer.len() as u64, remaining) as usize;
                    let bytes_read = src.read(&mut buffer[..to_read])?;
                    if bytes_read == 0 {
                        // 文件已读完, 剩余部分已是零 (文件为预分配)
                        break;
                    }
                    file.write_all(&buffer[..bytes_read])?;
                    remaining -= bytes_read as u64;
                }
            }
        }
    }

    Ok(())
}

// 构建 metadata 区域 (保留区加 geometry 信息加 metadata)
fn build_metadata_region(metadata: &LpMetadata) -> Vec<u8> {
    let mut data = Vec::new();

    // 保留区
    data.extend(vec![0u8; LP_PARTITION_RESERVED_BYTES as usize]);

    // 主 geometry 信息
    let geometry_data = serialize_geometry(&metadata.geometry);
    data.extend(&geometry_data);

    // 备份 geometry 信息
    data.extend(&geometry_data);

    // metadata 数据
    let metadata_blob = serialize_metadata(metadata);
    let mut padded_metadata = metadata_blob.clone();
    padded_metadata.resize(metadata.geometry.metadata_max_size as usize, 0);

    // 主 metadata
    for _ in 0..metadata.geometry.metadata_slot_count {
        data.extend(&padded_metadata);
    }

    // 备份 metadata
    for _ in 0..metadata.geometry.metadata_slot_count {
        data.extend(&padded_metadata);
    }

    data
}

// 写出 sparse image 空镜像 (仅含 metadata)
pub fn write_sparse_empty_image(path: &Path, metadata: &LpMetadata, block_size: u32) -> Result<()> {
    if metadata.block_devices.is_empty() {
        anyhow::bail!("no block devices");
    }

    let device_size = metadata.block_devices[0].size;
    let total_blocks = (device_size / block_size as u64) as u32;

    // 构建 metadata 区域
    let metadata_data = build_metadata_region(metadata);
    let metadata_blocks = (metadata_data.len() as u64).div_ceil(block_size as u64) as u32;

    // 计算剩余空间
    let remaining_blocks = total_blocks - metadata_blocks;

    let mut writer = SparseWriter::new(path, block_size, total_blocks)?;

    // 添加 metadata 的 raw chunk
    writer.add_raw_chunk(metadata_data);

    // 剩余空间使用 dont-care chunk 填充
    if remaining_blocks > 0 {
        writer.add_dont_care_chunk(remaining_blocks);
    }

    writer.write()
}

// 写出 sparse image (包含分区数据)
pub fn write_to_sparse_image_file_with_data(
    path: &Path,
    metadata: &LpMetadata,
    images: &std::collections::HashMap<String, String>,
    block_size: u32,
) -> Result<()> {
    if metadata.block_devices.is_empty() {
        anyhow::bail!("no block devices");
    }

    let device_size = metadata.block_devices[0].size;
    let total_blocks = (device_size / block_size as u64) as u32;

    // 收集分区数据块信息
    struct PartitionBlock {
        offset: u64,
        size: u64,
        image_path: String,
    }

    let mut partition_blocks: Vec<PartitionBlock> = Vec::new();

    for partition in &metadata.partitions {
        let partition_name = partition.get_name();
        if let Some(image_path) = images.get(&partition_name) {
            for i in 0..partition.num_extents {
                let extent_idx = (partition.first_extent_index + i) as usize;
                if extent_idx >= metadata.extents.len() {
                    continue;
                }
                let extent = &metadata.extents[extent_idx];
                if extent.target_type != LP_TARGET_TYPE_LINEAR {
                    continue;
                }

                partition_blocks.push(PartitionBlock {
                    offset: extent.target_data * LP_SECTOR_SIZE,
                    size: extent.num_sectors * LP_SECTOR_SIZE,
                    image_path: image_path.clone(),
                });
            }
        }
    }

    // 按偏移排序
    partition_blocks.sort_by_key(|b| b.offset);

    // 构建 metadata 区域
    let metadata_data = build_metadata_region(metadata);
    let metadata_size = metadata_data.len() as u64;
    let metadata_aligned_size = metadata_size.div_ceil(block_size as u64) * block_size as u64;

    let mut writer = SparseWriter::new(path, block_size, total_blocks)?;

    // 添加 metadata 的 raw chunk
    writer.add_raw_chunk(metadata_data);

    let mut current_offset = metadata_aligned_size;

    // 处理分区数据块
    for block in &partition_blocks {
        // 填补空洞
        if block.offset > current_offset {
            let gap_blocks = ((block.offset - current_offset) / block_size as u64) as u32;
            if gap_blocks > 0 {
                writer.add_dont_care_chunk(gap_blocks);
            }
        }

        // 添加分区数据
        writer.add_file_chunk(&block.image_path, block.size);
        current_offset = block.offset + block.size;
    }

    // 补齐到设备末尾
    if current_offset < device_size {
        let remaining_blocks = ((device_size - current_offset) / block_size as u64) as u32;
        if remaining_blocks > 0 {
            writer.add_dont_care_chunk(remaining_blocks);
        }
    }

    writer.write()
}
