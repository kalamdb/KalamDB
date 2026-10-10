# Tasks: PostgreSQL-Only SQL Language

**Input**: `specs/036-postgresql-sql-dialect/` | **Generated**: 2026-10-03 | **Revised**: 2026-10-04

**Prerequisites**: spec.md, plan.md, research.md, data-model.md, contracts/ (including efficiency-security-cleanup.md) and quickstart.md. Read `.specify/memory/constitution.md` and applicable AGENTS.md before implementation.

**Tests**: Explicitly required by FR-029 and the user-story acceptance criteria. Test tasks precede implementation; show relevant failures first. All runtime tasks below are pending.

**Format**: `- [ ] Tnnn [P?] [USn?] Description with file path`. Paths are repository-relative unless `../` identifies a separate documentation repository. New files are created by their task. Each model is in its own file.

**Parallelism**: [P] applies only to the adjacent independent test/document tasks within that phase, after all earlier phases and shared prerequisites are complete. Test pairs write different files. Do not parallelize modifications to shared classifier/parser/executor modules. Markers describe future scheduling; they do not request agent delegation during this planning task.

**Operational gates**: default dev profile; batch compile feedback; cargo nextest; CLI prebuild + server before smoke; no --no-fail-fast; async timeouts use observed healthy runtime × 1.5; perf runtimes in seconds. Never modify generated SDK output by hand.

## Phase 1: Setup

**Goal**: Freeze the baseline, scope and exact acceptance corpus before behavior changes.

**Independent validation**: Baseline evidence exists; no runtime compatibility is claimed from document review.

- [ ] T001 Create `specs/036-postgresql-sql-dialect/validation.md` with commit/dependency versions, command outcomes and an exhaustive parser/SQL-producer/stored-definition inventory starting from `plan.md`; identify every dispatcher branch and transport, and distinguish typed FDW calls from SQL input; expand the seeded `specs/036-postgresql-sql-dialect/cleanup.md` with verified old symbol/replacement/callers/evidence rows and record concrete source/depth/concurrency/cache memory budgets before implementation.
- [ ] T002 Create `backend/crates/kalamdb-dialect/tests/fixtures/postgresql/manifest.json` and per-case SQL/expected-result fixtures for every capability-matrix family; freeze current working GUI/JDBC text, rows and metadata, and record baseline pass/failure and transport applicability in `specs/036-postgresql-sql-dialect/validation.md`.
- [ ] T003 Add a reproducible baseline harness in `backend/crates/kalamdb-core/tests/postgresql_pipeline_perf.rs`; measure equal-work parse/classify/authorize/plan and warm prepared execution, parse-only timing and allocations using quickstart warmups/repetitions; include every custom family, equal-work concurrency, p50/p95, allocation counts/bytes, retained memory and clone counts; record per-case runtime seconds before parser changes in `specs/036-postgresql-sql-dialect/validation.md`.
- [ ] T004 Verify pinned AST type compatibility and required PostgreSQL parser gaps against `Cargo.toml`, sqlparser 0.62 and DataFusion 55.1; add minimal hook/AST-planning regression probes to `backend/crates/kalamdb-dialect/tests/postgresql_architecture.rs` and `backend/crates/kalamdb-core/tests/postgresql_pipeline.rs`, recording exact outcomes and helper-reuse/gap justifications in `specs/036-postgresql-sql-dialect/research.md`; follow upstream identifier/object-name/type/expression/list and visitor APIs, not copied grammar.

## Phase 2: Foundation

**Goal**: Build the shared typed pipeline internally; no public dialect flag or second production language.

**Independent validation**: Wrapper delegation, AST handoff, spans, batch boundaries and parse-count probes pass; US1/US2 may exercise the internal typed executor before US3 cutover.

