---
title: Implementation plan
status: all phases (0-8) implemented and verified live end-to-end; ready to publish
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

## Phase 5 — Diff, report, guarded delete — DONE 2026-07-05

- `lfs_listing::delete_object()`: `POST /{owner}/{repo}/settings/lfs/delete/{oid}`,
  no CSRF needed (confirmed in Phase 0). Success is landing back on the
  listing page; a redirect to `/user/login` is treated as a hard error
  (session died mid-run) rather than a silent no-op.
- `main.rs`: prints the full stale-object table (OID + exact byte size,
  sorted largest-first) unconditionally, then:
  - no `--execute` → prints "dry run: pass --execute..." and exits
    without touching anything;
  - `--execute` without `--yes` → interactive `Type 'yes' to confirm:`
    prompt, aborts cleanly on anything else;
  - `--execute --yes` → deletes every stale object, continuing past
    individual failures (collected and reported in a summary, non-zero
    exit if any failed) rather than aborting the whole run on the first
    error.
- **Verified live, all paths, against the real `example-org/example-repo`
  repo:**
  - Dry run: correctly reported 416 stale objects (17.7MB) and stopped.
  - `--execute` with the confirmation prompt answered "no": correctly
    aborted, nothing deleted.
  - `--execute --yes`, with the user's explicit go-ahead given the scale
    of the action (416 real objects, ~2.5 minutes): **all 416 deleted, 0
    failures.** Re-running the tool afterward confirmed 0 stale objects
    remain (317 objects exist, all referenced) — the repo is genuinely
    clean.

This completes the tool's core loop end-to-end on real production data,
not just synthetic tests.

## Phase 6 — Polish — DONE 2026-07-05

- `--dry-run` flag added, `conflicts_with = "execute"` (clap-enforced, exit
  code 2 if both are passed). It's a no-op on behavior — dry run was
  already the default — purely so scripts can state intent explicitly and
  get a hard error instead of silent ambiguity if both flags are ever
  passed together. Verified live.
- Exit codes were already correct from how errors propagate through
  `anyhow::Result` — verified live rather than changed: `0` on success
  (including a clean "nothing to delete" dry run), `1` on non-Forgejo
  instance / auth failure / any object failing to delete, `2` on clap
  argument-conflict errors.
- `README.md` added at the project root: requirements (git/git-lfs,
  repo-admin permission, login credentials not an API token and why),
  usage examples for both auth paths, the cookie-file escape hatch and
  when it's needed, an options table, exit codes, and known limitations
  linked back to `architecture.md`.

This closes out the original plan. The tool is feature-complete per the
initial ask: given an SSH URL, detect Forgejo, clone, compute referenced
objects, enumerate existing objects, diff, report, and (opt-in) delete —
all verified live against a real repo, including a full real cleanup run.

## Phase 7 — Automatic Firefox cookie detection — DONE 2026-07-05

Follow-up requested after Phase 6: rather than requiring the user to
manually export a `--cookie-file`, auto-detect an already-logged-in
Firefox session and use it, failing loudly if none is found.

- `src/firefox_cookies.rs`: enumerates Firefox profile directories across
  standard Linux (including Snap/Flatpak), macOS, and Windows locations;
  for each `cookies.sqlite` found, copies it (plus `-wal`/`-shm` sidecars)
  to a temp dir, checkpoints the WAL (handles Firefox holding the file
  open, as validated by hand in Phase 0), and queries for cookies matching
  the target host or its subdomains. Picks whichever profile's matching
  cookies have the most recent `lastAccessed` as the best guess for "the
  profile actually in use."
- This is now the **default auth source** in `main.rs` when neither
  `--cookie-file` nor `--username`/`--password` is given. Precedence:
  `--cookie-file` > `--username`+`--password` > Firefox auto-detect.
- **Fails loudly, not silently**, if no profile has cookies for the
  target domain — names every location checked and suggests the manual
  alternatives, per explicit request.
