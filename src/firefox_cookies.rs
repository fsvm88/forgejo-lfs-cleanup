use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result};
use reqwest::cookie::Jar;
use rusqlite::Connection;
use url::Url;

/// Locates a Firefox profile with cookies for `domain` and builds a
/// [`reqwest::cookie::Jar`] from them, so the tool can use an
/// already-logged-in browser session instead of requiring the user to
/// export a cookie file by hand or attempt a scripted login. Returns the
/// jar plus the profile directory it came from, for a user-visible "using
/// cookies from ..." message.
///
/// This is the automatic fallback when neither `--cookie-file` nor
/// `--username`/`--password` is given — see main.rs. If no profile has
/// any cookies for `domain` at all (Firefox isn't installed, no profile
/// exists, or the user simply isn't logged into that site), this fails
/// loudly with a message naming every location it checked, rather than
/// silently proceeding unauthenticated.
///
/// Firefox may be running and holding `cookies.sqlite` open; each
/// candidate database (plus its `-wal`/`-shm` files, if present) is
/// copied to a temp dir and checkpointed before querying, exactly as
/// validated by hand in ai-docs/research-forgejo-lfs-api.md §5b.
pub fn jar_from_firefox_profile(domain: &str) -> Result<(Arc<Jar>, PathBuf)> {
    let roots = candidate_roots();
    let mut checked = Vec::new();
    let mut best: Option<(PathBuf, Vec<CookieRow>, i64)> = None;

    for root in &roots {
        if !root.is_dir() {
            continue;
        }
        checked.push(root.display().to_string());

        let Ok(entries) = fs::read_dir(root) else {
            continue;
        };
        for entry in entries.flatten() {
            let profile_dir = entry.path();
            let cookies_db = profile_dir.join("cookies.sqlite");
            if !cookies_db.is_file() {
                continue;
            }

            if let Ok(rows) = read_cookies_for_domain(&cookies_db, domain) {
                if rows.is_empty() {
                    continue;
                }
                let max_last_accessed = rows.iter().map(|r| r.last_accessed).max().unwrap_or(0);
                let is_better = best
                    .as_ref()
                    .is_none_or(|(_, _, prev_max)| max_last_accessed > *prev_max);
                if is_better {
                    best = Some((profile_dir, rows, max_last_accessed));
                }
            }
        }
    }

    let (profile_dir, rows, _) = best.ok_or_else(|| {
        anyhow::anyhow!(
            "no Firefox cookies found for '{domain}'. Checked: {}. Log into {domain} in \
             Firefox first, or pass --cookie-file / --username+--password explicitly.",
            if checked.is_empty() {
                "no Firefox profile directories found on this machine".to_string()
            } else {
                checked.join(", ")
            }
        )
    })?;

    let jar = Jar::default();
    for row in &rows {
        let url = Url::parse(&format!("https://{domain}{}", row.path))
            .with_context(|| format!("couldn't build a URL for cookie path '{}'", row.path))?;
        jar.add_cookie_str(
            &format!("{}={}; Path={}", row.name, row.value, row.path),
            &url,
        );
    }

    Ok((Arc::new(jar), profile_dir))
}

struct CookieRow {
    name: String,
    value: String,
    path: String,
    last_accessed: i64,
}

/// Standard Firefox profile-root locations across Linux (including Snap
/// and Flatpak installs), macOS, and Windows. Not exhaustive of every
/// possible install layout, but covers the common cases; unmatched
/// installs fall back to `--cookie-file`.
fn candidate_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();

    if let Some(home) = std::env::var_os("HOME").map(PathBuf::from) {
        roots.push(home.join(".mozilla/firefox"));
        roots.push(home.join(".config/mozilla/firefox"));
        roots.push(home.join(".var/app/org.mozilla.firefox/.mozilla/firefox"));
        roots.push(home.join("snap/firefox/common/.mozilla/firefox"));
        roots.push(home.join("Library/Application Support/Firefox")); // macOS
    }
    if let Some(appdata) = std::env::var_os("APPDATA") {
        roots.push(PathBuf::from(appdata).join("Mozilla/Firefox"));
    }

    roots
}