- [ ] T005 Add one model per file under `backend/crates/kalamdb-dialect/src/models/`: `parsed_statement.rs`, `statement_payload.rs`, `postgres_compat_statement.rs`, `kalam_statement.rs`, `sql_source_span.rs`, `sql_source_origin.rs`, `sql_statement_class.rs`, `dialect_error.rs` and `mod.rs`; export from `src/lib.rs`, enforcing mutually exclusive payloads, original `Arc<str>` and typed errors.
- [ ] T006 Replace the alias in `backend/crates/kalamdb-dialect/src/dialect.rs` with a PostgreSQL-delegating dialect, including `dialect()` identity, capabilities and expression precedence; extend `tests/postgresql_architecture.rs` with upstream-versus-wrapper lexical/AST fixtures and keep recursion/resource limits.
- [ ] T007 Implement `backend/crates/kalamdb-dialect/src/parser/kalam_parser.rs` and exports in `parser/mod.rs`: one upstream tokenization handed to `Parser::with_tokens_with_locations`, one sqlparser cursor, borrowed token-lookahead dispatch, upstream nodes or typed owned nodes, no fallback parser chain/sentinels/shared mutable side channel; expose test-scoped parse counters.
- [ ] T008 Add pure AST/payload classification in `backend/crates/kalamdb-dialect/src/parser/classify_from_ast.rs` and adapt `classifier/types.rs`; derive only the payload-free syntax category without text parsing or role-dependent syntax; create existing context-bound `SqlStatementKind` payloads later from the same AST during name binding, never embedding a default namespace in cached syntax.
- [ ] T009 Add the internal typed planning/handler entry in `backend/crates/kalamdb-core/src/sql/ast_planner.rs` and wire `sql/mod.rs`; map upstream nodes to DataFusion `Statement::Statement` and `SessionState::statement_to_plan`, preserving permission-aware providers with no SQL serialization.
- [ ] T010 Replace independent statement-boundary scanning in `backend/crates/kalamdb-dialect/src/batch_execution.rs` with the shared tokenizer/parser result and source spans; add batch/escape/dollar/nested-comment/depth fixtures to `tests/postgresql_architecture.rs`.
- [ ] T011 Define token/AST-only compatibility adapters in `backend/crates/kalamdb-dialect/src/parser/compatibility.rs` for required context-keyword/JDBC forms; update `parser/utils.rs` to eliminate parse-failure normalization retries for converted paths and preserve original diagnostics.
- [ ] T012 Add safe ownership/source-origin/admission tests in `backend/crates/kalamdb-dialect/tests/postgresql_memory.rs`; implement bounded source/token handling in `src/parser/kalam_parser.rs` and `src/models/sql_source_origin.rs`, including UTF-8 span mapping, pre-tokenization byte admission, no retained token stream, preserved recursion protection and no new unsafe/leaked buffers.
- [ ] T013 Run the foundation architecture/pipeline probes and one captured dev-profile `cargo check -p kalamdb-server`; fix the edit batch together and record commands/results in `specs/036-postgresql-sql-dialect/validation.md`, including AST type identity and one-parse assertions.

## Phase 3: US1 — Standard PostgreSQL DDL and sessions (P1)

**Goal**: Execute the standard SQL baseline through typed handlers and existing authorization/session owners.

**Independent validation**: Fresh-schema DDL/identity/procedure script succeeds; ordered search-path and transaction tests prove state semantics and denied-role behavior. Transport cutover is validated in US3.

- [ ] T014 [P] [US1] Add positive/negative standard SQL and upstream-gap tests in `backend/crates/kalamdb-dialect/tests/postgresql_standard.rs` plus executable setup/cleanup fixtures in `backend/crates/kalamdb-core/tests/postgresql_ddl.rs`; include all FR-003 families, user-vs-table collision, procedure creation before CALL/GRANT, and rejection of unsupported options.
- [ ] T015 [P] [US1] Add ordered/quoted search-path, current-user-schema, SET LOCAL, rollback/reset, HTTP lifetime and denied-access scenarios in `backend/crates/kalamdb-core/tests/postgresql_sessions.rs`; unsupported savepoints/transaction options/GUCs must fail explicitly.
- [ ] T016 [US1] Convert standard schema/table/view/index/drop/alter/type/comment/policy/grant/CALL/EXPLAIN parsing to AST converters in `backend/crates/kalamdb-dialect/src/ddl/` (including `create_schema.rs`, `create_table/parser.rs`, `create_view.rs`, `create_index.rs`, `drop_table.rs`, `alter_table.rs`, `create_type.rs`, `alter_type.rs`, `comment.rs`, `policy_commands/mod.rs`, `grant_execute.rs`, `call.rs`); validate unsupported AST fields before mutation and retain nested ASTs.
- [ ] T017 [US1] Implement required PostgreSQL identity/role shared-cursor compatibility parsing in `backend/crates/kalamdb-dialect/src/ddl/user_commands.rs`; preserve password/login and existing Kalam options, reject unsupported elevated PostgreSQL powers, distinguish quoted identifiers and never route CREATE USER to table DDL.
- [ ] T018 [US1] Implement procedure shared-cursor parsing in `backend/crates/kalamdb-dialect/src/ddl/create_procedure.rs` and mapped DROP/CALL/signature handling; support OR REPLACE, LANGUAGE, tagged dollar body and SECURITY options, leave non-SQL bodies opaque, reject CALL LIMIT/OFFSET and unsupported runtime languages.
- [ ] T019 [US1] Add typed ordered search-path models in `backend/crates/kalamdb-commons/src/models/search_path.rs` and `search_path_entry.rs`, export them and adapt `backend/crates/kalamdb-core/src/sql/context/execution_context.rs`; preserve unquoted case folding and quoted identifier values without storing quote delimiters in domain IDs.
- [ ] T020 [US1] Integrate ordered lookup/creation rules and implicit pg_catalog into `backend/crates/kalamdb-core/src/schema_registry/policy_table_resolver.rs` and the permission-aware planning adapter `sql/ast_planner.rs`; share resolution with DDL and context/catalog functions, skipping nonexistent schemas without bypassing access checks.
- [ ] T021 [US1] Store session search paths and transaction-local overrides in `backend/crates/kalamdb-backend/src/session.rs` and `manager.rs`; implement SET/RESET lifecycle restoration on commit/rollback with existing transaction owners, and explicit unsupported-option handling.
- [ ] T022 [US1] Replace raw SET/SHOW/transaction syntax classification with typed actions in `backend/crates/kalamdb-postgres-wire/src/tx_control.rs`, `client_catalog/postgres_set.rs`, `client_catalog/postgres_show.rs` and core `sql/executor/sql_executor/postgres_meta.rs`; enumerate allowed inert client metadata GUCs and reject unknown settings.
- [ ] T023 [US1] Connect standard typed handlers and post-parse authorization in `backend/crates/kalamdb-core/src/sql/executor/handlers/typed.rs` and `handler_registry.rs`; preserve existing grants, policy roles and admin restrictions while distinguishing unsupported PostgreSQL execution from malformed syntax.
- [ ] T024 [US1] Run US1 standard/DDL/session suites against the internal typed pipeline, expand missing fixture cases for types/roles/procedures/transactions, and record exact capabilities and runtime seconds for any perf cases in `specs/036-postgresql-sql-dialect/validation.md`.

