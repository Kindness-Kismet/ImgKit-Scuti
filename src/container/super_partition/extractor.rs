// LP (Logical Partition) extractor.
// Provides extraction and unpacking of Super partition images.

use crate::container::sparse::SparseReader;
use crate::container::super_partition::metadata::LpMetadata;
use crate::utils::{progress, sanitize_single_component};
use anyhow::{Context, Result};
use std::fs::{self, File};
use std::io::{BufReader, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::time::Instant;

// Extraction configuration
pub struct ExtractConfig {
    // Path to the input Super image file
    pub input_image: String,
    // Base output directory
    pub output_dir: String,
    // Partition names to extract (empty means extract all)
    pub partition_names: Vec<String>,
}

// Extract a Super image
pub fn extract_image(config: ExtractConfig) -> Result<()> {
    if let Ok(mut sparse_reader) = SparseReader::new(&config.input_image) {
        return extract_with_reader(&mut sparse_reader, config);
    }

    let file = File::open(&config.input_image)?;
    let mut buf_reader = BufReader::new(file);
    extract_with_reader(&mut buf_reader, config)
}

// Extract an image using the given reader
fn extract_with_reader<R: Read + Seek>(reader: &mut R, config: ExtractConfig) -> Result<()> {
    // Parse LP metadata
    let metadata = LpMetadata::from_reader(reader).context("failed to parse LP metadata")?;

    // Create output directory
    let output_base = PathBuf::from(&config.output_dir);
    fs::create_dir_all(&output_base)?;

    // Filter partitions to extract
    let mut partitions_to_extract: Vec<_> = if config.partition_names.is_empty() {
        metadata.partitions.iter().collect()
    } else {
        metadata
            .partitions
            .iter()
            .filter(|p| config.partition_names.contains(&p.name))
            .collect()
    };

    // Extract only A-slot partitions; skip B-slot and empty partitions
    partitions_to_extract.retain(|p| {
        // Skip B-slot partitions
        if p.name.ends_with("_b") {
            return false;
        }

        // Skip empty partitions
        let extents = metadata.get_partition_extents(p);
        let total_sectors: u64 = extents.iter().map(|e| e.num_sectors).sum();
        total_sectors > 0
    });

    let total_partitions = partitions_to_extract.len();
    log::info!("extracting {} partition(s)", total_partitions);

    let start_time = Instant::now();

    // Extract each partition
    for (index, partition) in partitions_to_extract.iter().enumerate() {
        let partition_num = index + 1;

        // Get all extents for this partition
        let extents = metadata.get_partition_extents(partition);

        // Strip _a suffix from output name
        let output_name = if partition.name.ends_with("_a") {
            partition.name.trim_end_matches("_a").to_string()
        } else {
            partition.name.clone()
        };
        let output_name = sanitize_single_component(&output_name)
            .with_context(|| format!("invalid partition output name: {}", output_name))?;

        let partition_output = output_base.join(format!("{}.img", output_name));

        // Extract partition data
        extract_partition(
            reader,
            &partition_output,
            &extents,
            partition_num,
            total_partitions,
        )?;
    }

    progress::display_completion(start_time.elapsed());
    Ok(())
}

// Extract data for a single partition
fn extract_partition<R: Read + Seek>(
    reader: &mut R,
    output_path: &Path,
    extents: &[&crate::container::super_partition::metadata::LpMetadataExtent],
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

    // Display progress
    progress::display_progress(filename.as_str(), current_partition, total_partitions);

    Ok(())
}
