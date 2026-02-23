// IO layer
//
// Provide unified reading and writing interface and buffer management

pub mod buffer;
pub mod reader;
pub mod writer;

// Re-export common types

// Reader related
pub use reader::{BufferedReader, Reader};

// Writer related
pub use writer::{BufferedWriter, Writer};

// Buffer management related
pub use buffer::{BufferPool, PooledBuffer, RingBuffer};
