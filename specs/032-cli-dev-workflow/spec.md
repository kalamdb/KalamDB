# Feature Specification: KalamDB CLI Development Workflow

**Feature Branch**: `cli-dev`  
**Created**: June 6, 2026  
**Status**: Draft  
**Input**: User description: "Create a simple KalamDB project lifecycle where developers can initialize a project, define schema, generate SDK types, run local development orchestration, track schema history through migrations, and deploy safely using committed migrations, with `kalam dev` acting as the single local orchestration command."

## User Scenarios & Testing *(mandatory)*

### User Story 1 - Start a New KalamDB Project (Priority: P1)

As a developer starting a new application, I want `kalam init` to create the project files, defaults, and example assets I need so I can begin working with KalamDB immediately instead of assembling the workflow by hand.

**Why this priority**: The workflow cannot begin until a project has a usable configuration, schema source, migration directory, and generated-output location. This is the first-run experience for every team.

**Independent Test**: Run `kalam init` in an empty project directory, answer the setup prompts, and confirm the created project can move directly into local development without additional manual setup.

**Acceptance Scenarios**:

1. **Given** a developer runs `kalam init` in a new project, **When** setup completes, **Then** the project contains a `kalam.toml` file, schema source, migration directory, generated-output location, and environment example file.
2. **Given** a developer selects the default schema workflow during `kalam init`, **When** the project is scaffolded, **Then** the created configuration uses SQL as the active schema source and includes a starter schema when requested.
3. **Given** a developer chooses package-manager auto-detection during `kalam init`, **When** helper commands are later executed, **Then** the project uses the detected package manager according to the documented lockfile priority.

---

### User Story 2 - Keep Schema, Types, and Migration History Aligned (Priority: P1)

As a developer evolving application data structures, I want KalamDB to treat my configured schema source as the source of truth, generate client types from it, and maintain migration history so schema changes stay reviewable and reproducible.

**Why this priority**: Reliable schema evolution is the core of the workflow. Without consistent type generation and migration tracking, teams cannot safely review, share, or replay database changes across environments.

**Independent Test**: Configure a project in SQL mode, edit the schema source, run schema and migration commands, and confirm the generated types and migration history match the intended schema state. Repeat in remote mode and confirm generation and pull behavior follow that mode's rules.

**Acceptance Scenarios**:

1. **Given** a project is configured in SQL mode, **When** a developer runs `kalam schema gen`, **Then** KalamDB reads the local schema source and regenerates the configured client types without mutating the database.
2. **Given** a project is configured in remote mode, **When** a developer runs `kalam schema gen`, **Then** KalamDB reads the current database schema and regenerates the configured client types from the live schema state.
3. **Given** a developer creates or applies schema changes over time, **When** they inspect the migration directory and migration status, **Then** they can see the ordered history of schema changes and which migrations remain pending for the selected environment.

---

### User Story 3 - Use `kalam dev` as the Local Orchestrator (Priority: P1)

As a developer working locally, I want one command to start or connect to my local KalamDB instance, apply schema changes, generate client types, watch for schema updates, run my configured project processes, and stream clearly differentiated logs from each managed service so local development stays fast, predictable, and debuggable.

**Why this priority**: This is the central workflow promised by the feature. The user explicitly defines `kalam dev` as local orchestration rather than a narrow schema watcher, and that orchestration is only usable if developers can immediately see what the local server, frontend, agent, and schema pipeline are doing.

**Independent Test**: Run `kalam dev` in a configured project with one or more local processes, confirm the local database and namespace become ready, verify schema application and type generation happen automatically, then edit the schema source and observe migration creation, schema application, and regenerated client types.

**Acceptance Scenarios**:

