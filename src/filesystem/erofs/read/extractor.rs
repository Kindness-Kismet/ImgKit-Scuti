// EROFS filesystem extractor

use super::volume::ErofsVolume;
use crate::filesystem::erofs::*;
use crate::utils::create_symlink;
use rayon::prelude::*;
use std::collections::{HashMap, HashSet};
use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;
use zerocopy::TryFromBytes;

// Configuration for an extraction run.
pub struct ExtractConfig {
    pub input_image: String,
    pub output_dir: String,
    pub fs_config_path: Option<String>,
    pub file_contexts_path: Option<String>,
}

#[repr(C, packed)]
#[derive(zerocopy::FromZeros, zerocopy::IntoBytes, Debug, Clone, Copy)]
struct VfsCapData {
    magic_etc: u32,
    data: [CapData; 2],
}

#[repr(C, packed)]
#[derive(zerocopy::FromZeros, zerocopy::IntoBytes, Debug, Clone, Copy)]
struct CapData {
    permitted: u32,
    inheritable: u32,
}

impl VfsCapData {
    fn from_bytes(bytes: &[u8]) -> Option<Self> {
        Self::try_read_from_bytes(bytes).ok()
    }

    // Compute the effective capability mask from the VFS cap structure (v2 only).
    fn effective(&self) -> u64 {
        let magic = self.magic_etc;
        let version = magic & 0xFF000000;
        let effective_bit = (magic & 0x00000001) != 0;

        if version == 0x02000000 {
            let mut effective_caps = self.data[0].permitted as u64;
            if effective_bit {
                effective_caps |= (self.data[1].permitted as u64) << 32;
            }
            effective_caps
        } else {
            0
        }
    }
}

// A pending file or symlink extraction task.
#[derive(Clone)]
struct FileTask {
    inode_info: InodeInfo,
    path: PathBuf,
    output_path: PathBuf,
    link_target: String,
}

// Entry point: extract files and metadata from an EROFS image.
pub fn extract_image(config: ExtractConfig) -> anyhow::Result<()> {
    let fs_config_output_path = config.fs_config_path.as_ref().map(PathBuf::from);
    let file_contexts_output_path = config.file_contexts_path.as_ref().map(PathBuf::from);
    let image_path = Path::new(&config.input_image);
    let output_dir = Path::new(&config.output_dir);
    let config_output_dir = output_dir.join("config");

    extract(
        image_path,
        &config_output_dir,
        output_dir,
        fs_config_output_path,
        file_contexts_output_path,
    )
}

