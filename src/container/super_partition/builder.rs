// LP (Logical Partition) 元数据构建器
//
// 参考 Android 源码 liblp/builder.cpp

use crate::container::super_partition::format::*;
use anyhow::{Result, anyhow};

// block device 信息 (作为 builder 的输入)
#[derive(Debug, Clone)]
pub struct BlockDeviceInfo {
    pub partition_name: String,
    pub size: u64,
    pub alignment: u32,
    pub alignment_offset: u32,
    pub logical_block_size: u32,
}

impl BlockDeviceInfo {
    pub fn new(name: &str, size: u64) -> Self {
        Self {
            partition_name: name.to_string(),
            size,
            alignment: DEFAULT_PARTITION_ALIGNMENT,
            alignment_offset: 0,
            logical_block_size: DEFAULT_BLOCK_SIZE,
        }
    }

    pub fn with_alignment(mut self, alignment: u32, alignment_offset: u32) -> Self {
        self.alignment = alignment;
        self.alignment_offset = alignment_offset;
        self
    }

    pub fn with_block_size(mut self, block_size: u32) -> Self {
        self.logical_block_size = block_size;
        self
    }
}

// 分区信息 (作为 builder 的输入)
#[derive(Debug, Clone)]
pub struct PartitionInfo {
    pub name: String,
    pub group_name: String,
    pub attributes: u32,
    pub size: u64,
}

impl PartitionInfo {
    pub fn new(name: &str, group_name: &str, size: u64) -> Self {
        Self {
            name: name.to_string(),
            group_name: group_name.to_string(),
            attributes: LP_PARTITION_ATTR_NONE,
            size,
        }
    }

    pub fn readonly(mut self) -> Self {
        self.attributes |= LP_PARTITION_ATTR_READONLY;
        self
    }
}

// group 信息 (作为 builder 的输入)
#[derive(Debug, Clone)]
pub struct GroupInfo {
    pub name: String,
    pub maximum_size: u64,
}

impl GroupInfo {
    pub fn new(name: &str, maximum_size: u64) -> Self {
        Self {
            name: name.to_string(),
            maximum_size,
        }
    }
}

// LP metadata 构建器
pub struct MetadataBuilder {
    block_devices: Vec<BlockDeviceInfo>,
    groups: Vec<GroupInfo>,
    partitions: Vec<PartitionInfo>,
    metadata_max_size: u32,
    metadata_slot_count: u32,
    auto_slot_suffixing: bool,
    virtual_ab: bool,
}

impl MetadataBuilder {
    // 创建新的构建器
    pub fn new(
        block_devices: Vec<BlockDeviceInfo>,
        metadata_max_size: u32,
        metadata_slot_count: u32,
    ) -> Result<Self> {
        if block_devices.is_empty() {
            return Err(anyhow!("at least one block device is required"));
        }
        if metadata_max_size == 0 {
            return Err(anyhow!("metadata_max_size must be greater than 0"));
        }
        if metadata_slot_count == 0 {
            return Err(anyhow!("metadata_slot_count must be greater than 0"));
        }

        let mut builder = Self {
            block_devices,
            groups: Vec::new(),
            partitions: Vec::new(),
            metadata_max_size,
            metadata_slot_count,
            auto_slot_suffixing: false,
            virtual_ab: false,
        };

        // 添加默认 group
        builder.add_group(GroupInfo::new("default", 0))?;

        Ok(builder)
    }

    // 启用自动 slot 后缀
    pub fn set_auto_slot_suffixing(&mut self) {
        self.auto_slot_suffixing = true;
    }

    // 设置 Virtual A/B 标志位
    pub fn set_virtual_ab_device_flag(&mut self) {
        self.virtual_ab = true;
    }

    // 添加 group
    pub fn add_group(&mut self, group: GroupInfo) -> Result<()> {
        if self.groups.iter().any(|g| g.name == group.name) {
            return Err(anyhow!("partition group {} already exists", group.name));
        }
        self.groups.push(group);
        Ok(())
    }

    // 添加分区
    pub fn add_partition(&mut self, partition: PartitionInfo) -> Result<()> {
        // 检查分区名是否已存在
        if self.partitions.iter().any(|p| p.name == partition.name) {
            return Err(anyhow!("partition {} already exists", partition.name));
        }

        // 检查所属 group 是否存在
        if !self.groups.iter().any(|g| g.name == partition.group_name) {
            return Err(anyhow!(
                "partition group {} does not exist",
                partition.group_name
            ));
        }

        self.partitions.push(partition);
        Ok(())
    }

    // 计算首个逻辑 sector 的位置
    fn calculate_first_logical_sector(&self) -> u64 {
        let reserved = LP_PARTITION_RESERVED_BYTES;
        let geometry_size = LP_METADATA_GEOMETRY_SIZE * 2; // 主备各一份
        let metadata_size = self.metadata_max_size as u64 * self.metadata_slot_count as u64 * 2;

        let total = reserved + geometry_size + metadata_size;

        // 对齐到 block device 的 alignment 边界
        let alignment = self.block_devices[0].alignment as u64;
        let aligned = total.div_ceil(alignment) * alignment;

        aligned / LP_SECTOR_SIZE
    }

