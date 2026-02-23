// Unified writing interface
//
// Provides a unified write abstraction for file systems and containers

use std::io::{Seek, SeekFrom, Write};

// write error type
pub type Result<T> = std::result::Result<T, std::io::Error>;

// Unified Writer Interface
//
// Combines Write and Seek to provide convenience methods
pub trait Writer: Write + Seek {
    // Write data at the specified location
    fn write_at(&mut self, offset: u64, data: &[u8]) -> Result<usize> {
        self.seek(SeekFrom::Start(offset))?;
        self.write(data)
    }

    // Write all data at the specified location
    fn write_all_at(&mut self, offset: u64, data: &[u8]) -> Result<()> {
        self.seek(SeekFrom::Start(offset))?;
        self.write_all(data)
    }

    // Get current location
    fn position(&mut self) -> Result<u64> {
        self.stream_position()
    }

    // Get the total size written
    fn size(&mut self) -> Result<u64> {
        let old_pos = self.stream_position()?;
        let size = self.seek(SeekFrom::End(0))?;
        self.seek(SeekFrom::Start(old_pos))?;
        Ok(size)
    }

    // Pads the specified number of bytes with zeros
    fn write_zeros(&mut self, count: usize) -> Result<()> {
        let zeros = vec![0u8; count.min(4096)];
        let mut remaining = count;
        while remaining > 0 {
            let to_write = remaining.min(zeros.len());
            self.write_all(&zeros[..to_write])?;
            remaining -= to_write;
        }
        Ok(())
    }

    // Align to specified boundaries
    fn align_to(&mut self, alignment: u64) -> Result<()> {
        let pos = self.stream_position()?;
        let padding = ((alignment - (pos % alignment)) % alignment) as usize;
        if padding > 0 {
            self.write_zeros(padding)?;
        }
        Ok(())
    }

    // Flush and sync to disk
    fn sync(&mut self) -> Result<()> {
        self.flush()
    }
}

// Automatically implement Writer for all types that implement Write + Seek
impl<T: Write + Seek> Writer for T {}

// cache writer
//
// Provide data caching, batch writing to improve performance
pub struct BufferedWriter<W: Write + Seek> {
    inner: W,
    buffer: Vec<u8>,
    buffer_start: u64,
    position: u64,
}

impl<W: Write + Seek> BufferedWriter<W> {
    // Create a new cache writer
    pub fn new(inner: W) -> Self {
        Self::with_capacity(inner, 8192) // Default 8KB cache
    }

    // Creates a writer with a specified cache size
    pub fn with_capacity(inner: W, capacity: usize) -> Self {
        Self {
            inner,
            buffer: Vec::with_capacity(capacity),
            buffer_start: 0,
            position: 0,
        }
    }

    // Get a reference to the internal writer
    pub fn get_ref(&self) -> &W {
        &self.inner
    }

    // Get a mutable reference to the internal writer
    pub fn get_mut(&mut self) -> &mut W {
        &mut self.inner
    }

    // Consumes a BufferedWriter, returning the internal writer
    pub fn into_inner(mut self) -> Result<W> {
        self.flush_buffer()?;
        // Use ManuallyDrop to avoid calls to the Drop trait
        let writer = unsafe {
            let inner_ptr = &mut self.inner as *mut W;
            std::mem::forget(self); // prevent drop
            std::ptr::read(inner_ptr)
        };
        Ok(writer)
    }

    // refresh cache
    fn flush_buffer(&mut self) -> Result<()> {
        if !self.buffer.is_empty() {
            self.inner.seek(SeekFrom::Start(self.buffer_start))?;
            self.inner.write_all(&self.buffer)?;
            self.buffer.clear();
        }
        Ok(())
    }
}

impl<W: Write + Seek> Write for BufferedWriter<W> {
    fn write(&mut self, buf: &[u8]) -> Result<usize> {
        // Simplified implementation: write directly to internal writer
        // A complete implementation should use caching
        let n = self.inner.write(buf)?;
        self.position += n as u64;
        Ok(n)
    }

    fn flush(&mut self) -> Result<()> {
        self.flush_buffer()?;
        self.inner.flush()
    }
}

impl<W: Write + Seek> Seek for BufferedWriter<W> {
    fn seek(&mut self, pos: SeekFrom) -> Result<u64> {
        self.flush_buffer()?;
        let new_pos = self.inner.seek(pos)?;
        self.position = new_pos;
        self.buffer_start = new_pos;
        Ok(new_pos)
    }
}

impl<W: Write + Seek> Drop for BufferedWriter<W> {
    fn drop(&mut self) {
        // Try flushing cache, ignore errors
        let _ = self.flush_buffer();
    }
}
