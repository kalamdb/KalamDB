# Tasks: KalamDB CLI Development Workflow

**Input**: Design documents from `/specs/032-cli-dev-workflow/`

**Prerequisites**: `plan.md` (required), `spec.md` (required for user stories), `research.md`, `data-model.md`, `contracts/`, `quickstart.md`

**Tests**: This feature explicitly includes focused CLI, TypeScript, and Dart validation in the spec and plan, so story-specific test tasks are included below.

**Organization**: Tasks are grouped by user story so each story can be implemented and validated as an independently testable increment once Setup and Foundational work are complete.

## Format: `[ID] [P?] [Story] Description`

- **[P]**: Can run in parallel (different files, no dependencies)
- **[Story]**: Which user story this task belongs to (for example, `US1`, `US2`)
- Every task below follows the required checklist format and includes exact file paths

## Phase 1: Setup (Shared Infrastructure)

**Purpose**: Create the workflow-oriented module and test layout described by the implementation plan.

- [X] T001 Create workflow module skeleton in `cli/src/lib.rs`, `cli/src/commands/workflow.rs`, and `cli/src/workflow/mod.rs`
- [X] T002 Create workflow clap scaffolding in `cli/src/args.rs`, `cli/src/args/parsers.rs`, and `cli/src/args/workflow.rs`
- [X] T003 [P] Create workflow test scaffolding in `cli/tests/cli/test_project_workflow_init.rs`, `cli/tests/cli/test_project_workflow_schema.rs`, `cli/tests/cli/test_project_workflow_dev.rs`, `cli/tests/cli/test_project_workflow_status.rs`, and `cli/tests/cli/test_project_workflow_deploy.rs`
- [X] T004 [P] Add workflow documentation placeholders in `docs/getting-started/cli.md`, `link/sdks/typescript/orm/README.md`, and `link/sdks/dart/README.md`

---

## Phase 2: Foundational (Blocking Prerequisites)

**Purpose**: Build the shared workflow primitives that every user story depends on.

**⚠️ CRITICAL**: No user story work should begin until this phase is complete.

- [X] T005 Implement project configuration types and parsing for `kalam.toml` in `cli/src/workflow/project/config.rs`
- [X] T006 Implement environment resolution and secret-source seams in `cli/src/workflow/project/resolve.rs`, `cli/src/credentials.rs`, and `cli/src/connect.rs`
- [X] T007 [P] Implement the shared CLI output, prefixed color registry, filter-ready service source metadata, and file-log sink in `cli/src/output.rs`, `cli/src/config.rs`, and `cli/src/workflow/dev/logs.rs`
- [X] T008 [P] Implement the language-neutral schema model and target registry in `cli/src/workflow/schema/model.rs` and `cli/src/workflow/schema/emitters/mod.rs`
- [X] T009 [P] Implement shared migration metadata and state helpers in `cli/src/workflow/migration/mod.rs`, `cli/src/workflow/migration/create.rs`, and `cli/src/workflow/migration/status.rs`
- [X] T010 Implement common workflow dispatch entry points in `cli/src/commands/workflow.rs`, `cli/src/main.rs`, and `cli/src/lib.rs`

**Checkpoint**: Foundation ready - all user stories can now proceed in priority order or in parallel where noted.

---

## Phase 3: User Story 1 - Start a New KalamDB Project (Priority: P1) 🎯 MVP

**Goal**: Deliver `kalam init` so a developer can scaffold a new KalamDB project with schema source, migration directory, generated target configuration, and starter assets.

**Independent Test**: Run `kalam init` in an empty project directory, answer the prompts, and confirm the generated project can proceed directly into local development without manual file creation.

### Tests for User Story 1

- [X] T011 [P] [US1] Add `kalam init` integration coverage in `cli/tests/cli/test_project_workflow_init.rs`
- [X] T012 [P] [US1] Update CLI help and doc-surface checks for `kalam init` in `cli/tests/cli/test_cli_doc_matrix.rs`

### Implementation for User Story 1

