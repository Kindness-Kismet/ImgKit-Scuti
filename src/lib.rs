// Android filesystem image extraction and packing library.
// Supports F2FS, EXT4 filesystems and Super partition.

// Core abstraction layer
pub mod core;

// IO layer
pub mod io;

// Common utility functions
pub mod utils;

// General compression/decompression module
pub mod compression;

// Container layer
pub mod container;

// Filesystem layer
pub mod filesystem;

// CLI interface
mod cli;

use anyhow::{Result, anyhow};
use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "imgkit")]
#[command(about = "Android image tool - unpack and pack Super/F2FS/EXT4/EROFS images")]
#[command(after_help = FULL_HELP)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

const FULL_HELP: &str = r#"
================================================================================
                              Unpack Command
================================================================================

Usage: imgkit unpack [OPTIONS] -i <INPUT> -o <OUTPUT>

Supported formats: Super, F2FS, EXT4, EROFS (auto-detected)

Arguments:
  -i, --input <FILE>              Path to the input image file
  -o, --output <DIR>              Path to the output directory
      --fs-config-path <FILE>     Custom fs_config file path (optional)
      --file-contexts-path <FILE> Custom file_contexts file path (optional)
  -l, --level <0-3>               Log level: 0=silent 1=basic 2=verbose 3=debug [default: 1]
  -c, --clean                     Remove existing files in the output directory

Examples:
  imgkit unpack -i system.img -o output/
  imgkit unpack -i super.img -o output/ -l 2
  imgkit unpack -i system.img -o output/ --clean

================================================================================
                              Pack Command
================================================================================

Usage: imgkit pack --type <TYPE> [OPTIONS] -o <OUTPUT>

Supported types: super, f2fs, ext4, erofs

--------------------------------------------------------------------------------
                            Super Partition Packing
--------------------------------------------------------------------------------

Usage: imgkit pack --type super [OPTIONS] -o <OUTPUT>

Required:
  -o, --output <FILE>             Path to the output image file
  -d, --device-size <SIZE|auto>   Device size in bytes, or 'auto' to calculate
  -g, --group <name:max_size>     Partition group definition, repeatable
  -p, --partition <name:attrs:size:group>  Partition definition, repeatable
  -i, --image <name=path>         Partition image mapping, repeatable

Optional:
  -m, --metadata-size <SIZE>      Maximum metadata size [default: 65536]
      --slots <NUM>               Number of metadata slots [default: 2]
  -n, --name <NAME>               Block device name [default: super]
  -b, --block-size <SIZE>         Logical block size [default: 4096]
  -a, --alignment <SIZE>          Partition alignment size [default: 1048576]
  -O, --alignment-offset <SIZE>   Alignment offset [default: 0]
  -x, --auto-slot-suffixing       Enable automatic slot suffixing (A/B)
      --virtual-ab                Enable Virtual A/B flag
  -F, --force-full-image          Force full (non-sparse) image output
  -S, --sparse                    Output in sparse image format

Examples:
  # VAB mode + sparse format
  imgkit pack --type super -o super.img -d auto \
    -g qti_dynamic_partitions:8589934592 \
    -p system:readonly:2147483648:qti_dynamic_partitions \
    -p vendor:readonly:524288000:qti_dynamic_partitions \
    -i system=system.img -i vendor=vendor.img \
    --virtual-ab -x -S

  # Fixed device size + raw format
  imgkit pack --type super -o super.img -d 8589934592 \
    -g main:8589934592 -p system:readonly:2147483648:main \
    -i system=system.img -F

--------------------------------------------------------------------------------
                            F2FS Filesystem Packing
--------------------------------------------------------------------------------

Usage: imgkit pack --type f2fs [OPTIONS] -s <SOURCE> -o <OUTPUT> -z <SIZE>

Required:
  -s, --source <DIR>              Source directory path
  -o, --output <FILE>             Path to the output image file
  -z, --size <SIZE>               Image size in bytes

Optional:
  -m, --mount-point <PATH>        Mount point path [default: /]
      --file-contexts <FILE>      file_contexts file path (SELinux)
      --fs-config <FILE>          fs_config file path (permissions)
      --label <NAME>              Volume label
      --timestamp <UNIX_TIME>     Fixed timestamp (Unix epoch)
      --root-uid <UID>            Root user UID [default: 0]
      --root-gid <GID>            Root user GID [default: 0]
      --readonly                  Enable read-only mode
      --project-quota             Enable project quota
      --casefold                  Enable case folding
      --compression               Enable compression
  -S, --sparse                    Output in sparse image format

