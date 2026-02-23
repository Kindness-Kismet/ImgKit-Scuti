// LP (Logical Partition) metadata format definition
//
// Refer to Android source code liblp/include/liblp/metadata_format.h

// LP metadata magic number
pub const LP_METADATA_GEOMETRY_MAGIC: u32 = 0x616c4467; // "gDla"
pub const LP_METADATA_HEADER_MAGIC: u32 = 0x414c5030; // "0PLA"

// LP metadata geometry information size
pub const LP_METADATA_GEOMETRY_SIZE: u64 = 4096;

// LP metadata version
pub const LP_METADATA_MAJOR_VERSION: u16 = 10;
pub const LP_METADATA_MINOR_VERSION_MIN: u16 = 0;
pub const LP_METADATA_MINOR_VERSION_MAX: u16 = 2;

// Partition properties
pub const LP_PARTITION_ATTR_NONE: u32 = 0x0;
pub const LP_PARTITION_ATTR_READONLY: u32 = 1 << 0;
pub const LP_PARTITION_ATTR_SLOT_SUFFIXED: u32 = 1 << 1;
pub const LP_PARTITION_ATTR_UPDATED: u32 = 1 << 2;
pub const LP_PARTITION_ATTR_DISABLED: u32 = 1 << 3;

// Sector size (always 512 bytes, compatible with Linux kernel)
pub const LP_SECTOR_SIZE: u64 = 512;

// Space reserved at the beginning of the Super partition (to avoid creating unexpected boot sectors)
pub const LP_PARTITION_RESERVED_BYTES: u64 = 4096;

// extent target type
pub const LP_TARGET_TYPE_LINEAR: u32 = 0;
pub const LP_TARGET_TYPE_ZERO: u32 = 1;

// Partition group flag
pub const LP_GROUP_SLOT_SUFFIXED: u32 = 1 << 0;

// block device flag
pub const LP_BLOCK_DEVICE_SLOT_SUFFIXED: u32 = 1 << 0;

// Head mark
pub const LP_HEADER_FLAG_VIRTUAL_AB_DEVICE: u32 = 0x1;
pub const LP_HEADER_FLAG_OVERLAYS_ACTIVE: u32 = 0x2;

// default value
pub const LP_METADATA_DEFAULT_PARTITION_NAME: &str = "super";
pub const DEFAULT_PARTITION_ALIGNMENT: u32 = 1048576; // 1 MiB
pub const DEFAULT_BLOCK_SIZE: u32 = 4096;

// Structure size
pub const LP_METADATA_GEOMETRY_STRUCT_SIZE: u32 = 52;
pub const LP_METADATA_HEADER_V1_0_SIZE: u32 = 128;
pub const LP_METADATA_HEADER_V1_2_SIZE: u32 = 256;
pub const LP_METADATA_PARTITION_SIZE: u32 = 52;
pub const LP_METADATA_EXTENT_SIZE: u32 = 24;
pub const LP_METADATA_GROUP_SIZE: u32 = 48;
pub const LP_METADATA_BLOCK_DEVICE_SIZE: u32 = 64;

// LP metadata geometry information
#[derive(Debug, Clone)]
pub struct LpMetadataGeometry {
    // Magic number (LP_METADATA_GEOMETRY_MAGIC)
    pub magic: u32,
    // Structure size
    pub struct_size: u32,
    // SHA256 checksum
    pub checksum: [u8; 32],
    // Maximum size of a single metadata copy
    pub metadata_max_size: u32,
    // Number of metadata slots
    pub metadata_slot_count: u32,
    // logical block size
    pub logical_block_size: u32,
}

impl Default for LpMetadataGeometry {
    fn default() -> Self {
        Self {
            magic: LP_METADATA_GEOMETRY_MAGIC,
            struct_size: LP_METADATA_GEOMETRY_STRUCT_SIZE,
            checksum: [0u8; 32],
            metadata_max_size: 65536,
            metadata_slot_count: 2,
            logical_block_size: DEFAULT_BLOCK_SIZE,
        }
    }
}

