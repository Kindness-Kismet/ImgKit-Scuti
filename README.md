# imgkit

`imgkit` 是一个面向 Android 镜像处理场景的命令行工具，聚焦两件事：
- 解包已有镜像，恢复到可编辑目录
- 从目录或分区镜像重新打包，生成可刷写镜像

它统一了常见文件系统与容器格式的处理流程，适合 ROM 处理、分区内容分析、自动化构建链路。

## 支持的解包格式

`imgkit unpack` 会自动识别输入镜像格式，当前支持：

| 类型 | 是否支持解包 | 说明 |
|---|---|---|
| EXT4 | 是 | 直接识别与提取 |
| F2FS | 是 | 直接识别与提取 |
| EROFS | 是 | 直接识别与提取 |
| Super (Android LP) | 是 | 可按分区提取内容 |
| Android Sparse 镜像 | 是 | 自动去 sparse 后继续识别实际文件系统 |

## 支持的打包类型

`imgkit pack` 当前支持：
- `super`
- `ext4`
- `f2fs`
- `erofs`

在支持的场景下可输出 sparse 镜像。

## 工具优势

- 一套 CLI 覆盖多格式：无需在多个工具间切换。
- 解包自动识别：减少手工判断格式步骤。
- 打包能力完整：支持从目录构建 ext4、f2fs、erofs，也支持 super 分区组装。
- 适配 Android 元数据流程：可接入 `file_contexts` 与 `fs_config`。
- 工程化友好：命令参数稳定，适合脚本与 CI 集成。

## 快速开始

```bash
cargo build --release
./target/release/imgkit --help
```

## 命令总览

```bash
imgkit <SUBCOMMAND> [OPTIONS]
```

可用子命令：
- `unpack`：解包镜像到目录
- `pack`：从目录或分区镜像打包

查看详细参数：

```bash
imgkit --help
imgkit unpack --help
imgkit pack --help
```

## 解包用法

```bash
imgkit unpack -i <INPUT_IMAGE> -o <OUTPUT_DIR> [OPTIONS]
```

常用示例：

```bash
# 基本解包
imgkit unpack -i system.img -o out/system

# 解包前清理输出目录
imgkit unpack -i system.img -o out/system -c

# 调整日志级别（0-3）
imgkit unpack -i super.img -o out/super -l 2
```

## 打包用法

```bash
imgkit pack --type <super|ext4|f2fs|erofs> [OPTIONS]
```

### 打包 super

```bash
imgkit pack --type super -o super.img -d auto \
  -g main:8589934592 \
  -p system:readonly:2147483648:main \
  -p vendor:readonly:524288000:main \
  -i system=system.img \
  -i vendor=vendor.img \
  -S
```

### 打包 ext4

```bash
imgkit pack --type ext4 -s system/ -o system.img -z 2147483648
```

### 打包 f2fs

```bash
imgkit pack --type f2fs -s system/ -o system.img -z 2147483648 --readonly
```

### 打包 erofs

```bash
imgkit pack --type erofs -s system/ -o system.img --compress lz4hc --compress-level 9
```

## 常用参数提示

- `-l, --level <0-3>`：日志级别（默认 `1`）
- `-S, --sparse`：输出 sparse 镜像（支持场景下）
- `--file-contexts <FILE>`：SELinux 上下文文件
- `--fs-config <FILE>`：权限配置文件
