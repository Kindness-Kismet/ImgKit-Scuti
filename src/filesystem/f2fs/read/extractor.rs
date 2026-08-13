// F2FS 文件提取器
//
// 提供文件系统提取与配置文件生成功能

use crate::container::sparse::SparseReader;
use crate::filesystem::f2fs::{F2fsVolume, Inode, Nid};
use crate::utils::{
    check_windows_case_conflict, create_symlink, display_completion, display_progress,
    is_case_sensitive_directory, join_output_path, sanitize_single_component, write_file_contexts,
    write_fs_config,
};
use anyhow::Result;
use rayon::prelude::*;
use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{Read, Seek, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

// 提取配置
pub struct ExtractConfig {
    // 输入镜像文件路径
    pub input_image: String,
    // 输出根目录
    pub output_dir: String,
    // 自定义 fs_config 路径 (可选)
    pub fs_config_path: Option<String>,
    // 自定义 file_contexts 路径 (可选)
    pub file_contexts_path: Option<String>,
}

// 文件提取任务
#[derive(Clone)]
struct FileTask {
    inode: Inode,
    nid: Nid,
    path: PathBuf,
    output_path: PathBuf,
    file_type: u8,
}

// 入口: 自动识别 sparse 与 raw 镜像并分派到对应的读取实现
pub fn extract_image(config: ExtractConfig) -> Result<()> {
    if let Ok(sparse_reader) = SparseReader::new(&config.input_image) {
        let volume = F2fsVolume::from_reader(sparse_reader)
            .map_err(|e| anyhow::anyhow!("failed to parse sparse F2FS superblock: {}", e))?;
        return extract(config, volume, true);
    }

    let volume = F2fsVolume::new(&config.input_image)
        .map_err(|e| anyhow::anyhow!("failed to open F2FS image: {}", e))?;
    extract(config, volume, false)
}

fn extract<R: Read + Seek + Send + Sync>(
    config: ExtractConfig,
    volume: F2fsVolume<R>,
    is_sparse: bool,
) -> Result<()> {
    let start_time = Instant::now();

    // 识别分区名
    let partition_name = Path::new(&config.input_image)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("unknown")
        .to_string();
    let filename = Path::new(&config.input_image)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("unknown");

    // 创建输出目录结构
    let output_base = PathBuf::from(&config.output_dir);
    let extract_path = output_base.join(&partition_name);
    let config_dir = output_base.join("config");

    fs::create_dir_all(&extract_path)?;
    fs::create_dir_all(&config_dir)?;
    let case_sensitive = is_case_sensitive_directory(&extract_path)?;
    let mut case_map = HashMap::new();

    // 提取文件系统
    let root_nid = Nid(3);

    // 保存 fs_config 与 file_contexts 数据
    let mut fs_config = Vec::new();
    let mut file_contexts = HashMap::new();

    // 提取根目录的 xattr (与 EXT4/EROFS 保持一致)
    let root_node = volume.read_node(root_nid)?;
    let root_inode = Inode::from_bytes(&root_node)?;
    fs_config.push((
        PathBuf::from("/"),
        root_inode.uid,
        root_inode.gid,
        root_inode.mode & 0o777,
        String::new(),
        String::new(),
    ));
    extract_xattrs(
        &volume,
        &root_inode,
        root_nid,
        &PathBuf::from("/"),
        &mut file_contexts,
    );

    // 第一阶段: 按遍历顺序收集所有文件任务
    let mut file_tasks = Vec::new();
    let mut visited = std::collections::HashSet::new();
    collect_directory_tasks(
        &volume,
        root_nid,
        Path::new("/"),
        &extract_path,
        case_sensitive,
        &mut case_map,
        &mut visited,
        &mut file_tasks,
        &mut fs_config,
        &mut file_contexts,
    )?;

    // 第二阶段: 并行处理所有文件
    let image_path_arc = Arc::new(config.input_image.clone());
    let extracted_count = Arc::new(AtomicUsize::new(0));
    let failed_count = Arc::new(AtomicUsize::new(0));
    let total_task_count = file_tasks.len();

    macro_rules! process_task {
        ($vol:expr, $task:expr) => {{
            let result: Result<()> = if $task.file_type == 7 {
                match $vol.read_symlink_target(&$task.inode, $task.nid) {
                    Ok(link_target) => {
                        if $task.output_path.exists() {
                            let _ = fs::remove_file(&$task.output_path);
                        }
                        create_symlink(&link_target, &$task.output_path)
                    }
                    Err(e) => Err(anyhow::anyhow!("failed to read symlink target: {}", e)),
                }
            } else if $task.inode.is_reg() {
                match $vol.read_file_data(&$task.inode, $task.nid) {
                    Ok(data) => match File::create(&$task.output_path) {
                        Ok(mut file) => match file.write_all(&data) {
                            Ok(_) => Ok(()),
                            Err(e) => Err(anyhow::anyhow!("failed to write file: {}", e)),
                        },
                        Err(e) => Err(anyhow::anyhow!("failed to create file: {}", e)),
                    },
                    Err(e) => Err(anyhow::anyhow!("failed to read file data: {}", e)),
                }
            } else {
                Ok(())
            };

            if let Err(e) = result {
                log::warn!(" failed to extract {:?}: {}", $task.path, e);
                failed_count.fetch_add(1, Ordering::Relaxed);
            }

            let count = extracted_count.fetch_add(1, Ordering::Relaxed) + 1;
            display_progress(filename, count, total_task_count);
        }};
    }

    if is_sparse {
        file_tasks.par_iter().for_each_init(
            || {
                SparseReader::new(image_path_arc.as_str())
                    .ok()
                    .and_then(|reader| F2fsVolume::from_reader(reader).ok())
            },
            |thread_volume, task| {
                if let Some(volume) = thread_volume.as_ref() {
                    process_task!(volume, task);
                } else {
                    log::warn!("F2FS sparse volume init failed, skipping {:?}", task.path);
                    failed_count.fetch_add(1, Ordering::Relaxed);
                    let count = extracted_count.fetch_add(1, Ordering::Relaxed) + 1;
                    display_progress(filename, count, total_task_count);
                }
            },
        );
    } else {
        file_tasks.par_iter().for_each_init(
            || F2fsVolume::new(image_path_arc.as_str()).ok(),
            |thread_volume, task| {
                if let Some(volume) = thread_volume.as_ref() {
                    process_task!(volume, task);
                } else {
                    log::warn!("F2FS volume init failed, skipping {:?}", task.path);
                    failed_count.fetch_add(1, Ordering::Relaxed);
                    let count = extracted_count.fetch_add(1, Ordering::Relaxed) + 1;
                    display_progress(filename, count, total_task_count);
                }
            },
        );
    }

    display_completion(start_time.elapsed());

    // 生成配置文件
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

    write_fs_config(Path::new(&fs_config_path), &partition_name, &fs_config)?;
    write_file_contexts(
        Path::new(&file_contexts_path),
        &partition_name,
        &file_contexts,
    )?;

    let failed = failed_count.load(Ordering::Relaxed);
    if failed > 0 {
        return Err(anyhow::anyhow!(
            "F2FS extraction had {} failed entries",
            failed
        ));
    }

    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn collect_directory_tasks<R: Read + Seek + Send>(
    reader: &F2fsVolume<R>,
    nid: Nid,
    current_path: &Path,
    extract_path: &Path,
    case_sensitive: bool,
    case_map: &mut HashMap<String, PathBuf>,
    visited: &mut std::collections::HashSet<u32>,
    file_tasks: &mut Vec<FileTask>,
    fs_config: &mut Vec<(PathBuf, u32, u32, u16, String, String)>,
    file_contexts: &mut HashMap<PathBuf, String>,
) -> Result<()> {
    if !visited.insert(nid.0) {
        return Ok(());
    }

    let node = reader
        .read_node(nid)
        .map_err(|e| anyhow::anyhow!("failed to read node {}: {}", nid.0, e))?;
    let inode = Inode::from_bytes(&node)?;
    if !inode.is_dir() {
        return Ok(());
    }

    let entries = reader
        .read_dir(&inode, nid)
        .map_err(|e| anyhow::anyhow!("failed to read directory nid={}: {}", nid.0, e))?;

    for entry in entries {
        if entry.name == "." || entry.name == ".." {
            continue;
        }

        let safe_name = match sanitize_single_component(&entry.name) {
            Ok(value) => value,
            Err(err) => {
                log::warn!("skipping invalid dentry {:?}: {}", entry.name, err);
                continue;
            }
        };
        let entry_rel_path = current_path.join(&safe_name);
        if !case_sensitive {
            check_windows_case_conflict(case_map, extract_path, &entry_rel_path)?;
        }
        let entry_path = join_output_path(extract_path, &entry_rel_path)
            .map_err(|e| anyhow::anyhow!("invalid output path {:?}: {}", entry_rel_path, e))?;
        let entry_node = reader.read_node(entry.nid).map_err(|e| {
            anyhow::anyhow!(
                "failed to read entry node {} (nid={}): {}",
                entry.name,
                entry.nid.0,
                e
            )
        })?;
        let entry_inode = Inode::from_bytes(&entry_node)?;

        extract_xattrs(
            reader,
            &entry_inode,
            entry.nid,
            &entry_rel_path,
            file_contexts,
        );

        let mode = entry_inode.mode & 0o777;
        let link_target = if entry.file_type == 7 {
            reader
                .read_symlink_target(&entry_inode, entry.nid)
                .unwrap_or_default()
        } else {
            String::new()
        };

        fs_config.push((
            entry_rel_path.clone(),
            entry_inode.uid,
            entry_inode.gid,
            mode,
            String::new(),
            link_target,
        ));

        if entry_inode.is_dir() {
            fs::create_dir_all(&entry_path)?;
            collect_directory_tasks(
                reader,
                entry.nid,
                &entry_rel_path,
                extract_path,
                case_sensitive,
                case_map,
                visited,
                file_tasks,
                fs_config,
                file_contexts,
            )?;
        } else if entry.file_type == 7 || entry_inode.is_reg() {
            file_tasks.push(FileTask {
                inode: entry_inode.clone(),
                nid: entry.nid,
                path: entry_rel_path.clone(),
                output_path: entry_path,
                file_type: entry.file_type,
            });
        }
    }

    Ok(())
}

// 从 inode 提取扩展属性 (xattr)
fn extract_xattrs<R: Read + Seek + Send>(
    reader: &F2fsVolume<R>,
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
            // 忽略 xattr 读取失败, 部分文件可能没有 xattr
        }
    }
}