- [X] T013 [P] [US1] Implement init option models and prompt flow in `cli/src/workflow/project/init.rs` and `cli/src/args/workflow.rs`
- [X] T014 [P] [US1] Implement package-manager detection and scaffold writers in `cli/src/workflow/project/init.rs`
- [X] T015 [US1] Wire `kalam init` execution in `cli/src/commands/workflow.rs` and `cli/src/main.rs`
- [X] T016 [US1] Finalize generated project artifacts and onboarding docs in `cli/src/workflow/project/init.rs` and `docs/getting-started/cli.md`

**Checkpoint**: User Story 1 should now scaffold a usable KalamDB project independently.

---

## Phase 4: User Story 2 - Keep Schema, Types, and Migration History Aligned (Priority: P1)

**Goal**: Deliver schema loading, generation, pull behavior, and migration commands that keep project artifacts and migration history aligned for TypeScript and Dart targets.

**Independent Test**: Configure a project in `sql` mode, edit the schema source, run schema and migration commands, and confirm generated TypeScript and Dart outputs plus migration history match the intended schema state. Repeat with `remote` mode and verify pull/generation behavior.

### Tests for User Story 2

- [X] T017 [P] [US2] Add schema and migration CLI integration coverage in `cli/tests/cli/test_project_workflow_schema.rs`
- [X] T018 [P] [US2] Add TypeScript target regression coverage in `link/sdks/typescript/orm/tests/generate_workflow_targets.test.mjs`
- [X] T019 [P] [US2] Add Dart schema generation coverage in `link/sdks/dart/test/schema_generation_test.dart`

### Implementation for User Story 2

- [X] T020 [P] [US2] Implement schema loading for `sql` and `remote` modes in `cli/src/workflow/schema/load.rs` and `cli/src/workflow/schema/model.rs`
- [X] T021 [P] [US2] Implement TypeScript and Dart emitters in `cli/src/workflow/schema/emitters/typescript.rs` and `cli/src/workflow/schema/emitters/dart.rs`
- [X] T022 [US2] Implement `kalam schema gen` and `kalam schema pull` handlers in `cli/src/workflow/schema/gen.rs`, `cli/src/workflow/schema/load.rs`, `cli/src/commands/workflow.rs`, and `cli/src/args/workflow.rs`
- [X] T023 [US2] Implement migration create, status, and apply flows in `cli/src/workflow/migration/create.rs`, `cli/src/workflow/migration/status.rs`, `cli/src/workflow/migration/apply.rs`, and `cli/src/workflow/db/migrate.rs` (migration diff delegated to `kalam-schema-diff` placeholder; see T044-T045)
- [X] T024 [US2] Document TypeScript and Dart target configuration in `docs/getting-started/cli.md`, `link/sdks/typescript/orm/README.md`, and `link/sdks/dart/README.md`
- [X] T044 [P] [US2] Add `kalam-schema-diff` placeholder crate with file-based UP/DOWN API in `cli/crates/kalam-schema-diff/src/lib.rs` and `cli/crates/kalam-schema-diff/src/diff.rs`
- [X] T045 [US2] Wire `kalam migration create` to baseline-vs-schema diff via `cli/src/workflow/schema/diff.rs` and `cli/src/workflow/migration/create.rs`
- [ ] T046 [US2] Implement sqlparser-backed structural schema diff and rollback generation in `cli/crates/kalam-schema-diff/**` (future; replaces placeholder UP/DOWN bodies)

**Checkpoint**: User Story 2 should now regenerate TypeScript and Dart artifacts and manage migration history independently.

---

## Phase 5: User Story 3 - Use `kalam dev` as the Local Orchestrator (Priority: P1)

**Goal**: Deliver `kalam dev` as the single local orchestration command for database readiness, schema application, target regeneration, schema watching, process supervision, visible prefixed service logs, and paused-schema recovery with clear failure surfacing.

**Independent Test**: Run `kalam dev` in a configured project with local server, frontend, and agent processes, verify the development database and namespace become ready, confirm schema application and regeneration occur automatically, verify each source emits prefixed colored logs into the active console stream, then trigger an auto-migration failure and confirm the failure is shown prominently, only the schema pipeline pauses, the remaining service logs continue, and the developer can recover or retry without losing the active log context.

