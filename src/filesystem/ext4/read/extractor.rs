// EXT4 文件系统提取器

use crate::container::sparse::SparseReader;
use crate::filesystem::ext4::types::{Ext4Volume, Inode, VfsCapData};
use crate::utils::{
    check_windows_case_conflict, create_symlink_from_bytes, display_completion, display_progress,
    is_case_sensitive_directory, join_output_path, sanitize_single_component, write_file_contexts,
    write_fs_config,
};
use rayon::prelude::*;
use std::collections::HashMap;
use std::fs::{self, File};
use std::io::BufReader;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

// EXT4 提取配置
pub struct ExtractConfig {
    pub input_image: String,
    pub output_dir: String,
    pub fs_config_path: Option<String>,
    pub file_contexts_path: Option<String>,
}

// 文件提取任务
#[derive(Clone)]
struct FileTask {
    inode: Inode,
    path: PathBuf,
    output_path: PathBuf,
    link_target_bytes: Vec<u8>,
}

// 从 ext4 镜像中提取文件与元数据
pub fn extract_image(config: ExtractConfig) -> anyhow::Result<()> {
    let fs_config_output_path = config.fs_config_path.as_ref().map(PathBuf::from);
    let file_contexts_output_path = config.file_contexts_path.as_ref().map(PathBuf::from);
    let output_dir = Path::new(&config.output_dir);
    let config_output_dir = output_dir.join("config");

    if let Ok(sparse_reader) = SparseReader::new(&config.input_image) {
        let volume = Ext4Volume::new(sparse_reader)?;
        return extract(
            volume,
            &config.input_image,
            &config_output_dir,
            output_dir,
            true,
            fs_config_output_path,
            file_contexts_output_path,
        );
    }

    let file = File::open(&config.input_image)
        .map_err(|e| anyhow::anyhow!("failed to open image file: {}", e))?;
    let buf_reader = BufReader::new(file);
    let volume = Ext4Volume::new(buf_reader)?;
    extract(
        volume,
        &config.input_image,
        &config_output_dir,
        output_dir,
        false,
        fs_config_output_path,
        file_contexts_output_path,
    )
}

