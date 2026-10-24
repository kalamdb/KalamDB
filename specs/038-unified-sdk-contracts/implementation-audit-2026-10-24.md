# Spec 038 implementation reconciliation - 2026-10-24

**Audited remote commit:** `81e5fb15dc600d268581c1b22ea25b74e25b77f1`, branch `feat/pg-dialect`, PR #402.  
**Method:** review of production callers, generator templates, codec/binder source and repository evidence. No runtime, fuzz, performance, browser, Flutter or database recovery test was run by this documentation correction. Uninspected functionality is not presumed absent.

## Finding and status policy

The earlier specification commits are in the branch's ancestry and FR-048/051/052 still require shared RPC/WS/persistence encoding and complete MessagePack removal. The problem is not a missing push. The audited head is a documentation-only blanket closure commit, while production callers still contradict several closed tasks.

Preserve the implementation already present: KDOM domain framing, optional backend codec dependencies, binary HTTP routine dispatch, richer catalog/query metadata, parser-budget fixes and native-handle remediation. Finish integration using these owners; do not create another serializer, executor or SDK state engine.

A checked task means its entire stated acceptance has support. This correction distinguishes:

- **H (historical retained):** seven original Slice 1 records, not rerun here.
- **R (narrow record retained):** inspected implementation/artifact plus attributed prior checks supports the bounded task. Prior test reports are not represented as independently executed here.
- **C (contradicted/partial):** a named production path contradicts at least one acceptance condition.
- **E (acceptance evidence pending):** whole-task evidence was not established in the reviewed artifacts. Implementation may exist; locate it and attach evidence before rebuilding anything.
- **O (optional deferred):** P2 work not established complete and not required by mandatory release gates.

Retain checked: **T001, T002, T018, T019, T038, T045, T046, T048, T051, T064, T066, T069**. Reopen the other **109** tasks, including optional T059/T092. This is 109 reopened acceptance records, NOT a claim that 109 features are unimplemented. Retain all requirement/task IDs and acceptance scope. The blanket release closure is withdrawn; no runtime code or user data is changed.

## Source observations and required corrections

Paths below refer to the audited commit. Links are pinned so later changes do not rewrite the evidence.

### A01 - Shared core exists; end-to-end integration does not follow from an export

