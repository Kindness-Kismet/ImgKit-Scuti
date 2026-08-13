// F2FS 镜像打包命令
// 将目录打包为 F2FS 文件系统镜像

use anyhow::Result;
use std::path::PathBuf;

// 打包 F2FS 镜像
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
    use crate::container::sparse::convert_to_sparse;
    use crate::filesystem::f2fs::consts::F2FS_BLKSIZE;
    use crate::filesystem::f2fs::types::{F2fsBuilderConfig, F2fsFeatures};
    use crate::filesystem::f2fs::write::F2fsBuilder;

    // 解析镜像大小
    let image_size = super::parse_size(size)?;

    log::info!("source: {}", source);
    log::info!("output: {}", output);
    log::info!(
        "image size: {} bytes ({:.2} MB)",
        image_size,
        image_size as f64 / 1024.0 / 1024.0
    );

    // 构建特性标志
    // 在基础功能验证通过前, 禁用 inode_chksum 与 sb_chksum
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

    // 构建配置
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

    // 创建构建器并执行构建
    let mut builder = F2fsBuilder::new(config)?;
    builder.build()?;

    // 构建器仅输出 raw 镜像, 此处作为后处理步骤转换为 sparse
    // 转换失败时保留 raw 临时文件, 避免丢失刚构建好的镜像
    if sparse {
        let raw_tmp = format!("{}.raw.{}.tmp", output, std::process::id());
        std::fs::rename(output, &raw_tmp)?;
        match convert_to_sparse(
            std::path::Path::new(&raw_tmp),
            std::path::Path::new(output),
            F2FS_BLKSIZE as u32,
        ) {
            Ok(()) => {
                let _ = std::fs::remove_file(&raw_tmp);
            }
            Err(e) => {
                return Err(anyhow::anyhow!(
                    "convert_to_sparse failed: {}; raw image preserved at {}",
                    e,
                    raw_tmp
                ));
            }
        }
    }

    log::info!("F2FS image built: {}", output);
    Ok(())
}
