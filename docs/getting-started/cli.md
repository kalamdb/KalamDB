# Kalam CLI

Interactive terminal client for KalamDB, built on `kalam-client`.

Binary name: `kalam` (from the `cli` crate).

## Quickstart (first 5 minutes)

1) Start the server (default is `http://localhost:2900`)

```bash
cd backend
cargo run
```

2) Install the CLI

```bash
npm install -g @kalamdb/cli

# or
curl -fsSL https://kalamdb.org/install.sh | sh

kalam --help
```

3) Connect and run a query

```bash
./target/release/kalam

# inside kalam
\dt
SELECT * FROM information_schema.tables LIMIT 5;
```

4) Optional: start a live query

```bash
# inside kalam
\subscribe SELECT * FROM app.messages WHERE user_id = 'alice';
```

### Install

```bash
npm install -g @kalamdb/cli

# or
curl -fsSL https://kalamdb.org/install.sh | sh

# from source
cd cli && cargo build --release
```

### Tooling Commands

```bash
kalam version
kalam doctor
kalam update
kalam login --instance prod --url https://db.example.com --user root --password
kalam whoami --instance prod
kalam logout --instance prod
kalam token create --name ci-prod
```

### Connect

```bash
# Default – uses stored credentials/config, otherwise http://localhost:2900
kalam

# Explicit URL
kalam --url http://localhost:2900

# Host/port alternative (note: if you use --host without --port, the default port is 3000)
kalam --host localhost --port 2900

# User/password login
kalam login --instance dev --user alice --password Secret123!

# JWT
kalam --token "<JWT_TOKEN>"

# Save credentials and drop into the shell immediately when run from a terminal
kalam login --instance dev --user alice --password Secret123!
```

When `kalam login` runs in an interactive terminal, it enters the normal SQL shell immediately after a successful local or OIDC login. Non-interactive invocations still save credentials and exit so shell scripts can keep treating `login` as a one-shot command.

### Run SQL

```bash
# One command and exit
kalam -c "SELECT * FROM information_schema.tables LIMIT 5;"

# File and exit
kalam -f setup.sql
```

### Watch schema generation

```bash
# Run your schema generator once, then rerun it whenever app namespace metadata changes
kalam --watch-schema --namespace app --run "npm run schema:gen" --run-on-start

# Watch one specific table with a tighter poll interval
kalam --watch-schema --table app.messages --run "npm run schema:gen" --interval 2s
```

### Important flags

- `--url`, `-u` – server URL
- `--host`, `-H` and `--port`, `-p` – alternative to `--url`
- `--instance` – credential instance name (default: `local`)
- `--token` – JWT bearer token
- `--user` / `--password` – user/password login
- `--save-credentials` – save JWT token after login
- `--show-credentials` – show stored credentials for instance
- `--update-credentials` – login and update stored credentials
- `--delete-credentials` – delete stored credentials for instance
- `--list-instances` – list stored credential instances
- `--format` – `table` (default) | `json` | `csv`
- `--json` / `--csv` – shorthand for `--format`
- `--no-color` – disable colored output
- `--no-spinner` – disable spinners/animations
- `--loading-threshold-ms` – loading indicator threshold
- `--file`, `-f` – execute SQL file
- `--command`, `-c` – execute a single SQL statement
- `--config` – config path (default `~/.kalam/config.toml`)
- `--verbose`, `-v` – verbose logging
- `--timeout` – HTTP timeout in seconds
- `--connection-timeout` – connection timeout in seconds
- `--receive-timeout` – receive timeout in seconds
- `--auth-timeout` – WebSocket auth timeout in seconds
- `--fast-timeouts` / `--relaxed-timeouts` – timeout presets
- `--subscribe <SQL>` – subscribe (non-interactive) to a live query
- `--subscription-timeout` – subscription idle timeout in seconds (0 = no timeout)
- `--initial-data-timeout` – max seconds to wait for initial data batch
- `--list-subscriptions` – list active subscriptions
- `--watch-schema` – poll `information_schema.tables` and run a local command on schema changes
- `--namespace` – repeat to scope schema watch to one or more namespaces
- `--table` – repeat to scope schema watch to one or more `namespace.table` targets
- `--run` – shell command executed after schema changes are detected
- `--run-on-start` – execute the watch command once before polling
- `--interval` – schema watch poll interval, default `5s`

### Top-level commands

- `kalam version` – print CLI version/build metadata
- `kalam update [--version <version>] [--pre-release]` – replace the current binary with a verified GitHub release artifact
- `kalam doctor [--strict]` – inspect binary path, config, credentials, healthcheck, and auth reachability
- `kalam login --instance <name> --url <url>` – login, save access/refresh tokens, and enter the interactive shell immediately when run from a terminal
- `kalam logout [--all]` – remove saved credentials locally and best-effort notify the server
- `kalam whoami` – call `/v1/api/auth/me` with the resolved credentials
- `kalam token create --name <name>` – create a service account and print a fresh access/refresh token pair

