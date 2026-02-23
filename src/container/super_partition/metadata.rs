// Android LP (Logical Partition) metadata analysis
//
// Provides metadata structure and parsing functions for Super partitions

use anyhow::{Context, Result, anyhow};
use sha2::{Digest, Sha256};
use std::io::{Read, Seek, SeekFrom};

// LP metadata magic number
const LP_METADATA_GEOMETRY_MAGIC: u32 = 0x616c4467; // "gDla"
const LP_METADATA_HEADER_MAGIC: u32 = 0x414c5030; // "0PLA"

// LP metadata version
const LP_METADATA_MAJOR_VERSION: u16 = 10;
const LP_METADATA_MINOR_VERSION_MAX: u16 = 2;
const LP_METADATA_GEOMETRY_STRUCT_SIZE: u32 = 52;
const LP_METADATA_GEOMETRY_SIZE: usize = 4096;
const LP_METADATA_HEADER_V1_0_SIZE: u32 = 128;
const LP_METADATA_HEADER_V1_2_SIZE: u32 = 256;
const LP_METADATA_PARTITION_SIZE: u32 = 52;
const LP_METADATA_EXTENT_SIZE: u32 = 24;
const LP_METADATA_GROUP_SIZE: u32 = 48;
const LP_METADATA_BLOCK_DEVICE_SIZE: u32 = 64;
const LP_TARGET_TYPE_LINEAR: u32 = 0;
const LP_TARGET_TYPE_ZERO: u32 = 1;

fn sha256(data: &[u8]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(data);
    let digest = hasher.finalize();
    let mut out = [0u8; 32];
    out.copy_from_slice(&digest);
    out
}

fn compute_table_range(
    tables_size: u32,
    offset: u32,
    count: u32,
    entry_size: u32,
    min_entry_size: u32,
    name: &str,
) -> Result<(usize, usize, usize)> {
    if entry_size < min_entry_size {
        return Err(anyhow!(
            "{} 表条目大小过小: {}, 最小 {}",
            name,
            entry_size,
            min_entry_size
        ));
    }

    let table_bytes = count
        .checked_mul(entry_size)
        .ok_or_else(|| anyhow!("{} 表大小溢出", name))?;
    let end = offset
        .checked_add(table_bytes)
        .ok_or_else(|| anyhow!("{} 表偏移溢出", name))?;
    if end > tables_size {
        return Err(anyhow!(
            "{} 表越界: offset={}, bytes={}, tables_size={}",
            name,
            offset,
            table_bytes,
            tables_size
        ));
    }

    Ok((offset as usize, entry_size as usize, count as usize))
}

// LP metadata geometry information
#[derive(Debug, Clone)]
pub struct LpMetadataGeometry {
    pub magic: u32,
    pub struct_size: u32,
    pub checksum: [u8; 32],
    pub metadata_max_size: u32,
    pub metadata_slot_count: u32,
    pub logical_block_size: u32,
}

impl LpMetadataGeometry {
    pub fn from_reader<R: Read + Seek>(reader: &mut R, offset: u64) -> Result<Self> {
        reader.seek(SeekFrom::Start(offset))?;

        let mut buf = [0u8; LP_METADATA_GEOMETRY_SIZE];
        reader.read_exact(&mut buf)?;

        let magic = u32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]]);

        if magic != LP_METADATA_GEOMETRY_MAGIC {
            anyhow::bail!("无效的 LP 几何魔数: {:#x}", magic);
        }

        let struct_size = u32::from_le_bytes([buf[4], buf[5], buf[6], buf[7]]);
        if struct_size < LP_METADATA_GEOMETRY_STRUCT_SIZE || struct_size as usize > buf.len() {
            anyhow::bail!("无效的 LP 几何结构大小: {}", struct_size);
        }

        let mut checksum = [0u8; 32];
        checksum.copy_from_slice(&buf[8..40]);
        let mut checksum_data = buf;
        checksum_data[8..40].fill(0);
        let expected_checksum = sha256(&checksum_data[..struct_size as usize]);
        if checksum != expected_checksum {
            anyhow::bail!("LP 几何信息校验和不匹配");
        }

        let metadata_max_size = u32::from_le_bytes([buf[40], buf[41], buf[42], buf[43]]);
        let metadata_slot_count = u32::from_le_bytes([buf[44], buf[45], buf[46], buf[47]]);
        let logical_block_size = u32::from_le_bytes([buf[48], buf[49], buf[50], buf[51]]);
        if metadata_max_size == 0 {
            anyhow::bail!("LP metadata_max_size 不能为 0");
        }
        if metadata_slot_count == 0 {
            anyhow::bail!("LP metadata_slot_count 不能为 0");
        }
        if logical_block_size == 0 {
            anyhow::bail!("LP logical_block_size 不能为 0");
        }

        Ok(Self {
            magic,
            struct_size,
            checksum,
            metadata_max_size,
            metadata_slot_count,
            logical_block_size,
        })
    }
}

