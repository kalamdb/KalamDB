# Implementation Plan: KalamDB CLI Development Workflow

**Branch**: `cli-dev` | **Date**: June 6, 2026 | **Spec**: [spec.md](./spec.md)

**Input**: Feature specification from `/specs/032-cli-dev-workflow/spec.md` plus planning constraints for initial TypeScript and Dart generation support, unified CLI log routing, visible multiplexed service logs during `kalam dev`, paused schema recovery on migration failure, and a workflow-focused CLI folder structure under `cli/src/`.

## Summary

Build the KalamDB project lifecycle around a new project-oriented CLI workflow that makes `kalam init`, `kalam dev`, and `kalam deploy` the main entry points for project setup, local orchestration, and safe migration-backed rollout. The implementation will introduce a dedicated `cli/src/workflow/` tree for `project`, `schema`, `migration`, `db`, `dev`, and `deploy` logic; a thin shared CLI output layer for consistent stderr/stdout behavior, optional log-file persistence, and live multiplexed service logs with prefixed colored sources; and a language-neutral schema model with initial emitters for TypeScript and Dart so additional targets can be added later without reshaping the orchestration model.

## Technical Context

**Language/Version**: Rust 1.92 (workspace edition 2021) for the CLI and workflow engine; generated output targets for TypeScript and Dart in the first release

**Primary Dependencies**: Existing `kalam-cli` stack (`clap`, `tokio`, `toml`, `reqwest`, `colored`, `indicatif`, `anyhow`, `thiserror`, `dirs`, `serde`), `kalam-client` for connectivity, `kalam-schema-diff` for migration UP/DOWN generation (placeholder now; future `sqlparser` structural diff), existing TypeScript and Dart SDK packages under `link/sdks/typescript/**` and `link/sdks/dart/**`; no full tracing stack in the first pass

**Storage**: Project `kalam.toml`; local schema files such as `schema.sql`; migration files under `kalam/migrations/`; generated project artifacts for TypeScript and Dart; existing CLI home data under `~/.kalam/`; optional workflow log file at a configured project or CLI path

**Testing**: `cargo check --features e2e-tests`; `cargo nextest run --lib --features e2e-tests`; `cargo nextest run --test cli --features e2e-tests test_cli_doc_matrix`; focused CLI e2e tests for workflow commands; TypeScript generator tests under `link/sdks/typescript/**`; Dart tests under `link/sdks/dart/test`; targeted docs validation for CLI and SDK references

**Target Platform**: KalamDB CLI on macOS, Linux, and other supported local developer environments, with generated artifacts targeting TypeScript and Dart application projects

**Project Type**: Multi-surface workspace feature spanning the Rust CLI crate, generated project configuration, and SDK-facing TypeScript/Dart documentation and tests

**Performance Goals**: Preserve a short local feedback loop for `kalam dev`; keep schema diff, migration creation, and regeneration work bounded enough to satisfy the spec success target of refreshed outputs within 30 seconds for typical edits; avoid spawning unnecessary external toolchains in hot development paths; keep log multiplexing overhead low while still showing all managed services live; avoid noisy duplicate output

**Constraints**: First release supports only `sql` and `remote` schema modes and only TypeScript and Dart generation targets; exactly one active schema source per project; secrets stay outside `kalam.toml`; generated artifacts are not hand-edited; stdout remains reserved for data-oriented output while workflow/status logging and multiplexed service logs stay on stderr or the configured file sink; `kalam dev` must keep local server/frontend/agent logs visible with stable source prefixes and distinct colors; auto-migration failure must pause only the schema pipeline instead of tearing down the full session; the CLI reorganization must group dev/deploy/schema code under `cli/src/` without forcing an immediate workspace crate split; migration diff starts as a dedicated `kalam-schema-diff` helper crate with a placeholder file-based UP/DOWN API until sqlparser-backed structural diff lands

**Scale/Scope**: New project configuration contract, top-level workflow commands, development orchestration, migration lifecycle, status reporting, deployment gating, shared CLI output/logging, prefixed service-log multiplexing and failure surfacing for `kalam dev`, TypeScript and Dart generation targets, and supporting docs/tests across the CLI and SDK areas

## Constitution Check

*GATE: Must pass before Phase 0 research. Re-check after Phase 1 design.*

- **Performance-First Execution**: PASS. The plan keeps the hot development path inside the Rust CLI, avoids a full tracing/logging stack for v1, centers `kalam dev` around one orchestrator instead of multiple parallel command pipelines, and treats log multiplexing as a thin presentation layer over managed process output.
- **Boundary Ownership Before Convenience**: PASS. Workflow orchestration lives under `cli/src/workflow/`; interactive SQL behavior remains in `cli/src/session/`; project-facing SDK docs/tests remain owned by `link/sdks/typescript/**` and `link/sdks/dart/**`; generated project artifacts stay outside SDK-internal generated directories.
- **Minimal Dependency Expansion**: PASS. The plan reuses the current CLI dependency surface where possible and explicitly avoids adding a full new logging framework or immediate crate proliferation in the first pass.
- **Validation, Testing, and Documentation Ship Together**: PASS. The plan includes focused CLI checks, SDK target validation, and required doc updates for the CLI workflow plus TypeScript and Dart surfaces.
- **Composable, Low-Boilerplate APIs**: PASS. The schema workflow is designed around a language-neutral schema model, pluggable emitters, a shared project-resolution layer, and a shared CLI output sink with source-aware log metadata instead of duplicated per-command implementations.

