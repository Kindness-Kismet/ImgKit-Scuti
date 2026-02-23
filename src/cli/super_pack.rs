// Super partition pack command.
// Packs multiple partition images into a Super partition image.

use anyhow::{Result, anyhow};
use std::collections::HashMap;
use std::path::Path;

// Pack a Super partition image
#[allow(clippy::too_many_arguments)]
pub fn run_super_pack(
    output: &str,
    device_size: Option<String>,
    metadata_size: u32,
    slots: u32,
    super_name: &str,
    block_size: u32,
    alignment: u32,
    alignment_offset: u32,
    groups: &[String],
    partitions: &[String],
    images: &[String],
    auto_slot_suffixing: bool,
    virtual_ab: bool,
    force_full_image: bool,
    sparse: bool,
) -> Result<()> {
    use crate::container::super_partition::{
        BlockDeviceInfo, GroupInfo, LP_PARTITION_ATTR_READONLY, MetadataBuilder, PartitionInfo,
    };

    // Parse partition definitions
    let mut partition_infos: Vec<(String, u32, u64, String)> = Vec::new();
    for p in partitions {
        let parts: Vec<&str> = p.split(':').collect();
        if parts.len() < 3 {
            return Err(anyhow!(
                "invalid partition format: {}, expected name:attrs:size[:group]",
                p
            ));
        }
        let name = parts[0].to_string();
        let attrs = match parts[1] {
            "readonly" => LP_PARTITION_ATTR_READONLY,
            "none" => 0,
            _ => {
                return Err(anyhow!(
                    "unknown attribute: {}, expected 'readonly' or 'none'",
                    parts[1]
                ));
            }
        };
        let size: u64 = parts[2]
            .parse()
            .map_err(|_| anyhow!("invalid partition size: {}", parts[2]))?;
        let group = if parts.len() > 3 {
            parts[3].to_string()
        } else {
            "default".to_string()
        };
        partition_infos.push((name, attrs, size, group));
    }

    // Parse image mappings
    let mut image_map: HashMap<String, String> = HashMap::new();
    for img in images {
        let parts: Vec<&str> = img.splitn(2, '=').collect();
        if parts.len() != 2 {
            return Err(anyhow!("invalid image format: {}, expected name=path", img));
        }
        image_map.insert(parts[0].to_string(), parts[1].to_string());
    }

    // Calculate device size
    let alignment_u64 = alignment as u64;
    let calculated_device_size = match device_size.as_deref() {
        Some("auto") | None => {
            // Auto-calculate: metadata area + partition data (each partition aligned)
            let metadata_area = crate::container::super_partition::LP_PARTITION_RESERVED_BYTES
                + crate::container::super_partition::LP_METADATA_GEOMETRY_SIZE * 2
                + metadata_size as u64 * slots as u64 * 2;
            let first_sector = metadata_area.div_ceil(alignment_u64) * alignment_u64
                / crate::container::super_partition::LP_SECTOR_SIZE;

            let mut partition_area = 0u64;
            for (_, _, size, _) in &partition_infos {
                partition_area += (*size).div_ceil(alignment_u64) * alignment_u64;
            }

            let total =
                first_sector * crate::container::super_partition::LP_SECTOR_SIZE + partition_area;
            total.div_ceil(4096) * 4096
        }
        Some(s) => s
            .parse()
            .map_err(|_| anyhow!("invalid device size: {}", s))?,
    };

    log::info!(
        "device size: {} bytes ({:.2} MB)",
        calculated_device_size,
        calculated_device_size as f64 / 1024.0 / 1024.0
    );

    // Create builder
    let block_device = BlockDeviceInfo::new(super_name, calculated_device_size)
        .with_alignment(alignment, alignment_offset)
        .with_block_size(block_size);
    let mut builder = MetadataBuilder::new(vec![block_device], metadata_size, slots)?;

    // Set flags
    if auto_slot_suffixing {
        builder.set_auto_slot_suffixing();
    }
    if virtual_ab {
        builder.set_virtual_ab_device_flag();
    }

    // Add partition groups
    for g in groups {
        let parts: Vec<&str> = g.split(':').collect();
        if parts.len() != 2 {
            return Err(anyhow!(
                "invalid group format: {}, expected name:max_size",
                g
            ));
        }
        let name = parts[0];
        let max_size: u64 = parts[1]
            .parse()
            .map_err(|_| anyhow!("invalid group size: {}", parts[1]))?;
        builder.add_group(GroupInfo::new(name, max_size))?;
    }

    // Add partitions
    for (name, attrs, size, group) in &partition_infos {
        let mut partition = PartitionInfo::new(name, group, *size);
        partition.attributes = *attrs;
        builder.add_partition(partition)?;
        log::info!(
            "adding partition: {} (size: {} bytes, group: {})",
            name,
            size,
            group
        );
    }

    // Export metadata
    let metadata = builder.export()?;

    // Write image
    let output_path = Path::new(output);
    if force_full_image || !image_map.is_empty() {
        if sparse {
            crate::container::super_partition::write_to_sparse_image_file_with_data(
                output_path,
                &metadata,
                &image_map,
                block_size,
            )?;
        } else {
            crate::container::super_partition::write_to_image_file_with_data(
                output_path,
                &metadata,
                &image_map,
            )?;
        }
    } else if sparse {
        crate::container::super_partition::write_sparse_empty_image(
            output_path,
            &metadata,
            block_size,
        )?;
    } else {
        crate::container::super_partition::write_empty_image(output_path, &metadata)?;
    }

    log::info!("image written: {}", output);
    Ok(())
}
