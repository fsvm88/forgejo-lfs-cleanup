use std::fs;
use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, Result, bail};
use reqwest::blocking::Client;
use reqwest::cookie::Jar;
use url::Url;

/// A user-agent string sent on every request. The anti-bot layer observed
/// on Codeberg (see ai-docs/research-forgejo-lfs-api.md §5b) rejects
/// requests lacking browser-like framing; a real UA is cheap insurance
/// even where it turns out not to be load-bearing.
pub const USER_AGENT: &str =
    "Mozilla/5.0 (X11; Linux x86_64; rv:152.0) Gecko/20100101 Firefox/152.0";

/// Parses a Netscape/curl-format cookie jar file (`domain, includeSubdomains,
/// path, secure, expiry, name, value`, tab-separated; lines prefixed
/// `#HttpOnly_` are httpOnly cookies; other `#`-prefixed and blank lines are
/// skipped) into a [`reqwest::cookie::Jar`].
///
/// This is the escape hatch for instances/accounts where a scripted login
/// can't clear an anti-bot layer or 2FA — see architecture.md "Auth
/// strategy". The httpOnly flag is not enforced here; it only matters to
/// JS-based cookie access, not to us replaying cookies over HTTP.
pub fn jar_from_cookie_file(path: &Path) -> Result<Arc<Jar>> {
    let contents = fs::read_to_string(path)
        .with_context(|| format!("failed to read cookie file {}", path.display()))?;

    let jar = Jar::default();
    let mut count = 0usize;

    for (lineno, raw_line) in contents.lines().enumerate() {
        let line = raw_line.strip_prefix("#HttpOnly_").unwrap_or(raw_line);
        if line.is_empty() || (raw_line.starts_with('#') && !raw_line.starts_with("#HttpOnly_")) {
            continue;
        }

        let fields: Vec<&str> = line.split('\t').collect();
        if fields.len() != 7 {
            continue; // not a cookie line (e.g. stray blank/malformed line)
        }
        let [domain, _include_subdomains, cookie_path, _secure, _expiry, name, value] =
            fields.as_slice()
        else {
            unreachable!("checked len == 7 above")
        };

        let host = domain.trim_start_matches('.');
        let url = Url::parse(&format!("https://{host}{cookie_path}")).with_context(|| {
            format!(
                "cookie file {}:{}: couldn't build a URL from domain '{domain}' path '{cookie_path}'",
                path.display(),
                lineno + 1
            )
        })?;

        jar.add_cookie_str(&format!("{name}={value}; Path={cookie_path}"), &url);
        count += 1;
    }

    if count == 0 {
        bail!(
            "no cookies found in {} — is it a real Netscape-format cookie jar?",
            path.display()
        );
    }

    Ok(Arc::new(jar))
}

/// Performs a scripted form login (`POST /user/login`) against `client`,
/// which must already be built with a cookie jar (`cookie_store(true)` or
/// `.cookie_provider(...)`) so the resulting session cookies are retained
/// for subsequent requests. The login form itself carries no CSRF token
/// (confirmed live 2026-07-05 — see ai-docs/research-forgejo-lfs-api.md
/// §5b), so this is a plain form POST.
///
/// **Caveat, unverified live:** this has not been exercised against a
/// real account, because doing so needs real credentials and the point of
/// this tool's design is to avoid asking for those casually (see
/// ai-docs/architecture.md "Auth strategy"). It's also expected to
/// **fail outright on anti-bot-protected instances like Codeberg**: a
/// plain unauthenticated `GET /user/login` there sets no anti-bot cookie
/// at all (confirmed live), meaning that cookie is very likely issued
/// only after a JS-executed challenge that this HTTP-only client can
/// never run. On such instances, use `--cookie-file` instead. This path
/// exists for self-hosted Forgejo instances that don't have that layer.
pub fn login(client: &Client, web_base: &Url, username: &str, password: &str) -> Result<()> {
    let login_url = web_base
        .join("/user/login")
        .context("failed to build the login URL")?;

    let response = client
        .post(login_url.clone())
        .form(&[
            ("user_name", username),
            ("password", password),
            ("remember", "off"),
        ])
        .send()
        .with_context(|| format!("login request to {login_url} failed"))?;

    let final_url = response.url().clone();

    if final_url.path().contains("two_factor") {
        bail!(
            "login for '{username}' requires two-factor authentication, which this tool \
             doesn't yet handle — use --cookie-file instead"
        );
    }

    if final_url.path() == login_url.path() {
        bail!(
            "login for '{username}' did not redirect away from /user/login — likely wrong \
             credentials, or an anti-bot/JS challenge blocked it (common on public instances \
             like Codeberg; use --cookie-file there instead)"
        );
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use reqwest::cookie::CookieStore;

    #[test]
    fn parses_netscape_format_and_httponly_lines() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cookies.txt");
        fs::write(
            &path,
            "# Netscape HTTP Cookie File\n\
             codeberg.org\tFALSE\t/\tTRUE\t1234567890\tlang\ten-US\n\
             #HttpOnly_codeberg.org\tFALSE\t/\tTRUE\t1234567890\tcb_sessionid\tabc123\n\
             \n",
        )
        .unwrap();

        let jar = jar_from_cookie_file(&path).unwrap();
        let url = Url::parse("https://codeberg.org/example-org/example-repo/settings/lfs").unwrap();
        let header = jar.cookies(&url).expect("cookies should be set for this URL");
        let header_str = header.to_str().unwrap();
        assert!(header_str.contains("lang=en-US"));
        assert!(header_str.contains("cb_sessionid=abc123"));
    }

    #[test]
    fn rejects_file_with_no_cookies() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("empty.txt");
        fs::write(&path, "# just a comment\n").unwrap();
        assert!(jar_from_cookie_file(&path).is_err());
    }
}
