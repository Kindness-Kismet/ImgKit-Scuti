// file abstraction layer
//
// Define a unified interface for file system files

use std::io::{Read, Seek, SeekFrom, Write};

// File error type
pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

// Unified interface for files
pub trait File: Read + Seek {
    // Get file size
    fn size(&self) -> u64;

    // Read all data
    fn read_all(&mut self) -> Result<Vec<u8>> {
        let mut buf = Vec::with_capacity(self.size() as usize);
        self.read_to_end(&mut buf)?;
        Ok(buf)
    }

    // Read data in a specified range
    fn read_range(&mut self, offset: u64, length: usize) -> Result<Vec<u8>> {
        self.seek(SeekFrom::Start(offset))?;
        let mut buf = vec![0u8; length];
        self.read_exact(&mut buf)?;
        Ok(buf)
    }

    // Check if the file is empty
    fn is_empty(&self) -> bool {
        self.size() == 0
    }
}

// Extended interface for writable files
pub trait WritableFile: File + Write {
    // Write all data
    fn write_all_data(&mut self, data: &[u8]) -> Result<()> {
        self.write_all(data)?;
        Ok(())
    }

    // Write data at the specified location
    fn write_at(&mut self, offset: u64, data: &[u8]) -> Result<()> {
        self.seek(SeekFrom::Start(offset))?;
        self.write_all(data)?;
        Ok(())
    }

    // Truncate file
    fn truncate(&mut self, size: u64) -> Result<()>;

    // Flush to disk
    fn flush_file(&mut self) -> Result<()> {
        self.flush()?;
        Ok(())
    }
}

// Extension interface for compressed files
pub trait CompressedFile: File {
    // Get compression algorithm type
    fn compression_type(&self) -> &str;

    // Get the size before compression
    fn uncompressed_size(&self) -> u64;

    // Get the compressed size (i.e. actual file size)
    fn compressed_size(&self) -> u64 {
        self.size()
    }

    // Get compression ratio
    fn compression_ratio(&self) -> f64 {
        if self.uncompressed_size() == 0 {
            return 0.0;
        }
        (self.compressed_size() as f64) / (self.uncompressed_size() as f64)
    }
}
