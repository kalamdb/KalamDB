# Contract: CLI Command Surface

## Purpose

This contract defines the user-facing command surface for the KalamDB project workflow.

## Command Families

### `kalam init`

**Purpose**: Scaffold a new KalamDB project.

**Required behaviors**:
- Collect the project name.
- Collect the active schema source mode.
- Collect the initial generated language targets from the supported set.
- Offer example schema creation.
- Create `kalam.toml`, schema source files, migration directory, and generated-output locations.

### `kalam link`

**Purpose**: Store a URL and namespace for a named environment.

**Required behaviors**:
- Accept environment name, URL, and namespace inputs.
- Update project configuration without storing secrets.

### `kalam schema gen`

**Purpose**: Generate language artifacts from the resolved schema source.

**Required behaviors**:
- Read the active schema source.
- Regenerate all enabled language targets, or a selected subset when the command is scoped.
- Never mutate the database.

### `kalam schema pull`

**Purpose**: Sync live schema state into local artifacts when the active mode supports it.

**Required behaviors**:
- Refuse unsupported pull operations with a clear alternative.
- Update local schema artifacts only when the mode allows safe pull semantics.

### `kalam migration create <name>`

**Purpose**: Create a named migration file in project history.

### `kalam migration status`

**Purpose**: Report applied and pending migration state for the resolved environment.

### `kalam db migrate`

**Purpose**: Apply pending migration history to the resolved environment.

### `kalam dev`

**Purpose**: Run the local development orchestration workflow.

**Required behaviors**:
- Prepare the local development database when allowed.
- Apply schema changes to the development namespace.
- Generate enabled language targets.
- Watch the schema source when configured.
- Run configured child processes.
- Emit workflow logs consistently.
- Merge the live logs from the local KalamDB server, frontend, and agent into the active console stream when those services are managed by the session.
- Prefix each managed service line with a stable source label and distinct visual treatment so sources remain readable in mixed output.
- When auto-migration or schema apply fails, surface the failure prominently, pause only the schema pipeline, and keep the rest of the managed service logs visible.

### `kalam status`

**Purpose**: Report project, environment, schema, migration, and connection state.

### `kalam deploy`

**Purpose**: Run the deployment workflow for a selected environment.

**Required behaviors**:
- Build configured application processes.
- Validate migration readiness.
- Apply committed migrations only.
- Roll out configured processes.
- Run health checks.

## Output Contract

### Stdout

Reserved for:
- command data meant for direct consumption or piping
- explicitly generated file content when a command supports stdout output

### Stderr

Reserved for:
- human-facing status updates
- warnings
- errors
- workflow progress
- verbose diagnostics
- multiplexed managed-service logs shown during `kalam dev`

### File Logging

When enabled by configuration or flag:
- workflow diagnostic events are appended to a log file
- child process lines may be mirrored into the same log sink
- log output must redact secrets
- persisted logs retain service source prefixes even when console colors are unavailable

## Compatibility Rules

1. Project workflow commands are top-level commands, not REPL-only commands.
2. Existing stdout/stderr contracts must remain stable enough for scripts and tests.
3. `kalam dev` becomes the canonical local orchestration entry point for schema watch behavior.
4. Production-oriented commands must refuse automatic migration generation.
5. The default `kalam dev` log view is an all-services stream; future service-focused filtering may be added later without changing the default behavior.
