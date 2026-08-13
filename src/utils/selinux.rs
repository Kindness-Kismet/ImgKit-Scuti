// SELinux 配置文件写入

use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};

fn format_mode(mode: u16) -> String {
    if mode == 0 {
        "00".to_string()
    } else {
        format!("{:04o}", mode)
    }
}

// 将文件系统配置写入文件
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
        .map(|(_, o, g, m, ..)| (*o, *g, *m));

    if let Some((owner, group, mode)) = root_perm {
        let mode = format_mode(mode);
        writeln!(f, "/ {} {} {}", owner, group, mode)?;
        writeln!(f, "{}/ {} {} {}", prefix, owner, group, mode)?;
    }

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
        let line = format!(
            "{} {} {} {} {}",
            path_str,
            owner,
            group,
            format_mode(*mode),
            cap_link
        )
        .trim_end()
        .to_string();
        writeln!(f, "{}", line)?;
    }
    Ok(())
}

// 将 file_contexts 写入文件
pub fn write_file_contexts(
    path: &Path,
    prefix: &str,
    file_contexts: &std::collections::HashMap<PathBuf, String>,
) -> anyhow::Result<()> {
    let mut f = File::create(path)
        .map_err(|e| anyhow::anyhow!("failed to create file_contexts file: {}", e))?;
    let mut contexts: Vec<_> = file_contexts.iter().collect();
    contexts.sort_by_key(|(k, _)| (*k).clone());

    let root_context = file_contexts
        .get(&PathBuf::from("/"))
        .map(String::as_str)
        .unwrap_or("");
    writeln!(f, "/ {}", root_context)?;
    writeln!(f, "/{}(/.*)? {}", prefix, root_context)?;
    let lost_found_context = file_contexts
        .get(&PathBuf::from("/lost+found"))
        .map(String::as_str)
        .unwrap_or(root_context);
    writeln!(f, "/{}/lost\\+found {}", prefix, lost_found_context)?;

    for (p, context) in contexts {
        if p.as_os_str() == "/" || p.as_os_str() == "/lost+found" {
            continue;
        }
        let out_path = Path::new(prefix).join(p.strip_prefix("/").unwrap_or(p));
        let path_str = out_path
            .to_str()
            .ok_or_else(|| anyhow::anyhow!("path contains invalid UTF-8"))?
            .replace('\\', "/");
        let escaped_path = escape_file_context_path(&path_str);
        writeln!(f, "/{} {}", escaped_path, context)?;
    }
    Ok(())
}

fn escape_file_context_path(path_str: &str) -> String {
    path_str
        .replace('.', "\\.")
        .replace('+', "\\+")
        .replace('[', "\\[")
        .replace(']', "\\]")
        .replace('(', "\\(")
        .replace(')', "\\)")
        .replace('{', "\\{")
        .replace('}', "\\}")
        .replace('~', "\\~")
}
