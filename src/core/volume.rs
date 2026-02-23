// Volume management abstraction layer
//
// A unified interface for defining file system volumes

use std::io::{Read, Seek};
use std::path::Path;

// Volume error type
pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

// Unified interface for file system volumes
//
// A volume is the root object of a file system and provides access to the file system
pub trait Volume: Sized {
    // Volume metadata type
    type Metadata;

    // open volume
    //
    // # Parameters
    // - reader: Data source that implements Read + Seek
    fn open<R: Read + Seek + 'static>(reader: R) -> Result<Self>;

    // Open volume from file path
    fn open_path<P: AsRef<Path>>(path: P) -> Result<Self> {
        let file = std::fs::File::open(path)?;
        Self::open(file)
    }

    // Get volume metadata
    fn metadata(&self) -> &Self::Metadata;

    // Get the total size of the volume (bytes)
    fn size(&self) -> u64;

    // Get the block size of a volume
    fn block_size(&self) -> u32;

    // Get the block number of the volume
    fn block_count(&self) -> u64 {
        self.size() / self.block_size() as u64
    }

    // Verify volume validity
    fn validate(&self) -> Result<()>;
}

// Readable volume expansion interface
pub trait ReadableVolume: Volume {
    // Inode type
    type Inode;

    // Read root inode
    fn root_inode(&self) -> Result<Self::Inode>;

    // Read inode by inode number
    fn read_inode(&self, ino: u64) -> Result<Self::Inode>;
}

// Extension interface for writable volumes
pub trait WritableVolume: Volume {
    // Builder configuration type
    type Config;

    // Create new volume
    fn create<P: AsRef<Path>>(path: P, config: Self::Config) -> Result<Self>;

    // Refresh all data to be written
    fn flush(&mut self) -> Result<()>;

    // Sync to disk
    fn sync(&mut self) -> Result<()> {
        self.flush()
    }
}