## Phase 4: US2 — Table kinds without PostgreSQL collisions (P1)

**Goal**: Preserve all table kinds/options and shared default with canonical PostgreSQL-shaped DDL.

**Independent validation**: Create/read all three kinds using required stream options; shared/stream aliases agree; CREATE/DROP USER TABLE and invalid options fail before mutation.

- [ ] T025 [US2] Add kind/default/stream-option/constraint/alias/conflicting-option and removed-prefix cases in `backend/crates/kalamdb-dialect/tests/postgresql_table_kinds.rs`; add catalog assertions to `backend/crates/kalamdb-core/tests/postgresql_ddl.rs`.
- [ ] T026 [US2] Extend the standard AST table conversion from US1 in `backend/crates/kalamdb-dialect/src/ddl/create_table/parser.rs` and `types.rs` for the complete Kalam table-kind/options inventory; retain all supported typed options/defaults, reject duplicates/unknowns/unsupported constraints, and remove regex/global string replacements including AUTO_INCREMENT normalization.
- [ ] T027 [US2] Implement CREATE SHARED/STREAM TABLE aliases and AUTO_INCREMENT token-level compatibility in `backend/crates/kalamdb-dialect/src/parser/kalam_parser.rs` and owning table parser; reject CREATE/DROP USER TABLE with migration diagnostics without intercepting legal identity commands.
- [ ] T028 [US2] Run kind/identity/catalog tests, preserving storage/retention semantics and shared default; record US2 evidence and option inventory in `specs/036-postgresql-sql-dialect/validation.md`.

## Phase 5: US3 — One language through execution and every transport (P1)

**Goal**: Cut over standard SQL to one parsed tree, AST planning, strict dialect settings and shared metadata.

**Independent validation**: Shared SQL/corpus outcomes agree on HTTP/wire; unchanged GUI probes pass; bound/cache-hit/retry paths do not reparse or leak planning context.