Examples:
  imgkit pack --type f2fs -s system/ -o system.img -z 2147483648
  imgkit pack --type f2fs -s system/ -o system.img -z 2147483648 \
    --file-contexts file_contexts --fs-config fs_config \
    -m /system --readonly

--------------------------------------------------------------------------------
                            EXT4 Filesystem Packing
--------------------------------------------------------------------------------

Usage: imgkit pack --type ext4 [OPTIONS] -s <SOURCE> -o <OUTPUT> -z <SIZE>

Required:
  -s, --source <DIR>              Source directory path
  -o, --output <FILE>             Path to the output image file
  -z, --size <SIZE>               Image size in bytes

Optional:
  -m, --mount-point <PATH>        Mount point path [default: /]
      --file-contexts <FILE>      file_contexts file path (SELinux)
      --fs-config <FILE>          fs_config file path (permissions)
      --label <NAME>              Volume label
      --timestamp <UNIX_TIME>     Fixed timestamp (Unix epoch)
      --root-uid <UID>            Root user UID [default: 0]
      --root-gid <GID>            Root user GID [default: 0]

Examples:
  imgkit pack --type ext4 -s system/ -o system.img -z 2147483648
  imgkit pack --type ext4 -s system/ -o system.img -z 2147483648 \
    --file-contexts file_contexts --fs-config fs_config \
    -m /system --label system

--------------------------------------------------------------------------------
                            EROFS Filesystem Packing
--------------------------------------------------------------------------------

Usage: imgkit pack --type erofs [OPTIONS] -s <SOURCE> -o <OUTPUT>

Required:
  -s, --source <DIR>              Source directory path
  -o, --output <FILE>             Path to the output image file

Optional:
  -m, --mount-point <PATH>        Mount point path [default: /]
      --file-contexts <FILE>      file_contexts file path (SELinux)
      --fs-config <FILE>          fs_config file path (permissions)
      --label <NAME>              Volume label
  -b, --block-size <SIZE>         Block size [default: 4096]
      --timestamp <UNIX_TIME>     Fixed timestamp (Unix epoch)
      --uuid <UUID>               UUID (format: xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx)
      --root-uid <UID>            Root user UID [default: 0]
      --root-gid <GID>            Root user GID [default: 0]
      --compress <ALGO>           Compression algorithm: lz4, lz4hc, lzma, deflate, zstd
      --compress-level <LEVEL>    Compression level (range varies by algorithm, see below)

Compression level notes:
  lz4:     no level parameter
  lz4hc:   0-12 [default: 9]
  lzma:    0-9 (normal) or 100-109 (extreme) [default: 6]
  deflate: 0-9 [default: 1]
  zstd:    0-22 [default: 3]

Examples:
  imgkit pack --type erofs -s system/ -o system.img
  imgkit pack --type erofs -s system/ -o system.img \
    --compress lz4hc --compress-level 9 \
    --file-contexts file_contexts --fs-config fs_config \
    -m /system

================================================================================
"#;

