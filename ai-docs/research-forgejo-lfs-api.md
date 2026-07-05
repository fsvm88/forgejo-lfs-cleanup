---
title: Forgejo/Gitea LFS API research
status: fully confirmed live end-to-end against codeberg.org (list + delete)
last_updated: 2026-07-05
tags: [research, forgejo, gitea, lfs, api]
---

# Forgejo LFS API research

## Question

Can we manage (list / delete) Git LFS objects on a Forgejo repo via HTTP,
given only SSH clone access and an access token?

## Findings

### 1. Forgejo vs Gitea detection

- `GET /api/forgejo/v1/version` exists only on Forgejo. If it 200s, the
  instance is Forgejo. Gitea does not have this route.
- `GET /api/v1/version` exists on both. Pre-7.0 Forgejo returned a
  `+`-suffixed version derived from Gitea's; this is not reliable alone.
- **Decision: use `/api/forgejo/v1/version` as the sole detection check.**
  Treat non-200 as "not Forgejo" and abort (the tool explicitly does not
  support plain Gitea, since the delete mechanism below is web-route based
  and we have only verified/reasoned about it against Forgejo's fork of the
  code).

### 2. The standard Git LFS HTTP batch protocol has no delete operation

The batch API (`POST /{owner}/{repo}.git/info/lfs/objects/batch`) only
supports `download`, `upload`, and (server-side) `verify` operations. There
is no vendor-neutral LFS protocol call to delete an object. Any deletion
must go through Forgejo-specific surface area.

### 3. No REST (`/api/v1`) endpoint exists for LFS object listing or deletion

Reviewed `routers/api/v1/api.go` (upstream Gitea, which Forgejo forks and
which we have not found to diverge on this point): the only LFS-adjacent
`/api/v1` route is:

```
GET /repos/{owner}/{repo}/media/*   -> repo.GetRawFileOrLFS
```

This reads a single file transparently through LFS; it is not an
object-management endpoint. There is no `/api/v1/repos/{owner}/{repo}/lfs/objects`
or similar in either Gitea or Forgejo as of this research (2026-07-04).

### 4. The only object-level delete mechanism is a web (non-API) route

From `routers/web/repo/setting/lfs.go` and its route registration:

| Route | Method | Handler | Auth |
|---|---|---|---|
| `/{owner}/{repo}/settings/lfs` | GET | `LFSFiles` | `reqSignIn`, `reqRepoAdmin` |
| `/{owner}/{repo}/settings/lfs/show/{oid}` | GET | `LFSFileGet` | `reqSignIn`, `reqRepoAdmin` |
| `/{owner}/{repo}/settings/lfs/delete/{oid}` | POST | `LFSDelete` | `reqSignIn`, `reqRepoAdmin` |
| `/{owner}/{repo}/settings/lfs/find` | GET | `LFSFileFind` | `reqSignIn`, `reqRepoAdmin` |
| `/{owner}/{repo}/settings/lfs/pointers` | GET | `LFSPointerFiles` | `reqSignIn`, `reqRepoAdmin` |

**`LFSDelete` semantics** (this is the operation we depend on):

1. Removes the `LFSMetaObject` row associating this OID with this repo.
2. If that was the last repo referencing the OID (LFS storage is
   content-addressed and can be shared across repos/forks), it deletes the
   actual blob from LFS storage.
3. Known upstream FIXME: no lock is held between the reference-count check
   and the content delete, so there is a theoretical race with a concurrent
   push that re-references the same OID. We inherit this race — see
   "Known limitations" in the architecture doc.

**Required privilege: repo-admin on the target repository.** This is
*not* instance-wide site-admin — `reqRepoAdmin` checks permissions scoped
to that repo. A token/account with admin rights on the one repo being
cleaned is sufficient in principle.

### 5. CONFIRMED LIVE (2026-07-05): the web route does NOT accept token auth

Validated against production `codeberg.org` (a real Forgejo instance),
using a real account's API token (`example-org`, admin on the target repo) and
a real target repo (`example-org/example-repo`):

```
GET https://codeberg.org/example-org/example-repo/settings/lfs?page=1
```

tried with four different auth schemes:

| Auth scheme | Result |
|---|---|
| `Authorization: token <token>` | `303 See Other` → `Location: /user/login` |
| `Authorization: Bearer <token>` | `303 See Other` → `/user/login` |
| HTTP Basic auth (`user:token` as password) | `303 See Other` → `/user/login` |
| `?token=<token>` query param | `303 See Other` → `/user/login` |

**All four fail identically.** The `settings/lfs` web routes (and by
extension, almost certainly `settings/lfs/delete/{oid}`, which shares the
same `reqSignIn`+`reqRepoAdmin` middleware chain) only accept an
authenticated **session cookie**, not any API-token scheme. This
resolves the open question from source review: path (b) in the old
draft of this section is the *only* path — there is no token fallback to
"detect and skip."

**Consequence:** to enumerate or delete LFS objects, the tool needs a
real authenticated browser-equivalent session — an API access token is
insufficient no matter how it's presented.

### 5b. CONFIRMED LIVE (2026-07-05): full session cookie set required, plus browser-like headers — and the delete POST itself works, no CSRF token needed