- [ ] T029 [P] [US3] Add shared positive/negative PostgreSQL cases in `backend/crates/kalamdb-dialect/tests/postgresql_corpus.rs` and execution cases in `backend/crates/kalamdb-core/tests/postgresql_pipeline.rs`: ARRAY/UNNEST, JSON arrows, casts, CTEs/joins/windows, DML RETURNING/ON CONFLICT, unsupported features, lambda and dialect-setting rejection.
- [ ] T030 [P] [US3] Extend `backend/crates/kalamdb-postgres-wire/tests/wire_extended_query.rs` with Parse/Bind/Describe/Execute metadata, multi-statement rejection, typed null/parameter errors and changed principal/search-path/catalog scenarios; add typed SET/transaction parity cases in `tests/wire_transactions.rs`.
- [ ] T031 [US3] Migrate `backend/crates/kalamdb-core/src/sql/executor/sql_executor/mod.rs` and `prepared_execution_statement.rs` to shared `Arc<ParsedStatement>` for DML, metadata, retries and handlers; route query planning through `sql/ast_planner.rs` and remove duplicate parsed-DML/text-planning ownership.
- [ ] T032 [US3] Use parsed batches directly in `backend/crates/kalamdb-api/src/http/sql/statements.rs` and `execute.rs`; maintain original spans, statement ordering, request/session context and existing batch transaction behavior without split-then-reparse.
- [ ] T033 [US3] Migrate wire `backend/crates/kalamdb-postgres-wire/src/statement.rs`, `query.rs` and `connection.rs` to the same parsed result; infer parameter/result metadata from one permission-aware planning context and remove separate SQL parsing for each metadata pass.
- [ ] T034 [US3] Enforce typed bind isolation and context/version-aware replan in `backend/crates/kalamdb-core/src/sql/executor/parameter_binding.rs` and owning `backend/crates/kalamdb-plan-cache/src/lib.rs` (core `sql/plan_cache.rs` is a re-export); update `tests/prepared_metadata_cache.rs` for role/tenant/search-path/schema changes and verify no SQL text substitution or reparse on cache hits/retries.
- [ ] T035 [US3] Set PostgreSQL defensive defaults in `backend/crates/kalamdb-core/src/sql/datafusion_session.rs`; block alternate dialect SET/config/reset paths and unregistered syntax in shared validation, replacing permissive options in `backend/crates/kalamdb-dialect/src/parser/utils.rs` without banning PostgreSQL JSON arrows.
- [ ] T036 [US3] Replace necessary catalog/context-function/UNNEST text rewrites in `backend/crates/kalamdb-dialect/src/parser/utils.rs` and `parser/pg_unnest.rs` with AST/planner adaptation in `backend/crates/kalamdb-core/src/sql/ast_planner.rs` or existing owning providers; remove redundant rewrites and prove unchanged GUI fixture rows/metadata.
- [ ] T037 [US3] Probe every PG-QUERY/TYPE/ARRAY/JSON/UNSUPPORTED form in `specs/036-postgresql-sql-dialect/contracts/capability-matrix.md`; implement adaptation for executable pinned-planner forms, preserve baseline working cases, and record exact unsupported/parser-gap cases with fixture IDs in `validation.md` without downgrading Required rows.
- [ ] T038 [US3] Map shared syntax/unsupported/migration/authorization/resource errors and original positions in HTTP `backend/crates/kalamdb-api/src/http/sql/statements.rs` and wire `backend/crates/kalamdb-postgres-wire/src/query.rs`; add cross-transport error fixtures and ensure diagnostics redact credentials/protected metadata.
- [ ] T039 [US3] Add adversarial role/tenant/system-schema/CTE/view/UNION/prepared-context tests in `backend/crates/kalamdb-core/tests/postgresql_authorization.rs`; extend `backend/crates/kalamdb-dialect/tests/postgresql_corpus.rs` with recursion/depth/size/malformed quoting cases and assert limits remain enforced.
- [ ] T040 [US3] Add injection and secret-log regression fixtures in `backend/crates/kalamdb-core/tests/postgresql_injection.rs` plus memory/error/cancellation lifecycle cases in `tests/postgresql_memory_lifecycle.rs`; cover value/identifier/custom-option boundaries, stored SQL, per-member batch authorization, SECURITY DEFINER, cross-principal binds, large-batch source retention and application/dependency logging.
- [ ] T041 [US3] Audit and fix data-to-SQL construction in `backend/crates/kalamdb-core/src/sql/executor/parameter_binding.rs`, `sql/executor/handlers/typed.rs`, `functions/host.rs` and inventoried privileged/custom handlers; retain typed values/identifier ASTs, require EOF on single-command APIs, reauthorize every batch member and redact app/dependency errors/logs. Record each reviewed sink and fixture in `specs/036-postgresql-sql-dialect/validation.md`.
- [ ] T042 [US3] Add bounded retained-byte/cache admission using existing owners in `backend/crates/kalamdb-plan-cache/src/lib.rs`, core `sql/executor/prepared_execution_statement.rs` and wire `connection.rs`; compact original source once or decline oversized entries, prevent unrelated batch retention, preserve span origins, avoid duplicate full AST ownership/warm clones and prove limits/drop/eviction/cancel/concurrent isolation with `backend/crates/kalamdb-core/tests/postgresql_memory_lifecycle.rs`.
- [ ] T043 [US3] Perform the atomic standard-SQL cutover in `backend/crates/kalamdb-dialect/src/classifier/engine/core.rs`, retaining no alternate public dialect flag; run all US1–US3 suites including wire/HTTP parity and parse counts, and record results in `specs/036-postgresql-sql-dialect/validation.md`.

## Phase 6: US4 — All Kalam commands share the PostgreSQL parser (P2)

**Goal**: Preserve every existing custom-command family with shared lexical rules and typed payloads.

**Independent validation**: Each registered extension has success/error/quoting/role fixtures; subscriptions and nested stored SQL execute without independent lexers.

