// EROFS configuration parser
//
// Parse file_contexts and fs_config files.

#![allow(dead_code)]

use crate::filesystem::erofs::Result;
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
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 2 {
                let pattern = parts[0];
                let context = parts[1];

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
        if let Some(ctx) = self.cache.get(path) {
            return Some(ctx.clone());
        }

        let normalized = if path.starts_with('/') {
            path.to_string()
        } else {
            format!("/{}", path)
        };

        for entry in self.entries.iter().rev() {
            if entry.regex.is_match(&normalized) {
                self.cache.insert(path.to_string(), entry.context.clone());
                return Some(entry.context.clone());
            }
        }

        None
    }

    // SELinux context for lookup path (does not modify cache)
    pub fn lookup_without_mut(&self, path: &str) -> Option<String> {
        if let Some(ctx) = self.cache.get(path) {
            return Some(ctx.clone());
        }

        let normalized = if path.starts_with('/') {
            path.to_string()
        } else {
            format!("/{}", path)
        };

        for entry in self.entries.iter().rev() {
            if entry.regex.is_match(&normalized) {
                return Some(entry.context.clone());
            }
        }

        None
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

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
        let normalized = if path.starts_with('/') {
            path.to_string()
        } else {
            format!("/{}", path)
        };

        self.entries.get(&normalized)
    }

    // Get uid/gid/mode
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

    pub fn set_defaults(&mut self, uid: u32, gid: u32, dir_mode: u32, file_mode: u32) {
        self.default_uid = uid;
        self.default_gid = gid;
        self.default_dir_mode = dir_mode;
        self.default_file_mode = file_mode;
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

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

// EROFS build configuration
#[derive(Debug, Clone)]
pub struct ErofsConfig {
    pub source_dir: std::path::PathBuf,
    pub output_path: std::path::PathBuf,
    pub volume_label: String,
    pub block_size: u32,
    pub compress_algorithm: Option<String>,
    pub compress_level: Option<u32>,
    pub file_contexts: Option<std::path::PathBuf>,
    pub fs_config: Option<std::path::PathBuf>,
    pub mount_point: String,
    pub timestamp: Option<u64>,
    pub uuid: Option<[u8; 16]>,
    pub root_uid: u32,
    pub root_gid: u32,
}

impl Default for ErofsConfig {
    fn default() -> Self {
        ErofsConfig {
            source_dir: std::path::PathBuf::new(),
            output_path: std::path::PathBuf::new(),
            volume_label: String::new(),
            block_size: 4096,
            compress_algorithm: None,
            compress_level: None,
            file_contexts: None,
            fs_config: None,
            mount_point: "/".to_string(),
            timestamp: None,
            uuid: None,
            root_uid: 0,
            root_gid: 0,
        }
    }
}
