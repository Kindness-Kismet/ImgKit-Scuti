// F2FS image pack command.
// Packs a directory into an F2FS filesystem image.

use anyhow::Result;
use std::path::PathBuf;

// Pack an F2FS image
#[allow(clippy::too_many_arguments)]
pub fn run_f2fs_pack(
    source: &str,
    output: &str,
    size: &str,
    mount_point: &str,
    file_contexts: Option<String>,
    fs_config: Option<String>,
    sparse: bool,
    label: Option<String>,
    readonly: bool,
    project_quota: bool,
    casefold: bool,
    compression: bool,
    root_uid: u32,
    root_gid: u32,
    timestamp: Option<u64>,
) -> Result<()> {
    use crate::filesystem::f2fs::types::{F2fsBuilderConfig, F2fsFeatures};
    use crate::filesystem::f2fs::write::F2fsBuilder;

    // Parse image size
    let image_size = super::parse_size(size)?;

    log::info!("source: {}", source);
    log::info!("output: {}", output);
    log::info!(
        "image size: {} bytes ({:.2} MB)",
        image_size,
        image_size as f64 / 1024.0 / 1024.0
    );

    // Build feature flags.
    // inode_chksum and sb_chksum are disabled until basic functionality is verified.
    let features = F2fsFeatures {
        readonly,
        project_quota,
        casefold,
        compression,
        extra_attr: false,
        inode_chksum: false,
        sb_chksum: false,
        ..Default::default()
    };

    // Build config
    let config = F2fsBuilderConfig {
        source_dir: PathBuf::from(source),
        output_path: PathBuf::from(output),
        image_size,
        mount_point: mount_point.to_string(),
        file_contexts: file_contexts.map(PathBuf::from),
        fs_config: fs_config.map(PathBuf::from),
        sparse_mode: sparse,
        features,
        compression: None,
        volume_label: label.unwrap_or_default(),
        root_uid,
        root_gid,
        timestamp,
    };

    // Create builder and build
    let mut builder = F2fsBuilder::new(config)?;
    builder.build()?;

    log::info!("F2FS image built: {}", output);
    Ok(())
}
