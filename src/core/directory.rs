// directory abstraction layer
//
// A unified interface for defining file system directories

use super::inode::FileType;

// Directory error type
pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

// Catalog item information
#[derive(Debug, Clone)]
pub struct DirEntry {
    // file name
    pub name: String,
    // Inode number
    pub ino: u64,
    // File type
    pub file_type: FileType,
}

// Unified interface for directories
pub trait Directory {
    // Read all items in a directory
    fn read_dir(&self) -> Result<Vec<DirEntry>>;

    // Find directory entry with specified name
    fn lookup(&self, name: &str) -> Result<DirEntry> {
        self.read_dir()?
            .into_iter()
            .find(|entry| entry.name == name)
            .ok_or_else(|| format!("文件或目录 '{}' 不存在", name).into())
    }

    // Checks whether the directory contains the specified file
    fn contains(&self, name: &str) -> bool {
        self.lookup(name).is_ok()
    }

    // Get the number of catalog items
    fn entry_count(&self) -> Result<usize> {
        Ok(self.read_dir()?.len())
    }
}

// Extended interface for writable directories
pub trait WritableDirectory: Directory {
    // Create new catalog entry
    fn create_entry(&mut self, name: &str, ino: u64, file_type: FileType) -> Result<()>;

    // Delete directory entry
    fn remove_entry(&mut self, name: &str) -> Result<()>;

    // Rename directory entry
    fn rename_entry(&mut self, old_name: &str, new_name: &str) -> Result<()> {
        let entry = self.lookup(old_name)?;
        self.remove_entry(old_name)?;
        self.create_entry(new_name, entry.ino, entry.file_type)?;
        Ok(())
    }
}
