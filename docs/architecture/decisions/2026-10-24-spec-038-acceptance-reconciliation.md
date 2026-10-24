# Decision: reconcile Spec 038 acceptance with production callers

**Date:** 2026-10-24  
**Status:** accepted documentation correction; runtime implementation remains in progress.  
**Audited baseline:** `81e5fb15dc600d268581c1b22ea25b74e25b77f1`, `feat/pg-dialect`, PR #402.

## Context

Spec 038 already requires one shared Rust codec across backend and Link, native generated routine invocation, complete WebSocket MessagePack removal, a fresh owned persistence/Raft format and measured acceptance. A later documentation commit marked all tasks and gates complete, while actual WS, generator and persisted-value callers still contradict those conditions.

## Decision

Withdraw blanket completion and restore task-specific acceptance. Retain the existing KDOM/HTTP/core-hardening work and bounded historical records; do not erase valid implementation or introduce another codec/service. An open task can mean partial implementation or missing acceptance evidence, not necessarily absent code.

Prioritize real call-chain integration: generated TS/Dart through actual bridges to native HTTP and FunctionService; server live producers/receivers through the same selected codec into native/WASM/FRB; owned storage/Raft values through that codec. Distinct endpoint message kinds and external ecosystem containers remain legitimate.

The preliminary handwritten Postcard/Prost-style comparison does not establish real-library performance or production selection. Reopen selection/integration gates and require equivalent actual-library measurements and whole-system resource evidence. Preserve preliminary results as labeled history.

All earlier product decisions remain: fresh start, no old-format adapters, no MessagePack transition window, no automatic data deletion, P0 correctness/security/durability, P1 measured CPU/memory/throughput, optional P2 zero-copy, and mandatory APP/STABLE acceptance.

## Evidence and consequences

See [implementation audit](../../../specs/038-unified-sdk-contracts/implementation-audit-2026-10-24.md), [plan](../../../specs/038-unified-sdk-contracts/plan.md), [core tasks](../../../specs/038-unified-sdk-contracts/tasks.md) and [app/stability tasks](../../../specs/038-unified-sdk-contracts/tasks-app-stability.md).

Twelve historical/narrow-supported records remain checked; 109 task acceptances reopen, including two optional P2 tasks. Reclosure requires pinned implementation, actual callers, executed tests, raw results, platforms/configuration, negative controls and limitations. Production code, generated runtime artifacts, configuration and user data are unchanged by this commit. No runtime/fuzz/performance test was run by this correction.

## Subsequent implementation

The live WebSocket binary path now uses the shared `KDOM` codec in `kalamdb-serialization::ws_domain`. Server, native Link, WASM, and the Dart bridge encode and decode those frames. `serialization=msgpack` is rejected. JSON text remains the explicit external representation. `WS_CONTRACT_DESCRIPTOR_ID` is the message-layout revision and is checked before a live handler runs. It is not the SQL catalog descriptor pin.

Still open, and still not claimed here: generated Dart routines call SQL, owned FlexBuffers and Raft MessagePack writers remain, live delivery cursors are not the row-version checkpoint, and Gates D, P, APP, STABLE, and R do not have acceptance evidence.