### Interactive `\` commands

In interactive mode, meta-commands start with `\`:

| Command                          | Description                               |
|----------------------------------|-------------------------------------------|
| `\help`, `\?`                   | Show help                                 |
| `\quit`, `\q`                   | Exit                                      |
| `\info`, `\session`             | Show session info                         |
| `\history`, `\h`                | Open command history                      |
| `\dt`, `\tables`               | List tables (`information_schema.tables`) |
| `\d <table>`, `\describe <table>` | Describe table                         |
| `\as <user_id> <SQL>`           | Wrap one statement as `EXECUTE AS '<user_id>'` |
| `\stats`, `\metrics`          | Show `system.stats`                       |
| `\health`                       | Server healthcheck                        |
| `\flush`                        | Run `STORAGE FLUSH ALL`                   |
| `\format table|json|csv`        | Change output format                      |
| `\live <SQL>`, `\subscribe <SQL>` | Start live subscription (`\subscribe` is an alias) |
| `\cluster ...`                  | Cluster commands (see below)              |
| `\refresh-tables`, `\refresh` | Refresh autocomplete metadata             |
| `\sessions`                     | Show active sessions                      |
| `\consume <topic> ...`          | Consume topic messages                    |
| `\show-credentials`, `\credentials` | Show stored credentials               |
| `\update-credentials <u> <p>`  | Update stored credentials                 |
| `\delete-credentials`          | Delete stored credentials                 |

Backup/export SQL examples you can run directly from the CLI:

```sql
BACKUP DATABASE TO '/tmp/kalamdb-backup.tar.gz';
EXPORT USER DATA;
SHOW EXPORT;
```

### Cluster meta-commands

- `\cluster snapshot`
- `\cluster purge --upto <index>` (or `\cluster purge <index>`)
- `\cluster trigger-election`
- `\cluster transfer-leader <node_id>`
- `\cluster rebalance`
- `\cluster stepdown`
- `\cluster clear`
- `\cluster list` (alias: `\cluster ls`)
- `\cluster list groups`
- `\cluster join <node_id> <rpc_addr> <api_addr>`

### How follower writes are forwarded

KalamDB uses Multi-Raft groups. A request does not have to land on the leader node first.

Example:

```bash
kalam --url http://node-2:2900 --command "INSERT INTO app.messages (id, body) VALUES (101, 'hello')"
```

Assume the authenticated or effective user for that request is `user-42`, and `user-42` hashes to user data group `DataUserShard(7)`.

1. The request can hit any node, including a follower for that group.
2. The SQL layer prepares and classifies the statement once, then derives the target Raft group from the table type and current `user_id`.
3. For user and stream tables, KalamDB hashes `user_id` into one of `cluster.user_shards` groups.
4. If the receiving node is not the leader for that target group, it forwards the original SQL, params, auth header, and request id over gRPC to the current leader for that group.
5. The group leader executes the write, appends it to that Raft log, replicates it to followers, commits it, and returns the result.
6. The follower relays that leader-built response back to the client.

This keeps writes local to the correct group leader even when clients connect to follower nodes.

### Multi-Raft routing today

- KalamDB runs one metadata Raft group plus multiple user data groups.
- User and stream data are routed by `user_id`, so all rows for the same user go through the same user-data Raft group leader at a given time instead of scattering one user's working set across many leaders.
- That locality reduces cross-group coordination and improves cache and write-path behavior.
- Shared tables are different today: they currently route to a single shared group.
- Shared-table sharding is still a work in progress. The planned direction is partition-by-key so each shared table can define how a row is partitioned and where it should be placed.

### Output formats

- `table` – pretty table with row count and latency
- `json` – raw JSON rows
- `csv` – header + rows (good for piping)

### Live subscriptions

Start a live query from interactive mode:

```bash
kalam> \subscribe SELECT * FROM app.messages WHERE user_id = 'alice';
```

Or start one from a non-interactive invocation:

```bash
kalam --subscribe "SELECT * FROM app.messages WHERE user_id = 'alice';"
```

### Multiple Instances

```bash
# Setup credentials for different environments
kalam --update-credentials --instance dev --user dev_user
kalam --update-credentials --instance staging --user staging_user
kalam --update-credentials --instance prod --user prod_admin

# Switch between instances
kalam --instance dev      # Connect to dev
kalam --instance staging  # Connect to staging
kalam --instance prod     # Connect to production
```

### Advanced Queries

```bash
# Complex aggregation with output formatting
kalam --instance prod \
  --command "SELECT country, COUNT(*) as users FROM users GROUP BY country" \
  --format json \
  --no-color > stats.json