    // 导出 metadata
    pub fn export(&self) -> Result<LpMetadata> {
        let first_logical_sector = self.calculate_first_logical_sector();
        let logical_block_size = self.block_devices[0].logical_block_size;

        // 构建 block device 表
        let mut block_device_entries = Vec::new();
        for (i, bd) in self.block_devices.iter().enumerate() {
            let mut device = LpMetadataBlockDevice::new(&bd.partition_name, bd.size);
            device.first_logical_sector = if i == 0 { first_logical_sector } else { 0 };
            device.alignment = bd.alignment;
            device.alignment_offset = bd.alignment_offset;
            if self.auto_slot_suffixing {
                device.flags |= LP_BLOCK_DEVICE_SLOT_SUFFIXED;
            }
            block_device_entries.push(device);
        }

        // 构建 group 表
        let mut group_entries = Vec::new();
        for group in &self.groups {
            let mut g = LpMetadataPartitionGroup::new(&group.name, group.maximum_size);
            if self.auto_slot_suffixing && group.name != "default" {
                g.flags |= LP_GROUP_SLOT_SUFFIXED;
            }
            group_entries.push(g);
        }

        // 构建 partition table 与 extent 表
        let mut partition_entries = Vec::new();
        let mut extent_entries = Vec::new();
        let mut current_sector = first_logical_sector;
        let alignment = self.block_devices[0].alignment as u64;
        let alignment_sectors = alignment / LP_SECTOR_SIZE;

        for partition in &self.partitions {
            // 查找所属 group 的索引
            let group_index = self
                .groups
                .iter()
                .position(|g| g.name == partition.group_name)
                .ok_or_else(|| anyhow!("partition group {} not found", partition.group_name))?
                as u32;

            let num_sectors = partition.size / LP_SECTOR_SIZE;
            let first_extent_index = extent_entries.len() as u32;

            // 对齐当前 sector
            if !current_sector.is_multiple_of(alignment_sectors) {
                current_sector = current_sector.div_ceil(alignment_sectors) * alignment_sectors;
            }

            // 创建 extent
            let num_extents = if num_sectors > 0 {
                extent_entries.push(LpMetadataExtent::new_linear(num_sectors, current_sector, 0));
                current_sector += num_sectors;
                1
            } else {
                0
            };

            let mut p = LpMetadataPartition::new(&partition.name);
            p.attributes = partition.attributes;
            if self.auto_slot_suffixing {
                p.attributes |= LP_PARTITION_ATTR_SLOT_SUFFIXED;
            }
            p.first_extent_index = first_extent_index;
            p.num_extents = num_extents;
            p.group_index = group_index;
            partition_entries.push(p);
        }

        // 构建 geometry 信息
        let geometry = LpMetadataGeometry {
            magic: LP_METADATA_GEOMETRY_MAGIC,
            struct_size: LP_METADATA_GEOMETRY_STRUCT_SIZE,
            checksum: [0u8; 32],
            metadata_max_size: self.metadata_max_size,
            metadata_slot_count: self.metadata_slot_count,
            logical_block_size,
        };

        // 构建 metadata header
        let flags = if self.virtual_ab {
            LP_HEADER_FLAG_VIRTUAL_AB_DEVICE
        } else {
            0
        };

        // 依据是否需要扩展 header 选择版本
        let (header_size, minor_version) = if self.virtual_ab {
            (LP_METADATA_HEADER_V1_2_SIZE, LP_METADATA_MINOR_VERSION_MAX)
        } else {
            (LP_METADATA_HEADER_V1_0_SIZE, LP_METADATA_MINOR_VERSION_MIN)
        };

        let header = LpMetadataHeader {
            magic: LP_METADATA_HEADER_MAGIC,
            major_version: LP_METADATA_MAJOR_VERSION,
            minor_version,
            header_size,
            header_checksum: [0u8; 32],
            tables_size: 0,
            tables_checksum: [0u8; 32],
            partitions: LpMetadataTableDescriptor::default(),
            extents: LpMetadataTableDescriptor::default(),
            groups: LpMetadataTableDescriptor::default(),
            block_devices: LpMetadataTableDescriptor::default(),
            flags,
        };

        Ok(LpMetadata {
            geometry,
            header,
            partitions: partition_entries,
            extents: extent_entries,
            groups: group_entries,
            block_devices: block_device_entries,
        })
    }
}

// 辅助函数: 获取文件大小
pub fn get_file_size(path: &str) -> Result<u64> {
    let metadata =
        std::fs::metadata(path).map_err(|e| anyhow!("failed to get file size: {}: {}", path, e))?;
    Ok(metadata.len())
}

// 辅助函数: 按指定粒度对齐大小
pub fn align_to(value: u64, alignment: u64) -> u64 {
    if alignment == 0 {
        return value;
    }
    value.div_ceil(alignment) * alignment
}