fn extract<R: std::io::Read + std::io::Seek + Send>(
    mut volume: Ext4Volume<R>,
    image_path: &str,
    config_output_dir: &Path,
    file_extract_dir: &Path,
    is_sparse: bool,
    custom_fs_config_path: Option<PathBuf>,
    custom_file_contexts_path: Option<PathBuf>,
) -> anyhow::Result<()> {
    let start_time = Instant::now();

    let image_path_obj = Path::new(image_path);
    let prefix = image_path_obj
        .file_stem()
        .and_then(|s| s.to_str())
        .ok_or_else(|| anyhow::anyhow!("unable to get file name"))?;
    let filename = image_path_obj
        .file_name()
        .and_then(|s| s.to_str())
        .ok_or_else(|| anyhow::anyhow!("unable to get full file name"))?;

    fs::create_dir_all(config_output_dir)
        .map_err(|e| anyhow::anyhow!("failed to create config output directory: {}", e))?;
    fs::create_dir_all(file_extract_dir)
        .map_err(|e| anyhow::anyhow!("failed to create file extraction directory: {}", e))?;
    let extract_root = file_extract_dir.join(prefix);
    fs::create_dir_all(&extract_root)?;
    let case_sensitive = is_case_sensitive_directory(&extract_root)?;
    let mut case_map = HashMap::new();

    let mut fs_config = Vec::new();
    let mut file_contexts = HashMap::new();

    // 第一阶段: 收集所有文件任务
    let mut file_tasks = Vec::new();
    let mut stack = vec![(volume.root()?, PathBuf::from("/"))];
    let mut visited = std::collections::HashSet::new();

    while let Some((inode, path)) = stack.pop() {
        if !visited.insert(path.clone()) {
            continue;
        }

        let capabilities = extract_xattrs(&mut volume, &inode, &path, &mut file_contexts)?;

        let owner = inode.inode.i_uid();
        let group = inode.inode.i_gid();
        let mode = inode.inode.i_mode & 0o777;
        let mut link_target = String::new();
        let mut link_target_bytes: Vec<u8> = Vec::new();

        if inode.is_symlink() {
            link_target_bytes = inode.open_read(&mut volume)?;
            link_target = String::from_utf8_lossy(&link_target_bytes).to_string();
        }

        fs_config.push((
            path.clone(),
            owner,
            group,
            mode,
            capabilities,
            link_target.clone(),
        ));

        if !case_sensitive {
            check_windows_case_conflict(&mut case_map, &extract_root, &path)?;
        }

        let output_path = join_output_path(&extract_root, &path)
            .map_err(|e| anyhow::anyhow!("invalid output path {:?}: {}", path, e))?;

        if let Some(parent) = output_path.parent() {
            fs::create_dir_all(parent)?;
        }

        if inode.is_dir() {
            fs::create_dir_all(&output_path)?;
            for (name, inode_idx, _file_type) in inode.open_dir(&mut volume)? {
                if name == "." || name == ".." {
                    continue;
                }
                let safe_name = match sanitize_single_component(&name) {
                    Ok(value) => value,
                    Err(err) => {
                        log::warn!("skipping invalid directory entry {:?}: {}", name, err);
                        continue;
                    }
                };
                let sub_inode = volume.get_inode(inode_idx)?;
                stack.push((sub_inode, path.join(safe_name)));
            }
        } else if inode.is_file() || inode.is_symlink() {
            // 先收集文件任务, 不立即提取
            file_tasks.push(FileTask {
                inode: inode.clone(),
                path: path.clone(),
                output_path,
                link_target_bytes,
            });
        }
    }

    // 第二阶段: 并行处理所有文件
    let image_path_string = Arc::new(image_path.to_string());
    let extracted_count = Arc::new(AtomicUsize::new(0));
    let failed_count = Arc::new(AtomicUsize::new(0));
    let total_task_count = file_tasks.len();

    // 定义宏处理单个任务, 消除重复代码
    macro_rules! process_task {
        ($vol:expr, $task:expr) => {{
            let result: anyhow::Result<()> = if $task.inode.is_file() {
                match $task.inode.open_read($vol) {
                    Ok(data) => fs::write(&$task.output_path, data)
                        .map_err(|e| anyhow::anyhow!("failed to write file: {}", e)),
                    Err(e) => Err(anyhow::anyhow!("failed to read file data: {}", e)),
                }
            } else if $task.inode.is_symlink() {
                if $task.output_path.exists() {
                    let _ = fs::remove_file(&$task.output_path);
                }
                create_symlink_from_bytes(&$task.link_target_bytes, &$task.output_path)
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
                SparseReader::new(image_path_string.as_str())
                    .ok()
                    .and_then(|reader| Ext4Volume::new(reader).ok())
            },
            |thread_volume, task| {
                if let Some(volume) = thread_volume.as_mut() {
                    process_task!(volume, task);
                } else {
                    log::warn!(
                        "failed to init EXT4 volume in worker thread, skipping {:?}",
                        task.path
                    );
                    failed_count.fetch_add(1, Ordering::Relaxed);
                    let count = extracted_count.fetch_add(1, Ordering::Relaxed) + 1;
                    display_progress(filename, count, total_task_count);
                }
            },
        );
    } else {
        file_tasks.par_iter().for_each_init(
            || {
                File::open(image_path_string.as_str())
                    .ok()
                    .and_then(|file| Ext4Volume::new(BufReader::new(file)).ok())
            },
            |thread_volume, task| {
                if let Some(volume) = thread_volume.as_mut() {
                    process_task!(volume, task);
                } else {
                    log::warn!(
                        "failed to init EXT4 volume in worker thread, skipping {:?}",
                        task.path
                    );
                    failed_count.fetch_add(1, Ordering::Relaxed);
                    let count = extracted_count.fetch_add(1, Ordering::Relaxed) + 1;
                    display_progress(filename, count, total_task_count);
                }
            },
        );
    }

    display_completion(start_time.elapsed());

    let fs_config_output_path = custom_fs_config_path
        .unwrap_or_else(|| config_output_dir.join(format!("{}_fs_config", prefix)));
    let file_contexts_output_path = custom_file_contexts_path
        .unwrap_or_else(|| config_output_dir.join(format!("{}_file_contexts", prefix)));
    if let Some(parent) = fs_config_output_path.parent() {
        fs::create_dir_all(parent)?;
    }
    if let Some(parent) = file_contexts_output_path.parent() {
        fs::create_dir_all(parent)?;
    }

    write_fs_config(&fs_config_output_path, prefix, &fs_config)?;
    write_file_contexts(&file_contexts_output_path, prefix, &file_contexts)?;

    let failed = failed_count.load(Ordering::Relaxed);
    if failed > 0 {
        return Err(anyhow::anyhow!(
            "EXT4 extraction has {} failed entries",
            failed
        ));
    }

    Ok(())
}

// 从 inode 中提取 xattr
fn extract_xattrs<R: std::io::Read + std::io::Seek>(
    volume: &mut Ext4Volume<R>,
    inode: &Inode,
    path: &Path,
    file_contexts: &mut std::collections::HashMap<PathBuf, String>,
) -> anyhow::Result<String> {
    let mut capabilities = String::new();
    for (name, value) in inode.xattrs(volume)? {
        if name == "security.selinux" {
            let mut context = String::from_utf8_lossy(&value)
                .trim_start_matches('\0')
                .to_string();
            context.push_str(":s0");
            file_contexts.insert(path.to_path_buf(), context);
        } else if name == "security.capability"
            && let Some(cap_data) = VfsCapData::from_bytes(&value)
        {
            let effective_caps = cap_data.effective();
            if effective_caps > 0 {
                capabilities = format!("capabilities=0X{:X}", effective_caps);
            }
        }
    }
    Ok(capabilities)
}