- [ ] T044 [P] [US4] Create complete extension-family tests in `backend/crates/kalamdb-dialect/tests/postgresql_extensions.rs` from the setup inventory, including topic trigger versus PostgreSQL trigger, named errors, quoting/comments, full token consumption and role expectations.
- [ ] T045 [P] [US4] Add WebSocket grammar/error/authorization parity in `backend/crates/kalamdb-api/tests/postgresql_subscriptions.rs` and definition-version reuse tests in `backend/crates/kalamdb-core/tests/postgresql_stored_sql.rs`; preserve non-SQL bodies and dynamic SQL submission boundaries.
- [ ] T046 [US4] Create `backend/crates/kalamdb-dialect/tests/postgresql_custom_parse_once.rs` with per-request tokenization/parse counters for every inventoried custom and PostgreSQL-compatibility family, malformed suffixes, comments/quoting, embedded predicates and concurrent requests; extend `backend/crates/kalamdb-core/tests/postgresql_stored_sql.rs` for metadata/bind/retry/definition-cache reuse. Require all custom handlers to consume the same typed payload/Expr/Query without re-lexing remainder text.
- [ ] T047 [US4] Convert storage/flush/compact/manifest/cluster/backup/restore/export/job/live-query commands in `backend/crates/kalamdb-dialect/src/ddl/` to shared-cursor parsing and typed payload mapping; preserve exact inventoried options and delegate execution/storage operations to their existing owners.
- [ ] T048 [US4] Convert all topic/consumer/ack/reset/retention/source and topic-trigger syntax in `backend/crates/kalamdb-dialect/src/ddl/topic_commands.rs` and `create_trigger.rs`; distinguish standard table triggers without claiming their prefixes and retain typed embedded expressions.
- [ ] T049 [US4] Convert schedule create/alter/drop parsing in `backend/crates/kalamdb-dialect/src/ddl/create_schedule.rs`, `alter_schedule.rs` and `drop_schedule.rs`; validate embedded SQL through the shared contract and preserve existing execution options.
- [ ] T050 [US4] Convert subscriptions in `backend/crates/kalamdb-dialect/src/ddl/subscribe_commands.rs` and `backend/crates/kalamdb-api/src/ws/events/subscription.rs` to shared query/predicate ASTs and errors; preserve live-query restrictions and validate quoted identifiers without substring parsing.
- [ ] T051 [US4] Route SQL-bearing views/procedure host calls/schedules through the parsed pipeline in `backend/crates/kalamdb-core/src/functions/host.rs`, `executor.rs`, `lifecycle.rs`, `schedule_store.rs` and `views/mod.rs`; cache stored syntax by object/definition version, reauthorize execution, and invalidate on edits without interpreting non-SQL bodies.
- [ ] T052 [US4] Switch remaining custom dispatch to the typed parser and delete obsolete `backend/crates/kalamdb-dialect/src/parser/extensions.rs`, unused `ddl/parsing.rs` parsing helpers and alternative per-command tokenizers; update `classifier/engine/core.rs` and exports, then prove inventory completeness with the extension/stored-SQL/WS suites.

## Phase 7: US5 — Aliases, producers and explicit upgrade migration (P2)

**Goal**: Make active clients emit canonical SQL and give stored legacy SQL a safe explicit migration path.

**Independent validation**: Aliases have equivalent results; generated SQL passes the new parser; preflight reports legacy objects without writes and explicit migrations survive restart.

