// F2FS 配置解析器
//
// 解析 file_contexts 与 fs_config 文件.

use crate::filesystem::f2fs::Result;
use regex::Regex;
use std::collections::HashMap;
use std::fs;
use std::path::Path;

fn normalize_config_path(path: &str) -> String {
    let mut normalized = if path.starts_with('/') {
        path.to_string()
    } else {
        format!("/{}", path)
    };
    while normalized.len() > 1 && normalized.ends_with('/') {
        normalized.pop();
    }
    normalized
}

// SELinux 上下文条目
#[derive(Debug, Clone)]
pub struct SelinuxEntry {
    pub pattern: String,
    pub regex: Regex,
    pub context: String,
}

// SELinux 上下文管理器
#[derive(Debug)]
pub struct SelinuxContexts {
    entries: Vec<SelinuxEntry>,
    cache: HashMap<String, String>,
}

impl SelinuxContexts {
    // 从文件加载
    pub fn from_file(path: &Path) -> Result<Self> {
        let content = fs::read_to_string(path)?;
        Self::parse(&content)
    }

    // 解析 file_contexts 内容
    pub fn parse(content: &str) -> Result<Self> {
        let mut entries = Vec::new();

        for line in content.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }

            // 格式: <path_pattern> <context>
            // 例如: /system/bin/sh u:object_r:shell_exec:s0
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 2 {
                let pattern = parts[0];
                let context = parts[1];

                // 将 file_contexts 模式转换为正则表达式
                // file_contexts 使用 PCRE 语法, 但需要处理部分特殊场景
                let regex_pattern = format!("^{}$", pattern);
                match Regex::new(&regex_pattern) {
                    Ok(regex) => {
                        entries.push(SelinuxEntry {
                            pattern: pattern.to_string(),
                            regex,
                            context: context.to_string(),
                        });
                    }
                    Err(e) => {
                        log::warn!("failed to parse SELinux pattern '{}': {}", pattern, e);
                    }
                }
            }
        }

        Ok(SelinuxContexts {
            entries,
            cache: HashMap::new(),
        })
    }

    // 查找路径对应的 SELinux 上下文
    pub fn lookup(&mut self, path: &str) -> Option<String> {
        // 检查缓存
        if let Some(ctx) = self.cache.get(path) {
            return Some(ctx.clone());
        }

        // 规范化路径
        let normalized = if path.starts_with('/') {
            path.to_string()
        } else {
            format!("/{}", path)
        };

        // 从后向前匹配, 优先命中更具体的规则
        for entry in self.entries.iter().rev() {
            if entry.regex.is_match(&normalized) {
                self.cache.insert(path.to_string(), entry.context.clone());
                return Some(entry.context.clone());
            }
        }

        None
    }

    // 获取条目数量
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    // 是否为空
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

// 文件系统配置条目
#[derive(Debug, Clone)]
pub struct FsConfigEntry {
    pub path: String,
    pub uid: u32,
    pub gid: u32,
    pub mode: u32,
    pub capabilities: Option<u64>,
}

// 文件系统配置管理器
#[derive(Debug)]
pub struct FsConfig {
    entries: HashMap<String, FsConfigEntry>,
    order: HashMap<String, usize>,
    default_uid: u32,
    default_gid: u32,
    default_dir_mode: u32,
    default_file_mode: u32,
}

impl FsConfig {
    // 从文件加载
    pub fn from_file(path: &Path) -> Result<Self> {
        let content = fs::read_to_string(path)?;
        Self::parse(&content)
    }

    // 解析 fs_config 内容
    pub fn parse(content: &str) -> Result<Self> {
        let mut entries = HashMap::new();
        let mut order = HashMap::new();

        for line in content.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }

            // 格式: <path> <uid> <gid> <mode> [capabilities]
            // 例如: system/bin/sh 0 2000 0755
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 4 {
                let path = parts[0].to_string();
                let uid = parts[1].parse::<u32>().unwrap_or(0);
                let gid = parts[2].parse::<u32>().unwrap_or(0);
                let mode = u32::from_str_radix(parts[3], 8).unwrap_or(0o644);
                let capabilities = if parts.len() > 4 {
                    u64::from_str_radix(parts[4], 16).ok()
                } else {
                    None
                };

                // 规范化路径
                let normalized_path = normalize_config_path(&path);
                if !order.contains_key(&normalized_path) {
                    let idx = order.len();
                    order.insert(normalized_path.clone(), idx);
                }

                entries.insert(
                    normalized_path.clone(),
                    FsConfigEntry {
                        path: normalized_path,
                        uid,
                        gid,
                        mode,
                        capabilities,
                    },
                );
            }
        }

        Ok(FsConfig {
            entries,
            order,
            default_uid: 0,
            default_gid: 0,
            default_dir_mode: 0o755,
            default_file_mode: 0o644,
        })
    }

    // 查找路径配置
    pub fn lookup(&self, path: &str) -> Option<&FsConfigEntry> {
        let normalized = normalize_config_path(path);

        self.entries.get(&normalized)
    }

    // 获取 uid/gid/mode, 未配置时返回默认值
    pub fn get_attrs(&self, path: &str, is_dir: bool) -> (u32, u32, u32) {
        if let Some(entry) = self.lookup(path) {
            (entry.uid, entry.gid, entry.mode)
        } else {
            let mode = if is_dir {
                self.default_dir_mode
            } else {
                self.default_file_mode
            };
            (self.default_uid, self.default_gid, mode)
        }
    }

    // 设置默认值
    pub fn set_defaults(&mut self, uid: u32, gid: u32, dir_mode: u32, file_mode: u32) {
        self.default_uid = uid;
        self.default_gid = gid;
        self.default_dir_mode = dir_mode;
        self.default_file_mode = file_mode;
    }

    // 获取条目数量
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    // 获取路径在 fs_config 中的原始顺序索引
    pub fn order_of(&self, path: &str) -> Option<usize> {
        let normalized = normalize_config_path(path);
        self.order.get(&normalized).copied()
    }

    // 是否为空
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

impl Default for FsConfig {
    fn default() -> Self {
        FsConfig {
            entries: HashMap::new(),
            order: HashMap::new(),
            default_uid: 0,
            default_gid: 0,
            default_dir_mode: 0o755,
            default_file_mode: 0o644,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_selinux_parse() {
        let content = r#"
/ u:object_r:system_file:s0
/system u:object_r:system_file:s0
/system/bin u:object_r:system_file:s0
/system/bin/sh u:object_r:shell_exec:s0
/system/lib(64)? u:object_r:system_lib_file:s0
"#;
        let mut ctx = SelinuxContexts::parse(content).unwrap();
        assert_eq!(ctx.len(), 5);

        assert_eq!(
            ctx.lookup("/system/bin/sh"),
            Some("u:object_r:shell_exec:s0".to_string())
        );
        assert_eq!(
            ctx.lookup("/system/lib"),
            Some("u:object_r:system_lib_file:s0".to_string())
        );
        assert_eq!(
            ctx.lookup("/system/lib64"),
            Some("u:object_r:system_lib_file:s0".to_string())
        );
    }

    #[test]
    fn test_fs_config_parse() {
        let content = r#"
/ 0 0 0755
system/bin 0 2000 0755
system/bin/sh 0 2000 0755
"#;
        let config = FsConfig::parse(content).unwrap();
        assert_eq!(config.len(), 3);

        let entry = config.lookup("/system/bin/sh").unwrap();
        assert_eq!(entry.uid, 0);
        assert_eq!(entry.gid, 2000);
        assert_eq!(entry.mode, 0o755);
    }
}
