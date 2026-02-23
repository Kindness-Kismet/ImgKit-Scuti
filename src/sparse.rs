// Android sparse image module
//
// Provides read and write support for the sparse image format.

pub mod format;
pub mod reader;
pub mod writer;

pub use format::*;
pub use reader::{SparseReader, is_sparse_image};
pub use writer::{DataChunk, SparseWriter, convert_to_sparse};
