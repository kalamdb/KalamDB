# Phase 0 Research: KalamDB CLI Development Workflow

## Decision 1: Build the workflow around top-level project subcommands, not root flags

**Decision**: Introduce project-oriented top-level commands for `init`, `link`, `schema`, `migration`, `db`, `dev`, `status`, and `deploy`, and keep the existing interactive `parser.rs` meta-command surface focused on active-session commands rather than project workflow.

**Rationale**: The current CLI already separates top-level clap parsing in `cli/src/args.rs` from in-session meta-commands in `cli/src/parser.rs`. The requested workflow is project-scoped and often runs before a session exists, so it fits the top-level grammar and pre-session dispatch model far better than the REPL/meta-command path. This also aligns with the existing CLI handler refactor plan in `docs/development/cli-command-handler-refactor-plan.md`, which already pushes the codebase toward clearer pre-session and in-session boundaries.

**Alternatives considered**:
- Expose the full workflow through `parser.rs` meta-commands as well as top-level commands: rejected because it duplicates grammar and increases drift risk between `CliCommand` and `MetaCli`.
- Continue adding more root flags like `--watch-schema`: rejected because the requested workflow is a project lifecycle, not a one-off mode switch, and flags do not scale well to nested workflows like `schema gen` or `migration status`.

## Decision 2: Organize new workflow code under a dedicated `cli/src/workflow/` tree

**Decision**: Add a new workflow-oriented module tree under `cli/src/workflow/` and group `project`, `dev`, `schema`, `migration`, `db`, and `deploy` beneath it. Keep `commands/mod.rs` as a thin router, keep `session/` focused on the interactive SQL client, and design the new workflow tree so it can later be extracted into its own crate family if needed.

**Rationale**: Today the CLI mixes binary-only routing modules (`args`, `commands`, `connect`) with library-visible session modules. The user explicitly wants the CLI divided into subfolders and wants dev, deploy, and schema code grouped together. A `workflow/` tree matches that request, respects the repo's current separation between pre-session workflow and session logic, and creates a clean seam for later crate extraction without forcing a crate split during the first feature pass.

**Alternatives considered**:
- Put the new lifecycle code directly into `cli/src/commands/`: rejected because `commands/` is currently a binary-only dispatch area and would make long-term testing and reuse harder.
- Create multiple new workspace crates immediately: rejected for the first pass because it would make feature delivery and review riskier while the command surface is still being defined.

## Decision 3: Add a thin shared CLI output/logging layer instead of a full tracing stack

**Decision**: Standardize CLI output through a small shared output module that owns human-facing stderr messages, machine/data stdout messages, spinner creation, optional append-only log file output, and live log multiplexing for managed `kalam dev` services with stable source prefixes and distinct colors.

**Rationale**: The CLI currently uses direct `println!` and `eprintln!` calls across many modules and has no structured logging crate in `cli/Cargo.toml`. The feature needs a consistent place to send Kalam workflow logs without disrupting stdout/stderr contracts that existing command behavior and scripts depend on. A thin output layer satisfies the user's request for consistent logging across the CLI while minimizing dependency expansion and preserving current conventions.

**Alternatives considered**:
- Adopt `tracing`/subscriber infrastructure immediately: rejected because the CLI does not currently depend on it, and the first requirement is output consistency and log persistence, not distributed tracing.
- Keep ad-hoc `println!`/`eprintln!` and add logging only inside `kalam dev`: rejected because the user explicitly asked for consistent logging across the whole CLI codebase.

## Decision 4: Support TypeScript and Dart first through a language-neutral schema model and emitter interface

**Decision**: Design schema generation around a language-neutral internal schema model plus pluggable emitters, and implement initial emitters for TypeScript and Dart only. The configuration surface should allow one or more generated language targets now and additional languages later without reshaping the orchestration model.

**Rationale**: The existing repo already has strong TypeScript and Dart SDK surfaces under `link/sdks/typescript/` and `link/sdks/dart/`, but only TypeScript currently has schema-generation behavior and that behavior is tied to the `@kalamdb/orm` toolchain. The requested workflow must work for both SQL and remote schema sources and must support Dart as a first-class target. A language-neutral model keeps the workflow extensible and prevents the CLI from being hardcoded around one language's generator.