# Any SQL supported by the server works here.
```

### File uploads in INSERT/UPDATE

You can upload files directly from the CLI using the `file()` helper in `INSERT` or `UPDATE` statements.

```bash
KalamDB[cluster] root@0.0.0.0:2900 ❯ INSERT INTO chat.uploads (id, name, attachment)
  VALUES ('doc2', 'CLI Doc', file('/Users/user/document1.pdf', 'text/plain'));
Inserted 1 row(s)
Query OK, 1 rows affected
```

```bash
KalamDB[cluster] root@0.0.0.0:2900 ❯ UPDATE chat.uploads
  SET attachment = file('/Users/user/document1.pdf', 'text/plain')
  WHERE id = 'doc2';
```

Selecting the row returns file metadata in the column value.

---

## Smoke Tests

Fast end-to-end checks that your server and CLI are wired correctly. The suite covers:

- User table subscription lifecycle
- Shared table CRUD
- System tables and user lifecycle
- Stream table subscription
- User table row-level security (per-user isolation)

Requirements:

- Server running at http://localhost:2900

Run options:

1) Directly with Cargo

```bash
cargo test -p kalam-cli smoke -- --test-threads=1 --nocapture
```

Run individual tests (examples):

```bash
# User table subscription lifecycle
cargo test -p kalam-cli smoke_user_table_subscription_lifecycle -- --nocapture

# Shared table CRUD
cargo test -p kalam-cli smoke_shared_table_crud -- --nocapture

# System tables + user lifecycle
cargo test -p kalam-cli smoke_system_tables_and_user_lifecycle -- --nocapture

# Stream table subscription
cargo test -p kalam-cli smoke_stream_table_subscription -- --nocapture

# User table RLS (per-user isolation)
cargo test -p kalam-cli smoke_user_table_rls_isolation -- --nocapture
```

Notes:

- Default server URL for tests is http://localhost:2900.

---

## Keyboard Shortcuts

### Line Editing

| Shortcut | Action |
|----------|--------|
| `Ctrl+A` | Move to beginning of line |
| `Ctrl+E` | Move to end of line |
| `Ctrl+K` | Delete from cursor to end of line |
| `Ctrl+U` | Delete from cursor to beginning of line |
| `Ctrl+W` | Delete word before cursor |
| `Alt+D` | Delete word after cursor |

### History Navigation

| Shortcut | Action |
|----------|--------|
| `↑` | Previous command |
| `↓` | Next command |
| `Ctrl+R` | Reverse search history |
| `Ctrl+S` | Forward search history |

### Completion

| Shortcut | Action |
|----------|--------|
| `Tab` | Autocomplete SQL keywords, tables, columns |
| `Tab Tab` | Show all completions |

### Control

| Shortcut | Action |
|----------|--------|
| `Ctrl+C` | Cancel current query/subscription |
| `Ctrl+D` | Exit CLI (alternative to `\quit`) |
| `Ctrl+L` | Clear screen |

---

## Tips & Tricks

### 1. Auto-Completion

The CLI provides intelligent auto-completion:
- **SQL Keywords**: `SEL` + Tab → `SELECT`
- **Table Names**: `FROM us` + Tab → `FROM users`
- **Column Names**: Context-aware completion in SELECT/WHERE clauses

```sql
kalam> SELECT na[Tab]
kalam> SELECT name FROM us[Tab]
kalam> SELECT name FROM users WHERE a[Tab]
```

### 2. Loading Indicator

Queries taking longer than 200ms show a loading spinner:
```
Executing query...
```

### 3. Pretty Tables

Tables automatically adjust to terminal width:
- Columns exceeding 50 characters are truncated with `...`
- Total table width respects terminal size
- Change format with `\format json` or `\format csv`

### 4. Color Output

Disable colors for piping or logging:
```bash
kalam --no-color -c "SELECT * FROM users" > output.txt
```

### 5. Timing Information

Execution metadata is displayed for all queries:
```
(10 rows)

As: root
Took: 245.123 ms
```

### 6. Error Messages

Clear, actionable error messages:
```
ERROR 1001: Table 'users' not found
Details: Available tables: system.tables, system.users, events
```

### 7. Batch Operations

Execute multiple statements from a file:
```sql
-- migration.sql
CREATE TABLE products (id INT, name VARCHAR(100));
INSERT INTO products VALUES (1, 'Laptop'), (2, 'Phone');
SELECT * FROM products;
```

```bash
kalam -f migration.sql
```

### 8. Watch Mode (Real-Time)

Monitor live data changes:
```sql
-- Terminal 1: Start watching
kalam> \subscribe SELECT * FROM orders WHERE status = 'pending'

-- Terminal 2: Insert data
kalam> INSERT INTO orders (id, status) VALUES (1, 'pending');

