// Android 镜像提取与打包库, 解包支持 OTA payload。

// 通用工具函数
pub mod utils;

// 通用压缩与解压模块
pub mod compression;

// 容器层
pub mod container;

// 文件系统层
pub mod filesystem;

// 命令行接口
mod cli;

pub use cli::{Cli, Commands, run};
