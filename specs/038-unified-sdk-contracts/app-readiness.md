# Application readiness matrix and specification extension

**Revision:** 2026-10-22. Normative extension to [spec.md](spec.md), scoped to application development and stability. Core serialization requirements FR-001..FR-062 and SC-001..SC-022 remain unchanged. The requirements below add FR-063..FR-080 and SC-023..SC-032; [tasks-app-stability.md](tasks-app-stability.md) extends tasks with T094..T121.

**Evidence boundary:** reviewed source/docs at `3cab0762324572099f0b41818648e90103a7fb72`. Planned means specified, NOT shipped or tested. No implementation-absence claim is made solely because a feature was not found in the sampled files.

## US9 - Build and evolve a real application with minimal infrastructure code (P1)

A developer creates a seeded, authenticated school/helpdesk project, generates TS/Dart once, binds typed reads/live state and server commands, observes useful errors, tests multiple identities, deploys an intended target and edits the schema within the current epoch. The minimal online path does not require offline workers/Drift or a second ORM. React, server-rendered TS and supported Flutter have explicit lifecycle and auth boundaries.

Acceptance is DX-01..DX-04 in [application-development.md](contracts/application-development.md), with actual handwritten-glue/network/resource measurements and a truthful capability matrix. Existing business validation and UI remain application responsibilities.

## US10 - Depend on explicit database guarantees during faults (P0)

An application author knows the supported atomicity, isolation, durability and retry semantics for SQL statements, transactions, routines, files, live delivery and external jobs. Runtime invariants survive stated failures or surface explicit unsupported/unknown outcomes. New-format backup restore and crash recovery are mandatory even though old-release data migration was removed.

Acceptance is DB-01..DB-20 in [database-stability.md](contracts/database-stability.md), executed against the real engine with seeded histories, concurrency, fault injection, supported PostgreSQL comparisons and cross-platform SDKs. Documentation and mocks alone cannot close this story.

## Coverage matrix

| Capability | Baseline evidence / current scope | Requirement for this release | Tests |
| --- | --- | --- | --- |
| SQL-derived TS/Dart calls and shared binary | Well specified in 038; native binary runtime work still unchecked | Reuse existing plan, do not create another transport layer | Existing C0/B2 plus DB-14 |
| Minimal project setup and codegen | CLI/SDK foundations exist; complete auth-to-live workflow under-specified | One project config and isolated real dev/test lifecycle, deterministic artifacts | DX-03/04, T095/T104 |
| Auth/session and refresh | Client auth/OIDC APIs documented | One token source across surfaces, account fences, no privileged credentials in bundles | DB-09/14/15 |
| Organizations/roles | USER isolation described; organization membership is application-specific | Verified school/team membership policies and malicious-ID tests, not just user partitioning | DX-01, DB-09 |
| UI hooks / optimistic state | Existing React hooks; source uses key Sets for update/delete pending state | Typed command support, per-invocation tracking, safe overlay rollback and lazy queries | DB-14/15, T098 |
| Request-rendered TS / hydration | Not yet a complete 038 contract | Request-local auth/cache and scoped hydration frontier or provisional refetch | DB-15 |
| Keyset pagination and read models | Query/live descriptors planned; local limit differs from true top-N | Stable cursor semantics, named joined reads, explicit unsupported live windows | DB-01/10/20 |
| Runtime validators/errors | Exact scalar and structured error work already planned | Reuse generated field paths and error codes, not duplicate validators/exception matching | DB-01/14 |
| Transactional commands | Functions/transaction roots planned; single-data-group limit documented | Statement/transaction/isolation promise matrix, contested resource guards and unknown-outcome state | DB-02/03/04 |
| External work and scheduling | Existing job/topic owners; exact transaction link must be verified | Explicit supported intent/worker/idempotency recipe; absent atomic intent is a blocker for the recipe | DB-04/12/20 |
| FILE workflows | SDK helpers and file storage described | Authorized upload/attach/read/cancel/cleanup with non-atomic windows visible | DB-16 |
| Local multi-identity testing | Various existing suites; reusable app fixture experience incomplete | Small typed helpers over actual isolated server, no production impersonation endpoint | DX-04, DB-09 |
| Same-epoch schema/deploy | Generation and publication planned | Validate target, secret separation, staged schema/handler readiness, revision errors | DB-08, DX-03 |
| New-data backup/restore | Not a completed SDK contract | Consistent documented backup boundary and independent restore drill, measured RPO/RTO | DB-17 |
| SQL correctness | Prior PG compatibility work exists | Pinned subset differential and metamorphic wrong-result oracles | DB-01/19 |
| Atomicity and isolation | Direct DML statement-wide atomicity caveat in architecture docs | Reverify/fix/reject before advertising compatible atomic statements | DB-02/03 |
| Cluster durability | Raft is present; not proof of global serializability | Per-group history checks, partitions/leader loss/rejoin/snapshot and strict acknowledgement semantics | DB-05/06 |
| Hot/cold consistency | Tiered storage core | Concurrency/flush/manifest/compaction/tombstone crash invariants | DB-07 |
| Runtime security/resource stability | Codec fuzzing already planned | Full-path permissions, budgets, leak/cleanup, slow consumer and tenant-pressure coverage | DB-09/11/18 |
| Supported platforms and operations | TS browser/Node, Dart Android/iOS stated in sampled docs | Publish tested matrix; no unsupported target success based on generated stubs | DB-14/20 |
| Search, presence, hosted sites, arbitrary live joins | Not required to demonstrate this core SDK | Explicit deferred or external integration boundary; no marketing parity claim | Capability matrix |
| Browser persistent offline sync / branch merge / cross-group transactions | Outside existing 038 scope | Remain deferred; no extra runtime/state engine introduced | Explicit unsupported tests |

