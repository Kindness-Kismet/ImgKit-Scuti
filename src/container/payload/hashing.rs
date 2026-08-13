// OTA payload 的 SHA-256 完整性校验。

use crate::container::payload::manifest::PartitionUpdate;
use anyhow::{Result, bail};
use sha2::{Digest, Sha256};
use std::fs::File;
use std::io::{self, Read};
use std::path::Path;

const HASH_BUFFER_SIZE: usize = 1024 * 1024;

// 边读取边累计长度与摘要的包装读取器
pub struct HashingReader<R> {
    inner: R,
    hasher: Sha256,
    bytes_read: u64,
}

impl<R> HashingReader<R> {
    pub fn new(inner: R) -> Self {
        Self {
            inner,
            hasher: Sha256::new(),
            bytes_read: 0,
        }
    }

    fn finish(self) -> (u64, sha2::digest::Output<Sha256>) {
        (self.bytes_read, self.hasher.finalize())
    }
}

impl<R: Read> Read for HashingReader<R> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        let read_len = self.inner.read(buffer)?;
        self.hasher.update(&buffer[..read_len]);
        self.bytes_read += read_len as u64;
        Ok(read_len)
    }
}

// 校验操作数据块的读取长度与 SHA-256 摘要
pub fn verify_blob_hash<R: Read>(
    reader: HashingReader<R>,
    expected_size: u64,
    expected_hash: Option<&[u8]>,
    partition_name: &str,
    operation_index: usize,
) -> Result<()> {
    let (actual_size, actual_hash) = reader.finish();
    if actual_size != expected_size {
        bail!(
            "partition {} operation {} read {} of {} payload bytes",
            partition_name,
            operation_index,
            actual_size,
            expected_size
        );
    }
    if let Some(expected_hash) = expected_hash.filter(|hash| !hash.is_empty())
        && actual_hash[..] != expected_hash[..]
    {
        bail!(
            "partition {} operation {} failed data SHA-256 verification",
            partition_name,
            operation_index
        );
    }

    Ok(())
}

// 校验还原后的分区镜像整体 SHA-256 摘要
pub fn verify_partition_hash(output_path: &Path, partition: &PartitionUpdate) -> Result<()> {
    let Some(expected_hash) = partition
        .new_partition_info
        .as_ref()
        .and_then(|info| info.hash.as_deref())
        .filter(|hash| !hash.is_empty())
    else {
        return Ok(());
    };

    let mut file = File::open(output_path)?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; HASH_BUFFER_SIZE];
    loop {
        let read_len = file.read(&mut buffer)?;
        if read_len == 0 {
            break;
        }
        hasher.update(&buffer[..read_len]);
    }

    let actual_hash = hasher.finalize();
    if actual_hash[..] != expected_hash[..] {
        bail!(
            "payload partition {} failed SHA-256 verification",
            partition.partition_name
        );
    }

    Ok(())
}
