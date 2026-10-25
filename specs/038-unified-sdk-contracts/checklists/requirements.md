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
- [x] Concrete blockers are named in the audit and tasks: MessagePack leftovers, owned FlexBuffers persistence, exact-value binding, and dependency-only ownership checks. Generated TS/Dart SQL fallback is no longer the stated blocker.
- [x] Preliminary codec-report closure conclusions are withdrawn by the audit; existing observations are retained as attributed history, not silently treated as real-library benchmarks.
- [x] The spec and tasks require automated Markdown/link/task-reference validation and name the command that must be recorded. Execution of that command stays on the task list, not this requirements checklist.
- [x] The spec requires public examples to compile and run against generated clients and a real server, and says typecheck fixtures are not that proof. That execution stays on T044/T117.
- [x] The spec requires runtime, SDK, and external docs to match shipped behavior, and says inaccessible external destinations must be recorded rather than claimed. That reconciliation stays on T119.

## Release gates specified, not executed here

These items check that each gate is written as an acceptance requirement. They do not record that the gate has run. Open tasks own the execution evidence.

- [x] A: the spec requires one catalog/type/naming authority and paired generation coverage.
- [x] S: the spec requires bounded parsing, decompression, fuzz/property tests, and native-handle ownership, and it credits only the narrow repairs already named.
- [x] D: the spec requires actual pinned Postcard and Prost comparisons with equal semantics, and it rejects a winner claim from handwritten proxies.
- [x] C0: the spec requires a lean codec, native/WASM/FRB conformance, and a production-callsite ownership check with a failing negative control.
- [x] B2: the spec requires generated TS/Dart calls through the function route, exact values and errors, and permission parity. It does not treat a template string as that proof.
- [x] W: the spec requires one binary live path and removal of production MessagePack, with socket tests. A frame unit test is not defined as the whole gate.
- [x] P: the spec requires one fresh owned persistence/Raft codec, old-epoch rejection, and unchanged durability. External formats stay named exceptions.
- [x] APP: the spec requires DX-01..04 on the school and helpdesk paths, with equivalent-scope measurements.
- [x] STABLE: the spec requires DB-01..20, including statement rollback, faults, RLS, replay, and independent restore.
- [x] R: the spec requires pre-registered CPU, memory, throughput, and p99 targets after APP/STABLE, and it forbids hidden skipped tests.

## Proof required to close a task or gate

Record implementation commit, production entry points and call chain, exact executed commands, platform/features/configuration, raw artifacts, negative controls, failures/skips/limitations and dependency status. Test source alone is not execution evidence; a helper's roundtrip cannot prove production uses it. Claims and checkmarks must never outrun this record.

This correction edits documentation only. No Rust/TS/Dart/server/fuzz/cluster/restore/performance test or user-data reset was performed. Existing reports are attributed, not independently rerun. Finite test results are evidence within their scope, not a guarantee of no defects.