// LP metadata header
#[derive(Debug, Clone)]
pub struct LpMetadataHeader {
    pub magic: u32,
    pub major_version: u16,
    pub minor_version: u16,
    pub header_size: u32,
    pub header_checksum: [u8; 32],
    pub tables_size: u32,
    pub tables_checksum: [u8; 32],
    pub partitions_offset: u32,
    pub partitions_count: u32,
    pub partitions_entry_size: u32,
    pub extents_offset: u32,
    pub extents_count: u32,
    pub extents_entry_size: u32,
    pub groups_offset: u32,
    pub groups_count: u32,
    pub groups_entry_size: u32,
    pub block_devices_offset: u32,
    pub block_devices_count: u32,
    pub block_devices_entry_size: u32,
}

impl LpMetadataHeader {
    pub fn from_reader<R: Read>(reader: &mut R) -> Result<Self> {
        let mut buf = [0u8; 256];
        reader.read_exact(&mut buf)?;

        let magic = u32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]]);
        if magic != LP_METADATA_HEADER_MAGIC {
            anyhow::bail!("无效的 LP 元数据头魔数: {:#x}", magic);
        }

        let major_version = u16::from_le_bytes([buf[4], buf[5]]);
        let minor_version = u16::from_le_bytes([buf[6], buf[7]]);

        if major_version != LP_METADATA_MAJOR_VERSION
            || minor_version > LP_METADATA_MINOR_VERSION_MAX
        {
            anyhow::bail!(
                "不支持的 LP 元数据版本: {}.{}",
                major_version,
                minor_version
            );
        }

        let header_size = u32::from_le_bytes([buf[8], buf[9], buf[10], buf[11]]);
        if !(LP_METADATA_HEADER_V1_0_SIZE..=LP_METADATA_HEADER_V1_2_SIZE).contains(&header_size) {
            anyhow::bail!("无效的 LP 元数据头大小: {}", header_size);
        }
        let mut header_checksum = [0u8; 32];
        header_checksum.copy_from_slice(&buf[12..44]);

        let tables_size = u32::from_le_bytes([buf[44], buf[45], buf[46], buf[47]]);
        let mut tables_checksum = [0u8; 32];
        tables_checksum.copy_from_slice(&buf[48..80]);

        let mut checksum_data = buf;
        checksum_data[12..44].fill(0);
        let expected_checksum = sha256(&checksum_data[..header_size as usize]);
        if header_checksum != expected_checksum {
            anyhow::bail!("LP 元数据头校验和不匹配");
        }

        Ok(Self {
            magic,
            major_version,
            minor_version,
            header_size,
            header_checksum,
            tables_size,
            tables_checksum,
            partitions_offset: u32::from_le_bytes([buf[80], buf[81], buf[82], buf[83]]),
            partitions_count: u32::from_le_bytes([buf[84], buf[85], buf[86], buf[87]]),
            partitions_entry_size: u32::from_le_bytes([buf[88], buf[89], buf[90], buf[91]]),
            extents_offset: u32::from_le_bytes([buf[92], buf[93], buf[94], buf[95]]),
            extents_count: u32::from_le_bytes([buf[96], buf[97], buf[98], buf[99]]),
            extents_entry_size: u32::from_le_bytes([buf[100], buf[101], buf[102], buf[103]]),
            groups_offset: u32::from_le_bytes([buf[104], buf[105], buf[106], buf[107]]),
            groups_count: u32::from_le_bytes([buf[108], buf[109], buf[110], buf[111]]),
            groups_entry_size: u32::from_le_bytes([buf[112], buf[113], buf[114], buf[115]]),
            block_devices_offset: u32::from_le_bytes([buf[116], buf[117], buf[118], buf[119]]),
            block_devices_count: u32::from_le_bytes([buf[120], buf[121], buf[122], buf[123]]),
            block_devices_entry_size: u32::from_le_bytes([buf[124], buf[125], buf[126], buf[127]]),
        })
    }
}

