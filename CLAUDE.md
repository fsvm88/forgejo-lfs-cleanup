# forgejo-lfs-cleanup

Rust CLI that, given the SSH URL of a Forgejo repo, deletes LFS objects
unreachable from any ref (branch/tag/etc.) in that repo. Never operates on
plain Gitea (Forgejo is detected and required — see
`ai-docs/research-forgejo-lfs-api.md` §1,§7 for why).

## Documentation policy (follow this every session)

- **`ai-docs/`** holds every research finding, architecture decision, and
  plan for this project, as Markdown with YAML frontmatter
  (`title`, `status`, `last_updated`, `tags`). Three files exist so far:
  `research-forgejo-lfs-api.md`, `architecture.md`, `plan.md`. Keep using
  this split (research vs. decisions vs. roadmap) rather than dumping
  everything in one file.
- **Update docs as you go, not after the fact.** When you learn something
  new about the Forgejo API (especially resolving one of the open
  questions in `research-forgejo-lfs-api.md` §5/§6, or phase 0 in
  `plan.md`), write it into the relevant doc's "Findings"/"Changelog"
  section in the same turn you learn it — bump `last_updated`. Don't let
  `architecture.md`'s "Auth strategy" section describe a dual-path
  fallback after it's been empirically confirmed which path is real;
  collapse it.
- Before implementing anything that touches the two open questions in
  `ai-docs/research-forgejo-lfs-api.md` (§5 auth mechanism for the
  settings/lfs web routes, §6 HTML scraping shape), check whether
  `ai-docs/plan.md` phase 0 has been validated against a live instance
  yet. If not, and a live instance is now available, validate first —
  don't build further on unverified assumptions about routes that were
  reasoned about from source review rather than confirmed live.
- This CLAUDE.md itself should be updated if project-wide behavioral
  rules change (e.g. if Gitea support is added, if the delete mechanism
  changes because Forgejo ships a real API, if the auth fallback path
  turns out to be dead code and is removed).

## Design constraints (do not silently relax these)

- **Dry-run by default.** Never delete an LFS object unless the user
  passed an explicit execute flag. This is a data-destructive tool acting
  through an unofficial, version-dependent web route (not a stable API) —
  see `ai-docs/architecture.md` "Dry-run by default" and "Known
  limitations". Always print the full stale-object report before any
  deletion.
- **Fail loudly, not silently, on unexpected server responses.** If the
  HTML enumeration scraper doesn't find the expected table shape, or a
  delete POST returns something other than the expected success response,
  treat it as an error to surface — never silently produce a partial or
  wrong stale-object list.
- **Required privilege is repo-admin on the target repo**, not
  instance-wide site-admin (confirmed via source review of Forgejo's
  `reqRepoAdmin` middleware on the relevant routes). Don't ask users for
  more than that without a documented reason.
- Shell out to the `git`/`git-lfs` CLIs for reachability computation
  (chosen over a Rust git library — see `ai-docs/architecture.md`); check
  both binaries are present at startup with a clear error if missing.

## Known open items (see ai-docs/plan.md Phase 0)

1. ~~Whether the `settings/lfs` web routes accept `Authorization: token`~~
   **RESOLVED 2026-07-05: no token scheme works (tried 4), only a full
   session cookie jar.**
2. ~~Whether the delete POST works with a real session~~ **RESOLVED
   2026-07-05: validated live end-to-end on production codeberg.org**
   (`example-org/example-repo`) — listed, located, deleted, and confirmed
   gone. No CSRF token needed. Required cookies turned out to be more than
   just the Forgejo session cookie: also a Codeberg-specific
   `cb_sessionid` and an anti-bot `x-robot-challenge=passed` cookie,
   neither discoverable by reading Firefox's cookie DB directly — see
   `ai-docs/research-forgejo-lfs-api.md` §5b for exactly what worked.
3. ~~The exact HTML shape of the `settings/lfs` listing page~~ **RESOLVED
   2026-07-05:** `src/lfs_listing.rs` scrapes `table#lfs-files-table`,
   reading OID from the `.tw-font-mono` link's `title` attribute and
   **exact byte size from the `size=` query param** of the "Find commits"
   link (not the rounded "285 KiB" display text). 4 unit tests, plus live
   verification against codeberg.org.
4. ~~Whether `git lfs ls-files --all --long` works on a bare mirror~~
   **RESOLVED 2026-07-05: yes, no worktree needed** (see Phase 3 in
   plan.md).
5. **How the tool acquires the cookie jar — implemented two ways:**
   `--cookie-file` (Netscape format, **verified live end-to-end** against
   codeberg.org) and scripted `--username`/`--password` login (**NOT
   verified live** — no throwaway credentials available, and expected to
   fail outright on anti-bot-protected instances like Codeberg since an
   unauthenticated `GET /user/login` there sets no anti-bot cookie at all,
   implying it requires a JS challenge a plain HTTP client can't run).
   Scripted login remains for plain self-hosted Forgejo without that
   layer — don't remove it, but don't trust it either without a real test.

All of Phase 0's open items are now resolved or implemented as of
2026-07-05. The one live-unverified path is scripted login (item 5) —
treat `session::login()` as provisional until tested against a real
account, and prefer `--cookie-file` when validating anything that
depends on being logged in.
