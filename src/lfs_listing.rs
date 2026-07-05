use std::collections::HashMap;

use anyhow::{Context, Result};
use reqwest::blocking::Client;
use scraper::{Html, Selector};
use url::Url;

use crate::forgejo::RepoTarget;

/// One row of the `settings/lfs` admin listing: an LFS object's OID and
/// its size in bytes.
#[derive(Debug, Clone)]
pub struct LfsObject {
    pub oid: String,
    pub size: u64,
}

/// Fetches every page of `{owner}/{repo}/settings/lfs` and returns the
/// full set of LFS objects that currently exist on the server for this
/// repo, keyed by OID. There is no JSON API for this (see
/// ai-docs/research-forgejo-lfs-api.md §6) — it's HTML-scraped, which is
/// inherently version-fragile; any row this can't parse is a hard error
/// per CLAUDE.md's fail-loudly rule, not a silently-skipped row.
pub fn enumerate_all(client: &Client, target: &RepoTarget) -> Result<HashMap<String, LfsObject>> {
    let mut objects = HashMap::new();
    let mut page = 1u32;
    let mut total_pages = 1u32;

    loop {
        let url = listing_url(&target.web_base, &target.owner, &target.repo, page)?;
        let response = client
            .get(url.clone())
            .send()
            .with_context(|| format!("request to {url} failed"))?;

        let status = response.status();
        let body = response
            .text()
            .with_context(|| format!("failed to read response body from {url}"))?;

        if !status.is_success() {
            anyhow::bail!(
                "{url} returned {status} — expected an authenticated 200. \
                 Body starts with: {:.200}",
                body
            );
        }

        let (page_objects, page_total) = parse_listing_page(&body)
            .with_context(|| format!("failed to parse the LFS listing HTML from {url}"))?;

        if page == 1 {
            total_pages = page_total;
        }

        for obj in page_objects {
            objects.insert(obj.oid.clone(), obj);
        }

        if page >= total_pages {
            break;
        }
        page += 1;
    }

    Ok(objects)
}

fn listing_url(web_base: &Url, owner: &str, repo: &str, page: u32) -> Result<Url> {
    web_base
        .join(&format!("/{owner}/{repo}/settings/lfs?page={page}"))
        .context("failed to build the LFS listing URL")
}

/// Deletes one LFS object via `POST /{owner}/{repo}/settings/lfs/delete/{oid}`
/// — there is no REST API for this (see ai-docs/research-forgejo-lfs-api.md
/// §4); this is the same internal web route the repo settings UI uses, and
/// it requires no CSRF token (confirmed live 2026-07-05). An empty-body
/// POST that ends up back on the listing page is success; a redirect to
/// `/user/login` means the session died mid-run; anything else is a hard
/// error rather than a silently-assumed success.
pub fn delete_object(client: &Client, target: &RepoTarget, oid: &str) -> Result<()> {
    let delete_url = target
        .web_base
        .join(&format!(
            "/{}/{}/settings/lfs/delete/{oid}",
            target.owner, target.repo
        ))
        .context("failed to build the LFS delete URL")?;

    let response = client
        .post(delete_url.clone())
        .send()
        .with_context(|| format!("delete request to {delete_url} failed"))?;

    let final_url = response.url().clone();
    let status = response.status();

    if final_url.path().contains("/user/login") {
        anyhow::bail!(
            "delete of {oid} redirected to the login page — the session expired or was rejected \
             mid-run"
        );
    }

    let expected_path = format!("/{}/{}/settings/lfs", target.owner, target.repo);
    if !status.is_success() || final_url.path() != expected_path {
        let body = response.text().unwrap_or_default();
        anyhow::bail!(
            "delete of {oid} did not land back on the LFS listing page as expected \
             (ended at {final_url} with status {status}). Body starts with: {:.200}",
            body
        );
    }

    Ok(())
}