- **Superseded an earlier finding**: Phase 0 concluded that reading the
  cookie DB directly was unreliable (missed `x-robot-challenge`/
  `cb_sessionid`). Revisited now — those cookies show up fine in a direct
  query once a real browser session has actually cleared the anti-bot
  layer and Firefox has flushed them to disk. The original miss was
  session-timing, not a DB-reading limitation. See `architecture.md`
  "Auth strategy" for the full account.
- **Verified live, both paths**, against `example-org/example-repo`:
  running with zero auth flags correctly found the real profile
  (`~/.config/mozilla/firefox/7pyjgg36.default-release`) and authenticated
  (reported the correct 317 referenced / 0 stale state, post-cleanup);
  pointing `--web-url` at a domain with no matching cookies
  (`example.com`) correctly failed with a clear message and exit code 1.
- 3 new unit tests (exact/subdomain host matching, empty-result handling,
  clear failure when no profiles exist) — 13 total, all passing.

## Phase 8 — Publish readiness — DONE 2026-07-05

Requested: prepare the project to be published as a public git repo (not
crates.io). Ran a security-first audit before any other work.

- **Secret audit**: swept the entire working tree and full `git log -p`
  history for the `forgejo-mcp` API token and every real session-cookie
  value used during live validation. Clean — nothing was ever committed
  (the working cookie file lived only in the scratchpad, outside the
  repo).
- **Genericized personal data**: the real Codeberg account (`example-org`
  was `example-org`) and repo (`example-repo` was `example-repo`) used for
  live validation appeared in doc-comments, prose docs, and — more
  importantly — **test assertions** in `src/forgejo.rs` and
  `src/session.rs`. Replaced everywhere with `example-org`/`example-repo`
  placeholders. Tests re-verified passing after the rename.
- **Fixed a real "works on my machine" bug**: `src/forgejo.rs`'s
  non-Forgejo detection error told the user to see
  `ai-docs/research-forgejo-lfs-api.md §1/§7` — a path that only exists
  inside this repo checkout. Anyone running an installed binary (e.g. via
  `cargo install --git`) would hit a dangling reference. Every other
  `ai-docs`/`CLAUDE.md` mention in the codebase was in a `///` doc
  comment (developer-facing, fine); this was the one in an actual
  `bail!()` user-facing string. Rewrote it to be self-contained.
- **`cargo clippy --all-targets -- -D warnings`**: had never been run.
  Found one real finding (`sort_by` where `sort_by_key(Reverse(...))` is
  idiomatic) — fixed. Clean after.
- **`cargo fmt`**: had never been run either; `cargo build` being
  warning-free is not the same bar. Applied once, all files reformatted,
  re-verified build/test/clippy all still pass after.
- **`Cargo.toml` metadata**: added `description`, `license = "MIT"`,
  `readme`, `keywords`, `categories`, `rust-version = "1.85"` (the
  edition-2024 minimum). Removed `thiserror`, which had been added in
  Phase 1 speculatively and never actually used — `anyhow` covers all
  error handling in this codebase.
- **`LICENSE`**: added, MIT, per user's choice.
- **CI**: added `.forgejo/workflows/ci.yml` (fmt check, clippy, build,
  test) — Forgejo Actions, matching the project's own subject matter and
  the fact that validation happened against Codeberg (a Forgejo Actions
  host).
- Did **not** add `repository`/`homepage` to `Cargo.toml` — no hosting
  URL exists yet since the repo isn't published. Fill in once it has a
  home.

## Explicitly out of scope

- Any support for plain Gitea (see architecture.md "Known limitations").
- Any attempt to lock the repo against concurrent pushes during the
  clone-to-delete window (documented risk, not solved).
- Multi-repo/org-wide batch cleanup (single repo per invocation only, for
  now — could be a later CLI wrapper that loops this tool per repo).
