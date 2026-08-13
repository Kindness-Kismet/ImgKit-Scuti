// 子命令分发。

use crate::cli::{self, Cli, Commands};
use crate::utils::logger;
use anyhow::{Result, anyhow};

// 命令行主入口
pub fn run(cli: Cli) -> Result<()> {
    match cli.command {
        Commands::Unpack {
            input,
            output,
            fs_config_path,
            file_contexts_path,
            partition,
            list,
            level,
            clean,
        } => {
            logger::init(level);
            cli::run_extract(
                &input,
                output.as_deref(),
                fs_config_path,
                file_contexts_path,
                partition,
                list,
                clean,
            )
        }
        Commands::Pack {
            r#type,
            output,
            source,
            size,
            mount_point,
            file_contexts,
            fs_config,
            label,
            timestamp,
            root_uid,
            root_gid,
            readonly,
            project_quota,
            casefold,
            compression,
            compress,
            compress_level,
            uuid,
            device_size,
            metadata_size,
            slots,
            name,
            block_size,
            alignment,
            alignment_offset,
            group,
            partition,
            image,
            auto_slot_suffixing,
            virtual_ab,
            force_full_image,
            sparse,
            level,
        } => {
            logger::init(level);

            match r#type.to_lowercase().as_str() {
                "super" => cli::run_super_pack(
                    &output,
                    device_size,
                    metadata_size,
                    slots,
                    &name,
                    block_size,
                    alignment,
                    alignment_offset,
                    &group,
                    &partition,
                    &image,
                    auto_slot_suffixing,
                    virtual_ab,
                    force_full_image,
                    sparse,
                ),
                "f2fs" => {
                    let source = source.ok_or_else(|| anyhow!("F2FS packing requires --source"))?;
                    let size = size.ok_or_else(|| anyhow!("F2FS packing requires --size"))?;

                    cli::run_f2fs_pack(
                        &source,
                        &output,
                        &size,
                        &mount_point,
                        file_contexts,
                        fs_config,
                        sparse,
                        label,
                        readonly,
                        project_quota,
                        casefold,
                        compression,
                        root_uid,
                        root_gid,
                        timestamp,
                    )
                }
                "ext4" => {
                    let source = source.ok_or_else(|| anyhow!("EXT4 packing requires --source"))?;
                    let size = size.ok_or_else(|| anyhow!("EXT4 packing requires --size"))?;

                    cli::run_ext4_pack(
                        &source,
                        &output,
                        &size,
                        &mount_point,
                        file_contexts,
                        fs_config,
                        label,
                        timestamp,
                        root_uid,
                        root_gid,
                    )
                }
                "erofs" => {
                    let source =
                        source.ok_or_else(|| anyhow!("EROFS packing requires --source"))?;

                    cli::run_erofs_pack(
                        &source,
                        &output,
                        &mount_point,
                        file_contexts,
                        fs_config,
                        label,
                        block_size,
                        timestamp,
                        uuid,
                        root_uid,
                        root_gid,
                        compress,
                        compress_level,
                    )
                }
                _ => Err(anyhow!(
                    "unsupported image type: {}, supported types: super, f2fs, ext4, erofs",
                    r#type
                )),
            }
        }
    }
}
