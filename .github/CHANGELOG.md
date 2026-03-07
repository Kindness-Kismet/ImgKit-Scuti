## v1.2.4 更新日志

### 本次更新
- 修好 F2FS 读取 NAT journal 的逻辑, 现在会按当前生效的 checkpoint 取数据, 提取结果更稳。
- 调整 F2FS 奇偶块处理和 SELinux 规则输出, 减少回打包后配置不一致的问题。

<details>
<summary>English Version</summary>

## v1.2.4 Changelog

### Highlights
- Fixed F2FS NAT journal loading to follow the active checkpoint, making extraction results more stable.
- Adjusted F2FS parity handling and SELinux rule output to reduce repack configuration mismatches.

</details>