/// Copies `cookies_db` (and any `-wal`/`-shm` sidecar files) to a temp
/// dir, checkpoints the WAL so in-flight writes are visible, and returns
/// every cookie row matching `domain` (exact host, leading-dot host, or
/// subdomain).
fn read_cookies_for_domain(cookies_db: &Path, domain: &str) -> Result<Vec<CookieRow>> {
    let tmp = tempfile::tempdir().context("failed to create a temp dir for the cookie DB copy")?;
    let dest = tmp.path().join("cookies.sqlite");
    fs::copy(cookies_db, &dest)
        .with_context(|| format!("failed to copy {}", cookies_db.display()))?;

    for suffix in ["-wal", "-shm"] {
        let src = PathBuf::from(format!("{}{suffix}", cookies_db.display()));
        if src.is_file() {
            let dest_sidecar = PathBuf::from(format!("{}{suffix}", dest.display()));
            fs::copy(&src, &dest_sidecar).ok(); // best-effort; missing sidecar is normal
        }
    }

    let conn = Connection::open(&dest).with_context(|| {
        format!(
            "failed to open copied cookie DB from {}",
            cookies_db.display()
        )
    })?;
    conn.execute_batch("PRAGMA wal_checkpoint(FULL);").ok(); // best-effort if no WAL present

    let mut stmt = conn.prepare(
        "SELECT name, value, path, lastAccessed FROM moz_cookies \
         WHERE host = ?1 OR host = '.' || ?1 OR host LIKE ('%.' || ?1)",
    )?;
    let rows = stmt
        .query_map([domain], |row| {
            Ok(CookieRow {
                name: row.get(0)?,
                value: row.get(1)?,
                path: row.get(2)?,
                last_accessed: row.get(3)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()
        .context("failed to read rows from moz_cookies")?;

    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_test_db(path: &Path, host: &str, name: &str, value: &str) {
        let conn = Connection::open(path).unwrap();
        conn.execute_batch(
            "CREATE TABLE moz_cookies (
                id INTEGER PRIMARY KEY,
                host TEXT, name TEXT, value TEXT, path TEXT, lastAccessed INTEGER
            );",
        )
        .unwrap();
        conn.execute(
            "INSERT INTO moz_cookies (host, name, value, path, lastAccessed) VALUES (?1, ?2, ?3, '/', 1000)",
            rusqlite::params![host, name, value],
        )
        .unwrap();
    }

    #[test]
    fn reads_exact_and_subdomain_matches() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("cookies.sqlite");
        let conn = Connection::open(&db_path).unwrap();
        conn.execute_batch(
            "CREATE TABLE moz_cookies (
                id INTEGER PRIMARY KEY,
                host TEXT, name TEXT, value TEXT, path TEXT, lastAccessed INTEGER
            );
             INSERT INTO moz_cookies (host, name, value, path, lastAccessed) VALUES ('codeberg.org', 'a', '1', '/', 100);
             INSERT INTO moz_cookies (host, name, value, path, lastAccessed) VALUES ('sub.codeberg.org', 'b', '2', '/', 200);
             INSERT INTO moz_cookies (host, name, value, path, lastAccessed) VALUES ('other.example', 'c', '3', '/', 300);",
        )
        .unwrap();
        drop(conn);

        let rows = read_cookies_for_domain(&db_path, "codeberg.org").unwrap();
        let names: Vec<_> = rows.iter().map(|r| r.name.as_str()).collect();
        assert!(names.contains(&"a"));
        assert!(names.contains(&"b"));
        assert!(!names.contains(&"c"));
    }

    #[test]
    fn returns_empty_for_unknown_domain() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("cookies.sqlite");
        make_test_db(&db_path, "codeberg.org", "a", "1");

        let rows = read_cookies_for_domain(&db_path, "example.com").unwrap();
        assert!(rows.is_empty());
    }

    #[test]
    fn errors_clearly_when_no_profiles_have_cookies() {
        // With HOME pointing somewhere with no Firefox profiles at all,
        // this should fail with a message rather than silently produce
        // an empty jar.
        // SAFETY: single-threaded test process, no concurrent env access.
        unsafe {
            std::env::set_var("HOME", tempfile::tempdir().unwrap().path());
        }
        let result = jar_from_firefox_profile("example.invalid");
        assert!(result.is_err());
    }
}
