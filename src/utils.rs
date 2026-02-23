// General utility module

pub mod detect;
pub mod logger;
pub mod path;
pub mod progress;
pub mod selinux;
pub mod symlink;

// Re-export commonly used functions
pub use detect::detect_filesystem;
pub use path::{join_output_path, normalize_image_path, sanitize_single_component};
pub use progress::{display_completion, display_progress};
pub use selinux::{write_file_contexts, write_fs_config};
pub use symlink::{create_symlink, create_symlink_from_bytes};
