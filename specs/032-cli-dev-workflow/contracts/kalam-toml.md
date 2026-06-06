# Contract: `kalam.toml`

## Purpose

`kalam.toml` is the project-owned contract for KalamDB workflow commands. It defines how the CLI resolves environments, schema sources, generated language targets, migrations, development orchestration, and workflow logging.

## Contract Rules

1. The file is project-scoped and lives at the repository root.
2. It must not store secrets.
3. It must declare exactly one active schema source mode.
4. It may declare one or more generated language targets.
5. The initial supported generated language targets are `typescript` and `dart`.
6. Future language targets must be addable without changing existing target semantics.

## Required Sections

### `[project]`

```toml
[project]
name = "chat-app"
default_env = "dev"
```

**Fields**:
- `name`: Human-readable project name.
- `default_env`: Default environment when no override is supplied.

### `[connection.<env>]`

```toml
[connection.dev]
url = "http://localhost:2900"
namespace = "app"

[connection.prod]
url = "https://db.example.com"
namespace = "app"
```

**Fields**:
- `url`: KalamDB endpoint for the named environment.
- `namespace`: Namespace selected for schema and data operations.

### `[schema]`

```toml
[schema]
mode = "sql"
path = "schema.sql"
watch = true
languages = ["typescript", "dart"]
```

**Fields**:
- `mode`: Initial allowed values are `sql` and `remote`.
- `path`: Required for file-based schema modes.
- `watch`: Enables development watch behavior when supported.
- `languages`: Ordered list of active generation targets.

### `[schema.targets.<language>]`

```toml
[schema.targets.typescript]
output = "src/generated/kalam.ts"

[schema.targets.dart]
output = "lib/generated/kalam.dart"
```

**Fields**:
- `output`: Destination path for the generated artifact.

**Rules**:
- Target names must match entries listed in `schema.languages`.
- Output paths must not collide.

### `[migrations]`

```toml
[migrations]
dir = "kalam/migrations"
auto_create = true
```

**Fields**:
- `dir`: Migration history directory.
- `auto_create`: Enables automatic migration file creation during approved development workflows.

### `[dev]`

```toml
[dev]
auto_start_db = true
apply_schema = true
generate_types = true
watch = true
```

**Fields**:
- `auto_start_db`: Whether local orchestration may prepare the development database automatically.
- `apply_schema`: Whether schema application runs as part of `kalam dev`.
- `generate_types`: Whether language targets regenerate during `kalam dev`.
- `watch`: Whether schema watch behavior is enabled for the development session.

### `[dev.processes]`

```toml
[dev.processes]
frontend = "pnpm dev"
agent = "dart run bin/agent.dart"
```

**Rules**:
- Keys are stable process names.
- Values are project-owned commands executed by `kalam dev`.
- Managed process names are also used as the default source identity for prefixed `kalam dev` log output.

### `[logging]`

```toml
[logging]
file = true
path = ".kalam/logs/kalam.log"
capture_process_output = true
```

**Fields**:
- `file`: Enables append-only workflow log persistence.
- `path`: Optional explicit log path.
- `capture_process_output`: Controls whether child process lines are mirrored into the workflow log sink.

**Behavioral rules**:
- The default `kalam dev` experience streams all managed service logs to the console.
- Managed service logs must retain stable source prefixes even when color output is disabled.
- Future per-service log filtering or log-focus controls may extend this section without changing the default all-services stream contract.

## Resolution Rules

For commands that accept an environment override, resolution order is:

1. CLI flag
2. Environment variable
3. `kalam.toml`
4. Default `dev` environment

## Compatibility Rules

- A project may generate only TypeScript, only Dart, or both.
- Adding future language targets must be additive to the `[schema.targets.<language>]` contract.
- Generated artifacts remain generated artifacts even when committed to source control.
