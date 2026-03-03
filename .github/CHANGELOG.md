## v1.2.2 更新日志

### 本次更新
- 修复 F2FS 在部分镜像中提取为空目录的问题。
- 调整 F2FS 节点解析链路: 在初始化阶段读取最新 checkpoint 包, 并解析 compact summary 中的 NAT journal 映射。
- 查询 NID 映射时优先命中 NAT journal, 未命中时再读取主 NAT 区域条目。
- 增加 NAT 双副本兜底: 当主 NAT 条目为空或无效时, 回退读取第二副本并校验块地址有效性。

<details>
<summary>English Version</summary>

## v1.2.2 Changelog

### Highlights
- Fixed an F2FS issue where some images were extracted as empty directories.
- Updated node resolution flow to load the latest checkpoint package and parse NAT journal mappings from compact summary during initialization.
- NAT lookup now prefers journal mappings first, then falls back to the main NAT area when no journal entry is found.
- Added dual NAT copy fallback: when the primary NAT entry is empty or invalid, the reader falls back to the secondary copy with block-address validation.

</details>
