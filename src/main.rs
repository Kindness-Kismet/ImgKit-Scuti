use anyhow::Result;
use clap::Parser;

fn main() -> Result<()> {
    let cli = imgkit::Cli::parse();
    imgkit::run(cli)
}
