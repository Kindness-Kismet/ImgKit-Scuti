// 通用工具函数模块

#[cfg(windows)]
use std::fs::File;
#[cfg(windows)]
use std::io::Write;
use std::path::Path;

// symlink 信息
pub struct SymlinkInfo {
    pub is_symlink: bool,
    pub target: Option<String>,
}

// 检测文件是否为 symlink 并读取其目标路径
// Windows 下检测 !<symlink> 格式的文件
// Unix 下使用标准 API
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

        // 优先检测 Windows 原生 symlink
        let metadata = fs::symlink_metadata(path)?;
        if metadata.file_type().is_symlink() {
            let target = fs::read_link(path)?;
            return Ok(SymlinkInfo {
                is_symlink: true,
                target: Some(target.to_string_lossy().to_string()),
            });
        }

        // 检测 !<symlink> 格式的文件
        if metadata.is_file() {
            let mut file = fs::File::open(path)?;
            let mut header = [0u8; 10];
            if file.read_exact(&mut header).is_ok() && &header == b"!<symlink>" {
                // 读取剩余内容
                let mut content = Vec::new();
                file.read_to_end(&mut content)?;

                // 跳过 BOM (0xFF 0xFE) 并解码 UTF-16LE
                if content.len() >= 2 && content[0] == 0xFF && content[1] == 0xFE {
                    let utf16_bytes = &content[2..];
                    // 将 UTF-16LE 转换为 String
                    let mut utf16_chars = Vec::new();
                    for chunk in utf16_bytes.chunks(2) {
                        if chunk.len() == 2 {
                            let ch = u16::from_le_bytes([chunk[0], chunk[1]]);
                            if ch == 0 {
                                break; // 空终止符
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

// 跨平台创建 symlink
// Windows 下创建特殊格式的文件并设置 FILE_ATTRIBUTE_SYSTEM
// Unix 下创建标准 symlink
pub fn create_symlink(target: &str, link_path: &Path) -> anyhow::Result<()> {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        use winapi::um::fileapi::SetFileAttributesW;
        use winapi::um::winnt::FILE_ATTRIBUTE_SYSTEM;

        // Windows: 创建特殊格式的文件并设置系统属性
        // 格式: !<symlink> + UTF-16LE BOM + UTF-16LE 目标路径 + 两个空字节
        if link_path.exists() {
            std::fs::remove_file(link_path)?;
        }

        let mut file_content = Vec::new();
        file_content.extend_from_slice(b"!<symlink>");
        // 添加 UTF-16LE BOM
        file_content.extend_from_slice(b"\xff\xfe");

        // 将目标路径编码为 UTF-16LE
        for ch in target.encode_utf16() {
            file_content.extend_from_slice(&ch.to_le_bytes());
        }
        file_content.extend_from_slice(&[0u8, 0u8]);

        let mut file = File::create(link_path)?;
        file.write_all(&file_content)?;
        drop(file);

        // 设置 FILE_ATTRIBUTE_SYSTEM 使其成为有效的 symlink
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
        // Unix: 创建标准 symlink
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

// 从字节切片创建 symlink (用于 EXT4)
pub fn create_symlink_from_bytes(
    link_target_bytes: &[u8],
    output_path: &Path,
) -> anyhow::Result<()> {
    let link_target = String::from_utf8(link_target_bytes.to_vec())
        .map_err(|e| anyhow::anyhow!("failed to decode symlink target path: {}", e))?;
    create_symlink(&link_target, output_path)
}