impl LpMetadataGeometry {
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut buf = vec![0u8; LP_METADATA_GEOMETRY_SIZE as usize];
        buf[0..4].copy_from_slice(&self.magic.to_le_bytes());
        buf[4..8].copy_from_slice(&self.struct_size.to_le_bytes());
        buf[8..40].copy_from_slice(&self.checksum);
        buf[40..44].copy_from_slice(&self.metadata_max_size.to_le_bytes());
        buf[44..48].copy_from_slice(&self.metadata_slot_count.to_le_bytes());
        buf[48..52].copy_from_slice(&self.logical_block_size.to_le_bytes());
        buf
    }

    pub fn from_bytes(buf: &[u8]) -> Option<Self> {
        if buf.len() < LP_METADATA_GEOMETRY_STRUCT_SIZE as usize {
            return None;
        }
        let magic = u32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]]);
        if magic != LP_METADATA_GEOMETRY_MAGIC {
            return None;
        }
        let mut checksum = [0u8; 32];
        checksum.copy_from_slice(&buf[8..40]);
        Some(Self {
            magic,
            struct_size: u32::from_le_bytes([buf[4], buf[5], buf[6], buf[7]]),
            checksum,
            metadata_max_size: u32::from_le_bytes([buf[40], buf[41], buf[42], buf[43]]),
            metadata_slot_count: u32::from_le_bytes([buf[44], buf[45], buf[46], buf[47]]),
            logical_block_size: u32::from_le_bytes([buf[48], buf[49], buf[50], buf[51]]),
        })
    }
}

// LP metadata table descriptor
#[derive(Debug, Clone, Default)]
pub struct LpMetadataTableDescriptor {
    // Offset of the table (relative to after the metadata header)
    pub offset: u32,
    // Number of entries in the table
    pub num_entries: u32,
    // size of each entry
    pub entry_size: u32,
}

impl LpMetadataTableDescriptor {
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut buf = vec![0u8; 12];
        buf[0..4].copy_from_slice(&self.offset.to_le_bytes());
        buf[4..8].copy_from_slice(&self.num_entries.to_le_bytes());
        buf[8..12].copy_from_slice(&self.entry_size.to_le_bytes());
        buf
    }

    pub fn from_bytes(buf: &[u8]) -> Self {
        Self {
            offset: u32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]]),
            num_entries: u32::from_le_bytes([buf[4], buf[5], buf[6], buf[7]]),
            entry_size: u32::from_le_bytes([buf[8], buf[9], buf[10], buf[11]]),
        }
    }
}

// LP metadata header
#[derive(Debug, Clone)]
pub struct LpMetadataHeader {
    // Magic number (LP_METADATA_HEADER_MAGIC)
    pub magic: u32,
    // Major version number
    pub major_version: u16,
    // minor version number
    pub minor_version: u16,
    // head size
    pub header_size: u32,
    // Header SHA256 checksum
    pub header_checksum: [u8; 32],
    // total size of all tables
    pub tables_size: u32,
    // SHA256 checksum of table data
    pub tables_checksum: [u8; 32],
    // partition table descriptor
    pub partitions: LpMetadataTableDescriptor,
    // extent table descriptor
    pub extents: LpMetadataTableDescriptor,
    // Partition group table descriptor
    pub groups: LpMetadataTableDescriptor,
    // block device table descriptor
    pub block_devices: LpMetadataTableDescriptor,
    // Header flag (v1.2+)
    pub flags: u32,
}

impl Default for LpMetadataHeader {
    fn default() -> Self {
        Self {
            magic: LP_METADATA_HEADER_MAGIC,
            major_version: LP_METADATA_MAJOR_VERSION,
            minor_version: LP_METADATA_MINOR_VERSION_MAX,
            header_size: LP_METADATA_HEADER_V1_2_SIZE,
            header_checksum: [0u8; 32],
            tables_size: 0,
            tables_checksum: [0u8; 32],
            partitions: LpMetadataTableDescriptor::default(),
            extents: LpMetadataTableDescriptor::default(),
            groups: LpMetadataTableDescriptor::default(),
            block_devices: LpMetadataTableDescriptor::default(),
            flags: 0,
        }
    }
}

