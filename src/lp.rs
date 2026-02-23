// Android LP (Logical Partition) partition module
//
// Provides extraction and construction functions for Super partitions
// Reference Android source code liblp

pub mod builder;
pub mod extractor;
pub mod format;
pub mod metadata;
pub mod writer;

// Re-export common types
pub use builder::{
    BlockDeviceInfo, GroupInfo, MetadataBuilder, PartitionInfo, align_to, get_file_size,
};
pub use extractor::{ExtractConfig, extract_image};
pub use format::*;
pub use writer::{
    serialize_geometry, serialize_metadata, write_empty_image, write_sparse_empty_image,
    write_to_image_file, write_to_image_file_with_data, write_to_sparse_image_file_with_data,
};
