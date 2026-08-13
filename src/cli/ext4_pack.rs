// EXT4 镜像打包命令
// 将目录打包为 EXT4 文件系统镜像

use anyhow::Result;
use std::path::PathBuf;
use std::time::Instant;

// 打包 EXT4 镜像
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

    // 解析镜像大小
    let image_size = super::parse_size(size)?;

    log::info!("source: {}", source);
    log::info!("output: {}", output);
    log::info!(
        "image size: {} bytes ({:.2} MB)",
        image_size,
        image_size as f64 / 1024.0 / 1024.0
    );

    // 构建配置
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

    // 创建构建器并执行构建
    let mut builder = Ext4Builder::new(config)?;
    builder.build()?;

    let elapsed = start.elapsed();
    log::info!("EXT4 image built: {}", output);
    println!("elapsed {:.2}s", elapsed.as_secs_f64());

    Ok(())
}
