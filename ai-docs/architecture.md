---
title: Architecture decisions
status: draft
last_updated: 2026-07-04
tags: [architecture, decisions, rust]
---

# Architecture

## Goal

A Rust CLI, given the SSH URL of a Forgejo repo, that deletes LFS objects
which are not reachable from any ref (branch/tag/etc.) in that repo.

## High-level flow

1. **Detect Forgejo.** Derive the HTTPS base URL from the SSH remote (need
   host; port/path mapping may require a `--web-url` override since SSH and
   HTTP hosts/ports can differ — see open question in plan.md). Call
   `GET /api/forgejo/v1/version`. Abort with a clear error if it doesn't
   200 — this tool does not support plain Gitea (see research doc §1,§7).
2. **Bare clone.** `git clone --bare --mirror <ssh-url> <tmpdir>` into a
   scratch directory (respecting `TMPDIR`/explicit `--workdir`). `--mirror`
   ensures all refs (branches, tags, notes, PR refs where present) are
   fetched, not just the default branch — required for correct reachability.
3. **Compute the referenced set.** Shell out to
   `git -C <tmpdir> lfs ls-files --all --long`
   (validated per-repo at runtime — see open question in plan.md about bare
   repo compatibility; fallback is `git rev-list --all --objects` piped
   through `git cat-file --batch` filtering for the LFS pointer blob
   format `version https://git-lfs.github.com/spec/v1` + `oid sha256:...`).
   Produces a `HashSet<Oid>`.
4. **Enumerate the server's existing set.** Authenticate (see below), then
   paginate `GET /{owner}/{repo}/settings/lfs?page=N` scraping OIDs from the
   HTML table until an empty/short page. Produces a second `HashSet<Oid>`.
5. **Diff.** `stale = existing - referenced`.
6. **Report (always).** Print the stale set: OID, size (if scraped), count,
   total bytes.
7. **Delete (only with explicit opt-in).** For each stale OID:
   `POST /{owner}/{repo}/settings/lfs/delete/{oid}`. Requires `--yes`/
   `--execute` flag; **dry-run is the default** with no such flag.

## Key decisions

### Shell out to `git` + `git-lfs` CLI, not a Rust git library

Reuses git-lfs's own reachability logic (`ls-files --all`) instead of
reimplementing ref-walking and LFS pointer-blob parsing by hand. Cost: the
tool requires `git` and `git-lfs` binaries on PATH; check for both at
startup with a clear error if missing.

### Delete via the internal `settings/lfs/delete/{oid}` web route

There is no supported API for this (see research doc §4). We depend on
Forgejo-internal, version-dependent behavior deliberately, because it's the
only mechanism that performs actual, targeted, single-object deletion
without server shell/DB access. We pin the Forgejo versions we've validated
against in this doc's changelog, and the tool **fails loudly** (not
silently) on any unexpected HTTP status or response shape rather than
guessing.

### Auth strategy: session-cookie login only — API tokens do not work

**CONFIRMED LIVE 2026-07-05 against codeberg.org, full flow including the
actual delete** (research doc §5/§5b): `Authorization: token`,
`Authorization: Bearer`, HTTP Basic auth (token as password), and
`?token=` query param were all tried against `GET .../settings/lfs?page=1`
with a valid admin-on-repo token. All four returned `303 → /user/login`.
There is no token-based path into these web routes — **session-cookie
auth is the only path**.

Getting the session cookie alone is *not* sufficient, though. Confirmed
live requirements for a request to be treated as authenticated:

1. **The Forgejo session cookie** (name varies per instance config — was
   `gitea_incredible` on Codeberg; this is a deploy-time `COOKIE_NAME`
   setting, don't hardcode the literal name).
2. **A Codeberg-specific `cb_sessionid` cookie**, issued alongside the
   Forgejo session — this may be Codeberg-specific reverse-proxy/infra
   rather than stock Forgejo; don't assume every Forgejo instance has an
   equivalent, but do assume *some* instances might layer extra
   session-tracking cookies in front of Forgejo and design the cookie jar
   to pass through whatever the login response sets, not just the one
   cookie we know the name of.
3. **An anti-bot cookie** (`x-robot-challenge=passed` on Codeberg) — a
   bot-check layer distinct from the login session. A request with a
   perfectly valid session cookie but missing this was treated as fully
   unauthenticated (redirected to `/user/login`, not to a challenge page),
   which was initially misdiagnosed as a load-balancer session-affinity
   issue before the real cause was found.
4. Browser-like request headers (`User-Agent`, `Referer`, `Origin`,
   `Sec-Fetch-Site: same-origin`, etc.) — not confirmed strictly required
   individually, but were present in the working request and worth
   sending defensively since the anti-bot layer's exact criteria are
   opaque to us.

**No CSRF token is needed for the delete POST** — the delete confirmation
form has no hidden `_csrf` field, and an empty-body POST with just the
right cookies succeeded (`303` to the settings page, not to login).

**Practical implication for the tool:** don't try to reconstruct this
cookie set by reading the browser's cookie database directly (a `SELECT`
scoped to the site's host missed the `x-robot-challenge` and
`cb_sessionid` cookies entirely in testing — possibly a host-matching
quirk, possibly they're issued by JS after a challenge and never
persisted to the same store). Instead, the tool should:

