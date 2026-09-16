# Spec: Everyday CLI lifecycle

Agent-first source of truth for `kalam` local development. Batches 1–2 are in
scope. Remote schema pull and managed cloud stay later.

## Objective

Make `dev`, `up`, `down`, `status`, and `logs` the everyday surface. Every
command must resolve the same target. AI coding agents should be able to
discover, start, inspect, and test a project without guessing ports or
reusing someone else's database.

## Everyday commands

| Command | Responsibility |
|---|---|
| `kalam init` | Add KalamDB to an app, or scaffold a chosen template |
| `kalam dev` | Full development loop (schema watch, types, app processes) |
| `kalam up` | Start only the database, in the background |
| `kalam down` | Stop the managed local database; keep data |
| `kalam status` | Selected environment, database health, dev session, schema |
| `kalam logs` | Database logs; `--follow` streams |
| `kalam` / `kalam -c` | SQL against that same target |

`dev start/stop/status/logs` remain advanced controls for the development
session (app processes + watchers), not the database itself.

## Targets

| Target | Selection | Storage |
|---|---|---|
| Project-local | Default | Existing `kalam/server` if present, else `.kalam/` |
| Shared local | `--global` / `-g` | `~/.kalam/servers/default/` |
| Named environment | `--env NAME` | Linked URL + namespace |

`--global` is a shared database under the current OS user. It does not
expose the server on the network or install a service.

Reject `--global` together with `--env`, `--url`, or `--host`.

Precedence: CLI flags → `KALAM_ENV` / `KALAM_URL` / `KALAM_NAMESPACE` →
`kalam.toml`. `--link` must not change `project.default_env`.

A listening port is not ownership. Reuse a database only when
`.kalam/run/instance.json` (or the shared equivalent) matches the live
process and data directory.

## Layout

```text
my-app/
  kalam.toml
  schema.sql
  kalam/migrations/
  kalam/seed.sql
  kalam/server/            # honored when it already exists
  .kalam/data logs/ run/   # runtime; gitignored

~/.kalam/
  credentials.toml
  bin/<version>/kalamdb-server
  servers/default/
```

`kalam up` works in an empty folder. It must not require a language template.

## Ownership rules

- `dev` reuses a database started by `up` and leaves it running on exit.
- If `dev` started the database itself, it stops it on exit.
- `down` stops any registered `kalam dev` session first, then the database.
- Allocated ports persist in `instance.json`. Explicit `--port` that is busy
  is an error. Otherwise pick the next free HTTP (and PostgreSQL, if enabled)
  port.

## Reproducible development

- `kalam db reset` rebuilds only the resolved local target: stop, delete data
  (keep `server.toml`), start, migrate, seed. Never remote-reset by default.
- `kalam db seed` reruns `kalam/seed.sql`. Keep the file idempotent.
- Ordinary `up`/`dev` seed once per database (hash recorded on the instance).
- Production-purpose environments never auto-seed.
- `kalam dev --exec "cmd"` uses an isolated temporary database, injects
  connection settings, runs the command, cleans up, and returns that exit code.
- Pin `project.server_version` (default: CLI version) under
  `~/.kalam/bin/<version>/`.

## Agent contract

- `--agent` and `--json` work on lifecycle commands.
- Ready event: `KALAM_READY` plus a short human block.
- Errors use `KALAM_ERROR code=...`.
- Ordinary schema-watch failures emit `KALAM_ERROR` and keep the database and
  application running. Fix the file; the watcher retries. Destructive schema
  changes still stop `--agent` unless `--force` is passed.
- Do not inject root credentials into `[dev.processes]`. Public URL and
  namespace only. `--exec` may inject test-database credentials.
- Agents should read this spec, then `cli/src/workflow/target.rs` and
  `cli/src/workflow/instance.rs`.