impl LpMetadataHeader {
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut buf = vec![0u8; self.header_size as usize];
        buf[0..4].copy_from_slice(&self.magic.to_le_bytes());
        buf[4..6].copy_from_slice(&self.major_version.to_le_bytes());
        buf[6..8].copy_from_slice(&self.minor_version.to_le_bytes());
        buf[8..12].copy_from_slice(&self.header_size.to_le_bytes());
        buf[12..44].copy_from_slice(&self.header_checksum);
        buf[44..48].copy_from_slice(&self.tables_size.to_le_bytes());
        buf[48..80].copy_from_slice(&self.tables_checksum);
        buf[80..92].copy_from_slice(&self.partitions.to_bytes());
        buf[92..104].copy_from_slice(&self.extents.to_bytes());
        buf[104..116].copy_from_slice(&self.groups.to_bytes());
        buf[116..128].copy_from_slice(&self.block_devices.to_bytes());
        if self.header_size >= LP_METADATA_HEADER_V1_2_SIZE {
            buf[128..132].copy_from_slice(&self.flags.to_le_bytes());
        }
        buf
    }

    pub fn from_bytes(buf: &[u8]) -> Option<Self> {
        if buf.len() < LP_METADATA_HEADER_V1_0_SIZE as usize {
            return None;
        }
        let magic = u32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]]);
        if magic != LP_METADATA_HEADER_MAGIC {
            return None;
        }
        let mut header_checksum = [0u8; 32];
        header_checksum.copy_from_slice(&buf[12..44]);
        let mut tables_checksum = [0u8; 32];
        tables_checksum.copy_from_slice(&buf[48..80]);
        let header_size = u32::from_le_bytes([buf[8], buf[9], buf[10], buf[11]]);
        let flags = if header_size >= LP_METADATA_HEADER_V1_2_SIZE && buf.len() >= 132 {
            u32::from_le_bytes([buf[128], buf[129], buf[130], buf[131]])
        } else {
            0
        };
        Some(Self {
            magic,
            major_version: u16::from_le_bytes([buf[4], buf[5]]),
            minor_version: u16::from_le_bytes([buf[6], buf[7]]),
            header_size,
            header_checksum,
            tables_size: u32::from_le_bytes([buf[44], buf[45], buf[46], buf[47]]),
            tables_checksum,
            partitions: LpMetadataTableDescriptor::from_bytes(&buf[80..92]),
            extents: LpMetadataTableDescriptor::from_bytes(&buf[92..104]),
            groups: LpMetadataTableDescriptor::from_bytes(&buf[104..116]),
            block_devices: LpMetadataTableDescriptor::from_bytes(&buf[116..128]),
            flags,
        })
    }
}

// LP metadata partition
#[derive(Debug, Clone)]
pub struct LpMetadataPartition {
    // Partition name (36 bytes, null terminated)
    pub name: [u8; 36],
    // Partition properties
    pub attributes: u32,
    // index of first extent
    pub first_extent_index: u32,
    // Number of extents
    pub num_extents: u32,
    // Index of the partition group to which it belongs
    pub group_index: u32,
}

impl Default for LpMetadataPartition {
    fn default() -> Self {
        Self {
            name: [0u8; 36],
            attributes: LP_PARTITION_ATTR_NONE,
            first_extent_index: 0,
            num_extents: 0,
            group_index: 0,
        }
    }
}

impl LpMetadataPartition {
    pub fn new(name: &str) -> Self {
        let mut partition = Self::default();
        partition.set_name(name);
        partition
    }

    pub fn set_name(&mut self, name: &str) {
        self.name = [0u8; 36];
        let bytes = name.as_bytes();
        let len = bytes.len().min(35);
        self.name[..len].copy_from_slice(&bytes[..len]);
    }

    pub fn get_name(&self) -> String {
        let end = self.name.iter().position(|&b| b == 0).unwrap_or(36);
        String::from_utf8_lossy(&self.name[..end]).to_string()
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        let mut buf = vec![0u8; LP_METADATA_PARTITION_SIZE as usize];
        buf[0..36].copy_from_slice(&self.name);
        buf[36..40].copy_from_slice(&self.attributes.to_le_bytes());
        buf[40..44].copy_from_slice(&self.first_extent_index.to_le_bytes());
        buf[44..48].copy_from_slice(&self.num_extents.to_le_bytes());
        buf[48..52].copy_from_slice(&self.group_index.to_le_bytes());
        buf
    }

