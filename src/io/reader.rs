// Unified reading interface
//
// Provides a unified read abstraction for file systems and containers

use std::io::{Read, Seek, SeekFrom};

// Read error type
pub type Result<T> = std::result::Result<T, std::io::Error>;

// Unified Reader Interface
//
// Combines Read and Seek to provide convenience methods
pub trait Reader: Read + Seek {
    // Read data at a specified location
    fn read_at(&mut self, offset: u64, buf: &mut [u8]) -> Result<usize> {
        self.seek(SeekFrom::Start(offset))?;
        self.read(buf)
    }

    // Read the exact number of bytes at the specified location
    fn read_exact_at(&mut self, offset: u64, buf: &mut [u8]) -> Result<()> {
        self.seek(SeekFrom::Start(offset))?;
        self.read_exact(buf)
    }

    // Read all remaining data
    fn read_remaining(&mut self) -> Result<Vec<u8>> {
        let mut buf = Vec::new();
        self.read_to_end(&mut buf)?;
        Ok(buf)
    }

    // Get current location
    fn position(&mut self) -> Result<u64> {
        self.stream_position()
    }

    // Get total size
    fn size(&mut self) -> Result<u64> {
        let old_pos = self.stream_position()?;
        let size = self.seek(SeekFrom::End(0))?;
        self.seek(SeekFrom::Start(old_pos))?;
        Ok(size)
    }

    // skip specified number of bytes
    fn skip(&mut self, n: u64) -> Result<()> {
        self.seek(SeekFrom::Current(n as i64))?;
        Ok(())
    }

    // Read data in a specified range
    fn read_range(&mut self, offset: u64, length: usize) -> Result<Vec<u8>> {
        let mut buf = vec![0u8; length];
        self.read_exact_at(offset, &mut buf)?;
        Ok(buf)
    }
}

// Automatically implement Reader for all types that implement Read + Seek
impl<T: Read + Seek> Reader for T {}

// cache reader
//
// Provide data caching to reduce the overhead of small block reads
pub struct BufferedReader<R: Read + Seek> {
    inner: R,
    buffer: Vec<u8>,
    buffer_offset: u64,
    position: u64,
}

impl<R: Read + Seek> BufferedReader<R> {
    // Create a new cache reader
    pub fn new(inner: R) -> Self {
        Self::with_capacity(inner, 8192) // Default 8KB cache
    }

    // Creates a reader with a specified cache size
    pub fn with_capacity(inner: R, capacity: usize) -> Self {
        Self {
            inner,
            buffer: Vec::with_capacity(capacity),
            buffer_offset: 0,
            position: 0,
        }
    }

    // Get a reference to the internal reader
    pub fn get_ref(&self) -> &R {
        &self.inner
    }

    // Get a mutable reference to the internal reader
    pub fn get_mut(&mut self) -> &mut R {
        &mut self.inner
    }

    // Consumes a BufferedReader and returns the internal reader
    pub fn into_inner(self) -> R {
        self.inner
    }

    // Clear cache
    pub fn invalidate_cache(&mut self) {
        self.buffer.clear();
        self.buffer_offset = 0;
    }
}

impl<R: Read + Seek> Read for BufferedReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize> {
        // Simplified implementation: read directly from internal reader
        // A complete implementation should use caching
        let n = self.inner.read(buf)?;
        self.position += n as u64;
        Ok(n)
    }
}

impl<R: Read + Seek> Seek for BufferedReader<R> {
    fn seek(&mut self, pos: SeekFrom) -> Result<u64> {
        let new_pos = self.inner.seek(pos)?;
        self.position = new_pos;
        // If the jump location is not within the cache range, clear the cache
        if new_pos < self.buffer_offset || new_pos >= self.buffer_offset + self.buffer.len() as u64
        {
            self.invalidate_cache();
        }
        Ok(new_pos)
    }
}
