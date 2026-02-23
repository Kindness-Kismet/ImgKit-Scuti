// Container abstraction layer
//
// Define a unified interface for container formats, such as sparse, super, etc.

use std::io::{Read, Seek};
use std::path::Path;

// Container error type
pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

// Traits that combine Read and Seek
pub trait ReadSeek: Read + Seek {}

// Automatically implement ReadSeek for all types that implement Read and Seek
impl<T: Read + Seek> ReadSeek for T {}

// Partition information
#[derive(Debug, Clone)]
pub struct PartitionInfo {
    // Partition name
    pub name: String,
    // Partition size (bytes)
    pub size: u64,
    // The offset of the partition in the container
    pub offset: u64,
    // Partition attributes (such as readonly)
    pub attributes: Vec<String>,
}

// Container metadata
#[derive(Debug, Clone)]
pub struct ContainerMetadata {
    // Container type (e.g. "sparse", "super")
    pub container_type: String,
    // Container version
    pub version: u32,
    // total size
    pub total_size: u64,
    // Number of partitions
    pub partition_count: usize,
}

// Container traits
//
// A unified interface for defining container formats
// A container can contain one or more partitions/images
pub trait Container {
    // open container
    fn open<P: AsRef<Path>>(path: P) -> Result<Self>
    where
        Self: Sized;

    // List all partitions in a container
    fn list_partitions(&self) -> Result<Vec<PartitionInfo>>;

    // Extract the specified partition to Reader
    // Returns a Reader that can read and locate
    fn extract_partition(&mut self, name: &str) -> Result<Box<dyn ReadSeek>>;

    // Get container metadata
    fn metadata(&self) -> &ContainerMetadata;

    // Check whether the specified partition is included
    fn has_partition(&self, name: &str) -> bool {
        self.list_partitions()
            .map(|partitions| partitions.iter().any(|p| p.name == name))
            .unwrap_or(false)
    }

    // Get information about a specified partition
    fn get_partition_info(&self, name: &str) -> Result<PartitionInfo> {
        self.list_partitions()?
            .into_iter()
            .find(|p| p.name == name)
            .ok_or_else(|| format!("分区 {} 不存在", name).into())
    }
}
