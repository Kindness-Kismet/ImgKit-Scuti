<div align="center">

# ImgKit-Scuti

[![English](https://img.shields.io/badge/English-red)](README.md)
[![简体中文](https://img.shields.io/badge/简体中文-blue)](.github/docs/README.zh-CN.md)

A CLI toolkit for Android image unpacking and repacking.

</div>

## Quick Navigation

- [Supported Unpack Formats](#supported-unpack-formats)
- [Supported Pack Types](#supported-pack-types)
- [Why ImgKit-Scuti](#why-imgkit-scuti)
- [Quick Start](#quick-start)
- [Command Overview](#command-overview)
- [Help Reference](#help-reference)

## Supported Unpack Formats

`imgkit_scuti unpack` auto-detects input image type and supports:

| Type | Unpack Support | Notes |
|---|---|---|
| EXT4 | Yes | Direct detection and extraction |
| F2FS | Yes | Direct detection and extraction |
| EROFS | Yes | Direct detection and extraction |
| Super (Android LP) | Yes | Partition-level extraction |
| Android Sparse image | Yes | Auto-converts sparse then detects filesystem |
| OTA `payload.bin` | Yes | Extracts partition images from full OTA payloads |

Incremental OTA payloads require source partition images and are not supported yet.

## Supported Pack Types

`imgkit_scuti pack` currently supports:

- `super`
- `ext4`
- `f2fs`
- `erofs`

Sparse output is supported in applicable scenarios.

Packing OTA `payload.bin` is not supported: a usable payload must be signed with the OEM private key, so a self-built payload cannot be applied by `update_engine`.

## Why ImgKit-Scuti

- One CLI for multiple formats, no tool switching.
- Auto detection for unpack flow.
- Complete pack flow for `ext4`/`f2fs`/`erofs` and `super`.
- Android metadata integration with `file_contexts` and `fs_config`.
- Script-friendly command surface for CI pipelines.

## Quick Start

```bash
cargo build --release
./target/release/imgkit_scuti --help
```

## Command Overview

```bash
imgkit_scuti <SUBCOMMAND> [OPTIONS]
```

Available subcommands:

- `unpack`: extract image contents to directory
- `pack`: build image from directory or partition images

Show detailed help:

```bash
imgkit_scuti --help
imgkit_scuti unpack --help
imgkit_scuti pack --help
```

## Help Reference

### `unpack`

```bash
imgkit_scuti unpack [OPTIONS] -i <INPUT> -o <OUTPUT>
```

| Option | Required | Description |
|---|---|---|
| `-i, --input <FILE>` | Yes | Path to input image file |
| `-o, --output <DIR>` | Yes | Path to output directory |
| `--fs-config-path <FILE>` | No | Custom output path for generated `fs_config` |
| `--file-contexts-path <FILE>` | No | Custom output path for generated `file_contexts` |
| `-l, --level <0-3>` | No | Log level, default `1` |
| `-c, --clean` | No | Remove existing extracted files before unpack |

Examples:

```bash
imgkit_scuti unpack -i system.img -o out/system
imgkit_scuti unpack -i super.img -o out/super -l 2
imgkit_scuti unpack -i system.img -o out/system --clean
```

### `pack`

```bash
imgkit_scuti pack --type <TYPE> [OPTIONS] -o <OUTPUT>
```

`TYPE` supports: `super`, `f2fs`, `ext4`, `erofs`.

### `pack --type super`

```bash
imgkit_scuti pack --type super [OPTIONS] -o <OUTPUT>
```

| Option | Required | Description |
|---|---|---|
| `-o, --output <FILE>` | Yes | Output super image path |
| `-d, --device-size <SIZE|auto>` | No | Device size in bytes or `auto` |
| `-g, --group <name:max_size>` | No | Partition group definition, repeatable |
| `-p, --partition <name:attrs:size:group>` | No | Partition definition, repeatable |
| `-i, --image <name=path>` | No | Partition image mapping, repeatable |
| `-m, --metadata-size <SIZE>` | No | Metadata size, default `65536` |
| `--slots <NUM>` | No | Metadata slots, default `2` |
| `-n, --name <NAME>` | No | Block device name, default `super` |
| `-b, --block-size <SIZE>` | No | Logical block size, default `4096` |
| `-a, --alignment <SIZE>` | No | Alignment size, default `1048576` |
| `-O, --alignment-offset <SIZE>` | No | Alignment offset, default `0` |
| `-x, --auto-slot-suffixing` | No | Enable automatic A/B suffixing |
| `--virtual-ab` | No | Enable Virtual A/B flag |
| `-F, --force-full-image` | No | Force non-sparse output |
| `-S, --sparse` | No | Output sparse image |
| `-l, --level <0-3>` | No | Log level, default `1` |

Examples:

```bash
imgkit_scuti pack --type super -o super.img -d auto \
  -g qti_dynamic_partitions:8589934592 \
  -p system:readonly:2147483648:qti_dynamic_partitions \
  -p vendor:readonly:524288000:qti_dynamic_partitions \
  -i system=system.img -i vendor=vendor.img \
  --virtual-ab -x -S

imgkit_scuti pack --type super -o super.img -d 8589934592 \
  -g main:8589934592 -p system:readonly:2147483648:main \
  -i system=system.img -F
```

### `pack --type f2fs`

```bash
imgkit_scuti pack --type f2fs [OPTIONS] -s <SOURCE> -o <OUTPUT> -z <SIZE>
```

| Option | Required | Description |
|---|---|---|
| `-s, --source <DIR>` | Yes | Source directory |
| `-o, --output <FILE>` | Yes | Output image path |
| `-z, --size <SIZE>` | Yes | Image size in bytes |
| `-m, --mount-point <PATH>` | No | Mount point, default `/` |
| `--file-contexts <FILE>` | No | SELinux `file_contexts` path |
| `--fs-config <FILE>` | No | `fs_config` path |
| `--label <NAME>` | No | Volume label |
| `--timestamp <UNIX_TIME>` | No | Fixed timestamp |
| `--root-uid <UID>` | No | Root UID, default `0` |
| `--root-gid <GID>` | No | Root GID, default `0` |
| `--readonly` | No | Enable read-only mode |
| `--project-quota` | No | Enable project quota |
| `--casefold` | No | Enable case folding |
| `--compression` | No | Enable compression |
| `-S, --sparse` | No | Output sparse image |
| `-l, --level <0-3>` | No | Log level, default `1` |

Examples:

```bash
imgkit_scuti pack --type f2fs -s system/ -o system.img -z 2147483648
imgkit_scuti pack --type f2fs -s system/ -o system.img -z 2147483648 \
  --file-contexts file_contexts --fs-config fs_config \
  -m /system --readonly
```

### `pack --type ext4`

```bash
imgkit_scuti pack --type ext4 [OPTIONS] -s <SOURCE> -o <OUTPUT> -z <SIZE>
```

| Option | Required | Description |
|---|---|---|
| `-s, --source <DIR>` | Yes | Source directory |
| `-o, --output <FILE>` | Yes | Output image path |
| `-z, --size <SIZE>` | Yes | Image size in bytes |
| `-m, --mount-point <PATH>` | No | Mount point, default `/` |
| `--file-contexts <FILE>` | No | SELinux `file_contexts` path |
| `--fs-config <FILE>` | No | `fs_config` path |
| `--label <NAME>` | No | Volume label |
| `--timestamp <UNIX_TIME>` | No | Fixed timestamp |
| `--root-uid <UID>` | No | Root UID, default `0` |
| `--root-gid <GID>` | No | Root GID, default `0` |
| `-S, --sparse` | No | Output sparse image |
| `-l, --level <0-3>` | No | Log level, default `1` |

Examples:

```bash
imgkit_scuti pack --type ext4 -s system/ -o system.img -z 2147483648
imgkit_scuti pack --type ext4 -s system/ -o system.img -z 2147483648 \
  --file-contexts file_contexts --fs-config fs_config \
  -m /system --label system
```

### `pack --type erofs`

```bash
imgkit_scuti pack --type erofs [OPTIONS] -s <SOURCE> -o <OUTPUT>
```

| Option | Required | Description |
|---|---|---|
| `-s, --source <DIR>` | Yes | Source directory |
| `-o, --output <FILE>` | Yes | Output image path |
| `-m, --mount-point <PATH>` | No | Mount point, default `/` |
| `--file-contexts <FILE>` | No | SELinux `file_contexts` path |
| `--fs-config <FILE>` | No | `fs_config` path |
| `--label <NAME>` | No | Volume label |
| `-b, --block-size <SIZE>` | No | Block size, default `4096` |
| `--timestamp <UNIX_TIME>` | No | Fixed timestamp |
| `--uuid <UUID>` | No | UUID (`xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx`) |
| `--root-uid <UID>` | No | Root UID, default `0` |
| `--root-gid <GID>` | No | Root GID, default `0` |
| `--compress <ALGO>` | No | Compression: `lz4`, `lz4hc`, `lzma`, `deflate`, `zstd` |
| `--compress-level <LEVEL>` | No | Compression level by algorithm |
| `-S, --sparse` | No | Output sparse image |
| `-l, --level <0-3>` | No | Log level, default `1` |

Compression level notes:

- `lz4`: fixed level (no tuning)
- `lz4hc`: `0-12`, default `9`
- `lzma`: `0-9` and dictionary presets `100-109`
- `deflate`: `0-9`, default `6`
- `zstd`: `0-22`, default `3`

Examples:

```bash
imgkit_scuti pack --type erofs -s system/ -o system.img
imgkit_scuti pack --type erofs -s system/ -o system.img \
  --compress lz4hc --compress-level 9 \
  --file-contexts file_contexts --fs-config fs_config \
  -m /system
```
