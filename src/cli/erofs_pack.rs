// EROFS pack command implementation

use anyhow::{Result, anyhow};
use std::path::PathBuf;

use crate::filesystem::erofs::{ErofsConfig, build_erofs_image};

// Parse a UUID string into a 16-byte array
fn parse_uuid(uuid_str: &str) -> Result<[u8; 16]> {
    let hex_str: String = uuid_str.chars().filter(|c| c.is_ascii_hexdigit()).collect();
    if hex_str.len() != 32 {
        return Err(anyhow!("UUID must be exactly 32 hex characters"));
    }

    let mut uuid = [0u8; 16];
    for i in 0..16 {
        uuid[i] = u8::from_str_radix(&hex_str[i * 2..i * 2 + 2], 16)?;
    }
    Ok(uuid)
}

#[allow(clippy::too_many_arguments)]
pub fn run_erofs_pack(
    source: &str,
    output: &str,
    mount_point: &str,
    file_contexts: Option<String>,
    fs_config: Option<String>,
    label: Option<String>,
    block_size: u32,
    timestamp: Option<u64>,
    uuid: Option<String>,
    root_uid: u32,
    root_gid: u32,
    compress: Option<String>,
    compress_level: Option<u32>,
) -> Result<()> {
    let source_path = PathBuf::from(source);
    let output_path = PathBuf::from(output);

    if !source_path.exists() {
        return Err(anyhow!("source directory does not exist: {}", source));
    }

    if !source_path.is_dir() {
        return Err(anyhow!("source path is not a directory: {}", source));
    }

    // Validate block size: must be a power of two between 512 and 65536
    if !block_size.is_power_of_two() || !(512..=65536).contains(&block_size) {
        return Err(anyhow!(
            "block size must be a power of two between 512 and 65536"
        ));
    }

    // Parse UUID if provided
    let uuid_bytes = if let Some(ref uuid_str) = uuid {
        Some(parse_uuid(uuid_str)?)
    } else {
        None
    };

    log::info!("packing EROFS image");
    log::info!("  source: {}", source);
    log::info!("  output: {}", output);
    log::info!("  mount point: {}", mount_point);
    log::info!("  block size: {}", block_size);

    if let Some(ref alg) = compress {
        log::info!("  compress algorithm: {}", alg);
        if let Some(level) = compress_level {
            log::info!("  compress level: {}", level);
        }
    }

    if let Some(ref fc) = file_contexts {
        log::info!("  file_contexts: {}", fc);
    }
    if let Some(ref fc) = fs_config {
        log::info!("  fs_config: {}", fc);
    }

    let config = ErofsConfig {
        source_dir: source_path,
        output_path,
        volume_label: label.unwrap_or_default(),
        block_size,
        compress_algorithm: compress,
        compress_level,
        file_contexts: file_contexts.map(PathBuf::from),
        fs_config: fs_config.map(PathBuf::from),
        mount_point: mount_point.to_string(),
        timestamp,
        uuid: uuid_bytes,
        root_uid,
        root_gid,
    };

    build_erofs_image(config)?;

    log::info!("EROFS image packed: {}", output);

    Ok(())
}