## Additional functional requirements

- **FR-063:** The minimal application path MUST provide isolated project configuration/seeds, current schema/handler/codegen workflow and typed reads/commands/live state without requiring optional persistence, a second ORM or handwritten transport registration.
- **FR-064:** One supported auth/session integration MUST govern SQL, functions, files and WS; refresh, logout/account switches, service-secret separation and client-cache invalidation boundaries MUST be tested.
- **FR-065:** Multi-organization examples MUST enforce trusted membership/role rules server-side, including teacher/parent/outsider cases. Client organization selection and USER partitioning alone MUST NOT be presented as complete organization authorization.
- **FR-066:** Server-rendered TS MUST use request-scoped clients/auth/cache. Hydration MUST bind subject/query/revision and use a proved frontier handoff or explicit provisional refetch without secrets crossing the boundary.
- **FR-067:** Typed named reads and keyset pagination MUST have bounded deterministic ordering and scoped cursor semantics; arbitrary live pagination/joins or globally consistent dashboard snapshots MUST NOT be claimed without server support and tests.
- **FR-068:** Existing UI adapters MUST support descriptor-driven queries and commands, conditional start, per-invocation mutation state, conflict/unknown outcomes and isolated optimistic overlays without duplicating the core cache or silently enabling offline retries.
- **FR-069:** Generated model constraints, field paths and domain errors MUST reuse catalog/runtime truth. No invented business rules, unchecked any/dynamic success or exception-text parsing substitute for validation.
- **FR-070:** Public versus internal operations, transactional versus external work and scheduler/job intent MUST have enforced boundaries and explicit retry/commit semantics. Use existing services; no pseudo-atomic external effects or new arbitrary client workflow engine.
- **FR-071:** FILE upload, attach, download and cleanup MUST expose their actual ownership/transaction boundaries and prove scoped progress/cancel/error/orphan behavior.
- **FR-072:** Application tests MUST have concise isolated real-backend fixtures and typed identity helpers; UI/logic mocks MUST NOT certify database permissions, isolation, durability or binary behavior.
- **FR-073:** Same-epoch development/deployment MUST validate explicit targets, secrets, schema/routine revisions and staged readiness. No production reset, repeated acknowledged DDL or stale partially generated deployment is hidden by a convenience command.
- **FR-074:** Safe bounded diagnostics MUST correlate SDK requests with auth/query/function/commit/live outcomes, expose stable error paths/codes and support machine-readable agent tooling without exporting secrets or high-cardinality user data.
- **FR-075:** Each advertised database surface MUST declare atomicity/isolation/durability/failure scope and pass invariant/history tests. Multi-row statement partial writes or unsupported isolation MUST NOT be advertised as PostgreSQL-compatible success.
- **FR-076:** The supported SQL subset MUST have exact-result PostgreSQL differential and deterministic metamorphic oracles, minimized regressions and negative controls; throughput does not excuse wrong results.
- **FR-077:** Runtime release MUST exercise deterministic fault points plus real process/storage/cluster tests for WAL/quorum/ack, leader loss, partitions, hot/cold flush, manifests, compaction, tombstones, schema/cache invalidation and resource pressure.
- **FR-078:** Current-format backup MUST be restored into an independent isolated environment and verified at its declared consistency frontier; publish RPO/RTO evidence. Old-format import remains out of scope.
- **FR-079:** Supported native/browser/Flutter clients MUST pass lifecycle, concurrent mutation, reconnect, scoped cache, cancellation and sustained pressure tests against the real server. Test cadence/artifacts and failure budgets MUST be explicit.
- **FR-080:** Release MUST prove the school/helpdesk developer path and distinguish mandatory, conditional and deferred capabilities. Claim reduced boilerplate or competitive advantage only from equivalent-scope measured fixtures; do not add features merely to lengthen a checklist.

