<div align="center">

# ImgKit-Scuti

[![English](https://img.shields.io/badge/English-red)](../../README.md)
[![简体中文](https://img.shields.io/badge/简体中文-blue)](README.zh-CN.md)

面向 Android 镜像处理场景的命令行工具。

</div>

## 快速导航

- [支持的解包格式](#支持的解包格式)
- [支持的打包类型](#支持的打包类型)
- [工具优势](#工具优势)
- [快速开始](#快速开始)
- [命令总览](#命令总览)
- [帮助参数总表](#帮助参数总表)

## 支持的解包格式

`imgkit_scuti unpack` 会自动识别输入镜像格式，当前支持：

| 类型 | 是否支持解包 | 说明 |
|---|---|---|
| EXT4 | 是 | 直接识别与提取 |
| F2FS | 是 | 直接识别与提取 |
| EROFS | 是 | 直接识别与提取 |
| Super (Android LP) | 是 | 可按分区提取内容 |
| Android Sparse 镜像 | 是 | 自动去 sparse 后继续识别实际文件系统 |
| OTA `payload.bin` | 是 | 从完整 OTA 包中提取分区镜像 |

增量 OTA 需要旧版本分区镜像，当前暂不支持。

## 支持的打包类型

`imgkit_scuti pack` 当前支持：

- `super`
- `ext4`
- `f2fs`
- `erofs`

在支持的场景下可输出 sparse 镜像。

不支持打包 OTA `payload.bin`：可用的 payload 必须由厂商私钥签名，自行构建的 payload 无法被 `update_engine` 应用。

## 工具优势

- 一套 CLI 覆盖多格式，无需在多个工具间切换。
- 解包自动识别，减少手工判断格式步骤。
- 打包能力完整，支持从目录构建 `ext4`、`f2fs`、`erofs`，也支持 `super` 分区组装。
- 可接入 Android 元数据流程，支持 `file_contexts` 与 `fs_config`。
- 命令参数稳定，适合脚本与 CI 集成。

## 快速开始

```bash
cargo build --release
./target/release/imgkit_scuti --help
```

## 命令总览

```bash
imgkit_scuti <SUBCOMMAND> [OPTIONS]
```

可用子命令：

- `unpack`：解包镜像到目录
- `pack`：从目录或分区镜像打包

查看详细参数：

```bash
imgkit_scuti --help
imgkit_scuti unpack --help
imgkit_scuti pack --help
```

## 帮助参数总表

### `unpack`

```bash
imgkit_scuti unpack [OPTIONS] -i <INPUT> -o <OUTPUT>
```

| 参数 | 必填 | 说明 |
|---|---|---|
| `-i, --input <FILE>` | 是 | 输入镜像路径 |
| `-o, --output <DIR>` | 是 | 输出目录路径。使用 `--list` 时无需提供 |
| `--fs-config-path <FILE>` | 否 | 自定义导出的 `fs_config` 路径 |
| `--file-contexts-path <FILE>` | 否 | 自定义导出的 `file_contexts` 路径 |
| `-p, --partition <NAME>` | 否 | 仅提取指定分区，可重复传入。仅对 `super` 与 `payload.bin` 生效，默认提取全部 |
| `--list` | 否 | 仅列出分区名后退出，不执行提取。仅对 `super` 与 `payload.bin` 生效 |
| `-l, --level <0-3>` | 否 | 日志级别，默认 `1` |
| `-c, --clean` | 否 | 解包前删除已有输出 |

示例：

```bash
imgkit_scuti unpack -i system.img -o out/system
imgkit_scuti unpack -i super.img -o out/super -l 2
imgkit_scuti unpack -i payload.bin --list
imgkit_scuti unpack -i payload.bin -o out/payload -p system -p vendor
imgkit_scuti unpack -i super.img -o out/super -p mi_ext
imgkit_scuti unpack -i system.img -o out/system --clean
```

### `pack`

```bash
imgkit_scuti pack --type <TYPE> [OPTIONS] -o <OUTPUT>
```

`TYPE` 支持：`super`、`f2fs`、`ext4`、`erofs`。

### `pack --type super`

```bash
imgkit_scuti pack --type super [OPTIONS] -o <OUTPUT>
```

| 参数 | 必填 | 说明 |
|---|---|---|
| `-o, --output <FILE>` | 是 | 输出 super 镜像路径 |
| `-d, --device-size <SIZE|auto>` | 否 | 设备大小或 `auto` |
| `-g, --group <name:max_size>` | 否 | 分区组定义，可重复 |
| `-p, --partition <name:attrs:size:group>` | 否 | 分区定义，可重复 |
| `-i, --image <name=path>` | 否 | 分区镜像映射，可重复 |
| `--metadata-size <SIZE>` | 否 | 元数据大小，默认 `65536` |
| `--slots <NUM>` | 否 | 元数据槽数量，默认 `2` |
| `-n, --name <NAME>` | 否 | 块设备名，默认 `super` |
| `-b, --block-size <SIZE>` | 否 | 逻辑块大小，默认 `4096` |
| `-a, --alignment <SIZE>` | 否 | 分区对齐大小，默认 `1048576` |
| `-O, --alignment-offset <SIZE>` | 否 | 对齐偏移，默认 `0` |
| `-x, --auto-slot-suffixing` | 否 | 启用 A/B 自动后缀 |
| `--virtual-ab` | 否 | 启用 Virtual A/B 标志 |
| `-F, --force-full-image` | 否 | 强制输出非 sparse 镜像 |
| `-S, --sparse` | 否 | 输出 sparse 镜像 |
| `-l, --level <0-3>` | 否 | 日志级别，默认 `1` |

示例：

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

| 参数 | 必填 | 说明 |
|---|---|---|
| `-s, --source <DIR>` | 是 | 源目录路径 |
| `-o, --output <FILE>` | 是 | 输出镜像路径 |
| `-z, --size <SIZE>` | 是 | 镜像大小 |
| `-m, --mount-point <PATH>` | 否 | 挂载点，默认 `/` |
| `--file-contexts <FILE>` | 否 | SELinux `file_contexts` 路径 |
| `--fs-config <FILE>` | 否 | `fs_config` 路径 |
| `--label <NAME>` | 否 | 卷标 |
| `--timestamp <UNIX_TIME>` | 否 | 固定时间戳 |
| `--root-uid <UID>` | 否 | 根目录 UID，默认 `0` |
| `--root-gid <GID>` | 否 | 根目录 GID，默认 `0` |
| `--readonly` | 否 | 启用只读模式 |
| `--project-quota` | 否 | 启用 project quota |
| `--casefold` | 否 | 启用大小写折叠 |
| `--compression` | 否 | 启用压缩 |
| `-S, --sparse` | 否 | 输出 sparse 镜像 |
| `-l, --level <0-3>` | 否 | 日志级别，默认 `1` |

示例：

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

| 参数 | 必填 | 说明 |
|---|---|---|
| `-s, --source <DIR>` | 是 | 源目录路径 |
| `-o, --output <FILE>` | 是 | 输出镜像路径 |
| `-z, --size <SIZE>` | 是 | 镜像大小 |
| `-m, --mount-point <PATH>` | 否 | 挂载点，默认 `/` |
| `--file-contexts <FILE>` | 否 | SELinux `file_contexts` 路径 |
| `--fs-config <FILE>` | 否 | `fs_config` 路径 |
| `--label <NAME>` | 否 | 卷标 |
| `--timestamp <UNIX_TIME>` | 否 | 固定时间戳 |
| `--root-uid <UID>` | 否 | 根目录 UID，默认 `0` |
| `--root-gid <GID>` | 否 | 根目录 GID，默认 `0` |
| `-S, --sparse` | 否 | 输出 sparse 镜像 |
| `-l, --level <0-3>` | 否 | 日志级别，默认 `1` |

示例：

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

| 参数 | 必填 | 说明 |
|---|---|---|
| `-s, --source <DIR>` | 是 | 源目录路径 |
| `-o, --output <FILE>` | 是 | 输出镜像路径 |
| `-m, --mount-point <PATH>` | 否 | 挂载点，默认 `/` |
| `--file-contexts <FILE>` | 否 | SELinux `file_contexts` 路径 |
| `--fs-config <FILE>` | 否 | `fs_config` 路径 |
| `--label <NAME>` | 否 | 卷标 |
| `-b, --block-size <SIZE>` | 否 | 块大小，默认 `4096` |
| `--timestamp <UNIX_TIME>` | 否 | 固定时间戳 |
| `--uuid <UUID>` | 否 | UUID（`xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx`） |
| `--root-uid <UID>` | 否 | 根目录 UID，默认 `0` |
| `--root-gid <GID>` | 否 | 根目录 GID，默认 `0` |
| `--compress <ALGO>` | 否 | 压缩算法：`lz4`、`lz4hc`、`lzma`、`deflate`、`zstd` |
| `--compress-level <LEVEL>` | 否 | 压缩级别，取值随算法变化 |
| `-S, --sparse` | 否 | 输出 sparse 镜像 |
| `-l, --level <0-3>` | 否 | 日志级别，默认 `1` |

压缩级别说明：

- `lz4`：固定级别，不支持调节
- `lz4hc`：`0-12`，默认 `9`
- `lzma`：`0-9` 和字典预设 `100-109`
- `deflate`：`0-9`，默认 `6`
- `zstd`：`0-22`，默认 `3`

示例：

```bash
imgkit_scuti pack --type erofs -s system/ -o system.img
imgkit_scuti pack --type erofs -s system/ -o system.img \
  --compress lz4hc --compress-level 9 \
  --file-contexts file_contexts --fs-config fs_config \
  -m /system
```