1. **Given** a project has local development orchestration enabled, **When** a developer runs `kalam dev`, **Then** KalamDB prepares the local development database if needed, applies the configured schema to the development namespace, regenerates client types, and starts the configured local processes.
2. **Given** a project in SQL mode is running under `kalam dev`, **When** the schema source changes, **Then** KalamDB detects the change, creates a migration automatically when that behavior is enabled, applies the change to the development database, and regenerates client types.
3. **Given** a schema change would perform an unsafe local operation, **When** the developer has not supplied an override flag, **Then** KalamDB requires confirmation before applying the change.
4. **Given** a developer runs `kalam dev --force`, **When** an unsafe local schema change is detected, **Then** KalamDB proceeds according to the force policy instead of stopping for confirmation.
5. **Given** `kalam dev` is managing the local KalamDB server, frontend, and agent, **When** those services emit logs, **Then** each source appears in the running console stream with a stable source prefix and a unique color or other distinct visual treatment so the developer can tell sources apart quickly.
6. **Given** auto-migration or schema application fails during a running `kalam dev` session, **When** the failure is detected, **Then** the failure is shown prominently in the active console output, the schema pipeline pauses, and the other managed service logs continue streaming until the developer fixes the issue or stops the session.

---

### User Story 4 - Understand and Control Project Environment State (Priority: P2)

As a developer or operator, I want to link environments, resolve configuration consistently, and inspect project status in one place so I always know which database, namespace, schema mode, and migration state the CLI is acting on.

**Why this priority**: Once a project spans development and shared environments, clarity becomes as important as automation. Users need confidence that commands are acting on the intended target.

**Independent Test**: Link development and production environments, invoke commands with and without explicit environment selection, and confirm that `kalam status` and other commands reflect the correct target based on the documented precedence rules.

**Acceptance Scenarios**:

1. **Given** a developer links a named environment with `kalam link`, **When** the command succeeds, **Then** the project configuration records the environment's URL and namespace without storing secrets in `kalam.toml`.
2. **Given** the same project defines values in command flags, environment variables, and `kalam.toml`, **When** a command resolves its target environment, **Then** it follows the precedence order of command flag, then environment variable, then configuration file, then default.
3. **Given** a developer runs `kalam status`, **When** KalamDB inspects the active project, **Then** it reports the project identity, selected environment, target URL, namespace, schema mode, schema source, generated output path, migration directory, connection state, and schema sync state.

---

### User Story 5 - Deploy Safely With Committed Migrations (Priority: P2)

As a deployer, I want `kalam deploy` to build the application, verify migration readiness, apply only committed migrations, roll out configured processes, and run health checks so production changes are safe and repeatable.

**Why this priority**: The workflow ends with deployment. Safe deployment behavior is what turns local convenience into a trustworthy team process.

**Independent Test**: Run `kalam deploy` against a non-development environment with committed migrations and healthy processes, then confirm the deployment succeeds. Repeat with missing or uncommitted migration history and confirm the deployment is blocked before schema changes are applied.

**Acceptance Scenarios**:

1. **Given** a shared or production environment has pending committed migrations, **When** a deployer runs `kalam deploy`, **Then** KalamDB applies the pending committed migrations before rolling out configured processes.
2. **Given** a production deployment would require generating a new migration automatically, **When** a deployer runs `kalam deploy`, **Then** KalamDB refuses the deployment and instructs the user to commit migration files first.
3. **Given** a deployment finishes process rollout, **When** health checks fail, **Then** KalamDB reports the failure clearly and marks the deployment as unsuccessful.

### Edge Cases

