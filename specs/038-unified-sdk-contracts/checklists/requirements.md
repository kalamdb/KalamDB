# Specification quality and release checklist

**Revised:** 2026-10-22. **Feature:** [spec.md](../spec.md). Checkmarks concern reviewed specification decisions only, not runtime execution.

## Decisions documented

- [x] Fresh-start release: no historical data preservation, old-format readers, API aliases, mixed-release rollout, migration window or temporary MessagePack adapter.
- [x] Old stores/clients/peers receive explicit epoch errors. No automatic deletion, reformatting or reinterpretation is authorized.
- [x] Newly accepted data still requires crash recovery, replay and equivalent durability, plus current-format backup restore and same-epoch schema correctness.
- [x] P0 correctness/security, P1 measured resources/throughput and simple app development, P2 optional zero-copy are separate. Existing unsafe lifetime defects remain P0.
- [x] Backend and native/WASM/FRB Link share one lightweight Rust-owned domain-value codec and one SQL type authority. Public endpoints allowlist message kinds.
- [x] Codec choice is conditional on equivalent-semantics comparisons of the hardened Kalam candidate, Postcard and Prost; one production winner only.
- [x] Existing research and performance targets are labeled source-observed/proposed, not achieved speedups or security certification.
- [x] Application contract covers setup, auth/organizations, request-scoped SSR, UI mutations, queries/pagination, files/jobs, real test helpers, deployment and diagnostics without a second runtime/ORM/cache.
- [x] App readiness distinguishes required, conditional and deferred capabilities; no universal competitor-superiority or complete-platform claim.
- [x] Stability contract states atomicity/isolation/durability scopes and DB-01..DB-20 oracles, rather than assuming Raft makes SQL serializable.
- [x] Convex, Supabase, Firebase and Neon patterns plus SQLite, FoundationDB, PostgreSQL, Jepsen and SQLancer testing references are documented with limitations in research-app-platforms.md.

## Artifact review

- [x] Core spec owns FR-001..FR-062 / SC-001..SC-022; app-readiness.md normatively adds FR-063..FR-080 / SC-023..SC-032 without duplicate definitions.
- [x] Core tasks retain T001..T093 and seven historical Slice 1 markers. tasks-app-stability.md adds unchecked T094..T121; final T050 and Gate R require T121 APP/STABLE evidence.
- [x] Current quickstart has been rewritten in this pass, removing the earlier blocked-write gap and stale instructions to stop at Slice 1 or preserve a MessagePack migration window.
- [x] Main spec and plan explicitly link all new application/stability contracts and the task continuation. Existing API/protocol/performance contracts remain normative companions.
- [x] Documentation-only scope: no runtime code, generated source, production configuration or user data is changed by this revision.
- [ ] Run automated repository Markdown/link/fence/task-reference validation. This pass used connector content and diff review; a local checkout attempt could not resolve github.com, so no local validator or runtime suite is claimed.
- [ ] Compile and execute every example against actual generated/current SDKs. Prose snippets are proposed interfaces until fixtures pass.
- [ ] T119 updates the existing SDK/runtime docs and externally maintained docs/skills when the relevant implementation lands; do not mark those separate destinations updated by this spec-only commit.

## Release gates - evidence pending

- [ ] S: parsing-time bounds, strict decompression and native ownership remediation; full-path fuzz/property/advisory evidence.
- [ ] D/C0: pre-registered finalist comparison, selected codec, native/WASM/FRB conformance and lean dependencies.
- [ ] B2: direct typed routine execution, exact values, SQL/native permission/transaction parity and no generated SQL or binary JSON detours.
- [ ] W: one binary live/control/ack/replay path, bounded fanout/queues and complete runtime MessagePack removal.
- [ ] P: fresh owned persistence/Raft format, epoch rejection and unchanged-durability recovery/corruption evidence.
- [ ] APP: real DX-01..DX-04 school/helpdesk/setup/testkit workflows, secure request/session scopes, supported platforms, explicit conditional gaps and equivalent-scope boilerplate/resource measurements.
- [ ] STABLE: DB-01..DB-20 within advertised scope, including failed multi-row statement rollback, contested booking/isolation, leader/storage faults, hot/cold invariants, RLS, replay, SDK lifecycle and independent backup restore.
- [ ] R: existing CPU/memory/fixed-SLO throughput/p99 targets plus APP/STABLE evidence, raw measurements, no unresolved wrong-result/data-loss/security/false-atomicity defects or silently skipped tests.

No Rust, TS, Dart, server, fuzz, cluster, restore or performance tests were run by this documentation update. Finite tests are evidence within a stated failure model, not proof that a database has no defects. Historical review notes remain background evidence; current acceptance is defined by the linked spec, plan, contracts and both task files.
