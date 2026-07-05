use std::path::PathBuf;

use clap::Parser;

/// Delete LFS objects unreachable from any ref in a Forgejo repo.
#[derive(Parser, Debug)]
#[command(name = "forgejo-lfs-cleanup", version, about)]
pub struct Args {
    /// SSH URL of the repo to clean up, e.g. ssh://git@codeberg.org/owner/repo.git
    pub ssh_url: String,

    /// Override the HTTPS base URL if it can't be derived from the SSH URL
    /// (e.g. differing host/port between SSH and HTTP access).
    #[arg(long)]
    pub web_url: Option<String>,

    /// Directory to clone the bare mirror into (default: a temp dir, removed on exit).
    #[arg(long)]
    pub workdir: Option<PathBuf>,

    /// Forgejo/Codeberg username for the web-session login (an API token is not
    /// sufficient for the LFS admin routes this tool depends on).
    #[arg(long, env = "FORGEJO_USER")]
    pub username: Option<String>,

    /// Forgejo/Codeberg password. Prefer the FORGEJO_PASS env var over this flag
    /// to avoid the password showing up in shell history / process listings.
    #[arg(long, env = "FORGEJO_PASS", hide_env_values = true)]
    pub password: Option<String>,

    /// Path to a Netscape-format cookie jar (e.g. exported from a browser) to use
    /// instead of a scripted login, for instances/accounts where scripted login
    /// can't clear an anti-bot layer or 2FA.
    #[arg(long)]
    pub cookie_file: Option<PathBuf>,

    /// Actually delete stale LFS objects. Without this flag, the tool only reports
    /// what it would delete.
    #[arg(long)]
    pub execute: bool,

    /// Skip the interactive confirmation prompt when --execute is passed.
    #[arg(long)]
    pub yes: bool,
}