// LP partition information
#[derive(Debug, Clone)]
pub struct LpMetadataPartition {
    pub name: String,
    pub attributes: u32,
    pub first_extent_index: u32,
    pub num_extents: u32,
    pub group_index: u32,
}

impl LpMetadataPartition {
    /// Read partition information from byte array
    pub fn from_bytes(buf: &[u8]) -> Result<Self> {
        if buf.len() < 52 {
            anyhow::bail!("分区条目大小不足，需要至少52字节，实际{}字节", buf.len());
        }

        // Read partition name (36 bytes, null terminated)
        let name_bytes = &buf[0..36];
        let name_end = name_bytes.iter().position(|&b| b == 0).unwrap_or(36);
        let name = String::from_utf8_lossy(&name_bytes[..name_end]).to_string();

        Ok(Self {
            name,
            attributes: u32::from_le_bytes([buf[36], buf[37], buf[38], buf[39]]),
            first_extent_index: u32::from_le_bytes([buf[40], buf[41], buf[42], buf[43]]),
            num_extents: u32::from_le_bytes([buf[44], buf[45], buf[46], buf[47]]),
            group_index: u32::from_le_bytes([buf[48], buf[49], buf[50], buf[51]]),
        })
    }
}

// LP extent information
#[derive(Debug, Clone)]
pub struct LpMetadataExtent {
    pub num_sectors: u64,
    pub target_type: u32,
    pub target_data: u64,
    pub target_source: u32,
}

impl LpMetadataExtent {
    /// Read extent information from byte array
    pub fn from_bytes(buf: &[u8]) -> Result<Self> {
        if buf.len() < 24 {
            anyhow::bail!("扩展区条目大小不足，需要至少24字节，实际{}字节", buf.len());
        }

        Ok(Self {
            num_sectors: u64::from_le_bytes([
                buf[0], buf[1], buf[2], buf[3], buf[4], buf[5], buf[6], buf[7],
            ]),
            target_type: u32::from_le_bytes([buf[8], buf[9], buf[10], buf[11]]),
            target_data: u64::from_le_bytes([
                buf[12], buf[13], buf[14], buf[15], buf[16], buf[17], buf[18], buf[19],
            ]),
            target_source: u32::from_le_bytes([buf[20], buf[21], buf[22], buf[23]]),
        })
    }
}

// LP complete metadata
#[derive(Debug)]
pub struct LpMetadata {
    pub geometry: LpMetadataGeometry,
    pub header: LpMetadataHeader,
    pub partitions: Vec<LpMetadataPartition>,
    pub extents: Vec<LpMetadataExtent>,
}

