// Buffer management
//
// Provide memory buffer pool and buffer management

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

// buffer pool
//
// Reuse buffers to reduce memory allocation
pub struct BufferPool {
    buffers: Arc<Mutex<VecDeque<Vec<u8>>>>,
    buffer_size: usize,
    max_buffers: usize,
}

impl BufferPool {
    // Create new buffer pool
    pub fn new(buffer_size: usize, max_buffers: usize) -> Self {
        Self {
            buffers: Arc::new(Mutex::new(VecDeque::new())),
            buffer_size,
            max_buffers,
        }
    }

    // Get a buffer
    pub fn get(&self) -> Vec<u8> {
        let mut buffers = self.buffers.lock().unwrap();
        buffers
            .pop_front()
            .unwrap_or_else(|| Vec::with_capacity(self.buffer_size))
    }

    // return a buffer
    pub fn put(&self, mut buffer: Vec<u8>) {
        let mut buffers = self.buffers.lock().unwrap();
        if buffers.len() < self.max_buffers {
            buffer.clear();
            buffers.push_back(buffer);
        }
    }

    // Clear buffer pool
    pub fn clear(&self) {
        let mut buffers = self.buffers.lock().unwrap();
        buffers.clear();
    }

    // Get the current buffer number
    pub fn len(&self) -> usize {
        let buffers = self.buffers.lock().unwrap();
        buffers.len()
    }

    // Check if the buffer pool is empty
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

// Automatically returned buffer
//
// Automatically returned to the buffer pool when PooledBuffer drops
pub struct PooledBuffer {
    buffer: Option<Vec<u8>>,
    pool: Arc<Mutex<VecDeque<Vec<u8>>>>,
}

impl PooledBuffer {
    // Get from buffer pool
    #[allow(dead_code)]
    fn from_pool(pool: Arc<Mutex<VecDeque<Vec<u8>>>>, buffer_size: usize) -> Self {
        let buffer = {
            let mut buffers = pool.lock().unwrap();
            buffers
                .pop_front()
                .unwrap_or_else(|| Vec::with_capacity(buffer_size))
        };
        Self {
            buffer: Some(buffer),
            pool,
        }
    }

    // Get a reference to the buffer
    pub fn as_slice(&self) -> &[u8] {
        self.buffer.as_ref().unwrap()
    }

    // Get a mutable reference to a buffer
    pub fn as_mut_slice(&mut self) -> &mut [u8] {
        self.buffer.as_mut().unwrap()
    }

    // Get a reference to a Vec
    pub fn as_vec(&self) -> &Vec<u8> {
        self.buffer.as_ref().unwrap()
    }

    // Get a mutable reference to a Vec
    pub fn as_mut_vec(&mut self) -> &mut Vec<u8> {
        self.buffer.as_mut().unwrap()
    }
}

impl Drop for PooledBuffer {
    fn drop(&mut self) {
        if let Some(mut buffer) = self.buffer.take() {
            let mut buffers = self.pool.lock().unwrap();
            buffer.clear();
            buffers.push_back(buffer);
        }
    }
}

// ring buffer
//
// for streaming data processing
pub struct RingBuffer {
    buffer: Vec<u8>,
    read_pos: usize,
    write_pos: usize,
    size: usize,
}

impl RingBuffer {
    // Create a new ring buffer
    pub fn new(capacity: usize) -> Self {
        Self {
            buffer: vec![0u8; capacity],
            read_pos: 0,
            write_pos: 0,
            size: 0,
        }
    }

    // Get capacity
    pub fn capacity(&self) -> usize {
        self.buffer.len()
    }

    // Get the current data volume
    pub fn len(&self) -> usize {
        self.size
    }

    // Check if it is empty
    pub fn is_empty(&self) -> bool {
        self.size == 0
    }

    // Check if full
    pub fn is_full(&self) -> bool {
        self.size == self.buffer.len()
    }

    // Get available space
    pub fn available(&self) -> usize {
        self.buffer.len() - self.size
    }

    // Write data
    #[allow(clippy::needless_range_loop)]
    pub fn write(&mut self, data: &[u8]) -> usize {
        let available = self.available();
        let to_write = data.len().min(available);

        for i in 0..to_write {
            self.buffer[self.write_pos] = data[i];
            self.write_pos = (self.write_pos + 1) % self.buffer.len();
        }

        self.size += to_write;
        to_write
    }

    // Read data
    #[allow(clippy::needless_range_loop)]
    pub fn read(&mut self, buf: &mut [u8]) -> usize {
        let to_read = buf.len().min(self.size);

        for i in 0..to_read {
            buf[i] = self.buffer[self.read_pos];
            self.read_pos = (self.read_pos + 1) % self.buffer.len();
        }

        self.size -= to_read;
        to_read
    }

    // Clear buffer
    pub fn clear(&mut self) {
        self.read_pos = 0;
        self.write_pos = 0;
        self.size = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_buffer_pool() {
        let pool = BufferPool::new(1024, 10);
        assert_eq!(pool.len(), 0);

        let buf = pool.get();
        assert_eq!(buf.capacity(), 1024);

        pool.put(buf);
        assert_eq!(pool.len(), 1);
    }

    #[test]
    fn test_ring_buffer() {
        let mut ring = RingBuffer::new(10);
        assert!(ring.is_empty());
        assert_eq!(ring.available(), 10);

        let written = ring.write(b"hello");
        assert_eq!(written, 5);
        assert_eq!(ring.len(), 5);

        let mut buf = [0u8; 10];
        let read = ring.read(&mut buf[..3]);
        assert_eq!(read, 3);
        assert_eq!(&buf[..3], b"hel");
        assert_eq!(ring.len(), 2);
    }
}
