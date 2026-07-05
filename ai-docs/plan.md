---
title: Implementation plan
status: proposed
last_updated: 2026-07-05
tags: [plan, roadmap]
---

# Plan

## Phase 0 — Validate assumptions against a live instance (blocking)

**Items 1 and 2 are now CONFIRMED — full list+delete flow validated live
end-to-end** on 2026-07-05 against production `codeberg.org`
(`example-org/example-repo`, OID
`509fd48a70496f4bb9191f7acca0119054dff4bed16ef05486820a09e565d78c`, 33 KiB,
found on page 37/37, deleted via `POST .../settings/lfs/delete/{oid}`
with an empty body and no CSRF field, confirmed gone from the listing
afterward). See `ai-docs/research-forgejo-lfs-api.md` §5/§5b and
`architecture.md` "Auth strategy" for full details. **Items 3 and 4 are
now also CONFIRMED** (see Phase 3 and Phase 4 below). Item 5 (how the
tool acquires the cookie jar) is now implemented two ways — scripted
login (unverified live) and `--cookie-file` (verified live) — see
Phase 4.

1. ~~Does `GET .../settings/lfs?page=1` with only an API token work?~~
   **CONFIRMED NO** — login redirect (303) for all 4 token presentations
   tried.
2. ~~Does the delete POST succeed with a real session?~~ **CONFIRMED YES**
   — but it needed more than just the Forgejo session cookie: also
   required a `cb_sessionid` cookie and an `x-robot-challenge=passed`
   anti-bot cookie, neither of which showed up when reading Firefox's
   `cookies.sqlite` directly (the working set was captured from a real
   browser request instead). **No CSRF token was needed** — the delete
   form has no hidden `_csrf` field on this Forgejo version.
3. What does the actual `settings/lfs` HTML table look like (element
   classes/structure) for the target Forgejo version, so the scraper can
   be written precisely instead of speculatively? *(Partially answered —
   we now have real captured HTML for `example-org/example-repo` in the
   scratchpad from validation; still need to write the actual scraper
   against it.)*
4. ~~Does `git lfs ls-files --all --long` work directly against a
   `--bare --mirror` clone?~~ **CONFIRMED YES** — no worktree needed. Also
   confirmed `--all` genuinely walks full history (245 HEAD-only vs. 317
   with `--all` on the same repo), not just each ref's tip tree. See
   Phase 3 below.
5. **New, from validation:** how should the shipped tool itself acquire
   the full cookie jar (session + `cb_sessionid` + anti-bot cookie)?
   A scripted `POST /user/login` may or may not be able to satisfy the
   anti-bot layer without executing JS — untested. Design needs both a
   scripted-login primary path and a `--cookie-file` manual-export escape
   hatch (see architecture.md "Auth strategy" point 3).

Update `architecture.md`'s "Enumerate the server's existing set" section
with confirmed HTML shape once the scraper is written against the
captured page (item 3), and resolve item 4 before finalizing Phase 3.

## Phase 1 — Project scaffold — DONE 2026-07-05

- `cargo init` (edition 2024, rustc/cargo 1.96.0), `ai-docs/` and
  `CLAUDE.md` already in place from planning.
- Dependencies added: `clap` (derive, env), `reqwest` (blocking, cookies,
  json), `scraper`, `serde`/`serde_json`, `anyhow`, `thiserror`.
- `src/checks.rs`: `require_git_and_git_lfs()` runs `git --version` and
  `git-lfs version` at startup, fails loudly with an actionable error if
  either binary is missing — verified by running the built binary with an
  empty PATH.
- `src/cli.rs`: `clap::Parser` struct `Args` with the CLI surface decided
  from the architecture doc's confirmed auth design:
  `ssh_url` (positional), `--web-url`, `--workdir`, `--username`
  (env `FORGEJO_USER`), `--password` (env `FORGEJO_PASS`, hidden from
  `--help`/env dumps), `--cookie-file` (the escape hatch from
  architecture.md), `--execute`, `--yes`. No `--password`-only CLI path
  encouraged in docs (env var preferred, to avoid shell-history/process-list
  leakage) but the flag exists for scripting convenience.
- `src/main.rs` currently just parses args, runs the startup check, and
  prints the parsed args — real orchestration lands in phases 2–5.
- Verified: `cargo build` clean (no warnings), `--help` renders correctly,
  a normal run prints parsed args, and a PATH-stripped run fails with the
  expected git-missing error and non-zero exit.

## Phase 2 — Forgejo detection — DONE 2026-07-05

- `src/forgejo.rs`: `parse_ssh_url()` handles both `ssh://git@host[:port]/owner/repo.git`
  and scp-like `git@host:owner/repo.git` forms, extracting `owner`, `repo`,
  and deriving an `https://<host>` base URL (or using `--web-url` verbatim
  if given). 4 unit tests cover both URL forms, the override, and a
  malformed-path rejection.
- `detect_forgejo()` GETs `/api/forgejo/v1/version` and requires a 2xx +
  parseable `{"version": ...}` body; any other outcome is a hard error
  naming the URL, status, and pointing at the research doc.
- **Verified live against codeberg.org**: both URL forms correctly
  resolved `example-org/example-repo` and printed
  `confirmed Forgejo, version 15.0.0-156-02d7aaa8+gitea-1.22.0`.
