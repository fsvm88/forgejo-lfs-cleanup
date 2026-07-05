mod checks;
mod cli;
mod forgejo;

use anyhow::Context;
use clap::Parser;

fn main() -> anyhow::Result<()> {
    let args = cli::Args::parse();
    checks::require_git_and_git_lfs()?;

    let target = forgejo::parse_ssh_url(&args.ssh_url, args.web_url.as_deref())?;
    println!(
        "target: {}/{} via {}",
        target.owner, target.repo, target.web_base
    );

    let client = reqwest::blocking::Client::builder()
        .cookie_store(true)
        .build()
        .context("failed to build HTTP client")?;

    let version = forgejo::detect_forgejo(&client, &target.web_base)?;
    println!("confirmed Forgejo, version {version}");

    Ok(())
}
