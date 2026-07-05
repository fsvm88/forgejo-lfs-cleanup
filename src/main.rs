mod checks;
mod cli;
mod forgejo;
mod gitrepo;

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

    println!("cloning bare mirror...");
    let clone = gitrepo::clone_bare_mirror(&args.ssh_url, args.workdir.as_deref())?;
    println!("cloned into {}", clone.path.display());

    let referenced = gitrepo::referenced_lfs_oids(&clone)?;
    println!("{} LFS objects referenced from some ref", referenced.len());

    Ok(())
}
