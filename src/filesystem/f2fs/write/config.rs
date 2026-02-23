// F2FS configuration parser
//
// Parse file_contexts and fs_config files.

use crate::filesystem::f2fs::Result;
use regex::Regex;
use std::collections::HashMap;
use std::fs;
use std::path::Path;

// SELinux context entry
#[derive(Debug, Clone)]
pub struct SelinuxEntry {
    pub pattern: String,
    pub regex: Regex,
    pub context: String,
}

// SELinux context manager
#[derive(Debug)]
pub struct SelinuxContexts {
    entries: Vec<SelinuxEntry>,
    cache: HashMap<String, String>,
}

impl SelinuxContexts {
    // Load from file
    pub fn from_file(path: &Path) -> Result<Self> {
        let content = fs::read_to_string(path)?;
        Self::parse(&content)
    }

    // Parse file_contexts content
    pub fn parse(content: &str) -> Result<Self> {
        let mut entries = Vec::new();

        for line in content.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }

            // Format: <path_pattern> <context>
            // For example: /system/bin/sh u:object_r:shell_exec:s0
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 2 {
                let pattern = parts[0];
                let context = parts[1];

                // Convert file_contexts pattern to regular expression
                // file_contexts uses PCRE syntax, but we need to handle some special cases
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
                        log::warn!("无法解析 SELinux 模式 '{}': {}", pattern, e);
                    }
                }
            }
        }

        Ok(SelinuxContexts {
            entries,
            cache: HashMap::new(),
        })
    }

    // Find the SELinux context for a path
    pub fn lookup(&mut self, path: &str) -> Option<String> {
        // Check cache
        if let Some(ctx) = self.cache.get(path) {
            return Some(ctx.clone());
        }

        // normalized path
        let normalized = if path.starts_with('/') {
            path.to_string()
        } else {
            format!("/{}", path)
        };

        // Match from back to front, giving priority to more specific rules
        for entry in self.entries.iter().rev() {
            if entry.regex.is_match(&normalized) {
                self.cache.insert(path.to_string(), entry.context.clone());
                return Some(entry.context.clone());
            }
        }

        None
    }

    // Get the number of entries
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    // Is it empty
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

// File system configuration entries
#[derive(Debug, Clone)]
pub struct FsConfigEntry {
    pub path: String,
    pub uid: u32,
    pub gid: u32,
    pub mode: u32,
    pub capabilities: Option<u64>,
}

// File system configuration manager
#[derive(Debug)]
pub struct FsConfig {
    entries: HashMap<String, FsConfigEntry>,
    default_uid: u32,
    default_gid: u32,
    default_dir_mode: u32,
    default_file_mode: u32,
}

impl FsConfig {
    // Load from file
    pub fn from_file(path: &Path) -> Result<Self> {
        let content = fs::read_to_string(path)?;
        Self::parse(&content)
    }

    // Parse fs_config content
    pub fn parse(content: &str) -> Result<Self> {
        let mut entries = HashMap::new();

        for line in content.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }

            // Format: <path> <uid> <gid> <mode> [capabilities]
            // For example: system/bin/sh 0 2000 0755
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

                // normalized path
                let normalized_path = if path.starts_with('/') {
                    path
                } else {
                    format!("/{}", path)
                };

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
            default_uid: 0,
            default_gid: 0,
            default_dir_mode: 0o755,
            default_file_mode: 0o644,
        })
    }

    // Find path configuration
    pub fn lookup(&self, path: &str) -> Option<&FsConfigEntry> {
        // normalized path
        let normalized = if path.starts_with('/') {
            path.to_string()
        } else {
            format!("/{}", path)
        };

        self.entries.get(&normalized)
    }

    // Get uid/gid/mode, if not configured, return the default value
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

    // Set default value
    pub fn set_defaults(&mut self, uid: u32, gid: u32, dir_mode: u32, file_mode: u32) {
        self.default_uid = uid;
        self.default_gid = gid;
        self.default_dir_mode = dir_mode;
        self.default_file_mode = file_mode;
    }

    // Get the number of entries
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    // Is it empty
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

impl Default for FsConfig {
    fn default() -> Self {
        FsConfig {
            entries: HashMap::new(),
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
