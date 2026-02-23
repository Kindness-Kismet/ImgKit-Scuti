// EXT4 extended attributes (xattr) builder

use crate::filesystem::ext4::Result;
use crate::filesystem::ext4::types::*;

// Xattr name index
pub const XATTR_INDEX_USER: u8 = 1;
pub const XATTR_INDEX_POSIX_ACL_ACCESS: u8 = 2;
pub const XATTR_INDEX_POSIX_ACL_DEFAULT: u8 = 3;
pub const XATTR_INDEX_TRUSTED: u8 = 4;
pub const XATTR_INDEX_SECURITY: u8 = 6;

// Xattr entry
#[derive(Clone)]
pub struct XattrEntry {
    pub name_index: u8,
    pub name: Vec<u8>,
    pub value: Vec<u8>,
}

impl XattrEntry {
    // Create SELinux context xattr
    pub fn selinux(context: &str) -> Self {
        XattrEntry {
            name_index: XATTR_INDEX_SECURITY,
            name: b"selinux".to_vec(),
            value: context.as_bytes().to_vec(),
        }
    }

    // Calculate entry size (4-byte alignment)
    pub fn size(&self) -> usize {
        let base_size = 16 + self.name.len(); // sizeof(Ext4XattrEntry) + name
        (base_size + 3) & !3
    }

    // serialize to bytes
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut buf = Vec::new();

        // e_name_len
        buf.push(self.name.len() as u8);

        // e_name_index
        buf.push(self.name_index);

        // e_value_offs (populated later)
        buf.extend_from_slice(&0u16.to_le_bytes());

        // e_value_inum
        buf.extend_from_slice(&0u32.to_le_bytes());

        // e_value_size
        buf.extend_from_slice(&(self.value.len() as u32).to_le_bytes());

        // e_hash
        buf.extend_from_slice(&0u32.to_le_bytes());

        // e_name
        buf.extend_from_slice(&self.name);

        // Aligned to 4 bytes
        while buf.len() % 4 != 0 {
            buf.push(0);
        }

        buf
    }
}

// Xattr block builder
pub struct XattrBlockBuilder {
    entries: Vec<XattrEntry>,
}

impl XattrBlockBuilder {
    // Create a new xattr block builder
    pub fn new() -> Self {
        XattrBlockBuilder {
            entries: Vec::new(),
        }
    }

    // Add entry
    pub fn add_entry(&mut self, entry: XattrEntry) {
        self.entries.push(entry);
    }

    // Build xattr block
    pub fn build(&self, block_size: usize) -> Result<Vec<u8>> {
        let mut block = vec![0u8; block_size];

        // Write header
        let magic = EXT4_XATTR_HEADER_MAGIC;
        block[0..4].copy_from_slice(&magic.to_le_bytes());
        block[4..8].copy_from_slice(&1u32.to_le_bytes()); // h_refcount
        block[8..12].copy_from_slice(&1u32.to_le_bytes()); // h_blocks
        block[12..16].copy_from_slice(&0u32.to_le_bytes()); // h_hash
        block[16..20].copy_from_slice(&0u32.to_le_bytes()); // h_checksum

        let mut offset = 32; // head size
        let mut value_offset = block_size;

        // write entry
        for entry in &self.entries {
            let entry_bytes = entry.to_bytes();

            // update value_offset
            value_offset -= entry.value.len();
            value_offset = (value_offset / 4) * 4; // Alignment

            // Write entry header
            block[offset..offset + entry_bytes.len()].copy_from_slice(&entry_bytes);

            // Update e_value_offs
            let value_offs = (value_offset - offset) as u16;
            block[offset + 2..offset + 4].copy_from_slice(&value_offs.to_le_bytes());

            // write value
            block[value_offset..value_offset + entry.value.len()].copy_from_slice(&entry.value);

            offset += entry_bytes.len();
        }

        Ok(block)
    }

    // Is it empty
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

impl Default for XattrBlockBuilder {
    fn default() -> Self {
        Self::new()
    }
}

// Inline xattr builder (stored in inode)
pub struct InlineXattrBuilder {
    entries: Vec<XattrEntry>,
}

impl InlineXattrBuilder {
    // Create a new inline xattr builder
    pub fn new() -> Self {
        InlineXattrBuilder {
            entries: Vec::new(),
        }
    }

    // Add entry
    pub fn add_entry(&mut self, entry: XattrEntry) {
        self.entries.push(entry);
    }

    // Build inline xattr data
    pub fn build(&self, max_size: usize) -> Result<Vec<u8>> {
        let mut data = Vec::new();

        // Write the magic number in the head
        data.extend_from_slice(&EXT4_XATTR_HEADER_MAGIC.to_le_bytes());

        let mut value_offset = max_size;

        // write entry
        for entry in &self.entries {
            let entry_bytes = entry.to_bytes();

            // update value_offset
            value_offset -= entry.value.len();
            value_offset = (value_offset / 4) * 4; // Alignment

            // Write entry header
            data.extend_from_slice(&entry_bytes);

            // Update e_value_offs (relative to the start of the inline xattr region)
            let offs_pos = data.len() - entry_bytes.len() + 2;
            let value_offs = (value_offset - 4) as u16; // -4 because of the magic number
            data[offs_pos..offs_pos + 2].copy_from_slice(&value_offs.to_le_bytes());
        }

        // Add terminator
        data.extend_from_slice(&[0u8; 4]);

        // Pad to max_size
        if data.len() < max_size {
            // write value
            let mut values_data = vec![0u8; max_size - data.len()];
            let mut write_offset = max_size - data.len();

            for entry in self.entries.iter().rev() {
                write_offset -= entry.value.len();
                write_offset = (write_offset / 4) * 4;
                values_data[write_offset..write_offset + entry.value.len()]
                    .copy_from_slice(&entry.value);
            }

            data.extend_from_slice(&values_data);
        }

        Ok(data)
    }

    // Is it empty
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

impl Default for InlineXattrBuilder {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_xattr_entry() {
        let entry = XattrEntry::selinux("u:object_r:system_file:s0");
        assert_eq!(entry.name_index, XATTR_INDEX_SECURITY);
        assert_eq!(entry.name, b"selinux");
    }

    #[test]
    fn test_xattr_block_builder() {
        let mut builder = XattrBlockBuilder::new();
        builder.add_entry(XattrEntry::selinux("u:object_r:system_file:s0"));

        let block = builder.build(4096).unwrap();
        assert_eq!(block.len(), 4096);

        // Verify the magic number
        let magic = u32::from_le_bytes([block[0], block[1], block[2], block[3]]);
        assert_eq!(magic, EXT4_XATTR_HEADER_MAGIC);
    }
}
