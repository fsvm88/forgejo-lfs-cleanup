mod checks;
mod cli;

use clap::Parser;

fn main() -> anyhow::Result<()> {
    let args = cli::Args::parse();
    checks::require_git_and_git_lfs()?;

    println!("{args:#?}");
    Ok(())
}
