// 三种格式共用权限和上下文解析，避免规则处理不一致。
pub use crate::filesystem::f2fs::write::{FsConfig, SelinuxContexts};

// EROFS 构建配置
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
