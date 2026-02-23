// Image extraction command.
// Supports F2FS, EXT4, EROFS, and Super partition extraction.

use anyhow::{Result, anyhow};
use std::path::Path;

// Extract an image
pub fn run_extract(
    input: &str,
    output: &str,
    fs_config_path: Option<String>,
    file_contexts_path: Option<String>,
    clean: bool,
) -> Result<()> {
    let fs_type = crate::utils::detect_filesystem(Path::new(input))?;

    if let Some(stripped) = fs_type.strip_prefix("sparse_") {
        log::info!("detected sparse filesystem type: {}", stripped);
    } else {
        log::info!("detected filesystem type: {}", fs_type);
    }

    if clean {
        clean_output_directory(input, output)?;
    }

    match fs_type.as_str() {
        "f2fs" => {
            let config = crate::filesystem::f2fs::read::extractor::ExtractConfig {
                input_image: input.to_string(),
                output_dir: output.to_string(),
                fs_config_path,
                file_contexts_path,
            };
            crate::filesystem::f2fs::read::extractor::extract_image(config)?;
        }
        "ext4" | "sparse_ext4" => {
            let config = crate::filesystem::ext4::extractor::ExtractConfig {
                input_image: input.to_string(),
                output_dir: output.to_string(),
                fs_config_path,
                file_contexts_path,
            };
            crate::filesystem::ext4::extractor::extract_image(config)?;
        }
        "erofs" => {
            let config = crate::filesystem::erofs::read::extractor::ExtractConfig {
                input_image: input.to_string(),
                output_dir: output.to_string(),
                fs_config_path,
                file_contexts_path,
            };
            crate::filesystem::erofs::read::extractor::extract_image(config)?;
        }
        "super" | "sparse_super" => {
            let config = crate::container::super_partition::extractor::ExtractConfig {
                input_image: input.to_string(),
                output_dir: output.to_string(),
                partition_names: Vec::new(),
            };
            crate::container::super_partition::extractor::extract_image(config)?;
        }
        _ => {
            return Err(anyhow!(
                "unsupported filesystem: {}, supported: f2fs, ext4, erofs, super",
                fs_type
            ));
        }
    }

    Ok(())
}

// Remove the extracted directory and config files for the given input image
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
