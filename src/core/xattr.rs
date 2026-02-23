// Extended attribute abstraction layer
//
// A unified interface for defining file system extended attributes

// Extended attribute error type
pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

// extended attribute entry
#[derive(Debug, Clone)]
pub struct XattrEntry {
    // Property name
    pub name: String,
    // attribute value
    pub value: Vec<u8>,
}

// extended attribute namespace
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum XattrNamespace {
    // user namespace
    User,
    // system namespace
    System,
    // Security namespace (SELinux, etc.)
    Security,
    // Trust namespace
    Trusted,
}

impl XattrNamespace {
    // Get namespace from name prefix
    pub fn from_prefix(name: &str) -> Option<Self> {
        if name.starts_with("user.") {
            Some(XattrNamespace::User)
        } else if name.starts_with("system.") {
            Some(XattrNamespace::System)
        } else if name.starts_with("security.") {
            Some(XattrNamespace::Security)
        } else if name.starts_with("trusted.") {
            Some(XattrNamespace::Trusted)
        } else {
            None
        }
    }

    // Get namespace prefix
    pub fn prefix(&self) -> &'static str {
        match self {
            XattrNamespace::User => "user.",
            XattrNamespace::System => "system.",
            XattrNamespace::Security => "security.",
            XattrNamespace::Trusted => "trusted.",
        }
    }
}

// Unified interface for extended attributes
pub trait Xattr {
    // Read all extended attributes
    fn list_xattr(&self) -> Result<Vec<String>>;

    // Read the extended attribute of the specified name
    fn get_xattr(&self, name: &str) -> Result<Vec<u8>>;

    // Check if there is a specified extended attribute
    fn has_xattr(&self, name: &str) -> bool {
        self.get_xattr(name).is_ok()
    }

    // Get the key-value pairs of all extended attributes
    fn get_all_xattr(&self) -> Result<Vec<XattrEntry>> {
        let names = self.list_xattr()?;
        let mut entries = Vec::new();
        for name in names {
            if let Ok(value) = self.get_xattr(&name) {
                entries.push(XattrEntry { name, value });
            }
        }
        Ok(entries)
    }

    // Get the extended attributes of the specified namespace
    fn get_xattr_by_namespace(&self, namespace: XattrNamespace) -> Result<Vec<XattrEntry>> {
        let prefix = namespace.prefix();
        let entries = self.get_all_xattr()?;
        Ok(entries
            .into_iter()
            .filter(|entry| entry.name.starts_with(prefix))
            .collect())
    }
}

// Extension interface for writable extended properties
pub trait WritableXattr: Xattr {
    // Set extended attributes
    fn set_xattr(&mut self, name: &str, value: &[u8]) -> Result<()>;

    // Remove extended attributes
    fn remove_xattr(&mut self, name: &str) -> Result<()>;

    // Clear all extended attributes
    fn clear_xattr(&mut self) -> Result<()> {
        let names = self.list_xattr()?;
        for name in names {
            self.remove_xattr(&name)?;
        }
        Ok(())
    }

    // Set extended attributes in batches
    fn set_all_xattr(&mut self, entries: &[XattrEntry]) -> Result<()> {
        for entry in entries {
            self.set_xattr(&entry.name, &entry.value)?;
        }
        Ok(())
    }
}
