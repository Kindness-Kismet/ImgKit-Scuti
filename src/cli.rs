// CLI subcommand module.
// Provides implementations for each subcommand.

mod erofs_pack;
mod ext4_pack;
mod extract;
mod f2fs_pack;
mod super_pack;

pub use erofs_pack::run_erofs_pack;
pub use ext4_pack::run_ext4_pack;
pub use extract::run_extract;
pub use f2fs_pack::run_f2fs_pack;
pub use super_pack::run_super_pack;

use anyhow::Result;

// Parse a size string (plain byte count)
pub fn parse_size(size_str: &str) -> Result<u64> {
    use anyhow::anyhow;

    let size_str = size_str.trim();
    if size_str.is_empty() {
        return Err(anyhow!("size must not be empty"));
    }

    size_str
        .parse()
        .map_err(|_| anyhow!("invalid size: {}, must be a plain byte count", size_str))
}
