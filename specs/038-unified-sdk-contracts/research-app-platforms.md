# Application platform research and SDK coverage review

**Reviewed:** 2026-10-22. **Baseline:** PR #402 at `3cab0762324572099f0b41818648e90103a7fb72`. Evidence is documentation/source inspection, not execution, competitive benchmark results or a claim that all named features already work.

## 1. Main finding

Spec 038 already describes strong generated TS/Dart contracts, shared binary transport, typed native functions, live state, optional Dart durability and serialization safety. It did not yet specify the complete path from project setup to a secure, deployable and recoverable application. More codecs or method names cannot fill auth lifecycle, SSR, mutation convergence, safe booking, backup/restore and correctness-testing gaps.

The product opportunity is one coherent SQL + generated routine + reactive SDK model across web and mobile, with fewer duplicated contracts and recovery wrappers. This is a target to demonstrate, not evidence that KalamDB is superior to every platform. Prefer proving two real applications and their failure behavior over claiming an unbounded feature checklist.

## 2. Primary platform sources and scoped lessons

| Source | Verified upstream pattern | Application to KalamDB / boundary |
| --- | --- | --- |
| [Convex functions](https://docs.convex.dev/functions/overview) | Queries, transactional mutations and externally effectful actions are distinct function categories, exposed through clients | Distinguish reads, transactional commands and jobs; generate one contract but do not claim Convex transaction guarantees for the KalamDB runtime without tests |
| [Convex optimistic updates](https://docs.convex.dev/client/react/optimistic-updates) | Optimistic changes are attached to a mutation and reconciled with reactive results | Add reusable per-operation pending/error/overlay semantics on existing React/Flutter machinery; do not build another cache or enable unsafe offline retries |
| [Convex pagination](https://docs.convex.dev/database/pagination) | Cursor pagination is integrated with reactive queries and UI hooks | Define keyset/scope/revision behavior and true live-window membership before advertising reactive pagination; a local cap is not equivalent |
| [Convex server rendering](https://docs.convex.dev/client/nextjs/app-router/server-rendering) | A server-preloaded result can feed a reactive client hook; the source labels SSR support beta | Add request-scoped execution and a proven hydration/live frontier or explicit provisional refetch, without leaking private snapshots or global credentials |
| [convex-test](https://docs.convex.dev/testing/convex-test) | A JavaScript mock backend offers fast function-logic tests | Borrow ergonomic typed fixtures, but use the actual KalamDB backend for RLS, isolation, durability, native codecs and cluster evidence |
| [Supabase local development](https://supabase.com/docs/guides/local-development) | Local services, schema workflow, seeds, type generation, secrets and deployment share CLI tooling | Reuse Kalam init/dev/schema workflows, add isolated test lifecycle and separate schema/handler/app readiness; do not invent command flags before implementation |
| [Supabase database/RLS testing](https://supabase.com/docs/guides/local-development/testing/overview) | Tests cover schema, constraints, functions, positive/negative access and real application calls with isolated identities | Provide owner/member/outsider fixtures and denied CRUD/routine/file/live checks; do not assume PostgreSQL extensions such as pgTAP already run on KalamDB |
| [Supabase SSR auth](https://supabase.com/docs/guides/auth/server-side) | Server rendering requires deliberate client/session integration; cookie-backed sessions differ from browser storage | Keep public/service settings separate and auth state request-scoped; design CSRF, refresh, logout and cache boundaries rather than a global server singleton |
| [Firebase rules unit testing](https://firebase.google.com/docs/rules/unit-tests) | Emulator-backed rules tests can exercise authenticated and unauthenticated contexts | Make identity-based testing easy and run positive controls; emulator ergonomics do not substitute for actual backend crash or consensus tests |
| [Neon database testing](https://neon.com/blog/test-your-database-without-mocks) | Isolated database branches support real-data/constraint testing instead of mocked DB calls | Require isolated real test environments; optionally use KalamDB branching only after isolation of schema/data/auth/files/jobs is proven. No dependency on production data copies or branch merge completion |

Sources were opened or searched through their official publishers during this review. Beta status and capabilities are those described in the retrieved source, not an independent production guarantee. PowerSync/Appwrite details were not needed to establish these requirements; avoid filling gaps with unverified parity claims.

## 3. Database testing sources

[SQLite testing](https://www.sqlite.org/testing.html) describes independent harnesses, out-of-memory and I/O faults, crash/power-loss tests, malformed inputs, regression and leak checks. Adopt failure-point coverage and invariants rather than borrowing its test counts or maturity claims.

[FoundationDB simulation](https://apple.github.io/foundationdb/testing.html) describes deterministic cluster simulation alongside actual performance/hardware testing. Begin with seeded component fault schedules and real-process tests in KalamDB; do not imply that a whole-cluster deterministic simulator already exists or require rewriting the runtime in Flow.

[Jepsen consistency models](https://jepsen.io/consistency) defines legal histories and anomalies. State the actual guarantee for each KalamDB operation and use the appropriate checker. Raft replication by itself does not prove arbitrary SQL serializability.

[PostgreSQL 18 isolation](https://www.postgresql.org/docs/18/transaction-iso.html) distinguishes read committed, repeatable read and serializable behavior. Use a pinned server for the explicitly compatible SQL subset; do not require different isolation levels to produce identical histories or normalize away a real bug.

[SQLancer source](https://github.com/sqlancer/sqlancer) documents Ternary Logic Partitioning, NoREC and other oracles for wrong-result and crash testing, including DBMS-specific providers. Adapt relevant generators/algorithms and retain minimized failures. Supporting DataFusion in an upstream tool is not evidence that every KalamDB extension can be tested without an adapter.

## 4. Repository observations that change the work

These paths were inspected at the pinned baseline. Observations do not establish absence of every related implementation elsewhere.

| Source | Observation | Consequence |
| --- | --- | --- |
| [current plan](plan.md) | Strong codec/generation/live/sync track; no comprehensive named application or engine-stability acceptance matrix | Add application and database contracts, tasks and release dependencies, not a second SDK architecture |
| [TypeScript client README](../../link/sdks/typescript/client/README.md) | Materialized live API and automatic WASM are present; README describes limit as retained-row cap and getKey is sometimes configured manually | Preserve useful simplicity but distinguish memory cap from exact top-N and generate actual key metadata; align current docs when runtime changes |
| [Dart README](../../link/sdks/dart/link/README.md) | Supported platforms are stated as Android/iOS; generated function calls are described as SQL CALL wrappers | Do not claim desktop/web support from generated types alone; native typed RPC remains planned until implementation and real-runtime testing |
| [React exports](../../link/sdks/typescript/react/src/index.ts) and [mutation helper](../../link/sdks/typescript/react/src/hooks/useMutationState.ts) | Existing hooks can be extended; update/delete pending state uses Sets keyed by row | Test overlapping mutations to the same key: the first completion must not clear the pending state of another invocation. Source-level concern, not a reproduced UI incident |
| [transaction architecture](../../docs/architecture/transactions.md) | Explicit transaction commits are one-data-group; direct DML callbacks do not add statement-wide atomicity | Make failure-on-row-N and public atomicity claims release-blocking; reject/reroute unsupported operations before any partial write |
| [current API contract](contracts/public-api.md) | Good exact types and generated execution, but no complete SSR/auth organization/file/job workflow gates | Require a minimal default path and progressive opt-in examples; classify conditional capabilities explicitly |

## 5. Chosen product additions, not feature sprawl

Mandatory: secure local/project lifecycle and target selection; one auth/session source; tenant-membership enforcement examples; typed validation/errors; descriptor-driven UI state; request-scoped server rendering; bounded query/keyset pagination; supported optimistic conflict behavior; FILE ownership workflow; real multi-identity test helpers; new-format backup/restore; engine crash/consistency coverage.

Conditional but explicit: atomic job intent, scheduler/trigger integration, a live pagination window, commit-to-observation token and supported conditional version update. Either implement/test for a named fixture or reject the unsupported feature; do not fabricate success.

Deferred/external: email/payment provider delivery, hosted site builder, browser persistent offline sync, general reactive joins, presence/search/vector product UIs, cross-group transactions, arbitrary offline conflict resolution and branch-data/schema merge. Existing features can be reused after review, but none is silently asserted by a codegen declaration.

## 6. Evidence of reduced boilerplate

Build the same school and helpdesk app with the pinned current KalamDB workflow and the proposed SDK. Count handwritten infrastructure files/statements, duplicate contract declarations, manual auth/reconnect/projection code, setup steps, round trips, subscriptions, SDK footprint and resource usage. Exclude generated output and business/UI/test assertions. Require equivalent authorization, error, concurrent-write and recovery semantics.

Target at least 50% reduction of the measured generic glue, zero hand-written routine transport/DTO/codec registration and one reusable descriptor per query. Setup times are recorded from actual runs. External-platform examples are references or separately reproducible comparisons; never claim fewer lines by omitting their safety or offline functionality.

## 7. Deliverables and uncertainty

[Application contract](contracts/application-development.md) gives DX-01..DX-04. [Stability contract](contracts/database-stability.md) gives DB-01..DB-20. [Readiness matrix](app-readiness.md) distinguishes baseline coverage from required evidence. [Task extension](tasks-app-stability.md) assigns T094 onward and adds APP/STABLE prerequisites to the existing release gate.

No Rust/TS/Dart runtime was modified, no business workflow was executed, and no database stability benchmark/fuzz suite was run during this specification review. Implementation evidence must identify exact commits, configs, guarantees and test results. A research document is not a release certification.