    pub fn from_bytes(buf: &[u8]) -> Option<Self> {
        if buf.len() < LP_METADATA_PARTITION_SIZE as usize {
            return None;
        }
        let mut name = [0u8; 36];
        name.copy_from_slice(&buf[0..36]);
        Some(Self {
            name,
            attributes: u32::from_le_bytes([buf[36], buf[37], buf[38], buf[39]]),
            first_extent_index: u32::from_le_bytes([buf[40], buf[41], buf[42], buf[43]]),
            num_extents: u32::from_le_bytes([buf[44], buf[45], buf[46], buf[47]]),
            group_index: u32::from_le_bytes([buf[48], buf[49], buf[50], buf[51]]),
        })
    }
}

// LP metadata extent
#[derive(Debug, Clone, Default)]
pub struct LpMetadataExtent {
    // Extent size in 512-byte sectors
    pub num_sectors: u64,
    // Target type (LP_TARGET_TYPE_*)
    pub target_type: u32,
    // Target data (LINEAR: physical sector offset, ZERO: must be 0)
    pub target_data: u64,
    // Target source (LINEAR: block device index, ZERO: must be 0)
    pub target_source: u32,
}

impl LpMetadataExtent {
    pub fn new_linear(num_sectors: u64, physical_sector: u64, device_index: u32) -> Self {
        Self {
            num_sectors,
            target_type: LP_TARGET_TYPE_LINEAR,
            target_data: physical_sector,
            target_source: device_index,
        }
    }

    pub fn new_zero(num_sectors: u64) -> Self {
        Self {
            num_sectors,
            target_type: LP_TARGET_TYPE_ZERO,
            target_data: 0,
            target_source: 0,
        }
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        let mut buf = vec![0u8; LP_METADATA_EXTENT_SIZE as usize];
        buf[0..8].copy_from_slice(&self.num_sectors.to_le_bytes());
        buf[8..12].copy_from_slice(&self.target_type.to_le_bytes());
        buf[12..20].copy_from_slice(&self.target_data.to_le_bytes());
        buf[20..24].copy_from_slice(&self.target_source.to_le_bytes());
        buf
    }

    pub fn from_bytes(buf: &[u8]) -> Option<Self> {
        if buf.len() < LP_METADATA_EXTENT_SIZE as usize {
            return None;
        }
        Some(Self {
            num_sectors: u64::from_le_bytes([
                buf[0], buf[1], buf[2], buf[3], buf[4], buf[5], buf[6], buf[7],
            ]),
            target_type: u32::from_le_bytes([buf[8], buf[9], buf[10], buf[11]]),
            target_data: u64::from_le_bytes([
                buf[12], buf[13], buf[14], buf[15], buf[16], buf[17], buf[18], buf[19],
            ]),
            target_source: u32::from_le_bytes([buf[20], buf[21], buf[22], buf[23]]),
        })
    }
}

// LP metadata partition group
#[derive(Debug, Clone)]
pub struct LpMetadataPartitionGroup {
    // Partition group name (36 bytes, null terminated)
    pub name: [u8; 36],
    // logo
    pub flags: u32,
    // Maximum size (0 means unlimited)
    pub maximum_size: u64,
}

impl Default for LpMetadataPartitionGroup {
    fn default() -> Self {
        Self {
            name: [0u8; 36],
            flags: 0,
            maximum_size: 0,
        }
    }
}

impl LpMetadataPartitionGroup {
    pub fn new(name: &str, maximum_size: u64) -> Self {
        let mut group = Self::default();
        group.set_name(name);
        group.maximum_size = maximum_size;
        group
    }

    pub fn set_name(&mut self, name: &str) {
        self.name = [0u8; 36];
        let bytes = name.as_bytes();
        let len = bytes.len().min(35);
        self.name[..len].copy_from_slice(&bytes[..len]);
    }

    pub fn get_name(&self) -> String {
        let end = self.name.iter().position(|&b| b == 0).unwrap_or(36);
        String::from_utf8_lossy(&self.name[..end]).to_string()
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        let mut buf = vec![0u8; LP_METADATA_GROUP_SIZE as usize];
        buf[0..36].copy_from_slice(&self.name);
        buf[36..40].copy_from_slice(&self.flags.to_le_bytes());
        buf[40..48].copy_from_slice(&self.maximum_size.to_le_bytes());
        buf
    }

