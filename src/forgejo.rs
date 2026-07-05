use anyhow::{Context, Result, bail};
use serde::Deserialize;
use url::Url;

#[derive(Debug, Deserialize)]
struct ForgejoVersionResponse {
    version: String,
}

/// Confirms the instance at `web_base` is Forgejo (not plain Gitea or
/// something else entirely) via the Forgejo-only `/api/forgejo/v1/version`
/// route — see ai-docs/research-forgejo-lfs-api.md §1. Returns the
/// reported version string on success.
pub fn detect_forgejo(client: &reqwest::blocking::Client, web_base: &Url) -> Result<String> {
    let url = web_base
        .join("/api/forgejo/v1/version")
        .context("failed to build the version-check URL")?;

    let response = client
        .get(url.clone())
        .send()
        .with_context(|| format!("request to {url} failed — is the host reachable?"))?;

    let status = response.status();
    if !status.is_success() {
        bail!(
            "{url} returned {status} — this instance does not look like Forgejo. This tool \
             only supports Forgejo, not plain Gitea or other forges, because it depends on a \
             Forgejo-specific web route to delete LFS objects (there's no such route in Gitea)."
        );
    }

    let body: ForgejoVersionResponse = response.json().context(
        "/api/forgejo/v1/version returned a 2xx but the body wasn't the expected JSON shape",
    )?;

    Ok(body.version)
}

/// The repo this run targets, plus the HTTPS base URL used for all
/// Forgejo web/API requests (which may differ in host/port from the SSH
/// remote — see `--web-url`).
#[derive(Debug, Clone)]
pub struct RepoTarget {
    pub web_base: Url,
    pub owner: String,
    pub repo: String,
}

/// Parses an SSH repo URL (either `ssh://git@host[:port]/owner/repo.git`
/// or the scp-like `git@host:owner/repo.git` form) into a [`RepoTarget`].
/// `web_url_override` takes priority over host derivation entirely, for
/// instances where the HTTPS host/port genuinely differs from the SSH one.
pub fn parse_ssh_url(ssh_url: &str, web_url_override: Option<&str>) -> Result<RepoTarget> {
    let (host, path) = split_ssh_url(ssh_url)
        .with_context(|| format!("could not parse '{ssh_url}' as an SSH git URL"))?;

    let (owner, repo) = split_owner_repo(&path).with_context(|| {
        format!("could not extract owner/repo from path '{path}' in '{ssh_url}'")
    })?;

    let web_base = match web_url_override {
        Some(explicit) => Url::parse(explicit)
            .with_context(|| format!("'{explicit}' is not a valid --web-url"))?,
        None => Url::parse(&format!("https://{host}"))
            .with_context(|| format!("derived host '{host}' is not a valid URL host"))?,
    };

    Ok(RepoTarget {
        web_base,
        owner,
        repo,
    })
}

/// Returns (host, path) for either URL form. The scp-like form
/// (`user@host:path`) is not a URL the `url` crate understands, so it's
/// tried first via a plain split; a proper `ssh://` URL is tried second.
fn split_ssh_url(ssh_url: &str) -> Result<(String, String)> {
    if ssh_url.starts_with("ssh://") {
        let url = Url::parse(ssh_url).context("invalid ssh:// URL")?;
        let host = url
            .host_str()
            .context("ssh:// URL has no host")?
            .to_string();
        let path = url.path().trim_start_matches('/').to_string();
        return Ok((host, path));
    }

    // scp-like: [user@]host:path
    let without_user = ssh_url.split('@').next_back().unwrap_or(ssh_url);
    let (host, path) = without_user
        .split_once(':')
        .with_context(|| format!("'{ssh_url}' is neither an ssh:// URL nor user@host:path"))?;

    if host.is_empty() || path.is_empty() {
        bail!("'{ssh_url}' is missing a host or path");
    }

    Ok((host.to_string(), path.to_string()))
}

fn split_owner_repo(path: &str) -> Result<(String, String)> {
    let trimmed = path.trim_start_matches('/').trim_end_matches(".git");
    let mut segments = trimmed.splitn(2, '/');
    let owner = segments.next().filter(|s| !s.is_empty());
    let repo = segments.next().filter(|s| !s.is_empty());

    match (owner, repo) {
        (Some(owner), Some(repo)) => Ok((owner.to_string(), repo.to_string())),
        _ => bail!("expected an 'owner/repo' path, got '{path}'"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_ssh_scheme_url() {
        let target =
            parse_ssh_url("ssh://git@codeberg.org/example-org/example-repo.git", None).unwrap();
        assert_eq!(target.web_base.as_str(), "https://codeberg.org/");
        assert_eq!(target.owner, "example-org");
        assert_eq!(target.repo, "example-repo");
    }

    #[test]
    fn parses_scp_like_url() {
        let target = parse_ssh_url("git@codeberg.org:example-org/example-repo.git", None).unwrap();
        assert_eq!(target.web_base.as_str(), "https://codeberg.org/");
        assert_eq!(target.owner, "example-org");
        assert_eq!(target.repo, "example-repo");
    }

    #[test]
    fn respects_web_url_override() {
        let target = parse_ssh_url(
            "git@internal-git.example:owner/repo.git",
            Some("https://forgejo.example.com:3000"),
        )
        .unwrap();
        assert_eq!(
            target.web_base.as_str(),
            "https://forgejo.example.com:3000/"
        );
        assert_eq!(target.owner, "owner");
        assert_eq!(target.repo, "repo");
    }

    #[test]
    fn rejects_missing_repo_path() {
        assert!(parse_ssh_url("git@codeberg.org:example-org", None).is_err());
    }
}
