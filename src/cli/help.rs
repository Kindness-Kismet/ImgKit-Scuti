// 长帮助文本, 通过 clap 的 after_help 展示。

pub const FULL_HELP: &str = r#"
================================================================================
                              Unpack Command
================================================================================

Usage: imgkit_scuti unpack [OPTIONS] -i <INPUT> -o <OUTPUT>

Supported formats: Super, F2FS, EXT4, EROFS, OTA payload.bin (auto-detected)

Arguments:
  -i, --input <FILE>              Path to the input image file
  -o, --output <DIR>              Path to the output directory (not needed with --list)
      --fs-config-path <FILE>     Custom fs_config file path (optional)
      --file-contexts-path <FILE> Custom file_contexts file path (optional)
  -p, --partition <NAME>          Extract only the named partition, repeatable
                                  (super and payload only; default: all)
      --list                      List partition names without extracting
                                  (super and payload only)
  -l, --level <0-3>               Log level: 0=silent 1=basic 2=verbose 3=debug [default: 1]
  -c, --clean                     Remove existing files in the output directory

Examples:
  imgkit_scuti unpack -i system.img -o output/
  imgkit_scuti unpack -i super.img -o output/ -l 2
  imgkit_scuti unpack -i payload.bin -o partitions/
  imgkit_scuti unpack -i payload.bin --list
  imgkit_scuti unpack -i payload.bin -o partitions/ -p system -p vendor
  imgkit_scuti unpack -i super.img -o partitions/ -p mi_ext_a
  imgkit_scuti unpack -i system.img -o output/ --clean

================================================================================
                              Pack Command
================================================================================

Usage: imgkit_scuti pack --type <TYPE> [OPTIONS] -o <OUTPUT>

Supported types: super, f2fs, ext4, erofs

--------------------------------------------------------------------------------
                            Super Partition Packing
--------------------------------------------------------------------------------

Usage: imgkit_scuti pack --type super [OPTIONS] -o <OUTPUT>

Required:
  -o, --output <FILE>             Path to the output image file
  -d, --device-size <SIZE|auto>   Device size in bytes, or 'auto' to calculate
  -g, --group <name:max_size>     Partition group definition, repeatable
  -p, --partition <name:attrs:size:group>  Partition definition, repeatable
  -i, --image <name=path>         Partition image mapping, repeatable

Optional:
      --metadata-size <SIZE>      Maximum metadata size [default: 65536]
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
  imgkit_scuti pack --type super -o super.img -d auto \
    -g qti_dynamic_partitions:8589934592 \
    -p system:readonly:2147483648:qti_dynamic_partitions \
    -p vendor:readonly:524288000:qti_dynamic_partitions \
    -i system=system.img -i vendor=vendor.img \
    --virtual-ab -x -S

  # Fixed device size + raw format
  imgkit_scuti pack --type super -o super.img -d 8589934592 \
    -g main:8589934592 -p system:readonly:2147483648:main \
    -i system=system.img -F

--------------------------------------------------------------------------------
                            F2FS Filesystem Packing
--------------------------------------------------------------------------------

Usage: imgkit_scuti pack --type f2fs [OPTIONS] -s <SOURCE> -o <OUTPUT> -z <SIZE>

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
  imgkit_scuti pack --type f2fs -s system/ -o system.img -z 2147483648
  imgkit_scuti pack --type f2fs -s system/ -o system.img -z 2147483648 \
    --file-contexts file_contexts --fs-config fs_config \
    -m /system --readonly

--------------------------------------------------------------------------------
                            EXT4 Filesystem Packing
--------------------------------------------------------------------------------

Usage: imgkit_scuti pack --type ext4 [OPTIONS] -s <SOURCE> -o <OUTPUT> -z <SIZE>

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
  imgkit_scuti pack --type ext4 -s system/ -o system.img -z 2147483648
  imgkit_scuti pack --type ext4 -s system/ -o system.img -z 2147483648 \
    --file-contexts file_contexts --fs-config fs_config \
    -m /system --label system

--------------------------------------------------------------------------------
                            EROFS Filesystem Packing
--------------------------------------------------------------------------------

Usage: imgkit_scuti pack --type erofs [OPTIONS] -s <SOURCE> -o <OUTPUT>

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
  imgkit_scuti pack --type erofs -s system/ -o system.img
  imgkit_scuti pack --type erofs -s system/ -o system.img \
    --compress lz4hc --compress-level 9 \
    --file-contexts file_contexts --fs-config fs_config \
    -m /system

================================================================================
"#;
