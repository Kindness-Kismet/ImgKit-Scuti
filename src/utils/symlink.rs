// Common utility functions module

#[cfg(windows)]
use std::fs::File;
#[cfg(windows)]
use std::io::Write;
use std::path::Path;

// Symlink information
pub struct SymlinkInfo {
    pub is_symlink: bool,
    pub target: Option<String>,
}

// Detect whether a file is a symlink and read its target path.
// On Windows, detects files in the !<symlink> format.
// On Unix, uses the standard API.
pub fn read_symlink_info(path: &Path) -> anyhow::Result<SymlinkInfo> {
    #[cfg(unix)]
    {
        use std::fs;
        let metadata = fs::symlink_metadata(path)?;
        if metadata.file_type().is_symlink() {
            let target = fs::read_link(path)?;
            Ok(SymlinkInfo {
                is_symlink: true,
                target: Some(target.to_string_lossy().to_string()),
            })
        } else {
            Ok(SymlinkInfo {
                is_symlink: false,
                target: None,
            })
        }
    }

    #[cfg(windows)]
    {
        use std::fs;
        use std::io::Read;

        // Check for a native Windows symlink first
        let metadata = fs::symlink_metadata(path)?;
        if metadata.file_type().is_symlink() {
            let target = fs::read_link(path)?;
            return Ok(SymlinkInfo {
                is_symlink: true,
                target: Some(target.to_string_lossy().to_string()),
            });
        }

        // Check for a file in the !<symlink> format
        if metadata.is_file() {
            let mut file = fs::File::open(path)?;
            let mut header = [0u8; 10];
            if file.read_exact(&mut header).is_ok() && &header == b"!<symlink>" {
                // Read remaining content
                let mut content = Vec::new();
                file.read_to_end(&mut content)?;

                // Skip BOM (0xFF 0xFE) and decode UTF-16LE
                if content.len() >= 2 && content[0] == 0xFF && content[1] == 0xFE {
                    let utf16_bytes = &content[2..];
                    // Convert UTF-16LE to a String
                    let mut utf16_chars = Vec::new();
                    for chunk in utf16_bytes.chunks(2) {
                        if chunk.len() == 2 {
                            let ch = u16::from_le_bytes([chunk[0], chunk[1]]);
                            if ch == 0 {
                                break; // null terminator
                            }
                            utf16_chars.push(ch);
                        }
                    }
                    let target = String::from_utf16_lossy(&utf16_chars);
                    return Ok(SymlinkInfo {
                        is_symlink: true,
                        target: Some(target),
                    });
                }
            }
        }

        Ok(SymlinkInfo {
            is_symlink: false,
            target: None,
        })
    }

    #[cfg(not(any(unix, windows)))]
    {
        Ok(SymlinkInfo {
            is_symlink: false,
            target: None,
        })
    }
}

// Cross-platform symlink creation.
// On Windows, creates a specially formatted file and sets FILE_ATTRIBUTE_SYSTEM.
// On Unix, creates a standard symlink.
pub fn create_symlink(target: &str, link_path: &Path) -> anyhow::Result<()> {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        use winapi::um::fileapi::SetFileAttributesW;
        use winapi::um::winnt::FILE_ATTRIBUTE_SYSTEM;

        // Windows: create a special-format file and set the system attribute.
        // Format: !<symlink> + UTF-16LE BOM + UTF-16LE target path + two null bytes.
        if link_path.exists() {
            std::fs::remove_file(link_path)?;
        }

        let mut file_content = Vec::new();
        file_content.extend_from_slice(b"!<symlink>");
        // Add UTF-16LE BOM
        file_content.extend_from_slice(b"\xff\xfe");

        // Encode target path as UTF-16LE
        for ch in target.encode_utf16() {
            file_content.extend_from_slice(&ch.to_le_bytes());
        }
        file_content.extend_from_slice(&[0u8, 0u8]);

        let mut file = File::create(link_path)?;
        file.write_all(&file_content)?;
        drop(file);

        // Set FILE_ATTRIBUTE_SYSTEM to make it a proper symlink
        let path_wide: Vec<u16> = link_path.as_os_str().encode_wide().chain(Some(0)).collect();
        unsafe {
            if SetFileAttributesW(path_wide.as_ptr(), FILE_ATTRIBUTE_SYSTEM) == 0 {
                return Err(anyhow::anyhow!(
                    "failed to set file system attribute: {}",
                    std::io::Error::last_os_error()
                ));
            }
        }
        Ok(())
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::symlink;
        // Unix: create a standard symlink
        if link_path.exists() {
            std::fs::remove_file(link_path)?;
        }
        symlink(target, link_path).map_err(|e| anyhow::anyhow!("failed to create symlink: {}", e))
    }

    #[cfg(not(any(unix, windows)))]
    {
        Err(anyhow::anyhow!("symlinks are not supported on this OS"))
    }
}

// Create a symlink from a byte slice (used for EXT4)
pub fn create_symlink_from_bytes(
    link_target_bytes: &[u8],
    output_path: &Path,
) -> anyhow::Result<()> {
    let link_target = String::from_utf8(link_target_bytes.to_vec())
        .map_err(|e| anyhow::anyhow!("failed to decode symlink target path: {}", e))?;
    create_symlink(&link_target, output_path)
}
