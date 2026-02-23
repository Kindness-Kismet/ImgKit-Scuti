// SELinux config file writer

use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};

// Write filesystem config to a file
pub fn write_fs_config(
    path: &Path,
    prefix: &str,
    fs_config: &[(PathBuf, u32, u32, u16, String, String)],
) -> anyhow::Result<()> {
    let mut f = File::create(path)
        .map_err(|e| anyhow::anyhow!("failed to create fs_config file: {}", e))?;
    let root_perm = fs_config
        .iter()
        .find(|(p, ..)| p.as_os_str() == "/")
        .map(|(_, o, g, m, ..)| (*o, *g, *m))
        .unwrap_or((0, 0, 0o755));

    writeln!(f, "/ {} {} {:04o}", root_perm.0, root_perm.1, root_perm.2)?;
    writeln!(
        f,
        "{} {} {} {:04o}",
        prefix, root_perm.0, root_perm.1, root_perm.2
    )?;

    for (p, owner, group, mode, cap, link) in fs_config {
        if p.as_os_str() == "/" {
            continue;
        }
        let out_path = Path::new(prefix).join(p.strip_prefix("/").unwrap_or(p));
        let mut cap_link = cap.to_string();
        if !link.is_empty() {
            if !cap_link.is_empty() {
                cap_link.push(' ');
            }
            cap_link.push_str(link);
        }

        let path_str = out_path
            .to_str()
            .ok_or_else(|| anyhow::anyhow!("path contains invalid UTF-8"))?
            .replace('\\', "/");
        let line = format!("{} {} {} {:04o} {}", path_str, owner, group, mode, cap_link)
            .trim_end()
            .to_string();
        writeln!(f, "{}", line)?;
    }
    Ok(())
}

// Write file contexts to a file
pub fn write_file_contexts(
    path: &Path,
    prefix: &str,
    file_contexts: &std::collections::HashMap<PathBuf, String>,
) -> anyhow::Result<()> {
    let mut f = File::create(path)
        .map_err(|e| anyhow::anyhow!("failed to create file_contexts file: {}", e))?;
    let mut contexts: Vec<_> = file_contexts.iter().collect();
    contexts.sort_by_key(|(k, _)| (*k).clone());

    if let Some(root_context) = file_contexts.get(&PathBuf::from("/")) {
        writeln!(f, "/ {}", root_context)?;
        writeln!(f, "/{} {}", prefix, root_context)?;
        writeln!(f, "/{}/ {}", prefix, root_context)?;
    }

    for (p, context) in contexts {
        if p.as_os_str() == "/" {
            continue;
        }
        let out_path = Path::new(prefix).join(p.strip_prefix("/").unwrap_or(p));
        let path_str = out_path
            .to_str()
            .ok_or_else(|| anyhow::anyhow!("path contains invalid UTF-8"))?
            .replace('\\', "/");
        let escaped_path = path_str
            .replace('.', "\\.")
            .replace('+', "\\+")
            .replace('-', "\\-")
            .replace('[', "\\[")
            .replace(']', "\\]")
            .replace('(', "\\(")
            .replace(')', "\\)")
            .replace('{', "\\{")
            .replace('}', "\\}")
            .replace('~', "\\~");
        writeln!(f, "/{} {}", escaped_path, context)?;
    }
    Ok(())
}
