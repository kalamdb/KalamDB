# Specification quality and release checklist

**Revised:** 2026-10-24. **Feature:** [spec.md](../spec.md). **Status:** implementation and release acceptance reopened after [audit](../implementation-audit-2026-10-24.md) at `81e5fb15dc600d268581c1b22ea25b74e25b77f1`. A documentation decision checkbox does not mean its runtime requirement is implemented.

## Decisions documented

- [x] Fresh-start release: no historical-data migration, old-format readers, API aliases, mixed-release rollout, compatibility window or temporary MessagePack adapter.
- [x] Old stores/clients/peers receive explicit epoch errors. No automatic deletion, reformatting or reinterpretation is authorized.
- [x] Newly accepted data requires crash recovery, replay, equivalent durability, independent current-format restore and same-epoch schema correctness.
- [x] P0 correctness/security, P1 measured CPU/memory/throughput and simple app development, P2 optional zero-copy are separate. Existing lifetime defects remain P0.
- [x] The target requires one lightweight Rust-owned domain-value codec shared by backend and Link/native/WASM/FRB, with endpoint-specific kind allowlists. This target is NOT yet fully integrated.
- [x] Actual pinned library comparisons must justify one production codec; handwritten Postcard/Prost-style approximations cannot establish the named libraries' performance or Gate D/R completion.
- [x] Source observations, prior attributed test reports, proposed targets and independently measured outcomes must remain distinguishable.
- [x] Application contracts retain auth/organizations, request-scoped SSR, optimistic state, queries/pagination, files/jobs, real test helpers, deployment and diagnostics without another runtime/ORM/cache.
- [x] APP/STABLE require DX-01..04 and DB-01..20 evidence within an explicit supported scope; no universal superiority, security or serializability claim follows from checkboxes or Raft alone.

## Reconciliation and artifact review

- [x] Main specification and plan no longer claim all gates/tasks complete; they link the dated source/evidence audit and concrete remaining paths.
- [x] Task IDs and FR/SC acceptance remain intact. Core tasks T001-T093 plus T094-T121 continuation retain 12 historical/narrow-supported records and reopen 109 acceptance records.
- [x] Reopened records distinguish source-contradicted/partial, evidence-pending and optional P2; missing evidence is not a claim that all related implementation is absent.
- [x] The audit credits KDOM/HTTP progress, optional backend dependencies, parsing-budget fixes and the generation-fenced native-handle registry instead of erasing that work.
- [x] Concrete blockers include actual MessagePack WS callers, generated SQL fallback/Dart calls, separate FlexBuffers/MessagePack owned persistence, incomplete exact-value binding and dependency-only ownership checks.
- [x] Preliminary codec-report closure conclusions are withdrawn by the audit; existing observations are retained as attributed history, not silently treated as real-library benchmarks.
- [ ] Run automated Markdown/link/fence/unique-task/reference validation against the final repository checkout and attach command/output. Earlier reports are historical; this correction does not claim to rerun them.
- [ ] Compile and execute all public examples against actual generated clients and a real server. Model-only/typecheck fixtures do not establish RPC/WS or application integration.
- [ ] Complete T119 runtime/SDK/external docs reconciliation when actual implementation lands. A specification correction does not update inaccessible external destinations or make proposed code real.

## Release gates - open until acceptance evidence exists

- [ ] A: full catalog/type/naming semantics and paired generation coverage; existing metadata work retained.
- [ ] S: actual unary/body and aggregate parsing/conversion/decompression bounds, full-path fuzz/property/advisory evidence and native ownership acceptance. Narrow T064/T066 repairs remain credited.
- [ ] D: actual pinned candidate libraries, equivalent semantics/framing/validation and reproducible results; no production-winner conclusion from handwritten proxies.
- [ ] C0: lean dependencies plus real native/WASM/FRB conformance and effective production-callsite ownership CI with failing negative controls.
- [ ] B2: mandatory generated TS/Dart native calls through actual SDK bridges; no SQL fallback, exact descriptors/values/errors, permission/transaction parity and measured resource evidence.
- [ ] W: actual server and all maintained clients use the common binary live/control/ack/replay codec; all production MessagePack paths removed, with real socket/platform tests.
- [ ] P: one selected codec for fresh owned persistence/Raft values, old epoch rejection and unchanged-durability crash/corruption/restore evidence. External ecosystem formats remain narrow exceptions.
- [ ] APP: real DX-01..04 school/helpdesk/setup/testkit workflows and equivalent-scope developer/resource measurements.
- [ ] STABLE: DB-01..20 evidence, including statement rollback, concurrency histories, leader/storage faults, hot/cold invariants, RLS, replay, SDK lifecycle and independent restore.
- [ ] R: pre-registered CPU/allocated/retained-memory/throughput/p99 targets, APP/STABLE acceptance and no unresolved wrong-result/data-loss/security/false-atomicity failure or hidden skipped test.

## Proof required to close a task or gate

Record implementation commit, production entry points and call chain, exact executed commands, platform/features/configuration, raw artifacts, negative controls, failures/skips/limitations and dependency status. Test source alone is not execution evidence; a helper's roundtrip cannot prove production uses it. Claims and checkmarks must never outrun this record.

This correction edits documentation only. No Rust/TS/Dart/server/fuzz/cluster/restore/performance test or user-data reset was performed. Existing reports are attributed, not independently rerun. Finite test results are evidence within their scope, not a guarantee of no defects.
