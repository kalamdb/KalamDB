# Data Model: KalamDB CLI Development Workflow

## Overview

This feature adds a project-oriented workflow model to the CLI. The core data model is centered on project configuration, schema-source resolution, generated language targets, migration history, long-running development orchestration, and deployment readiness.

## Entities

### 1. Kalam Project

**Purpose**: Represents a local KalamDB-enabled application workspace managed by the CLI.

**Fields**:
- `name`: Human-readable project name.
- `root_path`: Filesystem root where `kalam.toml` lives.
- `schema_mode`: Active schema source mode for the project.
- `default_environment`: Environment name used when no override is provided.
- `migration_dir`: Directory containing ordered migration files.
- `logging_policy`: Default CLI workflow logging behavior for the project.

**Relationships**:
- Has many `Environment Links`.
- Has one `Schema Source Configuration`.
- Has one or more `Generated Language Targets`.
- Has many `Migration Files`.
- Has zero or more `Workflow Processes`.

**Validation rules**:
- Exactly one schema mode may be active at a time.
- The project must not embed secret material in the project configuration file.
- The migration directory must resolve inside the project workspace.

### 2. Environment Link

**Purpose**: Connects a project environment name such as `dev` or `prod` to a KalamDB URL and namespace.

**Fields**:
- `name`: Environment identifier.
- `url`: Target KalamDB base URL.
- `namespace`: Target namespace for schema and data operations.
- `secret_source`: External credential source used for authentication.

**Relationships**:
- Belongs to one `Kalam Project`.
- Is referenced by `Development Sessions` and `Deployment Sessions`.

**Validation rules**:
- Environment names must be unique within a project.
- The URL and namespace must both be present for a valid link.
- Secret-bearing values must resolve from external sources rather than `kalam.toml`.

### 3. Schema Source Configuration

**Purpose**: Defines where the authoritative schema state comes from.

**Fields**:
- `mode`: `sql` or `remote` in the initial release.
- `source_path`: Path to the local schema file when the mode is file-based.
- `watch_enabled`: Whether local watch behavior is enabled in development.
- `pull_supported`: Whether remote-to-local synchronization is allowed for this mode.

**Relationships**:
- Belongs to one `Kalam Project`.
- Feeds one or more `Schema Snapshots`.
- Drives one or more `Generated Language Targets`.

**Validation rules**:
- `source_path` is required for file-based schema modes.
- `pull_supported` must be false for any mode that cannot round-trip safely.
- Watch behavior may only be enabled for modes supported by the active workflow.

### 4. Schema Snapshot

**Purpose**: Represents the resolved schema state used for diffing, migration creation, and code generation.

**Fields**:
- `origin`: File-derived or remote-derived.
- `tables`: Logical table definitions.
- `columns`: Logical column definitions and type metadata.
- `constraints`: Primary key and other schema constraints required by generation and diffing.
- `captured_at`: Time the snapshot was resolved.

**Relationships**:
- Produced from one `Schema Source Configuration`.
- Compared against another `Schema Snapshot` to create a `Migration Plan`.
- Consumed by one or more `Generated Language Targets`.

**Validation rules**:
- The snapshot must normalize server data types into one canonical representation before generation.
- Each table and column identifier must be stable within the snapshot.

### 5. Generated Language Target

**Purpose**: Defines one language-specific generated output from the resolved schema.

**Fields**:
- `language`: Initial allowed values are `typescript` and `dart`.
- `output_path`: Destination file path for generated artifacts.
- `enabled`: Whether this target is active for the project.
- `style_profile`: Language-specific generation profile for naming and file layout.

**Relationships**:
- Belongs to one `Kalam Project`.
- Consumes one `Schema Snapshot`.
- Is refreshed by `Schema Generation Runs` and `Development Sessions`.

**Validation rules**:
- At least one generated language target must be enabled for code-generation workflows.
- Output paths for different targets must not collide.
- Generated files are treated as generated artifacts and must not be edited manually.

### 6. Migration File

**Purpose**: Records an ordered schema-history step for replay across environments.

**Fields**:
- `identifier`: Ordered unique migration identifier.
- `name`: Human-readable migration name.
- `path`: File path inside the migration directory.
- `created_from`: Source that produced the migration, such as manual creation or development auto-creation.
- `applied_state`: Pending or applied relative to a target environment.

**Relationships**:
- Belongs to one `Kalam Project`.
- May be created from a `Migration Plan`.
- Is applied by `Database Migration Runs` and `Deployment Sessions`.

**Validation rules**:
- Ordering must be stable and monotonic within the project.
- Production deployment may only use already-recorded migration files.