- **Verified the negative case against github.com** (`git@github.com:torvalds/linux.git`):
  correctly derived `https://github.com/`, got a 404 on the version route,
  and failed loudly with a clear message rather than proceeding.

## Phase 3 — Clone + referenced-object computation — DONE 2026-07-05

- **Resolved plan.md phase 0 item 4 empirically**: `git lfs ls-files --all
  --long` works directly against a `--bare --mirror` clone, no worktree
  needed. Confirmed `--all` genuinely walks full history, not just each
  ref's tip tree: 245 objects HEAD-only vs. 317 with `--all` on
  `example-org/example-repo`.
- `src/gitrepo.rs`: `clone_bare_mirror()` runs
  `git clone --bare --mirror <ssh_url> <dest>`. If `--workdir` is given,
  clones there and leaves it in place (user-owned, not deleted). If not
  given, clones into a `tempfile::TempDir`, which is removed on drop —
  including on `?`-early-return error paths, since returning `Err` from
  `fn main() -> anyhow::Result<()>` is a normal unwind, not a panic.
  Verified both paths live: default run left no leftover `/tmp` dir after
  exit; `--workdir` run left the bare repo in place afterward.
- `referenced_lfs_oids()` parses `git lfs ls-files --all --long` output
  (first whitespace-separated field per line is the OID) into a
  `HashSet<String>`.
- **Verified live end-to-end** against `example-org/example-repo`: full
  pipeline (detect → clone → compute) ran in ~1.9s and reported 317
  referenced LFS objects, matching the manual `curl`/`git` validation
  done by hand for this phase.

## Phase 4 — Server enumeration + auth — DONE 2026-07-05

- **Resolved plan.md phase 0 item 3** (HTML shape): `src/lfs_listing.rs`
  scrapes `table#lfs-files-table tbody tr`, reading the OID from the
  `title` attribute of the `.tw-font-mono` link (visible text is
  truncated to 10 hex chars) and the **exact byte size** from the `size=`
  query parameter of the row's "Find commits" link — deliberately not
  parsed from the rounded, unit-suffixed "285 KiB" display text. Total
  page count is the max `page=N` found in any pagination link (defaults
  to 1 if no pagination widget is present). 4 unit tests cover row
  parsing, single-page default, multi-page-link max-finding, and
  fail-loudly-on-unexpected-shape.
- **Discovered before implementing scripted login**: an unauthenticated
  `GET /user/login` on codeberg.org sets *no* anti-bot cookie at all —
  strong evidence the `x-robot-challenge` cookie is issued only after a
  client executes a JS challenge, which a plain HTTP client fundamentally
  cannot do. Also confirmed the login form itself has no CSRF field (just
  `user_name`/`password`/`remember`), so no token-carrying is needed
  between the login GET and POST.
- `src/session.rs`: `login()` implements the scripted `POST /user/login`
  path (form-encoded, success detected by checking the final URL moved
  away from `/user/login`; a `two_factor` redirect is called out
  explicitly as unsupported). **This path is unverified live** — no
  throwaway credentials were available to test it, and it's expected to
  fail outright on anti-bot-protected instances like Codeberg for the
  reason above. It remains in the tool for plain self-hosted Forgejo
  instances without such a layer.
- `jar_from_cookie_file()` implements the `--cookie-file` escape hatch
  (Netscape/curl format, `#HttpOnly_`-prefixed lines handled). 2 unit
  tests cover parsing and the empty-file rejection.
- `src/lfs_listing.rs`'s `enumerate_all()` drives the pagination loop:
  fetch page 1, learn the total page count from it, fetch the rest,
  merge all rows into one `HashMap<oid, LfsObject>`.
- **Verified live end-to-end against codeberg.org** using a cookie file
  built from the same working cookie set validated in Phase 0/§5b (not
  a scripted login, since that path is unverifiable without real
  credentials): full pipeline (detect → clone → compute referenced →
  enumerate existing → diff) ran in ~9s and reported 733 objects exist,
  317 referenced, 416 stale (17.7 MB) — internally consistent
  (733 − 416 = 317).

## Phase 5 — Diff, report, guarded delete

- Compute `existing - referenced`.
- Always print a report (count, total bytes, table of OIDs).
- Only delete when `--execute` (or similarly named) flag is passed;
  otherwise exit after reporting (dry-run is default).
- On execute: re-confirm interactively unless `--yes` is also passed
  (for CI/scripted use).
- Delete each stale OID via the web route; treat any non-success response
  as a hard error for that object (log and continue vs. abort — default to
  logging and continuing, summarizing failures at the end, so one bad
  object doesn't block cleanup of the rest).

## Phase 6 — Polish

- `--dry-run` explicit flag as an alias/no-op-confirmation of the default,
  for scripts that want to be explicit.
- Exit codes: 0 clean, non-zero if any deletion failed or instance wasn't
  Forgejo.
- README with usage, and required token privileges (repo-admin on target
  repo — see research doc §4).

## Explicitly out of scope

- Any support for plain Gitea (see architecture.md "Known limitations").
- Any attempt to lock the repo against concurrent pushes during the
  clone-to-delete window (documented risk, not solved).
- Multi-repo/org-wide batch cleanup (single repo per invocation only, for
  now — could be a later CLI wrapper that loops this tool per repo).