- [ ] T053 [US5] Add alias equivalence/negative cases in `backend/crates/kalamdb-dialect/tests/postgresql_aliases.rs` and persisted legacy definition/preflight/migration/restart cases in `backend/crates/kalamdb-core/tests/postgresql_upgrade.rs` covering every SQL-bearing object type found in setup.
- [ ] T054 [US5] Implement namespace/USE/DESCRIBE/product SHOW and retained type/user-extra aliases with shared tokens in `backend/crates/kalamdb-dialect/src/ddl/create_namespace.rs`, `drop_namespace.rs`, `alter_namespace.rs`, `use_namespace.rs`, `describe_table.rs` and related owning modules; map equivalent aliases to canonical payloads, not rewritten SQL.
- [ ] T055 [US5] Update CLI canonical SQL generation and batch boundaries in `cli/src/workflow/schema/typescript/mod.rs`, `sql_batch.rs`, `session/batch.rs` and their tests; ensure tagged dollar strings/comments/quoted semicolons survive and regenerated schemas pass shared parser fixtures.
- [ ] T056 [US5] Replace removed completion/templates in `ui/src/components/sql-studio-v2/input-form/sqlCompletionCatalog.ts` and update `sqlCompletionCatalog.test.ts`; list retained Kalam aliases separately and produce canonical table/schema/user SQL.
- [ ] T057 [US5] Update handwritten SQL producers/fixtures and tests under `link/sdks/typescript/` and `link/sdks/dart/` identified in the inventory, including `typescript/client/tests/browser-test.html` and `typescript/orm/tests/agent-consumer.test.mjs`; regenerate affected generated outputs through their build commands and update each affected package README.
- [ ] T058 [US5] Verify/update bridge-emitted SQL in `pg/src/fdw_ddl.rs` and fixtures in `pg/tests/e2e_ddl/`; test canonical DDL and typed FDW execution parity through `pg/test.sh`, without inventing SQL serialization for `pg/src/remote_executor.rs` typed requests.
- [ ] T059 [US5] Add a read-only stored-SQL upgrade preflight at `backend/crates/kalamdb-core/src/sql/executor/sql_executor/postgresql_upgrade.rs` using existing catalog APIs and the shared parser; expose an offline developer runner at `backend/crates/kalamdb-core/examples/check_postgresql_sql_upgrade.rs` accepting an exported JSON array of object kind/ID/version/SQL records on stdin before server replacement, integrate the same check before startup catalog mutations, report object IDs and safe remediation, and reject affected object execution after upgrade without silently rewriting or deleting definitions.
- [ ] T060 [US5] Write `docs/migrations/postgresql-dialect.md` with pre-upgrade detection, explicit canonical replacements, backup/rollback operational steps and post-restart validation; run `backend/crates/kalamdb-core/tests/postgresql_upgrade.rs` and record evidence in `specs/036-postgresql-sql-dialect/validation.md`.
- [ ] T061 [US5] Update `docs/reference/sql.md` and active repo examples/tests from the inventory; migrate removed forms while preserving labeled negative fixtures/historical records, document retained aliases and exact unsupported matrix entries, and run alias/producer suites.
- [ ] T062 [US5] Update canonical `../kalamdb-skills/skills/kalamdb/SKILL.md` and affected examples, run its mirror-generation workflow, and update corresponding affected SDK documentation starting at `../KalamSite/content/ts-sdk/querying.mdx`, `../KalamSite/content/dart-sdk/querying.mdx` and `../KalamSite/content/rust-sdk/querying.mdx` (record any additional affected pages; resolve `content/sdk/**` if present); record exact files, validation and any unavailable external work in `specs/036-postgresql-sql-dialect/validation.md`.

## Phase 8: US6 — One contributor path and regression guards (P3)

**Goal**: Prevent the split dialect/parser architecture from reappearing.

**Independent validation**: Contributor examples compile against the chosen wrapper/AST API and a guard demonstrably detects injected forbidden parsing paths.

- [ ] T063 [P] [US6] Extend `backend/crates/kalamdb-dialect/tests/postgresql_architecture.rs` with regression checks for delegated PostgreSQL behavior and a synthetic custom command parsed from the same cursor, demonstrating separate upstream/compatibility/Kalam payloads without shared mutable state.
- [ ] T064 [P] [US6] Document the implemented parser/wrapper/AST-planning flow, ownership, cache and compatibility boundaries in `docs/development/how-to-add-sql-statement.md` and new `docs/architecture/postgresql-dialect.md`; clearly explain the upstream hook return limit and required tests for each new command.
- [ ] T065 [US6] Implement `scripts/check-sql-parser-boundaries.py` to inventory alternative production dialect construction, raw user-SQL DataFusion text calls, string-prefix SQL dispatch and AST-to-text reparses; add narrow documented reference-test exceptions and negative guard fixtures in `scripts/tests/test_sql_parser_boundaries.py`.
- [ ] T066 [US6] Wire the parser-boundary guard into the existing workflow that owns backend checks in `.github/workflows/ci.yml`, run guard self-tests and the architecture suite, and reconcile contributor examples/capability docs with implemented behavior in `specs/036-postgresql-sql-dialect/validation.md`.
- [ ] T067 [US6] Complete the deletion ledger in `specs/036-postgresql-sql-dialect/cleanup.md` and remove any last obsolete `backend/crates/kalamdb-dialect/src/parser/system.rs` stub, legacy dispatch/rewrite/helper/export/model/test/flag paths identified there; delete parser-only unused dependencies/features from `Cargo.toml` and owning crate manifests only after caller checks. Preserve recursive protection, needed domain models/catalog shims and non-parser dependency consumers; extend `scripts/check-sql-parser-boundaries.py` to guard removed paths and duplicated lexers.
- [ ] T068 [US6] Add deterministic hostile/property cases in `backend/crates/kalamdb-dialect/tests/postgresql_hostile_input.rs` and a bounded wrapper/visitor fuzz target in `backend/crates/kalamdb-dialect/fuzz/fuzz_targets/postgresql_parser.rs` with its `fuzz/Cargo.toml`; reuse a workspace-declared fuzz dependency if needed, seed standard/custom/deep/Unicode/malformed cases, preserve resource limits, run a 120-second smoke campaign and retain minimized failures with runtime/toolchain evidence in `specs/036-postgresql-sql-dialect/validation.md`.

