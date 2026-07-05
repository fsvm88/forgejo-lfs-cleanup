use std::process::Command;

use anyhow::{Context, Result, bail};

/// Confirms `git` and `git-lfs` are on PATH and runnable, failing loudly
/// (per CLAUDE.md's fail-loudly rule) rather than letting a missing binary
/// surface later as a confusing clone/ls-files error.
pub fn require_git_and_git_lfs() -> Result<()> {
    require_binary("git", &["--version"])?;
    require_binary("git-lfs", &["version"])?;
    Ok(())
}

fn require_binary(name: &str, version_args: &[&str]) -> Result<()> {
    let output = Command::new(name)
        .args(version_args)
        .output()
        .with_context(|| format!("`{name}` was not found on PATH — install it before running this tool"))?;

    if !output.status.success() {
        bail!(
            "`{name} {}` exited with {}: {}",
            version_args.join(" "),
            output.status,
            String::from_utf8_lossy(&output.stderr)
        );
    }

    Ok(())
}
