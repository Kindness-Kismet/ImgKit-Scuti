// EXT4 image pack command.
// Packs a directory into an EXT4 filesystem image.

use anyhow::Result;
use std::path::PathBuf;
use std::time::Instant;

// Pack an EXT4 image
#[allow(clippy::too_many_arguments)]
pub fn run_ext4_pack(
    source: &str,
    output: &str,
    size: &str,
    mount_point: &str,
    file_contexts: Option<String>,
    fs_config: Option<String>,
    label: Option<String>,
    timestamp: Option<u64>,
    root_uid: u32,
    root_gid: u32,
) -> Result<()> {
    use crate::filesystem::ext4::write::{Ext4Builder, Ext4BuilderConfig};

    let start = Instant::now();

    // Parse image size
    let image_size = super::parse_size(size)?;

    log::info!("source: {}", source);
    log::info!("output: {}", output);
    log::info!(
        "image size: {} bytes ({:.2} MB)",
        image_size,
        image_size as f64 / 1024.0 / 1024.0
    );

    // Build config
    let config = Ext4BuilderConfig {
        source_dir: PathBuf::from(source),
        output_path: PathBuf::from(output),
        image_size,
        volume_label: label.unwrap_or_default(),
        mount_point: mount_point.to_string(),
        root_uid,
        root_gid,
        file_contexts: file_contexts.map(PathBuf::from),
        fs_config: fs_config.map(PathBuf::from),
        timestamp,
    };

    // Create builder and build
    let mut builder = Ext4Builder::new(config)?;
    builder.build()?;

    let elapsed = start.elapsed();
    log::info!("EXT4 image built: {}", output);
    println!("elapsed {:.2}s", elapsed.as_secs_f64());

    Ok(())
}