## Phase 9: Polish and release gates

**Goal**: Prove full migration readiness; no story completion alone authorizes releasing a partial dialect migration.

**Independent validation**: All required corpus cases, integration/security/smoke/performance gates and documentation deliverables are complete.

- [ ] T069 Audit every inventoried parser/producer path and every matrix fixture against `specs/036-postgresql-sql-dialect/contracts/capability-matrix.md`; remove remaining alternative runtime parse paths, fill observed-result evidence in `validation.md`, and confirm no required case was silently downgraded.
- [ ] T070 Run one captured dev-profile server check after the final edit batch, the affected dialect/core/API/backend/wire nextest suites and parser-boundary guard; verify error positions, auth/system/tenant restrictions and async timeout annotations, recording results in `specs/036-postgresql-sql-dialect/validation.md`.
- [ ] T071 Start the backend, prebuild `kalam-cli`, run `cargo nextest run -p kalam-cli-e2e --test e2e smoke` from `cli` without --no-fail-fast, then run applicable wire/WS/bridge/SDK/UI integration suites and unchanged GUI fixtures; record prerequisites and outcomes in `specs/036-postgresql-sql-dialect/validation.md`.
- [ ] T072 Rerun `backend/crates/kalamdb-core/tests/postgresql_pipeline_perf.rs` under the baseline environment; require SC-009 thresholds, record every relevant runtime in seconds and allocations/cache/parse counts, and resolve regressions before marking `specs/036-postgresql-sql-dialect/validation.md` passed.
- [ ] T073 Run injection, custom parse-count, safe-ownership and at least 10,000 mixed lifecycle/admission tests from `specs/036-postgresql-sql-dialect/quickstart.md`; verify dependency recursion/logging settings, byte-budget enforcement, post-cleanup live ownership and zero warm source/AST copies; reconcile `cleanup.md` and record SC-014–SC-017 results in `validation.md` without claiming safety solely from Rust or parser acceptance.
- [ ] T074 Reconcile `specs/036-postgresql-sql-dialect/checklists/requirements.md`, `validation.md`, contracts and canonical docs with actual evidence; rerun speckit-analyze on spec/plan/tasks, and leave external-doc or unavailable-test gates explicitly outstanding rather than marking the feature shipped.

## Dependencies and execution order

```text
Setup → Foundation → US1 → US2 → US3 → US4 → US5 → US6 → Final gates
```

This is a shared-parser migration; ordered integration avoids contradictory live dispatch. US1 and US2 use the internal typed executor before US3 changes public standard-SQL entry points. US3 reruns earlier acceptance suites. US4 requires that AST pipeline for embedded expressions and nested SQL. US5 requires the completed grammar to validate producer/upgrade fixtures. US6 codifies the final boundary. Each story has an independent validation suite; no story is an independently releasable public dialect mode.

Delete obsolete paths in the slice removing their last caller; the US6 ledger sweep is verification and removal of residuals, not permission to keep duplicates until release. The new safety/efficiency tasks are sequential dependencies in their phases.

Within each phase: create tests first, implement models before dependent converters/handlers, then integrate and run the focused suite. Sequential tasks may modify the same file. [P] pairs can execute concurrently only within their phase.

## Parallel execution examples by story

- **US1**: T014 + T015 may run together after the prior phase; join before implementation.
- **US2**: no task-level parallel edit is marked; shared parser/catalog or migration dependencies require sequential execution. Once implemented, independent fixture cases may be exercised concurrently by the test harness.
- **US3**: T029 + T030 may run together after the prior phase; join before implementation.
- **US4**: T044 + T045 may run together after the prior phase; join before implementation.
- **US5**: no task-level parallel edit is marked; shared parser/catalog or migration dependencies require sequential execution. Once implemented, independent fixture cases may be exercised concurrently by the test harness.
- **US6**: T063 + T064 may run together after the prior phase; join before implementation.

## Requirement coverage

Coverage is planned work, not completion evidence. Multiple tasks may contribute to a requirement; the final gates rerun integrated behavior.

