// 容器层，提供 sparse、Super 分区和 OTA payload 格式支持。

pub mod payload;
pub mod sparse;
pub mod super_partition;

// 重导出常用类型
pub use sparse::SparseReader;
pub use super_partition::{ExtractConfig, LpMetadata, extract_image};