## Additional success criteria

- **SC-023:** Untouched TS/Flutter school and helpdesk fixtures complete auth/read/command/live/setup/schema-edit flows using one generated contract and no manual wire DTO/codec/route list.
- **SC-024:** Owner/teacher/parent/member/outsider/anonymous tests and account/organization changes show no unauthorized reads/writes, live/file access or cross-request cache exposure.
- **SC-025:** SSR/hydration, React remount and Flutter resume plus overlapping same-key commands preserve accurate pending/confirmed/error states and bounded subscription lifetimes.
- **SC-026:** Cursor/named-read fixtures define snapshot/moving-page semantics, reduce measured N+1 work, enforce resource limits and reject unsupported exact live top-N rather than dropping required rows.
- **SC-027:** Existing CLI/project workflows and reusable real-server test helpers prove isolated, reproducible runs, explicit targets, redacted diagnostics and safe failed generation/activation.
- **SC-028:** Atomic statement tests leave no prefix on late-row failure; synchronized one-slot booking has exactly one committed success and no orphan related writes. Isolation tests check the actual advertised model.
- **SC-029:** Acknowledged durable effects survive the supported fault model; hot/cold and backup-restore invariants pass without lost/resurrected data or silent corruption.
- **SC-030:** Differential/metamorphic query tests, parser/codec fuzzing and real histories produce reproducible artifacts and retained minimized regressions, with checkers shown to reject seeded violations.
- **SC-031:** New and existing docs/examples match the supported platform/capability matrix; mocks, proposed signatures and incomplete runtime work are never labeled implemented/stable.
- **SC-032:** APP and STABLE gates have actual per-commit evidence plus equivalent-scope boilerplate/resource measurements; no unproved blanket claim of outperforming mature platforms.

## Binding release relationship

Gate APP (DX-01..04) and Gate STABLE (DB-01..20 for advertised scope) are prerequisites for existing Gate R and final T050 acceptance. Tests do not create a new transaction engine or change isolation by documentation. Fix violations or explicitly narrow advertised scope with owner approval; no silent downgrade. Existing P0/P1/P2 priorities, single codec, no backward compatibility and new-data crash durability remain unchanged.