fn extract(
    image_path: &Path,
    config_output_dir: &Path,
    file_extract_dir: &Path,
    custom_fs_config_path: Option<PathBuf>,
    custom_file_contexts_path: Option<PathBuf>,
) -> anyhow::Result<()> {
    let start_time = Instant::now();

    let file = File::open(image_path)?;
    let mut volume = ErofsVolume::new(file)?;
    let prefix = image_path
        .file_stem()
        .and_then(|s| s.to_str())
        .ok_or_else(|| anyhow::anyhow!("failed to get file stem"))?;
    let filename = image_path
        .file_name()
        .and_then(|s| s.to_str())
        .ok_or_else(|| anyhow::anyhow!("failed to get filename"))?;

    fs::create_dir_all(config_output_dir)?;
    fs::create_dir_all(file_extract_dir)?;
    let extract_root = file_extract_dir.join(prefix);
    fs::create_dir_all(&extract_root)?;

    let mut fs_config = Vec::new();
    let mut file_contexts = HashMap::new();

    let root_nid = volume.root_nid();
    let root_inode = volume.read_inode(root_nid)?;

    let mut stack = vec![(root_inode, PathBuf::from("/"))];
    let mut visited = HashSet::new();

    // Phase 1: walk the directory tree and collect file tasks.
    let mut file_tasks = Vec::new();
    while let Some((inode_info, path)) = stack.pop() {
        if !visited.insert(path.clone()) {
            continue;
        }

        let capabilities = extract_xattrs(&mut volume, &inode_info, &path, &mut file_contexts)
            .unwrap_or_else(|e| {
                log::warn!("failed to read xattr for {:?}: {}", path, e);
                String::new()
            });

        let mode = inode_info.mode & 0o777;
        let mut link_target = String::new();

        if is_symlink(&inode_info) {
            link_target = volume.read_symlink(&inode_info)?;
        }

        fs_config.push((
            path.clone(),
            inode_info.uid,
            inode_info.gid,
            mode,
            capabilities,
            link_target.clone(),
        ));

        let output_path = crate::utils::join_output_path(&extract_root, &path)
            .map_err(|e| anyhow::anyhow!("无效输出路径 {:?}: {}", path, e))?;

        if let Some(parent) = output_path.parent() {
            fs::create_dir_all(parent)?;
        }

        if is_dir(&inode_info) {
            fs::create_dir_all(&output_path)?;
            for (name, child_nid, _file_type) in volume.read_dir(&inode_info)? {
                let safe_name = match crate::utils::sanitize_single_component(&name) {
                    Ok(value) => value,
                    Err(err) => {
                        log::warn!("跳过非法目录项 {:?}: {}", name, err);
                        continue;
                    }
                };
                match volume.read_inode(child_nid) {
                    Ok(child_inode) => {
                        stack.push((child_inode, path.join(safe_name)));
                    }
                    Err(e) => {
                        log::warn!("failed to read child inode {} ({}): {}", child_nid, name, e);
                        continue;
                    }
                }
            }
        } else if is_regular(&inode_info) || is_symlink(&inode_info) {
            // Defer extraction to the parallel phase.
            file_tasks.push(FileTask {
                inode_info: inode_info.clone(),
                path: path.clone(),
                output_path,
                link_target,
            });
        }
    }

    // Phase 2: extract files in parallel, each thread opens its own file handle.
    let image_path_arc = Arc::new(image_path.to_path_buf());
    let extracted_count = Arc::new(AtomicUsize::new(0));
    let failed_count = Arc::new(AtomicUsize::new(0));
    let total_task_count = file_tasks.len();

    file_tasks.par_iter().for_each_init(
        || {
            File::open(image_path_arc.as_path())
                .ok()
                .and_then(|file| ErofsVolume::new(file).ok())
        },
        |thread_volume, task| {
            if let Some(volume) = thread_volume.as_mut() {
                let result: anyhow::Result<()> = if is_regular(&task.inode_info) {
                    match volume.read_file_data(&task.inode_info) {
                        Ok(data) => fs::write(&task.output_path, data)
                            .map_err(|e| anyhow::anyhow!("failed to write file: {}", e)),
                        Err(e) => Err(anyhow::anyhow!("failed to read file data: {}", e)),
                    }
                } else if is_symlink(&task.inode_info) {
                    if task.output_path.exists() {
                        let _ = fs::remove_file(&task.output_path);
                    }
                    create_symlink(&task.link_target, &task.output_path)
                } else {
                    Ok(())
                };

                if let Err(e) = result {
                    log::warn!("failed to extract {:?}: {}", task.path, e);
                    failed_count.fetch_add(1, Ordering::Relaxed);
                }
            } else {
                log::warn!("线程内 EROFS volume 初始化失败，跳过 {:?}", task.path);
                failed_count.fetch_add(1, Ordering::Relaxed);
            }

            let count = extracted_count.fetch_add(1, Ordering::Relaxed) + 1;
            crate::utils::display_progress(filename, count, total_task_count);
        },
    );

    crate::utils::display_completion(start_time.elapsed());

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

    crate::utils::write_fs_config(&fs_config_output_path, prefix, &fs_config)?;
    crate::utils::write_file_contexts(&file_contexts_output_path, prefix, &file_contexts)?;

    let failed = failed_count.load(Ordering::Relaxed);
    if failed > 0 {
        return Err(anyhow::anyhow!("EROFS 提取存在 {} 个失败条目", failed));
    }

    Ok(())
}

fn is_dir(inode: &InodeInfo) -> bool {
    (inode.mode & 0xF000) == 0x4000
}

fn is_regular(inode: &InodeInfo) -> bool {
    (inode.mode & 0xF000) == 0x8000
}

fn is_symlink(inode: &InodeInfo) -> bool {
    (inode.mode & 0xF000) == 0xA000
}

// Extract xattrs from an inode, populating SELinux contexts and capability strings.
fn extract_xattrs(
    volume: &mut ErofsVolume,
    inode: &InodeInfo,
    path: &Path,
    file_contexts: &mut HashMap<PathBuf, String>,
) -> anyhow::Result<String> {
    let mut capabilities = String::new();
    for (name, value) in volume.read_xattrs(inode)? {
        if name == "security.selinux" {
            let mut context = String::from_utf8_lossy(&value)
                .trim_start_matches('\0')
                .trim_end_matches('\0')
                .to_string();
            if !context.ends_with(":s0") {
                context.push_str(":s0");
            }
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