### Tests for User Story 3

- [X] T025 [P] [US3] Add `kalam dev` orchestration and local server/frontend/agent prefixed log-stream coverage in `cli/tests/cli/test_project_workflow_dev.rs`
- [X] T026 [P] [US3] Add schema-watch unsafe-change, auto-migration failure pause, retry or resume recovery, and `--force` coverage in `cli/tests/cli/test_project_workflow_dev_watch.rs`

### Implementation for User Story 3

- [X] T027 [P] [US3] Implement dev session orchestration and process supervision in `cli/src/workflow/dev/orchestrator.rs` and `cli/src/workflow/dev/processes.rs`
- [X] T028 [P] [US3] Move schema watch and apply behavior into `cli/src/workflow/dev/watch.rs` and delegate from `cli/src/commands/watch_schema.rs`
- [X] T029 [P] [US3] Integrate shared log sink, local server/frontend/agent multiplexing, per-service prefixes, unique colors, file logging, child-process capture, and future log-focus-ready source metadata in `cli/src/output.rs`, `cli/src/workflow/dev/logs.rs`, and `cli/src/workflow/dev/processes.rs`
- [X] T030 [US3] Wire `kalam dev` command flow, auto schema apply, regeneration, paused-schema-on-failure behavior, immediate failure display, recovery or retry UX, and `--force` handling in `cli/src/workflow/dev/mod.rs`, `cli/src/commands/workflow.rs`, `cli/src/args/workflow.rs`, and `cli/src/main.rs`

**Checkpoint**: User Story 3 should now provide the end-to-end local development loop independently.

---

## Phase 6: User Story 4 - Understand and Control Project Environment State (Priority: P2)

**Goal**: Deliver `kalam link`, `kalam status`, and deterministic environment resolution so developers can see and control which environment, namespace, schema mode, and migration state the workflow is using.

**Independent Test**: Link development and production environments, run commands with and without explicit overrides, and confirm `kalam status` reports the correct target according to the documented resolution order.

### Tests for User Story 4

- [X] T031 [P] [US4] Add `kalam link` and `kalam status` integration coverage in `cli/tests/cli/test_project_workflow_status.rs`
- [X] T032 [P] [US4] Add environment precedence coverage in `cli/tests/cli/test_project_workflow_resolution.rs`

### Implementation for User Story 4

- [X] T033 [P] [US4] Implement environment link persistence in `cli/src/workflow/project/link.rs` and `cli/src/workflow/project/config.rs`
- [X] T034 [P] [US4] Implement the status view and resolved-environment reporting in `cli/src/workflow/project/status.rs`, `cli/src/workflow/project/resolve.rs`, and `cli/src/connect.rs`
- [X] T035 [US4] Wire `kalam link` and `kalam status` commands in `cli/src/commands/workflow.rs`, `cli/src/args/workflow.rs`, and `cli/src/main.rs`

**Checkpoint**: User Story 4 should now let users link environments and inspect resolved state independently.

---

## Phase 7: User Story 5 - Deploy Safely With Committed Migrations (Priority: P2)

**Goal**: Deliver `kalam deploy` with migration readiness checks, committed-migration enforcement, rollout orchestration, and post-rollout health verification.

**Independent Test**: Run `kalam deploy` against a non-development environment with committed migrations and healthy processes to confirm success, then repeat with missing or uncommitted migration history to confirm deployment is blocked before schema changes are applied.

### Tests for User Story 5

- [X] T036 [P] [US5] Add deploy migration-gate and health-check coverage in `cli/tests/cli/test_project_workflow_deploy.rs`
- [X] T037 [P] [US5] Add committed-migration enforcement coverage in `cli/tests/cli/test_project_workflow_deploy_guardrails.rs`

### Implementation for User Story 5

- [X] T038 [P] [US5] Implement deployment rollout and health-check services in `cli/src/workflow/deploy/rollout.rs` and `cli/src/workflow/deploy/health.rs`
- [X] T039 [P] [US5] Implement deploy migration validation in `cli/src/workflow/deploy/mod.rs` and `cli/src/workflow/migration/apply.rs`
- [X] T040 [US5] Wire `kalam deploy` command flow in `cli/src/commands/workflow.rs`, `cli/src/args/workflow.rs`, and `cli/src/main.rs`

