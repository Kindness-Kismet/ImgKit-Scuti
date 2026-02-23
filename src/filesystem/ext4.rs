// EXT4 file system module

pub mod directory;
pub mod error;
pub mod extractor;
pub mod file;
pub mod types;
pub mod volume;
pub mod write;
pub mod xattr;

// Re-export common types
pub use error::{Ext4Error, Result};
pub use extractor::*;
pub use types::*;