No constitution violations are required.

## Project Structure

### Documentation (this feature)

```text
specs/032-cli-dev-workflow/
├── plan.md
├── research.md
├── data-model.md
├── quickstart.md
├── contracts/
│   ├── kalam-toml.md
│   ├── cli-command-surface.md
│   └── generated-language-targets.md
└── tasks.md
```

### Source Code (repository root)

```text
cli/
├── crates/
│   └── kalam-schema-diff/
│       ├── Cargo.toml
│       └── src/
│           ├── lib.rs
│           └── diff.rs
├── src/
│   ├── main.rs
│   ├── lib.rs
│   ├── args/
│   │   ├── mod.rs
│   │   ├── parsers.rs
│   │   └── workflow.rs
│   ├── commands/
│   │   ├── mod.rs
│   │   ├── auth.rs
│   │   ├── doctor.rs
│   │   └── workflow.rs
│   ├── workflow/
│   │   ├── mod.rs
│   │   ├── project/
│   │   │   ├── mod.rs
│   │   │   ├── config.rs
│   │   │   ├── resolve.rs
│   │   │   ├── init.rs
│   │   │   ├── link.rs
│   │   │   └── status.rs
│   │   ├── schema/
│   │   │   ├── mod.rs
│   │   │   ├── load.rs
│   │   │   ├── model.rs
│   │   │   ├── diff.rs
│   │   │   ├── gen.rs
│   │   │   └── emitters/
│   │   │       ├── mod.rs
│   │   │       ├── typescript.rs
│   │   │       └── dart.rs
│   ├── migration/
│   │   ├── mod.rs
│   │   ├── create.rs
│   │   ├── status.rs
│   │   └── apply.rs
│   ├── db/
│   │   ├── mod.rs
│   │   └── migrate.rs
│   ├── dev/
│   │   ├── mod.rs
│   │   ├── orchestrator.rs
│   │   ├── watch.rs
│   │   ├── processes.rs
│   │   └── logs.rs
│   └── deploy/
│       ├── mod.rs
│       ├── rollout.rs
│       └── health.rs
├── output.rs
├── connect.rs
├── config.rs
└── session/
    └── ...

link/sdks/typescript/
├── orm/
│   └── README.md
└── ...

link/sdks/dart/
├── README.md
└── test/

docs/getting-started/cli.md
```

**Structure Decision**: Keep the existing interactive/session surface stable and add the new lifecycle work under `cli/src/workflow/`, with `dev`, `deploy`, and `schema` grouped under that tree as requested. Keep top-level args and command dispatch thin, centralize shared logging in `cli/src/output.rs`, and extract migration schema diff into `cli/crates/kalam-schema-diff` as a helper crate that the CLI consumes through `cli/src/workflow/schema/diff.rs`. The helper crate ships a placeholder file-based UP/DOWN API now and will grow a sqlparser-backed structural diff later without reshaping the workflow command surface.

## Complexity Tracking

No constitution violations or explicit complexity exceptions are planned.

## Phase 0 Research Summary

See [research.md](./research.md). The planning decisions are settled:
- project lifecycle commands belong in the top-level CLI grammar, not the REPL meta-command surface
- workflow code should live under a dedicated `cli/src/workflow/` tree
- CLI logging should use a thin shared output/log sink rather than a full tracing stack in the first pass
- TypeScript and Dart are the first supported generation targets, backed by a language-neutral schema model that can add more targets later
- `kalam dev` becomes the canonical long-running parent for schema watching, process supervision, workflow logging, and visible source-differentiated service output
- when auto-migration or schema application fails during `kalam dev`, only the schema pipeline pauses while service logs continue streaming
- migration diff lives in `cli/crates/kalam-schema-diff` with a placeholder file-based UP/DOWN API now and sqlparser-backed structural diff planned for a later pass

## Phase 1 Design Summary

See [data-model.md](./data-model.md), [contracts/kalam-toml.md](./contracts/kalam-toml.md), [contracts/cli-command-surface.md](./contracts/cli-command-surface.md), [contracts/generated-language-targets.md](./contracts/generated-language-targets.md), and [quickstart.md](./quickstart.md).

## Post-Design Constitution Check

- **Performance-First Execution**: PASS. The design keeps core orchestration in-process, uses a light shared output layer for logs and colored prefixed service streams, and avoids binding the hot development loop to external generator toolchains.
- **Boundary Ownership Before Convenience**: PASS. The design groups project workflow code under `cli/src/workflow/`, preserves `session/` for interactive commands, and keeps SDK package docs/tests in their owned directories.
- **Minimal Dependency Expansion**: PASS. The design works with the current CLI stack and keeps any future extraction to crates as a later structural step rather than immediate scope.
- **Validation, Testing, and Documentation Ship Together**: PASS. The quickstart includes CLI, TypeScript, and Dart validation surfaces, and the design requires aligned docs/contracts for the command surface and config file.
- **Composable, Low-Boilerplate APIs**: PASS. The design relies on a shared project config model, a language-neutral schema model with target emitters, and a shared output/logging module that carries per-service source identity for present and future views.
