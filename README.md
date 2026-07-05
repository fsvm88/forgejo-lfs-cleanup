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
  for how this was confirmed. You need one of:
  - `--username`/`--password` (or the `FORGEJO_USER`/`FORGEJO_PASS` env
    vars) for a scripted login, **or**
  - `--cookie-file`, a Netscape/curl-format cookie jar exported from a
    logged-in browser session, for instances where a scripted login can't
    get past an anti-bot layer or 2FA (this is the case on Codeberg — see
    below).

## Usage

```sh
# Report what would be deleted, without deleting anything (the default):
forgejo-lfs-cleanup ssh://git@codeberg.org/owner/repo.git --username you --password '...'

# Same, using a cookie file instead of scripted login:
forgejo-lfs-cleanup ssh://git@codeberg.org/owner/repo.git --cookie-file cookies.txt

# Actually delete the stale objects, with an interactive confirmation prompt:
forgejo-lfs-cleanup ssh://git@codeberg.org/owner/repo.git --cookie-file cookies.txt --execute

# Same, but skip the prompt (for scripts/CI):
forgejo-lfs-cleanup ssh://git@codeberg.org/owner/repo.git --cookie-file cookies.txt --execute --yes
```

Credentials are best supplied via environment variables rather than flags,
to keep them out of shell history and process listings:

```sh
export FORGEJO_USER=you
export FORGEJO_PASS='...'
forgejo-lfs-cleanup ssh://git@codeberg.org/owner/repo.git
```

### Getting a cookie file

On instances with an anti-bot layer in front of the login page (Codeberg
confirmed to have one — see
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
  been verified against a real account — `--cookie-file` is the
  live-verified path.

## Development

See [`CLAUDE.md`](CLAUDE.md) and [`ai-docs/`](ai-docs/) for the full
research, architecture decisions, and implementation plan behind this
tool.
