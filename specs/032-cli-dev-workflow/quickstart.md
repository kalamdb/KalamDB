# Quickstart: KalamDB CLI Development Workflow

## Goal

Validate the planned workflow end to end for a local project that targets TypeScript, Dart, or both.

## 1. Initialize a project

```bash
kalam init
```

Expected outcomes:
- `kalam.toml` is created
- a schema source is created
- `kalam/migrations/` exists
- generated output targets are configured for the selected languages

## 2. Configure language targets

Example project configuration:

```toml
[project]
name = "chat-app"
default_env = "dev"

[connection.dev]
url = "http://localhost:2900"
namespace = "app"

[schema]
mode = "sql"
path = "schema.sql"
watch = true
languages = ["typescript", "dart"]

[schema.targets.typescript]
output = "src/generated/kalam.ts"

[schema.targets.dart]
output = "lib/generated/kalam.dart"

[migrations]
dir = "kalam/migrations"
auto_create = true

[dev]
auto_start_db = true
apply_schema = true
generate_types = true
watch = true

[dev.processes]
frontend = "pnpm dev"
agent = "dart run bin/agent.dart"

[logging]
file = true
path = ".kalam/logs/kalam.log"
capture_process_output = true
```

## 3. Generate artifacts without mutating the database

```bash
kalam schema gen
```

Validate:
- TypeScript output is regenerated at `src/generated/kalam.ts`
- Dart output is regenerated at `lib/generated/kalam.dart`
- no database mutation occurs

## 4. Run local orchestration

```bash
kalam dev
```

Validate:
- local development database becomes available
- schema is applied to the development namespace
- migration history updates when configured schema changes require it
- TypeScript and Dart outputs regenerate after schema changes
- configured child processes start
- workflow logs are visible on stderr and written to the configured log file
- local server, frontend, and agent logs appear in the same live console stream with stable prefixed source labels and distinct colors
- if auto-migration or schema apply fails, the failure is shown immediately in the console stream and the schema pipeline pauses while the remaining service logs continue

## 5. Inspect state

```bash
kalam status
```

Validate:
- project name is reported
- resolved environment is reported
- URL and namespace are reported
- schema mode and source are reported
- generated targets are reported
- migration readiness and schema sync state are reported

## 6. Validate migration-backed deployment

```bash
kalam deploy --env prod
```

Validate:
- build step runs
- pending committed migrations are detected and applied
- deployment is blocked if new or uncommitted migrations would be required
- health checks run after rollout

## 7. Recommended focused verification

Validated June 2026 against the workflow implementation in `cli/`:

```bash
cd cli
cargo test -p kalam-cli workflow
cargo test --features e2e-tests --test cli test_project_workflow -- --nocapture
```

Expected: all `test_project_workflow_*` integration tests pass, including dev log multiplexing, schema pause/`--force` recovery, environment precedence, deploy pending-migration blocking, and prod uncommitted-schema guardrails.

### CLI

```bash
cd cli
cargo check --features e2e-tests
cargo nextest run --lib --features e2e-tests
cargo nextest run --test cli --features e2e-tests test_cli_doc_matrix
```

### TypeScript SDK generation

```bash
cd link/sdks/typescript
npm test -- orm
```

### Dart SDK generation

```bash
cd link/sdks/dart
dart test
```

## Success signals

- Developers can bootstrap a project and reach a running `kalam dev` session quickly.
- Schema changes regenerate both TypeScript and Dart outputs consistently.
- Workflow logs are routed consistently across commands and persisted when enabled.
- Production deployment refuses automatic migration generation.