### 7. Workflow Process

**Purpose**: Describes a user-defined child process managed by `kalam dev`.

**Fields**:
- `name`: Stable process identifier.
- `command`: User-provided command to run.
- `log_prefix`: Stable label used in multiplexed console output.
- `log_color`: Distinct visual identity used when color output is enabled.
- `working_directory`: Optional execution directory.
- `restart_policy`: Development restart behavior.
- `status`: Pending, running, failed, or stopped.

**Relationships**:
- Belongs to one `Kalam Project`.
- Is supervised by one `Development Session`.
- Emits output to one `Log Sink Policy`.

**Validation rules**:
- Process names must be unique within the project.
- Commands must be explicit and non-empty.

### 8. Log Sink Policy

**Purpose**: Defines where CLI workflow logs and child-process logs are written.

**Fields**:
- `stderr_enabled`: Whether human-facing workflow logs are emitted to stderr.
- `file_enabled`: Whether logs are persisted to a file sink.
- `file_path`: Optional resolved log file path.
- `verbosity`: Normal or verbose diagnostic level.
- `prefix_mode`: Source-prefix strategy for merged service logs.
- `color_mode`: Whether service colors are enabled, disabled, or auto-detected.
- `child_process_capture`: Whether child process lines are mirrored into the same sink.

**Relationships**:
- Belongs to one `Kalam Project` or one `Development Session`.
- Receives events from `Development Sessions`, `Deployment Sessions`, and `Workflow Processes`.

**Validation rules**:
- Data-oriented stdout output must remain separate from diagnostic logging.
- File sink output must redact secrets and omit spinner-only animation noise.
- Managed service log lines must retain source identity even when color is unavailable.

### 9. Development Session

**Purpose**: Represents one running `kalam dev` orchestration lifecycle.

**Fields**:
- `environment`: Active environment name, normally `dev`.
- `database_state`: Not started, connecting, ready, or failed.
- `schema_state`: Idle, applying, synced, paused, or failed.
- `generation_state`: Idle, generating, synced, or failed.
- `process_state`: Pending, starting, running, degraded, or stopped.
- `force_mode`: Whether unsafe local changes can proceed without prompt.

**Relationships**:
- Uses one `Environment Link`.
- Uses one `Schema Source Configuration`.
- Refreshes one or more `Generated Language Targets`.
- Supervises zero or more `Workflow Processes`.
- Emits to one `Log Sink Policy`.

**Validation rules**:
- Unsafe schema changes must require confirmation unless force mode is enabled.
- Development auto-created migrations must be recorded before schema application completes.
- When schema application fails, the session must preserve log visibility for still-running managed services.

**State transitions**:
- `starting -> database_ready -> schema_syncing -> generation_syncing -> running`
- `running -> degraded` when a child process or schema generation fails but the session remains alive
- `running -> paused` when schema application or auto-migration fails and waits for user recovery
- `running -> blocked` when an unsafe schema change requires confirmation
- `running -> stopping -> stopped` on shutdown

### 10. Deployment Session

**Purpose**: Represents one `kalam deploy` execution.

**Fields**:
- `environment`: Target environment name.
- `build_state`: Pending, running, succeeded, or failed.
- `migration_check_state`: Pending, valid, or blocked.
- `migration_apply_state`: Pending, running, succeeded, or failed.
- `rollout_state`: Pending, running, succeeded, or failed.
- `health_state`: Pending, healthy, or unhealthy.

**Relationships**:
- Uses one `Environment Link`.
- Reads many `Migration Files`.
- Emits to one `Log Sink Policy`.

**Validation rules**:
- Deployment must not auto-create new migrations for shared or production targets.
- Deployment must stop before rollout if migration history is missing or inconsistent.

**State transitions**:
- `pending -> building -> migration_validating -> migrating -> rolling_out -> health_checking -> succeeded`
- Any stage may transition to `failed`
- `migration_validating -> blocked` when required migration history is absent or uncommitted

## Derived Views

### Project Status View

Computed from:
- Active `Kalam Project`
- Resolved `Environment Link`
- `Schema Source Configuration`
- Current `Migration File` status
- Latest `Development Session` or connectivity result

Used by:
- `kalam status`

### Schema Generation Run

Computed from:
- One `Schema Snapshot`
- One or more `Generated Language Targets`
- Active `Log Sink Policy`

Used by:
- `kalam schema gen`
- `kalam dev`

## Notes

- The model intentionally separates project-owned generated artifacts from SDK-package-internal generated files.
- The model supports TypeScript and Dart now and leaves room for future language targets without changing the orchestration entities.
