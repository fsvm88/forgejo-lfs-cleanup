mod checks;
mod cli;
mod firefox_cookies;
mod forgejo;
mod gitrepo;
mod lfs_listing;
mod session;

use std::io::{self, Write};
use std::sync::Arc;

use anyhow::{Context, bail};
use clap::Parser;
use lfs_listing::LfsObject;
use reqwest::blocking::Client;
use reqwest::cookie::Jar;

use crate::forgejo::RepoTarget;

fn main() -> anyhow::Result<()> {
    let args = cli::Args::parse();
    checks::require_git_and_git_lfs()?;

    let target = forgejo::parse_ssh_url(&args.ssh_url, args.web_url.as_deref())?;
    println!(
        "target: {}/{} via {}",
        target.owner, target.repo, target.web_base
    );

    let host = target
        .web_base
        .host_str()
        .context("--web-url/derived URL has no host")?
        .to_string();

    let (jar, login_creds) = if let Some(path) = &args.cookie_file {
        (session::jar_from_cookie_file(path)?, None)
    } else {
        match (&args.username, &args.password) {
            (Some(username), Some(password)) => {
                (Arc::new(Jar::default()), Some((username.clone(), password.clone())))
            }
            (None, None) => {
                let (jar, profile) = firefox_cookies::jar_from_firefox_profile(&host)?;
                println!("using cookies from Firefox profile {}", profile.display());
                (jar, None)
            }
            _ => bail!("--username and --password must be given together"),
        }
    };

    let client = reqwest::blocking::Client::builder()
        .cookie_provider(jar)
        .user_agent(session::USER_AGENT)
        .build()
        .context("failed to build HTTP client")?;

    let version = forgejo::detect_forgejo(&client, &target.web_base)?;
    println!("confirmed Forgejo, version {version}");

    if let Some((username, password)) = login_creds {
        session::login(&client, &target.web_base, &username, &password)?;
        println!("logged in as {username}");
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

    let mut stale: Vec<&LfsObject> = existing
        .values()
        .filter(|obj| !referenced.contains(&obj.oid))
        .collect();
    stale.sort_by(|a, b| b.size.cmp(&a.size));
    let stale_bytes: u64 = stale.iter().map(|o| o.size).sum();

    println!(
        "\n{} LFS objects are unreferenced (stale), {} bytes total:",
        stale.len(),
        stale_bytes
    );
    for obj in &stale {
        println!("  {}  {:>12} bytes", obj.oid, obj.size);
    }

    if stale.is_empty() {
        println!("\nnothing to delete");
        return Ok(());
    }

    if !args.execute {
        println!("\ndry run: pass --execute to delete these objects");
        return Ok(());
    }

    if !args.yes && !confirm(stale.len(), stale_bytes)? {
        println!("aborted, nothing deleted");
        return Ok(());
    }

    delete_stale(&client, &target, &stale)
}

fn confirm(count: usize, bytes: u64) -> anyhow::Result<bool> {
    print!("\ndelete {count} LFS objects ({bytes} bytes)? Type 'yes' to confirm: ");
    io::stdout().flush().ok();

    let mut input = String::new();
    io::stdin()
        .read_line(&mut input)
        .context("failed to read confirmation from stdin")?;

    Ok(input.trim().eq_ignore_ascii_case("yes"))
}

/// Deletes every stale object, continuing past individual failures rather
/// than aborting the whole run on the first one — see CLAUDE.md's
/// fail-loudly rule: failures are surfaced in the summary and via a
/// non-zero exit, not swallowed, but one bad object shouldn't block
/// cleanup of the rest.
fn delete_stale(client: &Client, target: &RepoTarget, stale: &[&LfsObject]) -> anyhow::Result<()> {
    let mut failures = Vec::new();

    for obj in stale {
        match lfs_listing::delete_object(client, target, &obj.oid) {
            Ok(()) => println!("deleted {}", obj.oid),
            Err(err) => {
                println!("FAILED to delete {}: {err:#}", obj.oid);
                failures.push(obj.oid.clone());
            }
        }
    }

    println!(
        "\n{} deleted, {} failed",
        stale.len() - failures.len(),
        failures.len()
    );

    if !failures.is_empty() {
        bail!("{} object(s) failed to delete: {}", failures.len(), failures.join(", "));
    }

    Ok(())
}