**Checkpoint**: User Story 5 should now enforce migration-backed deployment guardrails independently.

---

## Phase 8: Polish & Cross-Cutting Concerns

**Purpose**: Finish cross-story hardening, docs alignment, and validation.

- [X] T041 [P] Update cross-surface workflow docs in `docs/getting-started/cli.md`, `link/sdks/typescript/orm/README.md`, and `link/sdks/dart/README.md`
- [X] T042 [P] Run quickstart validation and record command expectations in `specs/032-cli-dev-workflow/quickstart.md`
- [X] T043 Harden redaction, error messaging, color-disabled fallback output, paused-schema failure summaries, and no-secret logging paths in `cli/src/output.rs`, `cli/src/workflow/project/resolve.rs`, and `cli/src/workflow/deploy/mod.rs`

---

## Dependencies & Execution Order

### Phase Dependencies

- **Phase 1: Setup**: No dependencies; start immediately.
- **Phase 2: Foundational**: Depends on Phase 1; blocks all user stories.
- **Phase 3: US1**: Depends on Phase 2.
- **Phase 4: US2**: Depends on Phase 2.
- **Phase 5: US3**: Depends on Phase 2.
- **Phase 6: US4**: Depends on Phase 2.
- **Phase 7: US5**: Depends on Phase 2.
- **Phase 8: Polish**: Depends on all selected user stories being complete.

### Recommended User Story Completion Order

1. **US1**: Establish scaffolding and a usable project entry point.
2. **US2**: Deliver schema generation and migration commands.
3. **US3**: Build `kalam dev` on top of the shared workflow and schema foundations.
4. **US4**: Add environment linking and status visibility.
5. **US5**: Finish with guarded deployment behavior.

### User Story Dependency Notes

- **US1** is independent after Foundational work.
- **US2** is independent after Foundational work.
- **US3** is independent after Foundational work because schema model, migration primitives, and logging are already established in Phase 2.
- **US4** is independent after Foundational work.
- **US5** is independent after Foundational work, though it benefits from the same migration primitives and environment resolution used elsewhere.

### Within Each User Story

- Story-specific tests must be written first and should fail before implementation.
- Command models and handlers should follow the clap and dispatch seams created in Setup/Foundational phases.
- Shared models/utilities must not be reimplemented inside story-specific modules.
- Each story should be validated at its checkpoint before moving to later stories.

### Parallel Opportunities

- `T003` and `T004` can run in parallel with each other after `T001` and `T002`.
- `T007`, `T008`, and `T009` can run in parallel in Phase 2 after `T005` and `T006`.
- In **US1**, `T011` and `T012` can run in parallel, and `T013` and `T014` can run in parallel.
- In **US2**, `T017`, `T018`, and `T019` can run in parallel, and `T020` and `T021` can run in parallel; `T044` and `T045` follow the migration create wiring; `T046` tracks the future sqlparser-backed diff implementation.
- In **US3**, `T025` and `T026` can run in parallel, and `T027`, `T028`, and `T029` can run in parallel.
- In **US4**, `T031` and `T032` can run in parallel, and `T033` and `T034` can run in parallel.
- In **US5**, `T036` and `T037` can run in parallel, and `T038` and `T039` can run in parallel.
- `T041` and `T042` can run in parallel during Polish.

---

## Parallel Example: User Story 1

```bash
Task: "Add `kalam init` integration coverage in cli/tests/cli/test_project_workflow_init.rs"
Task: "Update CLI help and doc-surface checks for `kalam init` in cli/tests/cli/test_cli_doc_matrix.rs"

Task: "Implement init option models and prompt flow in cli/src/workflow/project/init.rs and cli/src/args/workflow.rs"
Task: "Implement package-manager detection and scaffold writers in cli/src/workflow/project/init.rs"
```

## Parallel Example: User Story 2