#[derive(Subcommand)]
#[allow(clippy::large_enum_variant)]
pub enum Commands {
    #[command(about = "Unpack an image file (supports Super/F2FS/EXT4/EROFS)")]
    Unpack {
        #[arg(short, long, help = "Path to the input image file")]
        input: String,

        #[arg(short, long, help = "Path to the output directory")]
        output: String,

        #[arg(long, help = "Custom fs_config file path (optional)")]
        fs_config_path: Option<String>,

        #[arg(long, help = "Custom file_contexts file path (optional)")]
        file_contexts_path: Option<String>,

        #[arg(
            short,
            long,
            default_value = "1",
            help = "Log level: 0=silent 1=basic 2=verbose 3=debug"
        )]
        level: u8,

        #[arg(short, long, help = "Remove existing files in the output directory")]
        clean: bool,
    },

    #[command(about = "Pack an image (supports Super/F2FS/EXT4/EROFS)")]
    Pack {
        #[arg(short = 't', long, help = "Image type: super, f2fs, ext4, erofs")]
        r#type: String,

        #[arg(short, long, help = "Path to the output image file")]
        output: String,

        // Filesystem packing arguments (f2fs, ext4)
        #[arg(short, long, help = "Source directory path (required for f2fs/ext4)")]
        source: Option<String>,

        #[arg(
            short = 'z',
            long,
            help = "Image size in bytes (required for f2fs/ext4)"
        )]
        size: Option<String>,

        #[arg(
            short,
            long,
            default_value = "/",
            help = "Mount point path (f2fs/ext4)"
        )]
        mount_point: String,

        #[arg(long, help = "file_contexts file path (SELinux contexts)")]
        file_contexts: Option<String>,

        #[arg(long, help = "fs_config file path (permission config)")]
        fs_config: Option<String>,

        #[arg(long, help = "Volume label")]
        label: Option<String>,

        #[arg(long, help = "Fixed timestamp (Unix epoch)")]
        timestamp: Option<u64>,

        #[arg(long, default_value = "0", help = "Root user UID")]
        root_uid: u32,

        #[arg(long, default_value = "0", help = "Root user GID")]
        root_gid: u32,

        // F2FS-specific arguments
        #[arg(long, help = "Enable read-only mode (f2fs)")]
        readonly: bool,

        #[arg(long, help = "Enable project quota (f2fs)")]
        project_quota: bool,

        #[arg(long, help = "Enable case folding (f2fs)")]
        casefold: bool,

        #[arg(long, help = "Enable compression (f2fs)")]
        compression: bool,

        // EROFS-specific arguments
        #[arg(
            long,
            help = "Compression algorithm (erofs): lz4, lz4hc, lzma, deflate, zstd"
        )]
        compress: Option<String>,

        #[arg(
            long,
            help = "Compression level (erofs): lz4hc=0-12, lzma=0-9/100-109, deflate=0-9, zstd=0-22"
        )]
        compress_level: Option<u32>,

        #[arg(
            long,
            help = "UUID (erofs, format: xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx)"
        )]
        uuid: Option<String>,

        // Super partition arguments
        #[arg(short, long, help = "Device size in bytes, or 'auto' (super)")]
        device_size: Option<String>,

        #[arg(
            short,
            long,
            default_value = "65536",
            help = "Maximum metadata size in bytes (super)"
        )]
        metadata_size: u32,

        #[arg(
            long,
            default_value = "2",
            help = "Number of metadata slots (super, usually 2)"
        )]
        slots: u32,

        #[arg(
            short,
            long,
            default_value = "super",
            help = "Block device name (super)"
        )]
        name: String,

        #[arg(
            short = 'b',
            long,
            default_value = "4096",
            help = "Logical block size in bytes"
        )]
        block_size: u32,

        #[arg(
            short = 'a',
            long,
            default_value = "1048576",
            help = "Partition alignment size in bytes (super)"
        )]
        alignment: u32,

        #[arg(
            short = 'O',
            long,
            default_value = "0",
            help = "Alignment offset in bytes (super)"
        )]
        alignment_offset: u32,

        #[arg(
            short,
            long,
            help = "Partition group definition (super, format: name:max_size), repeatable"
        )]
        group: Vec<String>,

        #[arg(
            short,
            long,
            help = "Partition definition (super, format: name:attrs:size:group), repeatable"
        )]
        partition: Vec<String>,

        #[arg(
            short,
            long,
            help = "Partition image mapping (super, format: name=path), repeatable"
        )]
        image: Vec<String>,

        #[arg(
            short = 'x',
            long,
            help = "Enable automatic slot suffixing (super, A/B)"
        )]
        auto_slot_suffixing: bool,

        #[arg(long, help = "Enable Virtual A/B flag (super)")]
        virtual_ab: bool,

        #[arg(
            short = 'F',
            long,
            help = "Force full (non-sparse) image output (super)"
        )]
        force_full_image: bool,

        // Common arguments
        #[arg(short = 'S', long, help = "Output in sparse image format")]
        sparse: bool,

        #[arg(
            short,
            long,
            default_value = "1",
            help = "Log level: 0=silent 1=basic 2=verbose 3=debug"
        )]
        level: u8,
    },
}

// CLI main entry point
pub fn run(cli: Cli) -> Result<()> {
    match cli.command {
        Commands::Unpack {
            input,
            output,
            fs_config_path,
            file_contexts_path,
            level,
            clean,
        } => {
            crate::utils::logger::init(level);
            cli::run_extract(&input, &output, fs_config_path, file_contexts_path, clean)
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
            crate::utils::logger::init(level);

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
