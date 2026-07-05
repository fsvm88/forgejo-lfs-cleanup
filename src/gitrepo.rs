use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};
use tempfile::TempDir;

/// A bare mirror clone of the target repo. If cloned into a temp dir
/// (i.e. `--workdir` was not given), the temp dir is removed when this
/// value is dropped, including on early-return-via-`?` error paths, since
/// dropping a `Result`-returning `main`'s locals is a normal (non-panic)
/// unwind. An explicit `--workdir` is left in place for the caller to
/// inspect/reuse — we don't delete something the user pointed us at.
pub struct BareClone {
    pub path: PathBuf,
    _tempdir_guard: Option<TempDir>,
}

/// Clones `ssh_url` as a bare mirror (`--bare --mirror`, so all
/// branches/tags/etc. are fetched, not just the default branch) into
/// `workdir` if given, or a fresh temp directory otherwise.
pub fn clone_bare_mirror(ssh_url: &str, workdir: Option<&Path>) -> Result<BareClone> {
    let (path, guard) = match workdir {
        Some(dir) => (dir.to_path_buf(), None),
        None => {
            let tempdir = TempDir::with_prefix("forgejo-lfs-cleanup-")
                .context("failed to create a temp directory for the bare clone")?;
            (tempdir.path().to_path_buf(), Some(tempdir))
        }
    };

    let output = Command::new("git")
        .args(["clone", "--bare", "--mirror", ssh_url])
        .arg(&path)
        .output()
        .context("failed to run `git clone`")?;

    if !output.status.success() {
        bail!(
            "`git clone --bare --mirror {ssh_url} {}` failed: {}",
            path.display(),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    Ok(BareClone {
        path,
        _tempdir_guard: guard,
    })
}

/// Computes the set of LFS object OIDs referenced from any ref (branch,
/// tag, etc.) in the clone's full history, via `git lfs ls-files --all
/// --long`. Confirmed empirically to work directly against a
/// `--bare --mirror` clone with no worktree needed (see
/// ai-docs/plan.md Phase 3) — `--all` walks full history, not just each
/// ref's tip tree (245 HEAD-only vs. 317 with `--all` in the repo tested
/// against).
pub fn referenced_lfs_oids(clone: &BareClone) -> Result<HashSet<String>> {
    let output = Command::new("git")
        .args(["lfs", "ls-files", "--all", "--long"])
        .current_dir(&clone.path)
        .output()
        .context("failed to run `git lfs ls-files`")?;

    if !output.status.success() {
        bail!(
            "`git lfs ls-files --all --long` failed in {}: {}",
            clone.path.display(),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    let stdout = String::from_utf8(output.stdout)
        .context("`git lfs ls-files` output was not valid UTF-8")?;

    // Each line is "<oid> <marker> <path>"; the OID is the first
    // whitespace-separated field.
    let oids = stdout
        .lines()
        .filter_map(|line| line.split_whitespace().next())
        .map(str::to_string)
        .collect();

    Ok(oids)
}