The initial session-cookie-only attempt (copying just `gitea_incredible`
and `persistent` out of Firefox's `cookies.sqlite`) **also failed** with
the same `303 → /user/login`, which briefly suggested a load-balancer
session-affinity problem (Codeberg runs multiple named backend nodes,
observed via the `x-server-name` response header: `s_forgejo_main`,
`s_forgejo_secondary_alpspitz`, `s_forgejo_secondary_aburayama`) — but
that hypothesis was disproven (the same cookie failed even when a request
landed on `s_forgejo_main`, the node presumed to hold the session).

The actual missing pieces, found by capturing a real browser request
(devtools copy-as-curl) rather than reading the cookie DB in isolation:

- **`x-robot-challenge=passed`** — a Codeberg/Forgejo anti-bot cookie
  (their own lightweight bot-check, separate from the session). Without
  it, every request is treated as unauthenticated regardless of session
  cookie validity.
- **`cb_sessionid`** — a second, Codeberg-specific session-tracking
  cookie alongside Forgejo's own `gitea_incredible` session cookie.
- **`lang`** — not required for auth, but sent by real browsers; included
  for parity.

None of these three showed up in a plain `SELECT ... FROM moz_cookies`
query scoped to `host LIKE '%codeberg.org%'` in earlier attempts — worth
double-checking why (possibly a host-matching/subdomain quirk in the
query, or they were session-only cookies not yet flushed to disk in the
copy taken) before relying on a DB-read approach again; the working
extraction path ended up being "copy the real request out of browser
devtools," not "read the cookie database and reconstruct the request."

**With the full cookie set and browser-like headers (`User-Agent`,
`Referer`, `Origin`, `Sec-Fetch-*`), both the GET listing and the POST
delete succeeded**, against real production data:

- `GET /example-org/example-repo/settings/lfs?page=N` → `200`, correct
  HTML, real OID rows.
- The target OID `509fd48a70496f4bb9191f7acca0119054dff4bed16ef05486820a09e565d78c`
  was located on page 37 of 37 (33 KiB, uploaded 2026-07-04 15:11:16).
- The delete confirmation modal's `<form>` (`action=".../settings/lfs/delete/{oid}"`,
  `method="post"`) has **no hidden `_csrf` input field** — this
  Forgejo/Codeberg version does not require a per-form CSRF token for
  this route (consistent with an upstream FIXME noting CSRF token
  handling was reworked/dropped in some Gitea/Forgejo versions — likely
  relying on `SameSite=Lax` cookies + `Sec-Fetch-Site`/`Origin` header
  checks instead of an explicit token).
- `POST /example-org/example-repo/settings/lfs/delete/{oid}` with just the
  cookie set (empty body, no CSRF field) → `303 → /example-org/example-repo/settings/lfs`
  (the real success redirect, distinct from the `/user/login` failure
  redirect).
- Re-fetching the listing (pages 1, 36, 37) afterward confirmed the OID
  is gone from all of them. **Deletion verified end-to-end on live
  production Codeberg.**

This resolves the last open item in this doc: the delete mechanism is
now fully confirmed, not just reasoned about from source. What remains
open is *how a CLI tool acquires this cookie set programmatically*
(interactive login, and possibly solving/passing the anti-bot check) —
see `architecture.md` for the design implication.

### 6. Open question: enumerating "what LFS objects currently exist on the server for this repo"

To delete only *unreferenced* objects, we need the full existing-object
set on the server, not just the referenced set (which we compute locally
from the clone). There is no JSON list endpoint:

- `LFSFiles` (`GET .../settings/lfs`) renders **HTML only**
  (`ctx.HTML(...)`), backed by `git_model.GetLFSMetaObjects()`, and is
  **paginated** at `setting.UI.ExplorePagingNum` items/page (`?page=N`).
- No unpaginated or JSON variant exists in reviewed source.

**Decision: scrape the paginated HTML table** as the enumeration
mechanism, walking `?page=1,2,3...` until a short/empty page. This is
fragile against template changes across Forgejo versions — flagged as a
version-pinned, runtime-validated assumption in the architecture doc, not
a permanent guarantee.

### 7. Server-side alternative that already exists (and why we're not using it)

Forgejo/Gitea ship a server-side `doctor check --run gc-lfs` and a
`[cron.gc_lfs]` job that garbage-collect LFS objects unreferenced by *any*
repo sharing that storage pool. This requires shell/CLI access to the
server (out of scope for an SSH-clone-only tool) and has multiple
documented data-loss incidents where it deleted objects that were still
referenced (forgejo#1113, forgejo#3438, gitea#31271, gitea#36227). This
history is exactly why a careful, single-repo, git-native reachability
computation (rather than relying on server-side GC) is worth building —
it computes the referenced set itself instead of trusting instance-wide
bookkeeping.

## Sources consulted

- forgejo.org/docs (API usage, versions, config cheat sheet)
- github.com/go-gitea/gitea: `routers/api/v1/api.go`, `routers/web/web.go`,
  `routers/web/repo/setting/lfs.go`, `services/context/context.go`
- codeberg.org/forgejo/forgejo issues #1113, #3438, #949
- github.com/go-gitea/gitea issues #7045, #26093, #31271, #36227, #36448

All source review was done via automated fetch/summarization of upstream
files on 2026-07-04 against the `main`/`forgejo` branch tips at that date
— re-verify line numbers/behavior against the actual target instance's
version before relying on them.
