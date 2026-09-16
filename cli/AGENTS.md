# CLI agent notes

Everyday development is `dev`, `up`, `down`, `status`, and `logs`. Spec:
`../specs/035-cli-everyday-lifecycle/spec.md`.

## Read these files first

- Target resolution: `src/workflow/target.rs`
- Process identity: `src/workflow/instance.rs`
- Command dispatch: `src/commands/workflow/`
- Lifecycle: `src/workflow/lifecycle/`
- `kalam dev`: `src/workflow/dev/`
- `kalam db`: `src/workflow/db/`
- `kalam functions`: `src/workflow/functions/`

## Command map

| Command | Does |
|---|---|
| `kalam init` | Add KalamDB to an app, or scaffold a template |
| `kalam up [-g]` | Start only the database, in the background |
| `kalam down [-g]` | Stop that database; keep data |
| `kalam status` | Environment, health, session, schema (partial when offline) |
| `kalam logs --follow` | Database logs |
| `kalam dev --agent` | Full loop: DB, migrations, types, app processes, watch |
| `kalam dev --exec "cmd"` | Isolated temp DB, then the command's exit code |
| `kalam` / `kalam -c` | SQL against the same resolved target |

`--global` / `-g` is `~/.kalam/servers/default/`, not a network service. Do not
combine it with `--env`, `--url`, or `--host`.

## Agent contract

- Prefer `--agent` (and `--json` when you need structured status).
- Ready: `KALAM_READY` plus a short human block.
- Failures: `KALAM_ERROR code=...`. Ordinary schema-watch failures keep the
  database and application running; the watcher retries after the file is fixed.
- Do not assume a healthy port belongs to this project. Ownership is
  `.kalam/run/instance.json` (or the shared-server equivalent).
- Application processes get `KALAM_URL` and `KALAM_NAMESPACE` only. Never inject
  the CLI root password into `[dev.processes]`.
- `schema.mode = "remote"` is rejected; keep `schema.sql` as the source of truth.
- `link` saves a named environment; it does not change `project.default_env`.