- A developer runs `kalam init` in a project that already contains a `kalam.toml`, schema source, or migration directory.
- A project is configured for package-manager auto-detection but none of the supported lockfiles are present.
- A project defines multiple schema modes or switches schema modes after migration history already exists.
- `kalam schema pull` is requested for a mode that cannot safely round-trip the remote schema into the configured source format.
- `kalam dev` starts while the local database is unavailable, partially configured, or already running with a different namespace state.
- A schema change in development would drop or rewrite existing data.
- A configured local process exits immediately, fails health checks, or conflicts with another process managed by `kalam dev`.
- Managed service logs become noisy or interleaved enough that developers cannot tell which source emitted an error.
- Color output is disabled or unavailable, but service-prefixed logs still need to remain distinguishable.
- Auto-migration fails while the local server, frontend, and agent are otherwise healthy and still emitting logs.
- A user links an environment but relies on secrets that are missing from the expected external credential source.
- Migration history in the target environment does not match the committed migration files in the project.
- `kalam deploy` is attempted while there are local schema changes that have not been captured in committed migration history.

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001**: The system MUST provide a `kalam init` command that scaffolds a new KalamDB project for local development.
- **FR-002**: `kalam init` MUST collect the project name, preferred language, schema source mode, package-manager behavior, and whether to create an example schema.
- **FR-003**: `kalam init` MUST create a `kalam.toml` file that defines project metadata, environment connections, schema settings, migration settings, development orchestration settings, and named development processes.
- **FR-004**: `kalam init` MUST create the default schema source, generated-output location, migration directory, and environment example file expected by the selected workflow.
- **FR-005**: The system MUST support exactly one active schema source mode per project.
- **FR-006**: The initial release MUST support `sql` mode and `remote` mode as active schema source options.
- **FR-007**: In `sql` mode, the system MUST treat the configured local schema file as the source of truth for schema generation and development-time schema application.
- **FR-008**: In `remote` mode, the system MUST treat the current connected database schema as the source of truth for type generation and schema pull operations.
- **FR-009**: The system MUST provide a `kalam link` command that records a URL and namespace for a named environment in project configuration.
- **FR-010**: The system MUST NOT store secrets in `kalam.toml` and MUST direct secret material to external secret or credential storage.
- **FR-011**: The system MUST provide a `kalam schema gen` command that regenerates configured client SDK types from the active schema source without mutating the database.
- **FR-012**: The system MUST provide a `kalam schema pull` command that syncs the current database schema into local project artifacts according to the active schema mode.
- **FR-013**: When a schema mode cannot safely support `kalam schema pull`, the system MUST refuse the operation and explain the supported alternative.
- **FR-014**: The system MUST provide commands to create migrations, inspect migration status, and apply pending migrations to a selected environment.
- **FR-015**: Migration history MUST be stored as ordered files in the configured migration directory and treated as the record of how the database reached the desired schema state.
- **FR-016**: The system MUST provide a `kalam dev` command that acts as the single local orchestration entry point for database startup, schema application, client type generation, schema watching, and configured developer processes.
- **FR-017**: When `kalam dev` runs in `sql` mode and the schema source changes, the system MUST diff the change against the development database state and, when automatic migration creation is enabled, generate a migration file before applying the change.
- **FR-018**: After `kalam dev` applies a schema change, the system MUST regenerate client SDK types so project code stays aligned with the active schema.
- **FR-019**: `kalam dev` MUST require confirmation for unsafe schema changes unless the developer has explicitly supplied the force behavior supported by the command.
- **FR-020**: The system MUST allow developers to define one or more named local processes for `kalam dev` to run and supervise.
- **FR-021**: The system MUST provide a `kalam status` command that reports the resolved project, environment, connection target, schema source, generated output, migration directory, connection status, and schema sync status.
- **FR-022**: The system MUST resolve environment settings using the precedence order: command flag, then environment variable, then `kalam.toml`, then the default development environment.
- **FR-023**: When package-manager behavior is set to automatic detection, the system MUST choose the package manager using the documented lockfile priority before falling back to a default.
- **FR-024**: The system MUST treat generated client SDK files as generated artifacts and MUST not require users to edit them directly.
- **FR-025**: The system MUST provide a `kalam deploy` command that builds the configured application processes, verifies migration readiness, applies pending committed migrations, deploys the configured processes, and runs a health check.
- **FR-026**: `kalam deploy` MUST refuse to generate migrations automatically for shared or production environments.
- **FR-027**: Shared and production deployments MUST apply only migration files that already exist in project history.
- **FR-028**: The system MUST block deployment when required migration history is missing, inconsistent with the target environment, or not committed according to deployment policy.
- **FR-029**: `kalam dev` MUST stream logs from every managed development service, including the local KalamDB server when it is started or supervised by the workflow, into the active console output while the session is running.
- **FR-030**: Each managed service log line shown by `kalam dev` MUST include a stable source prefix and a unique visual treatment for that source, with a readable prefix-only fallback when color output is disabled or unsupported.
- **FR-031**: Workflow warnings and failures, including schema diff, migration creation, schema application, process supervision, and health problems, MUST be surfaced in the running console output and MUST NOT be hidden only in background files.
- **FR-032**: When auto-migration or schema application fails during `kalam dev`, the system MUST display the failure immediately in the active log stream, pause only the schema pipeline, keep other managed service logs visible, and preserve enough context for the developer to retry or recover without losing session visibility.
- **FR-033**: The `kalam dev` log-stream model MUST preserve per-service source identity so future service-focused log filtering or log-view switching can be added without changing the default all-services stream contract.

