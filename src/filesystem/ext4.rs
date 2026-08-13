// EXT4 文件系统模块。

// 类型与错误定义
pub mod error;
pub mod types;

// 读取功能
pub mod read;

// 写入功能
pub mod write;

// 重导出常用类型
pub use error::{Ext4Error, Result};
pub use read::extractor::*;
pub use types::*;
