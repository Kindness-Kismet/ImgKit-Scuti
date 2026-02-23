use anyhow::{Result, anyhow};
use std::path::{Component, Path, PathBuf};

pub fn normalize_image_path(path: &Path) -> Result<PathBuf> {
    let mut normalized = PathBuf::new();

    for component in path.components() {
        match component {
            Component::Normal(name) => normalized.push(name),
            Component::RootDir | Component::CurDir => {}
            Component::ParentDir => {
                return Err(anyhow!("路径包含父目录跳转: {:?}", path));
            }
            Component::Prefix(_) => {
                return Err(anyhow!("路径包含不允许的盘符前缀: {:?}", path));
            }
        }
    }

    Ok(normalized)
}

pub fn sanitize_single_component(name: &str) -> Result<String> {
    let normalized = normalize_image_path(Path::new(name))?;
    let mut components = normalized.components();

    let first = components
        .next()
        .ok_or_else(|| anyhow!("路径组件为空: {}", name))?;
    if components.next().is_some() {
        return Err(anyhow!("路径组件包含分隔符: {}", name));
    }

    match first {
        Component::Normal(value) => Ok(value.to_string_lossy().to_string()),
        _ => Err(anyhow!("无效的路径组件: {}", name)),
    }
}

pub fn join_output_path(root: &Path, image_path: &Path) -> Result<PathBuf> {
    let normalized = normalize_image_path(image_path)?;
    Ok(root.join(normalized))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normalize_image_path_basic() {
        let normalized = normalize_image_path(Path::new("/system/bin/sh")).unwrap();
        assert_eq!(normalized, PathBuf::from("system/bin/sh"));
    }

    #[test]
    fn test_normalize_image_path_reject_parent_dir() {
        assert!(normalize_image_path(Path::new("../etc/passwd")).is_err());
    }

    #[test]
    fn test_sanitize_single_component() {
        assert_eq!(sanitize_single_component("vendor").unwrap(), "vendor");
        assert!(sanitize_single_component("../vendor").is_err());
        assert!(sanitize_single_component("a/b").is_err());
    }
}
