# CLI Command Handler Refactor Plan

## Goals

- Keep clap as the source of truth for command-line grammar whenever possible.
- Keep SQL execution, file import, and interactive session behavior reusable from one session surface.
- Give commands a shared handler boundary so a command can run before a session exists or inside an active `CLISession` without duplicating orchestration code.
- Make every command path testable through parser tests, handler-selection tests, doc/help matrix tests, or end-to-end CLI tests.

## Current Architecture

- Top-level CLI flags are parsed by clap in `cli/src/args.rs`.
- Interactive backslash commands are parsed by clap-derived meta-command structs in `cli/src/parser.rs`, with `\as` kept on a custom parser because it must preserve the SQL tail exactly.
- Pre-session commands are routed through `CommandHandler` in `cli/src/commands/mod.rs`.
- In-session commands are routed through `SessionCommandHandler` in `cli/src/session/commands.rs`.
- SQL file import remains centralized in `CLISession::execute_file` and `execute_batch`.

## Phase 1: Clap Grammar Consolidation

Status: complete.

- Convert interactive meta-command parsing from manual argument loops to clap `Parser`, `Subcommand`, `Args`, and `ValueEnum` derives.
- Keep unknown command compatibility for `\unknown` style input.
- Keep parser tests for aliases, cluster subcommands, consume flags, and format validation.
- Add a completion-to-parser test so autocomplete entries stay registered in the clap parser.

## Phase 2: Shared Handler Boundaries

Status: complete.

- Add a pre-session `CommandHandler` trait for credential, login, watch-schema, and subscription modes.
- Route `main.rs` through `handle_pre_session_commands` before creating a normal command/file/interactive session.
- Add a session-side `SessionCommandHandler` trait for parsed `Command` execution inside `CLISession`.
- Keep existing behavior in place while creating a stable seam for future per-command handler extraction.

## Phase 3: Per-Command Handler Extraction

Status: planned.

- Move high-churn command families into separate handler modules:
  - `session/commands/sql.rs` for SQL, describe, stats, sessions, and execute-as wrappers.
  - `session/commands/cluster.rs` for cluster list/action commands.
  - `session/commands/live.rs` for live query and subscribe commands.
  - `session/commands/credentials.rs` for in-session credential commands.
  - `session/commands/consume.rs` for topic consumer mode.
- Each handler should implement the session handler trait or expose a small typed command struct consumed by the trait implementation.
- Keep conversion from `parser::Command` exhaustive, with no wildcard match over command variants.

## Phase 4: Top-Level Clap Command Mode Cleanup

Status: planned.

- Convert mutually exclusive command modes into a typed execution-mode resolver instead of tuple matching raw fields in `main.rs`.
- Preserve existing flags for compatibility.
- Prefer clap constraints for invalid combinations where clap can express them cleanly.
- Keep runtime validation only for cases that depend on resolved config, credentials, or interactive terminal state.

## Test Gates

- `cargo check --features e2e-tests`
- `cargo nextest run --lib --features e2e-tests`
- `cargo nextest run --test cli --features e2e-tests test_cli_doc_matrix`
- Focused end-to-end tests for commands that hit a running server, especially SQL file import, consume mode, live query, and credential flows.
- `./run-tests.sh` before landing broad CLI behavior changes.

## Coverage Rules

- A new backslash command must be added to the clap meta parser, command completions, help output, and doc matrix coverage together.
- A new top-level mode must implement the pre-session handler trait or the normal session execution path.
- A new in-session command must be represented by `parser::Command` and execute through the session handler trait.
- Commands that only build SQL should have unit tests for their SQL builder and at least one parser test.
- Commands that contact the server should have focused e2e coverage in `cli/tests`.