1. Perform its own real login flow: `GET /user/login` (to pick up
   whatever the anti-bot layer requires, if it's satisfiable headlessly)
   → `POST /user/login` with credentials → capture **every** `Set-Cookie`
   from the whole exchange into a cookie jar, not just the ones we can
   name in advance.
2. Treat the anti-bot layer as a real, first-class unknown per target
   instance — it may not exist on self-hosted Forgejo at all (Codeberg
   specifically is a large public target and likely to have more
   anti-scraping infra than a private self-hosted instance), or it may
   require solving a JS challenge that a plain HTTP client can't do,
   in which case the tool needs a documented manual fallback (e.g.
   "export your browser's cookies for this site and pass them via
   `--cookie-file`") rather than silently failing.
3. Support `--cookie-file` (Netscape cookie-jar format, same as
   `curl -b`) as an escape hatch for instances where scripted login can't
   clear the anti-bot layer, alongside the scripted username+password
   login as the primary path.
4. If the account has 2FA, the scripted login must handle the second
   factor (TOTP prompt at minimum) — not yet tested against a 2FA-enabled
   account; surface a clear error rather than failing silently if the
   login response indicates a 2FA challenge the tool doesn't handle yet.
5. Reuse the same cookie jar for both enumeration (GET, paginated) and
   deletion (POST) within one run.

This whole flow — list, locate an OID, delete it, verify it's gone — was
exercised live end-to-end on production Codeberg (`example-org/example-repo`,
OID `509fd48a...565d78c`) by hand with `curl` before writing any Rust, so
the mechanism itself (routes, form shape, response codes, no-CSRF) is
solid; what remains to design is only the *acquisition* of the cookie jar
by the tool itself instead of by hand.

### Dry-run by default

Given (a) this deletes data, (b) the delete mechanism is an unofficial
internal route, and (c) Forgejo's own native LFS GC has multiple
documented data-loss incidents (research doc §7), the tool never deletes
anything unless invoked with an explicit execute flag. Even then, it
prints the full stale-object list with total size before proceeding and
supports a `--confirm` interactive prompt in addition to the flag (flag
alone suffices for scripted/CI use).

## Known limitations

- **Clone-to-delete race.** An object unreferenced at clone time could
  become referenced by a push that happens after enumeration but before
  deletion. We do not hold any lock (none is available to us as an
  external client). Mitigation: keep the window short (don't do anything
  slow between steps 3–7), and document this as a real, accepted risk —
  advise users to run during a quiet window / low-traffic period.
- **HTML scraping is version-fragile.** The enumeration mechanism (research
  doc §6) depends on the shape of the `settings/lfs` HTML table, which is
  not a stable contract. Parse defensively and fail loudly (not silently
  produce a wrong/partial list) if the expected table structure isn't
  found.
- **Shared LFS storage across forks.** Since LFS objects are
  content-addressed and can be shared by other repos/forks pointing at the
  same OID, deleting via `LFSDelete` only actually frees storage when this
  repo was the last referencer — this is handled server-side by Forgejo's
  own reference counting, not something we need to compute, but it means
  "stale for this repo" doesn't necessarily mean "freed disk space
  instance-wide," which is worth surfacing in tool output.
- **Gitea (non-Forgejo) is explicitly unsupported**, even though most of
  the underlying code is shared, because the detection step deliberately
  gates on the Forgejo-specific version route.
- **Anti-bot layers are an unknown per-instance variable.** Codeberg
  requires an `x-robot-challenge=passed` cookie that a plain scripted
  login may or may not be able to obtain without executing JS. Self-hosted
  Forgejo instances likely don't have this, but the tool shouldn't assume
  either way — see "Auth strategy" above for the `--cookie-file` escape
  hatch this motivates.

## Changelog

- 2026-07-04: Initial architecture drafted from source-review research
  (no live Forgejo instance validated yet).
- 2026-07-05: Full list+delete flow validated live end-to-end against
  production codeberg.org (`example-org/example-repo`, one real orphaned
  LFS object). Auth strategy section rewritten from "reasoned dual-path"
  to "confirmed cookie/header requirements + acquisition strategy."
  Discovered and documented the anti-bot cookie requirement and the
  `--cookie-file` escape hatch it motivates.
