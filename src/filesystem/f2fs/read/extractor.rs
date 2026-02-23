// F2FS file extractor
//
// Provides file system extraction and configuration generation functions

use crate::filesystem::f2fs::{F2fsVolume, Inode, Nid};
use crate::utils::create_symlink;
use anyhow::Result;
use rayon::prelude::*;
use std::collections::HashMap;
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

// Extract configuration
pub struct ExtractConfig {
    // Enter the image file path
    pub input_image: String,
    // Output base directory
    pub output_dir: String,
    // Custom fs_config path (optional)
    pub fs_config_path: Option<String>,
    // Custom file_contexts path (optional)
    pub file_contexts_path: Option<String>,
}

// File extraction task
#[derive(Clone)]
struct FileTask {
    inode: Inode,
    nid: Nid,
    path: PathBuf,
    output_path: PathBuf,
    file_type: u8,
}

// Extract the F2FS image file to the specified directory
pub fn extract_image(config: ExtractConfig) -> Result<()> {
    let start_time = Instant::now();

    let reader = F2fsVolume::new(&config.input_image)?;

    // Detect partition name
    let partition_name = Path::new(&config.input_image)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("unknown")
        .to_string();
    let filename = Path::new(&config.input_image)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("unknown");

    // Create output directory structure
    let output_base = PathBuf::from(&config.output_dir);
    let extract_path = output_base.join(&partition_name);
    let config_dir = output_base.join("config");

    fs::create_dir_all(&extract_path)?;
    fs::create_dir_all(&config_dir)?;

    // Extract file system
    let root_nid = Nid(3);

    // Store fs_config and file_contexts data
    let mut fs_config = Vec::new();
    let mut file_contexts = HashMap::new();

    // Extract xattr of root directory (consistent with EXT4/EROFS)
    let root_node = reader.read_node(root_nid)?;
    let root_inode = Inode::from_bytes(&root_node)?;
    extract_xattrs(
        &reader,
        &root_inode,
        root_nid,
        &PathBuf::from("/"),
        &mut file_contexts,
    );

    // Phase One: Collect All Documents Task
    let mut file_tasks = Vec::new();
    let mut stack = vec![(root_nid, PathBuf::from("/"))];
    let mut visited = std::collections::HashSet::new();

    while let Some((nid, current_path)) = stack.pop() {
        // Prevent circular references
        if !visited.insert(nid.0) {
            continue;
        }

        let node = reader
            .read_node(nid)
            .map_err(|e| anyhow::anyhow!("读取节点失败 {}: {}", nid.0, e))?;

        let inode = Inode::from_bytes(&node)?;

        if !inode.is_dir() {
            continue;
        }

        let entries = reader
            .read_dir(&inode, nid)
            .map_err(|e| anyhow::anyhow!("读取目录失败 nid={}: {}", nid.0, e))?;

        for entry in entries {
            if entry.name == "." || entry.name == ".." {
                continue;
            }

            let safe_name = match crate::utils::sanitize_single_component(&entry.name) {
                Ok(value) => value,
                Err(err) => {
                    log::warn!("跳过非法目录项 {:?}: {}", entry.name, err);
                    continue;
                }
            };
            let entry_rel_path = current_path.join(&safe_name);
            let entry_path = crate::utils::join_output_path(&extract_path, &entry_rel_path)
                .map_err(|e| anyhow::anyhow!("无效输出路径 {:?}: {}", entry_rel_path, e))?;
            let entry_node = reader.read_node(entry.nid).map_err(|e| {
                anyhow::anyhow!(
                    "读取条目节点失败 {} (nid={}): {}",
                    entry.name,
                    entry.nid.0,
                    e
                )
            })?;
            let entry_inode = Inode::from_bytes(&entry_node)?;

            let rel_path_for_config = entry_rel_path.clone();

            extract_xattrs(
                &reader,
                &entry_inode,
                entry.nid,
                &rel_path_for_config,
                &mut file_contexts,
            );

            // Collect fs_config information
            let mode = entry_inode.mode & 0o777;
            let link_target = if entry.file_type == 7 {
                // F2FS_FT_SYMLINK
                reader
                    .read_symlink_target(&entry_inode, entry.nid)
                    .unwrap_or_default()
            } else {
                String::new()
            };

            fs_config.push((
                rel_path_for_config.clone(),
                entry_inode.uid,
                entry_inode.gid,
                mode,
                String::new(),
                link_target,
            ));

            if entry_inode.is_dir() {
                fs::create_dir_all(&entry_path)?;
                stack.push((entry.nid, entry_rel_path));
            } else if entry.file_type == 7 || entry_inode.is_reg() {
                // Collect file tasks instead of extracting immediately
                file_tasks.push(FileTask {
                    inode: entry_inode.clone(),
                    nid: entry.nid,
                    path: rel_path_for_config.clone(),
                    output_path: entry_path,
                    file_type: entry.file_type,
                });
            }
        }
    }

    // Phase 2: Process all files in parallel
    let image_path_arc = Arc::new(config.input_image.clone());
    let extracted_count = Arc::new(AtomicUsize::new(0));
    let failed_count = Arc::new(AtomicUsize::new(0));
    let total_task_count = file_tasks.len();

    file_tasks.par_iter().for_each_init(
        || F2fsVolume::new(image_path_arc.as_str()).ok(),
        |thread_volume, task| {
            if let Some(volume) = thread_volume.as_ref() {
                let result: Result<()> = if task.file_type == 7 {
                    match volume.read_symlink_target(&task.inode, task.nid) {
                        Ok(link_target) => {
                            if task.output_path.exists() {
                                let _ = fs::remove_file(&task.output_path);
                            }
                            create_symlink(&link_target, &task.output_path)
                        }
                        Err(e) => Err(anyhow::anyhow!("读取符号链接目标失败: {}", e)),
                    }
                } else if task.inode.is_reg() {
                    match volume.read_file_data(&task.inode, task.nid) {
                        Ok(data) => match File::create(&task.output_path) {
                            Ok(mut file) => match file.write_all(&data) {
                                Ok(_) => Ok(()),
                                Err(e) => Err(anyhow::anyhow!("写入文件失败: {}", e)),
                            },
                            Err(e) => Err(anyhow::anyhow!("创建文件失败: {}", e)),
                        },
                        Err(e) => Err(anyhow::anyhow!("读取文件数据失败: {}", e)),
                    }
                } else {
                    Ok(())
                };

                if let Err(e) = result {
                    log::warn!(" 提取 {:?} 失败: {}", task.path, e);
                    failed_count.fetch_add(1, Ordering::Relaxed);
                }
            } else {
                log::warn!("线程内 F2FS volume 初始化失败，跳过 {:?}", task.path);
                failed_count.fetch_add(1, Ordering::Relaxed);
            }

            let count = extracted_count.fetch_add(1, Ordering::Relaxed) + 1;
            crate::utils::display_progress(filename, count, total_task_count);
        },
    );

    crate::utils::display_completion(start_time.elapsed());

    // Generate configuration file
    let fs_config_path = config.fs_config_path.unwrap_or_else(|| {
        config_dir
            .join(format!("{}_fs_config", partition_name))
            .to_string_lossy()
            .to_string()
    });

    let file_contexts_path = config.file_contexts_path.unwrap_or_else(|| {
        config_dir
            .join(format!("{}_file_contexts", partition_name))
            .to_string_lossy()
            .to_string()
    });

    if let Some(parent) = Path::new(&fs_config_path).parent() {
        fs::create_dir_all(parent)?;
    }
    if let Some(parent) = Path::new(&file_contexts_path).parent() {
        fs::create_dir_all(parent)?;
    }

    crate::utils::write_fs_config(Path::new(&fs_config_path), &partition_name, &fs_config)?;
    crate::utils::write_file_contexts(
        Path::new(&file_contexts_path),
        &partition_name,
        &file_contexts,
    )?;

    let failed = failed_count.load(Ordering::Relaxed);
    if failed > 0 {
        return Err(anyhow::anyhow!("F2FS 提取存在 {} 个失败条目", failed));
    }

    Ok(())
}

// Extract extended attributes (xattrs) from inode
fn extract_xattrs(
    reader: &F2fsVolume,
    inode: &Inode,
    nid: Nid,
    path: &Path,
    file_contexts: &mut std::collections::HashMap<PathBuf, String>,
) {
    match reader.read_xattrs(inode, nid) {
        Ok(xattrs) => {
            for (name, value) in xattrs {
                if name == "security.selinux" {
                    let mut context = String::from_utf8_lossy(&value)
                        .trim_start_matches('\0')
                        .trim_end_matches('\0')
                        .to_string();
                    if !context.is_empty() {
                        if !context.ends_with(":s0") {
                            context.push_str(":s0");
                        }
                        file_contexts.insert(path.to_path_buf(), context);
                    }
                }
            }
        }
        Err(_) => {
            // Ignore xattr read failures, some files may not have xattr
        }
    }
}