-- Terminal 1 automatically shows the new row
```

### 9. Quick Health Check

Health check is a CLI meta-command (interactive mode):

```bash
kalam

# inside kalam
\health
```

### 10. System Introspection

```sql
-- Find large tables
SELECT table_name, row_count 
FROM system.tables 
ORDER BY row_count DESC;

-- Monitor active connections
SELECT * FROM system.users WHERE last_seen > NOW() - INTERVAL 5 MINUTES;

-- Check running jobs
SELECT * FROM system.jobs WHERE status = 'running';
```

### 11. Cache Statistics and System Metrics

View server metrics using the `\stats` command (alias: `\metrics`). This runs:

```sql
SELECT metric_name, metric_value FROM system.stats ORDER BY metric_name;
```

```bash
# Show all cache statistics
kalam> \stats

# Or use the alias
kalam> \metrics
```

Recent slow queries are available as a system view:

```sql
SELECT timestamp, duration_ms, user_id, table_name, query
FROM system.slow_queries
ORDER BY timestamp_ms DESC
LIMIT 20;
```



---

## Troubleshooting

### Connection Issues

```bash
# Use interactive health check
kalam --url http://localhost:2900

# inside kalam
\health

# Verbose mode for debugging
kalam --verbose --url http://localhost:2900
```

### Authentication Failures

```bash
# Verify stored credentials
kalam --show-credentials --instance local

# Clear and re-enter credentials
kalam --delete-credentials --instance local
kalam --update-credentials --instance local
```

### Performance Issues

```bash
# Reduce timeout for faster failures (CLI flag)
kalam --timeout 10

# Check query execution time
kalam> SELECT * FROM large_table LIMIT 1;
# Took: 1234.567 ms
```

### Display Issues

```bash
# Disable colors if rendering incorrectly
kalam --no-color

# Switch to JSON for machine-readable output
kalam --format json

# Adjust terminal width or use CSV
kalam --csv
```

---

## Related Documentation

- [API Examples (Bruno collection)](../API-Kalam/) - REST API request examples
- [SQL Syntax](../reference/sql.md) - Complete SQL syntax guide
- [WebSocket Protocol](../api/websocket-protocol.md) - Real-time subscription details
- [Development Setup](../development/development-setup.md) - Build and development guide

---

## Project workflow commands

KalamDB projects use a `kalam.toml` file at the repository root to configure schema sources, generated language targets, migrations, and local development orchestration.

### Initialize a project

```bash
kalam init --yes --name my-app --schema-mode sql --languages typescript,dart
```

This creates:

- `kalam.toml` — project configuration
- `schema.sql` — file-based schema source (sql mode)
- `kalam/migrations/` — ordered migration history
- `src/generated/kalam.ts` and `lib/generated/kalam.dart` — generated output directories
- `.env.example` — environment override template

### Schema and migrations (sql mode)

```bash
# Regenerate workflow artifacts
# TypeScript uses @kalamdb/orm against the resolved server/namespace.
# Dart currently writes a placeholder file.
kalam schema gen

# Create a migration from the current schema
kalam migration create add_profile

# Inspect local migration state
kalam migration status

# Apply pending migrations (local state tracking in v1)
kalam db migrate
```

Environment resolution order: CLI flag → environment variable (`KALAM_ENV`, `KALAM_URL`, `KALAM_NAMESPACE`) → `kalam.toml` → default `dev`.

`kalam schema pull` requires a connected KalamDB server when using remote schema mode.

### Link environments

```bash
kalam link --env prod --url https://db.example.com --namespace app
```

Stores URL and namespace in `kalam.toml` only — credentials stay in `~/.kalam/`.

### Local development orchestration

```bash
kalam dev
kalam dev --force   # retry a paused schema pipeline
```

`kalam dev`:

- applies pending migrations and regenerates enabled language targets when configured
- watches `schema.sql` for changes (2s poll) and re-runs the schema pipeline
- supervises `[dev.processes]` child commands with prefixed, color-coded stderr logs
- pauses only the schema pipeline on migration/apply failure while keeping processes running

### Inspect project state

```bash
kalam status
kalam status --env prod
```

Reports project name, resolved environment (with precedence source), schema mode, generated targets, and migration counts.

### Deploy with migration guardrails

```bash
kalam db migrate          # apply locally first
kalam deploy --env prod
```

Deploy blocks when:

- pending migrations exist (run `kalam db migrate` first)
- production schema drift exists without a committed migration file

After rollout, deploy runs `GET {url}/ui` and accepts 2xx/3xx responses.

---

## Support

For issues, questions, or contributions:
- GitHub: [github.com/kalamdb/KalamDB](https://github.com/kalamdb/KalamDB)
- Documentation: [docs/README.md](../README.md)

---

**Version**: 0.1.3  
**Last Updated**: October 28, 2025
