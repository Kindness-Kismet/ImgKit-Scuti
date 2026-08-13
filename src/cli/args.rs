// CLI argument definitions.

use crate::cli::help::FULL_HELP;
use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "imgkit_scuti")]
#[command(about = "Android image tool with OTA payload unpacking support")]
#[command(after_help = FULL_HELP)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand)]
#[allow(clippy::large_enum_variant)]
pub enum Commands {
    #[command(about = "Unpack an image file (supports Super/F2FS/EXT4/EROFS/Payload)")]
    Unpack {
        #[arg(short, long, help = "Path to the input image file")]
        input: String,

        #[arg(
            short,
            long,
            required_unless_present = "list",
            help = "Path to the output directory"
        )]
        output: Option<String>,

        #[arg(long, help = "Custom fs_config file path (optional)")]
        fs_config_path: Option<String>,

        #[arg(long, help = "Custom file_contexts file path (optional)")]
        file_contexts_path: Option<String>,

        #[arg(
            short,
            long,
            help = "Extract only the named partition (super/payload), repeatable"
        )]
        partition: Vec<String>,

        #[arg(long, help = "List partition names without extracting (super/payload)")]
        list: bool,

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

        // 短选项 -m 已由 mount_point 占用, 这里只提供长选项
        #[arg(
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