impl LpMetadata {
    /// Parse complete LP metadata from Reader
    pub fn from_reader<R: Read + Seek>(reader: &mut R) -> Result<Self> {
        // Try multiple possible LP geometry locations
        let possible_offsets = [0u64, 16, 512, 4096];

        let mut geometry = None;
        let mut geometry_offset = 0u64;

        for offset in possible_offsets {
            match LpMetadataGeometry::from_reader(reader, offset) {
                Ok(geo) => {
                    geometry = Some(geo);
                    geometry_offset = offset;
                    break;
                }
                Err(_) => continue,
            }
        }

        let geometry = geometry.context("在所有可能的偏移位置都未找到有效的 LP 几何信息")?;

        // Try reading the metadata header, probably at geometry information +4096 or +8192
        let possible_metadata_offsets = [geometry_offset + 4096, geometry_offset + 8192];

        let mut header = None;
        let mut primary_metadata_offset = 0u64;

        for offset in possible_metadata_offsets {
            reader.seek(SeekFrom::Start(offset))?;
            match LpMetadataHeader::from_reader(reader) {
                Ok(h) => {
                    header = Some(h);
                    primary_metadata_offset = offset;
                    break;
                }
                Err(_) => continue,
            }
        }

        let header = header.context("在所有可能的偏移位置都未找到有效的 LP 元数据头")?;
        if header.header_size > geometry.metadata_max_size {
            anyhow::bail!(
                "LP header_size 超过 metadata_max_size: {} > {}",
                header.header_size,
                geometry.metadata_max_size
            );
        }
        if header.tables_size > geometry.metadata_max_size {
            anyhow::bail!(
                "LP tables_size 超过 metadata_max_size: {} > {}",
                header.tables_size,
                geometry.metadata_max_size
            );
        }

        let tables_start = primary_metadata_offset
            .checked_add(header.header_size as u64)
            .ok_or_else(|| anyhow!("LP tables_start 溢出"))?;
        reader.seek(SeekFrom::Start(tables_start))?;
        let mut tables_data = vec![0u8; header.tables_size as usize];
        reader.read_exact(&mut tables_data)?;

        let expected_tables_checksum = sha256(&tables_data);
        if header.tables_checksum != expected_tables_checksum {
            anyhow::bail!("LP 表校验和不匹配");
        }

        let (partitions_offset, partitions_entry_size, partitions_count) = compute_table_range(
            header.tables_size,
            header.partitions_offset,
            header.partitions_count,
            header.partitions_entry_size,
            LP_METADATA_PARTITION_SIZE,
            "partitions",
        )?;
        let (extents_offset, extents_entry_size, extents_count) = compute_table_range(
            header.tables_size,
            header.extents_offset,
            header.extents_count,
            header.extents_entry_size,
            LP_METADATA_EXTENT_SIZE,
            "extents",
        )?;
        compute_table_range(
            header.tables_size,
            header.groups_offset,
            header.groups_count,
            header.groups_entry_size,
            LP_METADATA_GROUP_SIZE,
            "groups",
        )?;
        compute_table_range(
            header.tables_size,
            header.block_devices_offset,
            header.block_devices_count,
            header.block_devices_entry_size,
            LP_METADATA_BLOCK_DEVICE_SIZE,
            "block_devices",
        )?;

        let mut partitions = Vec::with_capacity(partitions_count);
        for index in 0..partitions_count {
            let start = partitions_offset + index * partitions_entry_size;
            let end = start + partitions_entry_size;
            partitions.push(LpMetadataPartition::from_bytes(&tables_data[start..end])?);
        }

        let mut extents = Vec::with_capacity(extents_count);
        for index in 0..extents_count {
            let start = extents_offset + index * extents_entry_size;
            let end = start + extents_entry_size;
            let extent = LpMetadataExtent::from_bytes(&tables_data[start..end])?;
            match extent.target_type {
                LP_TARGET_TYPE_LINEAR => {
                    if extent.target_source >= header.block_devices_count {
                        anyhow::bail!(
                            "扩展区目标块设备索引越界: {} >= {}",
                            extent.target_source,
                            header.block_devices_count
                        );
                    }
                }
                LP_TARGET_TYPE_ZERO => {
                    if extent.target_data != 0 || extent.target_source != 0 {
                        anyhow::bail!("ZERO 扩展区必须使用 target_data=0 且 target_source=0");
                    }
                }
                _ => anyhow::bail!("不支持的扩展区类型: {}", extent.target_type),
            }
            extents.push(extent);
        }

        for partition in &partitions {
            let extents_end = partition
                .first_extent_index
                .checked_add(partition.num_extents)
                .ok_or_else(|| anyhow!("分区 {} 扩展区范围溢出", partition.name))?;
            if extents_end > header.extents_count {
                anyhow::bail!(
                    "分区 {} 扩展区越界: first={}, count={}, total={}",
                    partition.name,
                    partition.first_extent_index,
                    partition.num_extents,
                    header.extents_count
                );
            }
        }

        Ok(Self {
            geometry,
            header,
            partitions,
            extents,
        })
    }

    /// Get the partition with the specified name
    pub fn get_partition(&self, name: &str) -> Option<&LpMetadataPartition> {
        self.partitions.iter().find(|p| p.name == name)
    }

    /// Get all extents of a partition
    pub fn get_partition_extents(&self, partition: &LpMetadataPartition) -> Vec<&LpMetadataExtent> {
        // If the partition does not have an extent, an empty Vec is returned directly.
        if partition.num_extents == 0 {
            return Vec::new();
        }

        let start = partition.first_extent_index as usize;
        let end = start + partition.num_extents as usize;

        // Boundary checking
        if start >= self.extents.len() || end > self.extents.len() {
            log::warn!(
                "分区 {} 的扩展区索引越界: start={}, end={}, total_extents={}",
                partition.name,
                start,
                end,
                self.extents.len()
            );
            return Vec::new();
        }

        self.extents[start..end].iter().collect()
    }
}