```bash
Task: "Add schema and migration CLI integration coverage in cli/tests/cli/test_project_workflow_schema.rs"
Task: "Add TypeScript target regression coverage in link/sdks/typescript/orm/tests/generate_workflow_targets.test.mjs"
Task: "Add Dart schema generation coverage in link/sdks/dart/test/schema_generation_test.dart"

Task: "Implement schema loading for sql and remote modes in cli/src/workflow/schema/load.rs and cli/src/workflow/schema/model.rs"
Task: "Implement TypeScript and Dart emitters in cli/src/workflow/schema/emitters/typescript.rs and cli/src/workflow/schema/emitters/dart.rs"
```

## Parallel Example: User Story 3

```bash
Task: "Add `kalam dev` orchestration and local server/frontend/agent prefixed log-stream coverage in cli/tests/cli/test_project_workflow_dev.rs"
Task: "Add schema-watch unsafe-change, auto-migration failure pause, retry or resume recovery, and `--force` coverage in cli/tests/cli/test_project_workflow_dev_watch.rs"

Task: "Implement dev session orchestration and process supervision in cli/src/workflow/dev/orchestrator.rs and cli/src/workflow/dev/processes.rs"
Task: "Move schema watch and apply behavior into cli/src/workflow/dev/watch.rs and delegate from cli/src/commands/watch_schema.rs"
Task: "Integrate shared log sink, local server/frontend/agent multiplexing, per-service prefixes, unique colors, file logging, child-process capture, and future log-focus-ready source metadata in cli/src/output.rs, cli/src/workflow/dev/logs.rs, and cli/src/workflow/dev/processes.rs"
```

## Parallel Example: User Story 4

```bash
Task: "Add `kalam link` and `kalam status` integration coverage in cli/tests/cli/test_project_workflow_status.rs"
Task: "Add environment precedence coverage in cli/tests/cli/test_project_workflow_resolution.rs"

Task: "Implement environment link persistence in cli/src/workflow/project/link.rs and cli/src/workflow/project/config.rs"
Task: "Implement the status view and resolved-environment reporting in cli/src/workflow/project/status.rs, cli/src/workflow/project/resolve.rs, and cli/src/connect.rs"
```

## Parallel Example: User Story 5

```bash
Task: "Add deploy migration-gate and health-check coverage in cli/tests/cli/test_project_workflow_deploy.rs"
Task: "Add committed-migration enforcement coverage in cli/tests/cli/test_project_workflow_deploy_guardrails.rs"

Task: "Implement deployment rollout and health-check services in cli/src/workflow/deploy/rollout.rs and cli/src/workflow/deploy/health.rs"
Task: "Implement deploy migration validation in cli/src/workflow/deploy/mod.rs and cli/src/workflow/migration/apply.rs"
```

---

## Implementation Strategy

### MVP First (User Story 1 Only)

1. Complete Phase 1: Setup.
2. Complete Phase 2: Foundational.
3. Complete Phase 3: User Story 1.
4. Stop and validate that `kalam init` scaffolds a project correctly.

### Incremental Delivery

1. Finish Setup and Foundational phases once.
2. Deliver **US1** for project scaffolding.
3. Deliver **US2** for schema generation and migration commands.
4. Deliver **US3** for the full `kalam dev` orchestration loop.
5. Deliver **US4** for environment linking and status visibility.
6. Deliver **US5** for guarded deployment behavior.
7. Finish Polish and quickstart validation.

### Parallel Team Strategy

With multiple developers:

1. Pair on Setup and Foundational work.
2. After Phase 2 completes:
   - Developer A can take **US1**
   - Developer B can take **US2**
   - Developer C can take **US4**
3. Once schema and workflow primitives are stable, another developer can take **US3** and **US5** in sequence or in parallel with careful coordination on shared workflow files.

---

## Notes

- All tasks use the required checklist format with task IDs, optional parallel markers, story labels where required, and explicit file paths.
- Tests are included because the spec and plan explicitly require focused validation for the CLI plus TypeScript and Dart generation surfaces.
- The first release scope remains limited to `sql` and `remote` schema modes and to `typescript` and `dart` targets, while keeping the file layout extensible for future languages.
