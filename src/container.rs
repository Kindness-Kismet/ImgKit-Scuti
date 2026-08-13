// Container layer module
//
// Provides sparse image, super partition and OTA payload container formats.

pub mod payload;
pub mod sparse;
pub mod super_partition;

// Re-export commonly used types
pub use sparse::SparseReader;
pub use super_partition::{ExtractConfig, LpMetadata, extract_image};