### Key Entities *(include if feature involves data)*

- **Kalam Project Configuration**: The user-owned project definition stored in `kalam.toml`, including project identity, schema settings, environment targets, migration behavior, development automation, and local process commands.
- **Schema Source**: The single authoritative schema input for a project, such as a local SQL file or the current remote database schema.
- **Generated Client Types**: Recreated project artifacts derived from the active schema source and intended for application developers to consume rather than edit directly.
- **Migration File**: An ordered schema-history artifact that records a discrete step in how a database moved from one schema state to another.
- **Environment Link**: The named association between a project and a target KalamDB URL plus namespace for a specific environment such as development or production.
- **Development Session**: The managed local workflow started by `kalam dev`, including the local database state, schema watch loop, generated outputs, and supervised project processes.
- **Deployment Session**: The execution of `kalam deploy`, including build preparation, migration validation, migration application, process rollout, and health verification.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: 90% of developers can go from an empty project directory to a working KalamDB project scaffold in under 10 minutes using `kalam init`.
- **SC-002**: In test projects using the default workflow, 95% of schema edits made during `kalam dev` produce updated local schema state, migration history, and regenerated client types within 30 seconds.
- **SC-003**: 100% of tested `kalam schema gen` runs leave the target database unchanged while still refreshing client SDK types from the configured schema source.
- **SC-004**: 100% of tested environment-resolution cases follow the documented precedence order across schema, migration, status, and deployment commands.
- **SC-005**: 100% of tested production deployment attempts that require new or uncommitted migrations are blocked before schema changes are applied.
- **SC-006**: 95% of developers can identify the active environment, schema mode, schema sync state, and migration readiness from a single `kalam status` run in under 30 seconds.
- **SC-007**: 100% of successful shared-environment deployments complete with committed migration history, process rollout, and passing post-deploy health checks.
- **SC-008**: 95% of developers using `kalam dev` can correctly identify whether a log line came from the local server, frontend, or agent without needing to inspect the underlying command text.
- **SC-009**: 100% of tested auto-migration and schema-apply failures during `kalam dev` are visible in the active console stream within 5 seconds, and in all such cases the schema pipeline pauses while the rest of the managed service logs remain visible.

## Assumptions

- The first release of this workflow focuses on `sql` and `remote` schema modes; future schema-source integrations remain outside the scope of this feature.
- A project may define zero or more developer processes for local orchestration, and those processes are user-supplied commands rather than built-in templates.
- `kalam dev` manages one active local KalamDB target and one active namespace context at a time for the selected environment.
- Safe secret storage is handled outside `kalam.toml`, such as through environment files, dedicated credential files, or operating-system credential storage.
- The initial `kalam deploy` experience covers safe migration-backed application rollout and health verification, while provider-specific hosting integrations remain outside this feature's scope.
- The first release of `kalam dev` shows all managed service logs by default; future log filtering or focused per-service views are a planned extension rather than a launch requirement.
