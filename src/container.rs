// Container layer module
//
// Provides sparse image and super partition container formats.

pub mod sparse;
pub mod super_partition;

// Re-export commonly used types
pub use sparse::SparseReader;
pub use super_partition::{ExtractConfig, LpMetadata, extract_image};
