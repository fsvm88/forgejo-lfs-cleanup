mod checks;
mod cli;
mod forgejo;
mod gitrepo;
mod lfs_listing;
mod session;

use std::sync::Arc;

use anyhow::{Context, bail};
use clap::Parser;
use reqwest::cookie::Jar;

fn main() -> anyhow::Result<()> {
    let args = cli::Args::parse();
    checks::require_git_and_git_lfs()?;

    let target = forgejo::parse_ssh_url(&args.ssh_url, args.web_url.as_deref())?;
    println!(
        "target: {}/{} via {}",
        target.owner, target.repo, target.web_base
    );

    let jar = match &args.cookie_file {
        Some(path) => session::jar_from_cookie_file(path)?,
        None => Arc::new(Jar::default()),
    };

    let client = reqwest::blocking::Client::builder()
        .cookie_provider(jar)
        .user_agent(session::USER_AGENT)
        .build()
        .context("failed to build HTTP client")?;

    let version = forgejo::detect_forgejo(&client, &target.web_base)?;
    println!("confirmed Forgejo, version {version}");

    if args.cookie_file.is_none() {
        match (&args.username, &args.password) {
            (Some(username), Some(password)) => {
                session::login(&client, &target.web_base, username, password)?;
                println!("logged in as {username}");
            }
            _ => bail!(
                "no authentication provided — pass --cookie-file, or both --username/FORGEJO_USER \
                 and --password/FORGEJO_PASS. A bare API token is not sufficient for the LFS \
                 admin routes this tool depends on (see ai-docs/research-forgejo-lfs-api.md §5)."
            ),
        }
    }

    println!("cloning bare mirror...");
    let clone = gitrepo::clone_bare_mirror(&args.ssh_url, args.workdir.as_deref())?;
    println!("cloned into {}", clone.path.display());

    let referenced = gitrepo::referenced_lfs_oids(&clone)?;
    println!("{} LFS objects referenced from some ref", referenced.len());

    println!("enumerating LFS objects on the server...");
    let existing = lfs_listing::enumerate_all(&client, &target)?;
    let total_bytes: u64 = existing.values().map(|o| o.size).sum();
    println!(
        "{} LFS objects exist on the server, {} bytes total",
        existing.len(),
        total_bytes
    );

    let stale: Vec<_> = existing
        .keys()
        .filter(|oid| !referenced.contains(*oid))
        .collect();
    let stale_bytes: u64 = stale
        .iter()
        .filter_map(|oid| existing.get(*oid))
        .map(|o| o.size)
        .sum();
    println!(
        "{} LFS objects are unreferenced (stale), {} bytes total",
        stale.len(),
        stale_bytes
    );

    Ok(())
}
