// Android filesystem image extraction and packing library.
// Supports F2FS, EXT4 filesystems and Super partition.

// Common utility functions
pub mod utils;

// General compression/decompression module
pub mod compression;

// Container layer
pub mod container;

// Filesystem layer
pub mod filesystem;

// CLI interface
mod cli;

pub use cli::{Cli, Commands, run};