/// Parses one `settings/lfs` HTML page into its LFS object rows and the
/// total page count (the highest `page=N` value linked anywhere in the
/// pagination widget; defaults to 1 if no pagination links are present,
/// i.e. everything fits on one page).
///
/// Selectors target `table#lfs-files-table` rows, reading the OID from
/// the `title` attribute of the object's `.tw-font-mono` link (the
/// visible text is truncated to 10 hex chars) and the exact byte size
/// from the `size=` query parameter of the row's "Find commits" link
/// (rather than parsing the human-readable "285 KiB" text, which is
/// rounded and unit-suffixed). Confirmed against real captured HTML from
/// codeberg.org on 2026-07-05 — see ai-docs/plan.md Phase 4.
fn parse_listing_page(html: &str) -> Result<(Vec<LfsObject>, u32)> {
    let document = Html::parse_document(html);
    let row_sel = Selector::parse("table#lfs-files-table tbody tr").expect("valid selector");
    let oid_link_sel = Selector::parse("a.tw-font-mono").expect("valid selector");
    let find_link_sel =
        Selector::parse("a[href*='/settings/lfs/find']").expect("valid selector");
    let page_link_sel = Selector::parse("a[href*='page=']").expect("valid selector");

    let mut objects = Vec::new();
    for row in document.select(&row_sel) {
        let oid = row
            .select(&oid_link_sel)
            .next()
            .and_then(|el| el.value().attr("title"))
            .context(
                "an LFS listing row is missing its OID link/title — the page layout may have \
                 changed since this scraper was written",
            )?
            .to_string();

        let find_href = row
            .select(&find_link_sel)
            .next()
            .and_then(|el| el.value().attr("href"))
            .context(
                "an LFS listing row is missing its 'Find commits' link — the page layout may \
                 have changed since this scraper was written",
            )?;

        let size: u64 = query_param(find_href, "size")
            .with_context(|| format!("'Find commits' link has no size= param: {find_href}"))?
            .parse()
            .with_context(|| format!("size= param in '{find_href}' is not a valid number"))?;

        objects.push(LfsObject { oid, size });
    }

    let total_pages = document
        .select(&page_link_sel)
        .filter_map(|el| el.value().attr("href"))
        .filter_map(|href| query_param(href, "page"))
        .filter_map(|page| page.parse::<u32>().ok())
        .max()
        .unwrap_or(1);

    Ok((objects, total_pages))
}

/// Reads a single query parameter out of a relative href (`/path?a=1&b=2`)
/// without needing to resolve it against a base URL first.
fn query_param<'a>(href: &'a str, key: &str) -> Option<&'a str> {
    let query = href.split('?').nth(1)?;
    query.split('&').find_map(|pair| {
        let (k, v) = pair.split_once('=')?;
        (k == key).then_some(v)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_ROW: &str = r#"
        <table id="lfs-files-table"><tbody>
            <tr>
                <td><a href="/o/r/settings/lfs/show/deadbeef" title="deadbeefcafe0123456789" class="ui brown button tw-font-mono">deadbeefca</a></td>
                <td>33 KiB</td>
                <td>2026-07-04</td>
                <td><a class="ui primary button" href="/o/r/settings/lfs/find?oid=deadbeefcafe0123456789&size=33955">Find commits</a></td>
            </tr>
        </tbody></table>
    "#;

    #[test]
    fn parses_oid_and_exact_byte_size_from_row() {
        let (objects, total_pages) = parse_listing_page(SAMPLE_ROW).unwrap();
        assert_eq!(objects.len(), 1);
        assert_eq!(objects[0].oid, "deadbeefcafe0123456789");
        assert_eq!(objects[0].size, 33955);
        assert_eq!(total_pages, 1); // no pagination links present
    }

    #[test]
    fn defaults_to_one_page_with_no_pagination_widget() {
        let (_, total_pages) = parse_listing_page("<table id=\"lfs-files-table\"><tbody></tbody></table>").unwrap();
        assert_eq!(total_pages, 1);
    }

    #[test]
    fn finds_max_page_from_pagination_links() {
        let html = format!(
            "{SAMPLE_ROW}<div class=\"pagination\">\
             <a href=\"?page=2\">2</a><a href=\"?page=5\">5</a><a href=\"?page=3\">3</a></div>"
        );
        let (_, total_pages) = parse_listing_page(&html).unwrap();
        assert_eq!(total_pages, 5);
    }

    #[test]
    fn errors_loudly_on_missing_oid_rather_than_skipping() {
        let bad_html = r#"
            <table id="lfs-files-table"><tbody>
                <tr><td>no oid link here</td></tr>
            </tbody></table>
        "#;
        assert!(parse_listing_page(bad_html).is_err());
    }
}
