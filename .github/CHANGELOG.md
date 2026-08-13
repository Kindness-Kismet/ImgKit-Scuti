## v1.2.6 更新日志

### 本次更新
- 新增 OTA `payload.bin` 解包：`unpack` 自动识别全量 OTA 包并提取其中的分区镜像，覆盖 REPLACE、REPLACE_BZ、REPLACE_XZ、ZSTD、ZERO 操作，逐个分区做 SHA-256 校验。差分包需要旧版本镜像参与还原，解析阶段即报错说明不支持；打包 payload 需要 OEM 私钥签名，同样不提供。
- 新增按分区提取：`-p, --partition` 指定要提取的分区，可重复传入；`--list` 列出镜像内的分区名。两者适用于 `super.img` 与 `payload.bin`，用 `--list` 时不必给 `-o`。super 的分区名可省略槽位后缀，写 `mi_ext` 即可匹配 `mi_ext_a`；分区名不存在时会报错并列出可用的分区。
- 错误提示与日志输出统一为英文。
- 修好 F2FS 打包的尺寸校验：`-z` 给出的尺寸不足以容纳文件系统时，直接报错说明镜像尺寸过小。
- 修好 `pack` 的短选项冲突：`-m` 固定表示挂载点，super 打包的元数据大小改用 `--metadata-size`。
- 修复 lz4 解压库的高危漏洞：升级至修复版本并默认启用安全解码，避免处理损坏或恶意构造的镜像时读出内存中的残留数据。

<details>
<summary>English Version</summary>

## v1.2.6 Changelog

### Highlights
- Added OTA `payload.bin` unpacking: `unpack` detects full OTA packages automatically and extracts the partition images inside, covering REPLACE, REPLACE_BZ, REPLACE_XZ, ZSTD and ZERO operations, with a SHA-256 check per partition. Incremental payloads need the previous images to reconstruct and are rejected during parsing; packing a payload requires the OEM private key and is likewise not offered.
- Added per-partition extraction: `-p, --partition` names a partition to extract and can be repeated, and `--list` prints the partition names in an image. Both apply to `super.img` and `payload.bin`, and `-o` is not required with `--list`. Super partition names may omit the slot suffix, so `mi_ext` matches `mi_ext_a`; an unknown name fails with the list of available partitions.
- Error messages and log output are now all in English.
- Fixed the F2FS packing size check: a `-z` too small to hold the filesystem now fails with a message stating that the image size is insufficient.
- Fixed the short option clash in `pack`: `-m` always means the mount point, and the super metadata size uses `--metadata-size`.
- Fixed a high-severity flaw in the lz4 decompression library: upgraded to a fixed version with safe decoding enabled by default, so corrupt or maliciously crafted images cannot expose leftover memory contents.

</details>