    pub fn from_bytes(buf: &[u8]) -> Option<Self> {
        if buf.len() < LP_METADATA_GROUP_SIZE as usize {
            return None;
        }
        let mut name = [0u8; 36];
        name.copy_from_slice(&buf[0..36]);
        Some(Self {
            name,
            flags: u32::from_le_bytes([buf[36], buf[37], buf[38], buf[39]]),
            maximum_size: u64::from_le_bytes([
                buf[40], buf[41], buf[42], buf[43], buf[44], buf[45], buf[46], buf[47],
            ]),
        })
    }
}

// LP metadata block device
#[derive(Debug, Clone)]
pub struct LpMetadataBlockDevice {
    // The first sector available for allocation of a logical partition
    pub first_logical_sector: u64,
    // Partition alignment
    pub alignment: u32,
    // Alignment offset
    pub alignment_offset: u32,
    // block device size
    pub size: u64,
    // Partition name (36 bytes, null terminated)
    pub partition_name: [u8; 36],
    // logo
    pub flags: u32,
}

impl Default for LpMetadataBlockDevice {
    fn default() -> Self {
        Self {
            first_logical_sector: 0,
            alignment: DEFAULT_PARTITION_ALIGNMENT,
            alignment_offset: 0,
            size: 0,
            partition_name: [0u8; 36],
            flags: 0,
        }
    }
}

impl LpMetadataBlockDevice {
    pub fn new(name: &str, size: u64) -> Self {
        let mut device = Self::default();
        device.set_partition_name(name);
        device.size = size;
        device
    }

    pub fn set_partition_name(&mut self, name: &str) {
        self.partition_name = [0u8; 36];
        let bytes = name.as_bytes();
        let len = bytes.len().min(35);
        self.partition_name[..len].copy_from_slice(&bytes[..len]);
    }

    pub fn get_partition_name(&self) -> String {
        let end = self
            .partition_name
            .iter()
            .position(|&b| b == 0)
            .unwrap_or(36);
        String::from_utf8_lossy(&self.partition_name[..end]).to_string()
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        let mut buf = vec![0u8; LP_METADATA_BLOCK_DEVICE_SIZE as usize];
        buf[0..8].copy_from_slice(&self.first_logical_sector.to_le_bytes());
        buf[8..12].copy_from_slice(&self.alignment.to_le_bytes());
        buf[12..16].copy_from_slice(&self.alignment_offset.to_le_bytes());
        buf[16..24].copy_from_slice(&self.size.to_le_bytes());
        buf[24..60].copy_from_slice(&self.partition_name);
        buf[60..64].copy_from_slice(&self.flags.to_le_bytes());
        buf
    }

    pub fn from_bytes(buf: &[u8]) -> Option<Self> {
        if buf.len() < LP_METADATA_BLOCK_DEVICE_SIZE as usize {
            return None;
        }
        let mut partition_name = [0u8; 36];
        partition_name.copy_from_slice(&buf[24..60]);
        Some(Self {
            first_logical_sector: u64::from_le_bytes([
                buf[0], buf[1], buf[2], buf[3], buf[4], buf[5], buf[6], buf[7],
            ]),
            alignment: u32::from_le_bytes([buf[8], buf[9], buf[10], buf[11]]),
            alignment_offset: u32::from_le_bytes([buf[12], buf[13], buf[14], buf[15]]),
            size: u64::from_le_bytes([
                buf[16], buf[17], buf[18], buf[19], buf[20], buf[21], buf[22], buf[23],
            ]),
            partition_name,
            flags: u32::from_le_bytes([buf[60], buf[61], buf[62], buf[63]]),
        })
    }
}

// Full LP metadata
#[derive(Debug, Clone, Default)]
pub struct LpMetadata {
    pub geometry: LpMetadataGeometry,
    pub header: LpMetadataHeader,
    pub partitions: Vec<LpMetadataPartition>,
    pub extents: Vec<LpMetadataExtent>,
    pub groups: Vec<LpMetadataPartitionGroup>,
    pub block_devices: Vec<LpMetadataBlockDevice>,
}