**Alternatives considered**:
- Reuse `@kalamdb/orm` as the only generator and defer Dart: rejected because the user explicitly wants Dart covered now and because the existing TypeScript generator is remote-schema-focused.
- Generate only one language per project: rejected because the user wants the plan to cover both TypeScript and Dart now and future languages later.

## Decision 5: Keep generated workflow artifacts separate from SDK runtime-generated files

**Decision**: Treat `kalam schema gen` outputs as project-owned generated artifacts and keep them separate from SDK-package-internal generated runtime files such as the Dart FRB-generated bindings in `link/sdks/dart/lib/src/generated/`.

**Rationale**: The repo already has generated SDK implementation files that are maintained by SDK build pipelines, and those are not the same thing as project schema outputs like `src/generated/kalam.ts` or a generated Dart schema file in an application. Mixing those two categories would blur ownership, complicate docs, and violate the repo rule against editing generated SDK outputs manually.

**Alternatives considered**:
- Emit project schema files into existing SDK-generated directories: rejected because those directories belong to SDK package build workflows, not user projects.
- Make generated workflow outputs editable once committed: rejected because the spec defines them as generated artifacts and the workflow should regenerate rather than hand-edit them.

## Decision 6: Make `kalam dev` the long-running parent workflow and retire standalone schema-watch behavior into it

**Decision**: Treat `kalam dev` as the parent long-running orchestration mode and move schema watch/apply/regenerate behavior under the `dev` workflow rather than preserving schema watching as an independent top-level concept.

**Rationale**: The user explicitly stated that `kalam dev` is not just schema watching. The existing `cli/src/commands/watch_schema.rs` provides useful watch/apply orchestration precedent, but the new workflow also needs database readiness, local process supervision, migration creation, and unified logging. Those concerns belong together under `kalam dev`.

**Alternatives considered**:
- Keep `watch_schema` as the primary developer entry point and add `kalam dev` as a wrapper later: rejected because it preserves the old mental model instead of moving the product to the requested lifecycle.
- Remove watch behavior entirely and require manual reruns of schema generation: rejected because the requested developer experience depends on watch-driven local feedback.

## Decision 7: On auto-migration failure, pause only the schema pipeline and keep service logs flowing

**Decision**: When auto-migration or schema-apply fails during `kalam dev`, pause only the schema pipeline, surface the failure prominently in the active console stream, and keep the local server, frontend, and agent logs visible so the developer can diagnose and recover without losing the running session context.

**Rationale**: The user explicitly chose this behavior during refinement. It balances safety and usability better than tearing down the full development session after every schema failure, and it makes the log stream itself part of the recovery experience rather than something the developer has to reconstruct from hidden files.

**Alternatives considered**:
- Stop the entire `kalam dev` session immediately: rejected because it throws away active process context and makes recovery slower.
- Keep everything running without pausing schema work: rejected because repeated failed schema operations would create noisy, potentially unsafe behavior and make failures harder to reason about.

## Decision 8: Extract migration schema diff into a dedicated helper crate with a sqlparser future path

**Decision**: Add `cli/crates/kalam-schema-diff` as a workspace helper crate consumed by the CLI workflow. The first release exposes `diff_schema_files(before, after)` and `diff_schema_sql(before, after)` that return placeholder UP/DOWN sections. A later pass replaces the placeholder body with a structural diff built on the workspace `sqlparser` dependency.

**Rationale**: Migration creation needs a stable seam that can grow from placeholder behavior into real DDL diffing without bloating `cli/src/workflow/migration/create.rs` or blocking the rest of the workflow delivery. A helper crate keeps ownership clear, makes the future sqlparser work testable in isolation, and matches the preference to defer deep diff logic while still wiring the command surface correctly.

**Alternatives considered**:
- Inline comment-only diff inside `cli/src/workflow/migration/create.rs`: rejected because it hides the future sqlparser dependency and makes the migration module harder to evolve.
- Jump directly to sqlparser-backed diff in the first pass: rejected because the first delivery should expose the API and migration file shape without blocking on full structural diff correctness.
