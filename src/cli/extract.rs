// 镜像提取命令, 支持文件系统、Super 分区与 OTA payload。

use crate::{
    container::{
        payload::extractor as payload_extractor, super_partition::extractor as super_extractor,
    },
    filesystem::{
        erofs::read::extractor as erofs_extractor, ext4::read::extractor as ext4_extractor,
        f2fs::read::extractor as f2fs_extractor,
    },
    utils::detect_filesystem,
};
use anyhow::{Result, anyhow};
use std::path::Path;

// 提取镜像
pub fn run_extract(
    input: &str,
    output: Option<&str>,
    fs_config_path: Option<String>,
    file_contexts_path: Option<String>,
    partition_names: Vec<String>,
    list_only: bool,
    clean: bool,
) -> Result<()> {
    let fs_type = detect_filesystem(Path::new(input))?;

    if let Some(stripped) = fs_type.strip_prefix("sparse_") {
        log::info!("detected sparse filesystem type: {}", stripped);
    } else {
        log::info!("detected filesystem type: {}", fs_type);
    }

    // 分区筛选与列举只对容器格式有效
    if (!partition_names.is_empty() || list_only) && !is_container_type(&fs_type) {
        return Err(anyhow!(
            "--partition and --list only apply to container images (super, payload), got: {}",
            fs_type
        ));
    }

    if list_only {
        return list_partitions(input, &fs_type);
    }

    // clap 已保证非列举模式下必须提供输出目录
    let output = output.ok_or_else(|| anyhow!("--output is required unless --list is used"))?;

    if clean {
        clean_output_directory(input, output)?;
    }

    match fs_type.as_str() {
        "f2fs" | "sparse_f2fs" => {
            let config = f2fs_extractor::ExtractConfig {
                input_image: input.to_string(),
                output_dir: output.to_string(),
                fs_config_path,
                file_contexts_path,
            };
            f2fs_extractor::extract_image(config)?;
        }
        "ext4" | "sparse_ext4" => {
            let config = ext4_extractor::ExtractConfig {
                input_image: input.to_string(),
                output_dir: output.to_string(),
                fs_config_path,
                file_contexts_path,
            };
            ext4_extractor::extract_image(config)?;
        }
        "erofs" => {
            let config = erofs_extractor::ExtractConfig {
                input_image: input.to_string(),
                output_dir: output.to_string(),
                fs_config_path,
                file_contexts_path,
            };
            erofs_extractor::extract_image(config)?;
        }
        "super" | "sparse_super" => {
            let config = super_extractor::ExtractConfig {
                input_image: input.to_string(),
                output_dir: output.to_string(),
                partition_names,
            };
            super_extractor::extract_image(config)?;
        }
        "payload" => {
            let config = payload_extractor::ExtractConfig {
                input_payload: input.to_string(),
                output_dir: output.to_string(),
                partition_names,
            };
            payload_extractor::extract_image(config)?;
        }
        _ => {
            return Err(anyhow!(
                "unsupported image: {}, supported: f2fs, ext4, erofs, super, payload",
                fs_type
            ));
        }
    }

    Ok(())
}

// 判断是否为支持按分区筛选的容器格式
fn is_container_type(fs_type: &str) -> bool {
    matches!(fs_type, "super" | "sparse_super" | "payload")
}

// 打印容器内的分区名, 不做提取
fn list_partitions(input: &str, fs_type: &str) -> Result<()> {
    let names = match fs_type {
        "payload" => payload_extractor::list_partitions(input)?,
        _ => super_extractor::list_partitions(input)?,
    };

    for name in names {
        println!("{}", name);
    }

    Ok(())
}

// 删除指定输入镜像对应的提取目录与配置文件
fn clean_output_directory(input_path: &str, output_dir: &str) -> Result<()> {
    use std::fs;

    let input_path = Path::new(input_path);
    let output_path = Path::new(output_dir);

    let partition_name = input_path
        .file_stem()
        .and_then(|s| s.to_str())
        .ok_or_else(|| anyhow!("failed to determine partition name"))?;

    let target_extract_dir = output_path.join(partition_name);
    if target_extract_dir.exists() {
        log::info!(
            "removing extract directory: {}",
            target_extract_dir.display()
        );
        fs::remove_dir_all(&target_extract_dir)?;
    }

    let config_dir = output_path.join("config");
    if config_dir.exists() {
        let fs_config_file = config_dir.join(format!("{}_fs_config", partition_name));
        let file_contexts_file = config_dir.join(format!("{}_file_contexts", partition_name));

        if fs_config_file.exists() {
            log::info!("removing config file: {}", fs_config_file.display());
            fs::remove_file(&fs_config_file)?;
        }

        if file_contexts_file.exists() {
            log::info!("removing config file: {}", file_contexts_file.display());
            fs::remove_file(&file_contexts_file)?;
        }
    }

    log::info!("clean complete");
    Ok(())
}
