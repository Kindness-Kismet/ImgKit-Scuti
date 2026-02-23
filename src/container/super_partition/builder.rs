// LP (Logical Partition) metadata builder
//
// Reference Android source code liblp/builder.cpp

use crate::container::super_partition::format::*;
use anyhow::{Result, anyhow};

// Block device information (for builder input)
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

// Partition information (for builder input)
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

// Section group information (for builder input)
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

// LP metadata builder
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
    // Create new builder
    pub fn new(
        block_devices: Vec<BlockDeviceInfo>,
        metadata_max_size: u32,
        metadata_slot_count: u32,
    ) -> Result<Self> {
        if block_devices.is_empty() {
            return Err(anyhow!("至少需要一个块设备"));
        }
        if metadata_max_size == 0 {
            return Err(anyhow!("metadata_max_size 必须大于 0"));
        }
        if metadata_slot_count == 0 {
            return Err(anyhow!("metadata_slot_count 必须大于 0"));
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

        // Add default section group
        builder.add_group(GroupInfo::new("default", 0))?;

        Ok(builder)
    }

    // Set automatic slot suffix
    pub fn set_auto_slot_suffixing(&mut self) {
        self.auto_slot_suffixing = true;
    }

    // Set Virtual A/B flag
    pub fn set_virtual_ab_device_flag(&mut self) {
        self.virtual_ab = true;
    }

    // Add section group
    pub fn add_group(&mut self, group: GroupInfo) -> Result<()> {
        if self.groups.iter().any(|g| g.name == group.name) {
            return Err(anyhow!("分区组 {} 已存在", group.name));
        }
        self.groups.push(group);
        Ok(())
    }

    // Add partition
    pub fn add_partition(&mut self, partition: PartitionInfo) -> Result<()> {
        // Check if the partition name already exists
        if self.partitions.iter().any(|p| p.name == partition.name) {
            return Err(anyhow!("分区 {} 已存在", partition.name));
        }

        // Check if the partition group exists
        if !self.groups.iter().any(|g| g.name == partition.group_name) {
            return Err(anyhow!("分区组 {} 不存在", partition.group_name));
        }

        self.partitions.push(partition);
        Ok(())
    }

    // Calculate the position of the first logical sector
    fn calculate_first_logical_sector(&self) -> u64 {
        let reserved = LP_PARTITION_RESERVED_BYTES;
        let geometry_size = LP_METADATA_GEOMETRY_SIZE * 2; // Two primary and backup copies
        let metadata_size = self.metadata_max_size as u64 * self.metadata_slot_count as u64 * 2;

        let total = reserved + geometry_size + metadata_size;

        // Align to the alignment boundaries of the block device
        let alignment = self.block_devices[0].alignment as u64;
        let aligned = total.div_ceil(alignment) * alignment;

        aligned / LP_SECTOR_SIZE
    }

    // Export metadata
    pub fn export(&self) -> Result<LpMetadata> {
        let first_logical_sector = self.calculate_first_logical_sector();
        let logical_block_size = self.block_devices[0].logical_block_size;

        // Building block device table
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

        // Build partition group table
        let mut group_entries = Vec::new();
        for group in &self.groups {
            let mut g = LpMetadataPartitionGroup::new(&group.name, group.maximum_size);
            if self.auto_slot_suffixing && group.name != "default" {
                g.flags |= LP_GROUP_SLOT_SUFFIXED;
            }
            group_entries.push(g);
        }

        // Build partition tables and extent tables
        let mut partition_entries = Vec::new();
        let mut extent_entries = Vec::new();
        let mut current_sector = first_logical_sector;
        let alignment = self.block_devices[0].alignment as u64;
        let alignment_sectors = alignment / LP_SECTOR_SIZE;

        for partition in &self.partitions {
            // Find partition group index
            let group_index = self
                .groups
                .iter()
                .position(|g| g.name == partition.group_name)
                .ok_or_else(|| anyhow!("找不到分区组 {}", partition.group_name))?
                as u32;

            let num_sectors = partition.size / LP_SECTOR_SIZE;
            let first_extent_index = extent_entries.len() as u32;

            // Align current sector
            if !current_sector.is_multiple_of(alignment_sectors) {
                current_sector = current_sector.div_ceil(alignment_sectors) * alignment_sectors;
            }

            // Create extension
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

        // Build geometric information
        let geometry = LpMetadataGeometry {
            magic: LP_METADATA_GEOMETRY_MAGIC,
            struct_size: LP_METADATA_GEOMETRY_STRUCT_SIZE,
            checksum: [0u8; 32],
            metadata_max_size: self.metadata_max_size,
            metadata_slot_count: self.metadata_slot_count,
            logical_block_size,
        };

        // Build the header
        let flags = if self.virtual_ab {
            LP_HEADER_FLAG_VIRTUAL_AB_DEVICE
        } else {
            0
        };

        // Select a version based on whether you need to extend the header
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

// Helper function: get size from file
pub fn get_file_size(path: &str) -> Result<u64> {
    let metadata =
        std::fs::metadata(path).map_err(|e| anyhow!("获取文件大小失败: {}: {}", path, e))?;
    Ok(metadata.len())
}

// Helper function: align size
pub fn align_to(value: u64, alignment: u64) -> u64 {
    if alignment == 0 {
        return value;
    }
    value.div_ceil(alignment) * alignment
}