[domain.rs](https://github.com/kalamdb/KalamDB/blob/81e5fb15dc600d268581c1b22ea25b74e25b77f1/backend/crates/kalamdb-serialization/src/domain.rs) defines KDOM, scalar variants, decode budgets, endpoint surfaces and RPC/WS/durable/Raft kinds. [Cargo.toml](https://github.com/kalamdb/KalamDB/blob/81e5fb15dc600d268581c1b22ea25b74e25b77f1/backend/crates/kalamdb-serialization/Cargo.toml) makes Arrow/DataFusion optional; [Link lib.rs](https://github.com/kalamdb/KalamDB/blob/81e5fb15dc600d268581c1b22ea25b74e25b77f1/link/link-common/src/lib.rs) re-exports the domain codec. Credit T069's extraction and the prior reported checks.

Exporting an encoder and defining WsChanges do not connect actual WS/SDK callers. Keep T070/T071 integration and gates open. KDOM is the implemented candidate, not a proven fastest winner. No competing production codec should be introduced without the agreed decision evidence.

### A02 - Actual WebSocket paths still use MessagePack

[Backend messages](https://github.com/kalamdb/KalamDB/blob/81e5fb15dc600d268581c1b22ea25b74e25b77f1/backend/crates/kalamdb-api/src/ws/messages.rs) decodes SerializationType::MessagePack through rmp_serde::from_slice. [Protocol options](https://github.com/kalamdb/KalamDB/blob/81e5fb15dc600d268581c1b22ea25b74e25b77f1/backend/crates/kalamdb-commons/src/websocket_protocol.rs) still exposes Json/MessagePack and JSON/gzip defaults. [Native Link](https://github.com/kalamdb/KalamDB/blob/81e5fb15dc600d268581c1b22ea25b74e25b77f1/link/link-common/src/connection/websocket.rs) and [WASM helpers](https://github.com/kalamdb/KalamDB/blob/81e5fb15dc600d268581c1b22ea25b74e25b77f1/link/kalam-link-wasm/src/helpers.rs) still parse/send MessagePack.

Reopen T073-T078 and Gate W. Wire both directions and every auth/control/data/error/ack/resume variant through the selected shared codec. Regenerate Dart/FRB and WASM bindings from source. Remove old paths directly, with no compatibility window. Real socket tests must prove that production callers reach the shared encoder/decoder; an isolated frame roundtrip is insufficient.

### A03 - Generated function calls still synthesize SQL

[TS template](https://github.com/kalamdb/KalamDB/blob/81e5fb15dc600d268581c1b22ea25b74e25b77f1/cli/templates/schema-gen/typescript/src/generated/kalam.ts.template) declares invokeRoutine optional, falls back to client.query(call_sql), casts the response, retains spread namespace aliases and lacks the complete generated signature/codec/revision descriptor. [Dart template](https://github.com/kalamdb/KalamDB/blob/81e5fb15dc600d268581c1b22ea25b74e25b77f1/cli/templates/schema-gen/dart/lib/generated/kalam.dart.template) imports kalam_sync and emits KalamFunctions calling _client.query. [Dart generator](https://github.com/kalamdb/KalamDB/blob/81e5fb15dc600d268581c1b22ea25b74e25b77f1/cli/src/workflow/schema/dart/functions.rs) still prepares call_sql and params_list.

Make the native typed executor mandatory and implement it in the actual TS/Dart SDKs and shared bridges. Remove generated SQL fallback/aliases, split function-only dependencies, validate actual results and regenerate fixtures. Tests must use normal clients, not a fake client supplying invokeRoutine. Intentional SQL/PGWire remains a product feature. T006-T010/T042/T049/T054 and dependent gates are incomplete.

### A04 - Binary HTTP is present; binder and type fidelity need further work

[invoke.rs](https://github.com/kalamdb/KalamDB/blob/81e5fb15dc600d268581c1b22ea25b74e25b77f1/backend/crates/kalamdb-api/src/http/functions/invoke.rs) decodes KDOM and calls FunctionService: preserve that progress. Its magic/octet-stream detection, optional revision header and JSON errors on the binary path need an explicit current-epoch content/error policy and tests. Transport-level auth/HTTP failures must be distinguished from typed routine errors; the decoder must not guess another format after a failed mutation.

[bind.rs](https://github.com/kalamdb/KalamDB/blob/81e5fb15dc600d268581c1b22ea25b74e25b77f1/backend/crates/kalamdb-core/src/functions/bind.rs) finds the first Composite slot without rejecting all duplicate/unknown slots. Omitted nullable input handling differs from the Array branch. Verify unambiguous descriptor-hash framing and transitive type/layout/default/security dependency invalidation, rather than only concatenated metadata/type names.

[convert.rs](https://github.com/kalamdb/KalamDB/blob/81e5fb15dc600d268581c1b22ea25b74e25b77f1/backend/crates/kalamdb-core/src/functions/convert.rs) infers array type from the first value, defaults an empty list to Utf8, converts composite slots to numeric field names and has temporal truncation/saturation paths. Some mappings collapse omission, null or void. These are source-observed conformance risks, not reproduced exploits. Decode against recursive authoritative descriptors; test empty/null-first arrays, named composites, domains/enums, local-vs-instant timestamps, exact precision/range, void and set results. Reject unsupported loss rather than silently converting. Reopen T005/T008/T052-T057/T060/T063 acceptance.

### A05 - Owned persistence still has competing value encodings

[object/generic.rs](https://github.com/kalamdb/KalamDB/blob/81e5fb15dc600d268581c1b22ea25b74e25b77f1/backend/crates/kalamdb-serialization/src/object/generic.rs) uses FlexBuffers for generic values. [protocol.rs](https://github.com/kalamdb/KalamDB/blob/81e5fb15dc600d268581c1b22ea25b74e25b77f1/backend/crates/kalamdb-serialization/src/protocol.rs) uses MessagePack for StateMachine and FlexBuffers for other payloads, plus separate packed framing. Distinct envelope kinds can be legitimate; competing domain-value serializers do not meet FR-048/051/053.

T080/T081 and Gate P stay open. Inventory actual store/apply/stream/snapshot callers and move owned payloads to the selected core, removing historical readers and unused dependencies. Start fresh without automatically deleting directories. External RocksDB containers, Parquet, PGWire and tonic framing remain narrowly defined interfaces, not targets of a blind text deletion.

### A06 - Gate D did not benchmark actual Postcard and Prost libraries

[domain.rs comparison helpers](https://github.com/kalamdb/KalamDB/blob/81e5fb15dc600d268581c1b22ea25b74e25b77f1/backend/crates/kalamdb-serialization/src/domain.rs) implement encode_postcard_candidate/read_postcard_val and encode_prost_dynamic_candidate/read_prost_val locally. These are handwritten approximations, not measured implementations of the named crates. The [preliminary report](../../link/tests/sdk-contracts/gate-d-codec-selection.md) compares framed/budgeted KDOM with unframed candidates and a synthetic timing loop; it does not establish the full CPU, allocation, retained-memory, p99, platform or integration gates.

Preserve those observations as attributed preliminary data but withdraw Gate D/R closure and production-winner conclusions. Reopen T082 and downstream benchmark gates. Use actual pinned libraries with equivalent supported semantics, framing, budgets, exact types and conversion costs, repeated raw results and uncertainty. The existing KDOM implementation remains a candidate to improve, not a mandatory rewrite or an excuse to add several production serializers.

### A07 - Ownership checks do not inspect the actual forbidden callers

[check_codec_ownership.py](https://github.com/kalamdb/KalamDB/blob/81e5fb15dc600d268581c1b22ea25b74e25b77f1/link/tests/sdk-contracts/check_codec_ownership.py) checks a dependency tree and that Link's manifest mentions kalamdb-serialization. It does not scan production codec calls. [check-serialization-boundary.py](https://github.com/kalamdb/KalamDB/blob/81e5fb15dc600d268581c1b22ea25b74e25b77f1/scripts/check-serialization-boundary.py) remains persistence-focused and exempts API/WebSocket paths.

T071/T077/T088 require callsite/dependency enforcement across backend, Link, WASM/FRB and generated templates, with narrow external exceptions. Add a negative control that reintroduces a forbidden WS serializer and makes CI fail. A dependency-only success cannot close Gate W/P.

### A08 - Real hardening fixes do not establish the whole security gate

[row/value.rs](https://github.com/kalamdb/KalamDB/blob/81e5fb15dc600d268581c1b22ea25b74e25b77f1/backend/crates/kalamdb-serialization/src/row/value.rs) now enforces parsing budgets; [row/batch.rs](https://github.com/kalamdb/KalamDB/blob/81e5fb15dc600d268581c1b22ea25b74e25b77f1/backend/crates/kalamdb-serialization/src/row/batch.rs) now traverses nested arrays. [V8 conversion](https://github.com/kalamdb/KalamDB/blob/81e5fb15dc600d268581c1b22ea25b74e25b77f1/backend/crates/kalamdb-functions/src/abi/convert.rs) replaces raw boxed handles with a bounded generation-fenced registry. Retain narrow T064/T066 records and their [attributed tests](../../link/tests/sdk-contracts/gate-s-evidence.md); do not repeat the obsolete claim that these fixes are absent.

T065/T067/T068 remain open for whole-task acceptance: compression policy/integrity, full-path campaigns, aggregate work/allocation budgets, actual resolved advisory review and unary RPC body limits. Multipart tests do not establish the separate Bytes extractor's pre-buffering behavior. Verify direction-specific endpoint allowlists and charges for post-decode conversion. Named unit tests are useful but not substitutes for the required fuzz/platform campaign. Prior reports were read, not rerun here.

### A09 - UI and bridge criteria are not yet satisfied

[TS client](https://github.com/kalamdb/KalamDB/blob/81e5fb15dc600d268581c1b22ea25b74e25b77f1/link/sdks/typescript/client/src/client.ts) still parses JSON strings for live callbacks and exposes lastVersionId checkpoints. [React mutation helper](https://github.com/kalamdb/KalamDB/blob/81e5fb15dc600d268581c1b22ea25b74e25b77f1/link/sdks/typescript/react/src/hooks/useMutationState.ts) still uses Sets for same-key pending update/delete state. [Bulk Arrow conversion](https://github.com/kalamdb/KalamDB/blob/81e5fb15dc600d268581c1b22ea25b74e25b77f1/backend/crates/kalamdb-serialization/src/row/batch.rs) still builds Vec<Vec<ScalarValue>> before arrays.

Reopen the corresponding T023/T024/T029/T030/T086/T098 acceptance. Preserve existing live/Drift work; implement real delivery cursors, measured bulk bridges and overlapping mutation state without a second cache. Row versions do not prove durable delivery progress. JSON removal and fewer copies require measured end-to-end benefit, not a label.

### A10 - Scaffolding does not prove APP/STABLE/R

[check_generated.py](https://github.com/kalamdb/KalamDB/blob/81e5fb15dc600d268581c1b22ea25b74e25b77f1/link/tests/sdk-contracts/check_generated.py) generates/types/checks model fixtures. It does not start a real backend, exercise binary RPC/WS, run school/helpdesk apps, inject cluster/storage faults or restore backups. Retain T002's fixture work without promoting it to those gates.

Reviewed evidence included baseline-measurements.md, gate-s-evidence.md, gate-d-codec-selection.md and t051-inventory.md. Those records do not establish all DX-01..04, DB-01..20 or Gate R outcomes. The prior exact-commit PR-triggered workflow query returned no runs; that limited result is NOT proof that no other CI/manual tests ran. Reopen evidence-dependent acceptance until pinned implementations, commands, logs, platforms, seeds, limits and skipped cases are attached. Reuse real tests that already exist elsewhere.

## Disposition ledger

| State | IDs | Basis |
| --- | --- | --- |
| H checked | T001, T018, T019, T038, T045, T046, T048 | Original Slice 1 record, not current-cohort certification |
| R checked | T002, T051, T064, T066, T069 | Narrow source/artifact support as above |
| C open | T005, T006, T007, T008, T009, T010, T042, T049, T052, T053, T054, T055, T056, T057, T062 | A03/A04 native SDK, binding and exact-type/clean-break gaps |
| C open | T023, T024, T029, T030, T070, T071, T073, T074, T075, T077, T078, T080, T081, T086, T088, T098, T100, T119 | A01/A02/A05/A07/A09 transport, ownership, type/UI and documentation gaps |
| C open | T050, T060, T063, T082, T093, T121 | Contradicted prerequisites or invalid completion/selection evidence |
| O open | T059, T092 | Optional P2, not mandatory dependencies |
| E open | Every remaining ID in T001-T121 | Whole-task evidence not established in this scoped review; not an implementation-absence claim |

All original task descriptions and FR/SC obligations remain binding. Do not weaken requirements to match unfinished code or erase valid prior work. Add task-specific proof before rechecking a box; gate evidence must name the implementation commit, production callers, test commands, platform/config, raw artifacts, negative controls and limitations. A test of an unused helper cannot prove its production caller changed.

## Recovery sequence

1. Retain and verify the implemented hardening/core. Complete security coverage and genuine codec-selection evidence; do not invent another type/codec authority.
2. Finish recursive typed binding and mandatory TS/Dart native invocation with no SQL fallback, checked results and real bridge execution.
3. Connect backend producers/receivers and Link/native/WASM/FRB to the same WS codec, eliminate MessagePack and generic JSON detours, and prove live semantics.
4. Replace owned persisted/Raft value formats for the fresh epoch; preserve new-data durability and external ecosystem protocols.
5. Run performance, multi-identity app, fault-history, backup/restore and cross-platform tests. Safe owned values can ship; new zero-copy is P2.
6. Close APP/STABLE/R and the feature only from task-linked evidence, never a mass checkbox replacement.