| Requirement | Tasks |
|---|---|
| FR-001 | T006, T007, T029, T032, T033, T035, T043, T069 |
| FR-002 | T008, T009, T029, T031, T043, T069 |
| FR-003 | T004, T014, T016, T017, T022, T023, T026, T043, T052, T069 |
| FR-004 | T014, T017, T027 |
| FR-005 | T025, T026, T027, T028 |
| FR-006 | T025, T027, T028, T053, T054 |
| FR-007 | T053, T054, T061 |
| FR-008 | T015, T019, T021, T022, T053, T054, T061 |
| FR-009 | T014, T018 |
| FR-010 | T044, T048, T052 |
| FR-011 | T007, T044, T047, T048, T049, T050, T052, T054, T069 |
| FR-012 | T029, T035 |
| FR-013 | T002, T016, T029, T036 |
| FR-014 | T003, T004, T005, T007, T008, T009, T010, T011, T013, T030, T031, T032, T033, T034, T036, T043, T045, T046, T049, T050, T051, T052, T063, T065, T072 |
| FR-015 | T005, T007, T011, T016, T018, T027, T038, T044, T047, T048, T052, T059, T070 |
| FR-016 | T008, T015, T017, T020, T023, T034, T039, T045, T047, T050, T051, T070 |
| FR-017 | T053, T055, T056, T057, T060, T061, T062, T071 |
| FR-018 | T063, T064, T065, T066 |
| FR-019 | T025, T026, T028 |
| FR-020 | T014, T023, T024, T032, T033, T043, T058, T071 |
| FR-021 | T001, T002, T004, T014, T024, T029, T036, T037, T061, T066, T069, T074 |
| FR-022 | T006, T010, T011, T015, T018, T019, T020, T026, T039, T044, T055, T070 |
| FR-023 | T015, T019, T020, T021, T022, T030, T032, T070 |
| FR-024 | T001, T010, T022, T032, T033, T043, T045, T049, T050, T051, T052, T055, T057, T058, T069, T071 |
| FR-025 | T006, T027, T029, T035, T037, T054, T065, T069 |
| FR-026 | T009, T030, T031, T033, T034, T039, T051, T070 |
| FR-027 | T001, T053, T059, T060, T074 |
| FR-028 | T005, T011, T015, T022, T023, T030, T032, T037, T038, T045, T050, T059, T070 |
| FR-029 | T002, T003, T006, T007, T010, T013, T024, T028, T029, T038, T039, T043, T044, T045, T058, T063, T065, T066, T070, T071, T072 |
| FR-030 | T056, T057, T060, T061, T062, T064, T066, T071, T074 |
| FR-031 | T001, T052, T067, T069, T073 |
| FR-032 | T004, T007, T012, T016, T047, T048, T049, T050, T063, T064, T067, T073 |
| FR-033 | T005, T006, T012, T031, T034, T040, T042, T045, T068, T070, T073 |
| FR-034 | T017, T023, T034, T038, T039, T040, T041, T068, T073 |
| FR-035 | T007, T010, T044, T045, T046, T051, T052, T068, T073 |
| FR-036 | T001, T003, T012, T040, T042, T072, T073 |
| SC-001 | T014, T021, T023, T024, T032, T043, T071 |
| SC-002 | T014, T017, T024, T025, T027, T028 |
| SC-003 | T053, T055, T056, T057, T061, T062, T071 |
| SC-004 | T029, T035, T071 |
| SC-005 | T002, T029, T036, T071 |
| SC-006 | T014, T016, T018, T024, T043, T052, T070 |
| SC-007 | T006, T063, T064, T065, T066 |
| SC-008 | T029, T035, T043, T065, T069 |
| SC-009 | T003, T072 |
| SC-010 | T001, T002, T024, T028, T029, T037, T038, T058, T061, T066, T069, T071, T074 |
| SC-011 | T004, T007, T009, T010, T013, T030, T031, T032, T033, T034, T043, T045, T046, T051, T052, T070, T072 |
| SC-012 | T015, T020, T021, T023, T030, T034, T039, T045, T051, T070 |
| SC-013 | T053, T059, T060, T074 |

| SC-014 | T001, T052, T065, T067, T069, T073 |
| SC-015 | T003, T012, T040, T042, T068, T072, T073 |
| SC-016 | T039, T040, T041, T068, T070, T073 |
| SC-017 | T003, T042, T046, T068, T072, T073 |

## Implementation strategy

1. Capture the unchanged baseline and exercise the foundation APIs before production cutover.
2. First demonstration milestone: US1 standard DDL/session tests on the internal typed pipeline. PostgreSQL migration MVP: Foundation + US1 + US2 + US3, including AST planning and removal of the public DuckDB path.
3. Complete US4 and US5 before shipping the replacement: existing extensions and stored/client SQL must remain supported or explicitly migrated.
4. Finish US6 guards and all cross-cutting gates. Resolve runtime gaps or explicitly amend requirements; do not hide them by weakening tests or substituting GUI queries.
5. External canonical skills/site updates are delivery work. If the expected directory moved, resolve the corresponding maintained file; if unavailable, record the exact missing deliverable and do not claim it validated.

## Task summary

- Total: 74 tasks.
- Phase 1: Setup: 4 tasks.
- Phase 2: Foundation: 9 tasks.
- US1: 11 tasks.
- US2: 4 tasks.
- US3: 15 tasks.
- US4: 9 tasks.
- US5: 10 tasks.
- US6: 6 tasks.
- Phase 9: Polish and release gates: 6 tasks.
- Parallel-marked tasks: 8.
