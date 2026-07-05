# forgejo-lfs-cleanup

Deletes Git LFS objects that are no longer reachable from any ref (branch,
tag, etc.) in a [Forgejo](https://forgejo.org/) repository.

It works by cloning the repo as a bare mirror, computing every LFS object
still referenced anywhere in its full history, comparing that against
every LFS object that actually exists on the server, and deleting the
difference. Dry-run is the default — nothing is ever deleted unless you
pass `--execute`.

**Forgejo only, not plain Gitea.** The tool checks `/api/forgejo/v1/version`
on startup and refuses to continue if that's not present. See
[`ai-docs/research-forgejo-lfs-api.md`](ai-docs/research-forgejo-lfs-api.md)
for why.

## Requirements

- `git` and `git-lfs` installed and on `PATH`.
- SSH access to the repo (the same access `git clone` would need).
- An account with **repo-admin** permission on the target repo — not
  instance-wide site-admin. This is a delete operation, so treat it with
  the same care as any other admin action.
- **Login credentials, not an API token.** There is no REST API for
  listing or deleting LFS objects; this tool drives the same internal web
  routes the repo settings UI uses (`/{owner}/{repo}/settings/lfs`), which
  only accept an authenticated session — a personal access token is not
  sufficient no matter how it's presented. See
  [`ai-docs/research-forgejo-lfs-api.md`](ai-docs/research-forgejo-lfs-api.md) §5
  for how this was confirmed. You need one of, tried in this order:
  1. `--cookie-file`, a Netscape/curl-format cookie jar, if given.
  2. `--username`/`--password` (or `FORGEJO_USER`/`FORGEJO_PASS`) for a
     scripted login, if given.
  3. **Otherwise, automatically**: an already-logged-in Firefox session
     for the target site, auto-detected from your local Firefox profile.
     If none is found, the tool stops and tells you so — it never
     proceeds unauthenticated.

## Usage

The simplest form relies on auto-detecting your logged-in Firefox
session — if you're logged into the instance in Firefox on this machine,
this just works:

```sh
# Report what would be deleted, without deleting anything (the default):
forgejo-lfs-cleanup ssh://git@codeberg.org/owner/repo.git

# Actually delete the stale objects, with an interactive confirmation prompt:
forgejo-lfs-cleanup ssh://git@codeberg.org/owner/repo.git --execute

# Same, but skip the prompt (for scripts/CI):
forgejo-lfs-cleanup ssh://git@codeberg.org/owner/repo.git --execute --yes
```

Or supply credentials explicitly, e.g. for scripted/CI use or a different
browser/machine than the one you're logged in on:

```sh
# Scripted login (best supplied via env vars, to keep them out of shell
# history and process listings):
export FORGEJO_USER=you
export FORGEJO_PASS='...'
forgejo-lfs-cleanup ssh://git@codeberg.org/owner/repo.git

# Or a cookie file exported from any browser:
forgejo-lfs-cleanup ssh://git@codeberg.org/owner/repo.git --cookie-file cookies.txt
```

### Automatic Firefox cookie detection

If neither `--cookie-file` nor `--username`/`--password` is given, the
tool looks for a Firefox profile on this machine with cookies for the
target site (checking standard Linux, Snap, Flatpak, macOS, and Windows
profile locations) and uses whichever one looks most recently active.
Firefox can be running at the time — its cookie database is copied and
checkpointed rather than read in place. If no profile has cookies for the
site, the tool stops with an error naming every location it checked
rather than proceeding unauthenticated.

### Getting a cookie file

For browsers other than Firefox, or when auto-detection can't find what
it needs: on instances with an anti-bot layer in front of the login page
(Codeberg confirmed to have one — see
[`ai-docs/research-forgejo-lfs-api.md`](ai-docs/research-forgejo-lfs-api.md) §5b),
a scripted login cannot complete, because the anti-bot cookie is only
issued after a JS challenge a plain HTTP client can't run. Export your
browser's cookies for the site instead (e.g. a "cookies.txt" browser
extension, or `yt-dlp --cookies-from-browser` style tooling that writes
the standard Netscape format) and pass the file via `--cookie-file`.

### All options

| Flag | Purpose |
|---|---|
| `<SSH_URL>` | SSH URL of the repo to clean up (required, positional) |
| `--web-url` | Override the derived HTTPS base URL, if it differs from the SSH host/port |
| `--workdir` | Clone the bare mirror here instead of a temp dir (left in place afterward, not auto-deleted) |
| `--username` / `FORGEJO_USER` | Username for scripted login |
| `--password` / `FORGEJO_PASS` | Password for scripted login (prefer the env var) |
| `--cookie-file` | Netscape-format cookie jar, used instead of scripted login |
| `--execute` | Actually delete stale objects (default is report-only) |
| `--dry-run` | Explicitly state you want a dry run; conflicts with `--execute` |
| `--yes` | Skip the confirmation prompt when `--execute` is passed |

### Exit codes

`0` on success (including a clean dry run with nothing to delete). `1` if
the instance isn't Forgejo, authentication fails, or any object failed to
delete. `2` on invalid CLI arguments.

## Known limitations

See [`ai-docs/architecture.md`](ai-docs/architecture.md) "Known
limitations" for the full list, notably:

- A theoretical race between enumeration and deletion if something pushes
  a new reference to an object in between — run during a quiet window.
- The server-side enumeration and deletion both depend on Forgejo's
  internal (not versioned/stable) web UI routes, not a public API, since
  no public API for this exists. A future Forgejo release could change
  these routes; the tool fails loudly rather than silently misbehaving if
  that happens.
- Scripted login (`--username`/`--password`) is implemented but has not
  been verified against a real account — Firefox auto-detection and
  `--cookie-file` are the live-verified paths.
- Automatic cookie detection currently only supports Firefox, not
  Chrome/Chromium (Chrome encrypts cookies at rest via the OS keyring,
  which is more work to support) — use `--cookie-file` there instead.

## Development

See [`CLAUDE.md`](CLAUDE.md) and [`ai-docs/`](ai-docs/) for the full
research, architecture decisions, and implementation plan behind this
tool.
