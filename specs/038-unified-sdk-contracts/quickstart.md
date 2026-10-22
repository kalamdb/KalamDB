# Current developer path: build, test and recover an application

**Updated:** 2026-10-22. This replaces the former Slice-1-only handoff and MessagePack transition instructions. Examples describe the proposed current generated API; T044/T107 must turn them into compiled, real-server fixtures before they are advertised as implemented.

## 1. Start a safe project

Use the repository's supported init, dev and schema-generation workflows with an explicitly selected disposable environment, a current server/SDK cohort and deterministic fixtures. Verify available command flags against the actual CLI implementation. Configure one endpoint/auth provider and separate public settings from secrets. Do not put DBA/service credentials in a browser/mobile application.

There is no old-data import, old SDK alias or MessagePack compatibility window. Old format/API epochs fail explicitly; no file or deployed directory is automatically erased. The seven historical Slice 1 task markers remain records, not an instruction to stop implementing the current plan.

The project contract declares SQL schema, constraints/policies and server routine signatures once. Generate TS and Dart, including models, parameter/result contracts, real keys, named reads and server handler bindings. Same-epoch schema edits must regenerate safely; a failed generation or handler compilation cannot expose a half-published application.

## 2. Read, observe and call a typed business operation

Proposed school fixture:

```ts
const db = createKalam(client);
const lessons = db.queries.school.myLessons({ weekStart });
const rows = await lessons.get();
const live = lessons.live();
const stop = live.subscribe(renderSnapshot);
await live.start();

// A separate user action:
const booking = await db.functions.school.bookLesson({ lessonId });
```

On teardown, unsubscribe and dispose the owned live handle. Framework adapters own that lifecycle rather than opening another subscription on every render. React uses descriptor-driven hooks; Flutter uses a handle outside build and its replaying stream. Loading, empty, stale, disconnected, optimistic and confirmed states stay distinct.

Dart receives corresponding generated input classes and methods from the same SQL catalog. TypeScript table access stays Drizzle; Dart remote tables use the thin generated builder. No hand-written routine-name strings, SQL wrapper invocation, transport controllers, wire codecs or duplicate request/result interfaces are required for these generated calls.

The school routine must enforce supported uniqueness/conditional-write/isolation rules on the server. A client-side availability check is not a booking guarantee. Named read models can avoid N+1 UI requests. Pagination uses scoped stable cursors; a local row limit is not automatically a correct globally maintained live top-N.

## 3. Use optional features only when needed

A simple online app does not initialize Drift, a durable queue or background workers. Supported Flutter offline workflows opt into Kalam.open with one managed or attached Drift connection, generated mirror definitions and eligible durable operations. Accepted server result, optimistic UI and live reconciliation are separate states.

For request-rendered TypeScript, create a request-scoped client/auth context. Hydrate a scoped snapshot only with a proved live frontier or explicit provisional refetch. Never reuse an authenticated singleton across users.

Use typed FILE helpers for uploads/attachments and the existing jobs/topics owners for external work. Their non-atomic external boundaries and replay policies must be explicit. Email/payment providers are simulated in tests, not called using production credentials. Features not implemented or verified are listed as conditional/deferred in app-readiness.md.

## 4. Prove the application and the database

Run the reference school and helpdesk apps through auth, organization membership, read/command/live flows, files, optimistic state, error paths, same-epoch schema edit and target-safe deployment. Test owner/teacher/parent/member/outsider/anonymous identities on the actual backend, not a mock permission engine.

For the school, race 100 independent clients for one slot: exactly one supported booking succeeds, losers receive typed outcomes, and no related enrollment/invoice/intent is left partially committed. Test multi-row statement failure midway, explicit transaction rollback and commit-response loss.

Exercise new-format crash/restart, leader failover and partitions when clustering is advertised, hot/cold flush/tombstones, current backup restore, malformed binary inputs, memory pressure, slow consumers, duplicate delivery and cursor gaps. No lost event is hidden behind a successful acknowledgement. Restore only into a separate isolated environment.

Measure handwritten generic glue, duplicate contracts, round trips, retained subscriptions and setup steps using equivalent auth/error/recovery scope. Compare CPU/memory/throughput under the existing performance contract; neither this guide nor successful compilation establishes a speedup or stable database.

## 5. Implementation order and authoritative documents

Read spec.md and plan.md, then tasks.md together with its normative continuation tasks-app-stability.md. Start guarantee-matrix and fault-oracle work early alongside decoder hardening, not after UI development.

Required gates: S, C0/D, B2, W and P, plus APP and STABLE, then R. No historical backward compatibility is added. Existing lifetime defects are P0; new zero-copy is optional P2. Reuse canonical type, codec, query, transaction and state owners instead of building duplicate systems.

Documents: [spec](spec.md), [plan](plan.md), [core tasks](tasks.md), [app/stability tasks](tasks-app-stability.md), [readiness matrix](app-readiness.md), [application contract](contracts/application-development.md), [stability tests](contracts/database-stability.md), [API](contracts/public-api.md), [protocol](contracts/protocol.md), [performance](contracts/serialization-performance.md), [platform research](research-app-platforms.md).
