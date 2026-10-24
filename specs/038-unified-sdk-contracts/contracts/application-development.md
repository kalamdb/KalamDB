# Application development contract

**Reviewed:** 2026-10-22. **Status:** proposed requirements, not implemented APIs or proof of platform superiority. Companion to [spec](../spec.md), [API](public-api.md), [protocol](protocol.md), [stability](database-stability.md), [coverage](../app-readiness.md) and [research](../research-app-platforms.md).

## 1. Goal and scope

A developer should declare the SQL schema/policies and server business operations once, generate TS/Dart contracts, and build a secure reactive application without hand-writing transport controllers, duplicated DTOs, binary codecs, reconnect reducers or permission filters in each screen. Business validation and UI behavior still belong to the application; generation cannot invent them.

Keep the clean-break format/API cohort and no MessagePack/old-reader adapters. Zero-copy remains optional P2. Do not trade security, consistency or supported-platform truthfulness for fewer lines of code. The mandatory outcome is two working vertical slices on a real server, plus the database reliability gate, not a larger list of method names.

## 2. Three explicit developer experiences

**Minimal online application:** client/session configuration, generated facade, one query/live descriptor and one typed command. No Drift, workers, offline queue, full schema registry download or extra framework dependency on this path.

**Reactive UI:** reuse existing React and Flutter handles rather than create a second cache/state engine. Loading, empty, error, disconnected, cached, optimistic and confirmed states are distinct. A component opts out of querying until required input/auth is ready; rapid input changes cancel or fence old responses. Share subscriptions within identical authorized scope. React StrictMode/remount, Flutter suspend/resume and hot reload must not duplicate long-lived work.

**Durable Flutter workflow:** opt into the existing one-Drift session for cache, outbox and workers. Browser persistent offline sync is NOT introduced or implied. Remote write acceptance, local optimistic state and live reconciliation are distinct. Unsupported Dart targets must fail explicitly; do not claim desktop/web support solely because generated types compile.

## 3. Progressive generated API

The existing canonical facade remains the sole high-level surface. Example below is a proposed fixture, not current source:

```ts
const db = createKalam(client);
const lessons = db.queries.school.myLessons({ weekStart });
const live = lessons.live();
const booking = await db.functions.school.bookLesson({ lessonId });
```

Equivalent Dart classes are generated from the same SQL catalog. SQL/Drizzle remains the one TS table-query interface; no second TS ORM. `get`, `getOne`, `getOptional`, `page` and supported `live` share one analyzed descriptor, exact types and error model. Framework adapters accept descriptors directly. No hand-written RPC names or request/response interfaces in application code.

Generate field paths and stable domain error metadata so a form can map validation failures without matching exception text. Provide only constraints the catalog actually knows; do not turn NOT NULL, domains or type limits into invented business rules or a new form framework. One convention selects module exports/server handlers; missing/duplicate/unexpected exports fail the build. No parallel route registration list.

## 4. Local development, CI and target safety

Reuse existing init/dev/schema generation/test entry points after auditing their actual CLI schema. New command names in design discussions are proposals until implemented. One project configuration selects environment, schema, function modules, generated languages and runtime endpoints; public settings are separated from secrets.

The dev loop boots an isolated real backend, seeds deterministic non-production fixtures, validates SQL/policies, compiles handlers, publishes one checked generation and reports separate schema/runtime/app readiness. Repeated runs are idempotent; failed compilation leaves the last valid deployment usable without publishing mismatched SDK files. An acknowledged DDL change is not silently replayed on a handler activation retry.

CI uses unique run namespaces/databases/processes and a finite lifecycle, never a production endpoint by default. Schema diff/deploy requires an explicit target and preflight; reject destructive or unsupported same-epoch changes without approval. Starting this release fresh does not permit erasing future production data for routine development. Sandbox resets and real-database test fixtures are explicit and separately scoped.

For agent-assisted development, emit a compact machine-readable manifest, capabilities, stable error locations, deterministic exit statuses and JSON diagnostic output from existing CLI surfaces. Agent instructions and examples are generated/versioned with the contract. Do not add an autonomous write agent or expose secrets in diagnostics. A malformed schema/type must fail generation rather than silently generate permissive any/dynamic models.

## 5. Auth, organization membership and environment isolation

Use one session/token provider across SQL, RPC, files and WS. Document initial sign-in through supported auth/OIDC adapters, refresh single-flight behavior, expiry, signout, subject changes, offline restrictions and service credentials. Templates never embed a DBA/service credential in a browser/mobile build. Fixtures can have local test identities; these must be clearly non-production.

An organization selector is untrusted input. USER-table partitioning is not by itself a complete multi-organization membership model. For shared school/team data, policies and trusted membership records enforce school, teacher assignment, parent-child relationships and roles. Test a user belonging to two organizations, an unauthorized organization ID, guessed resource IDs and membership revocation. Use typed trusted session scope internally; client type branding is not authorization.

On logout/account change, fence old requests/subscriptions, clear that account's active UI view and follow explicit persisted-cache retention policy. Do not claim instantaneous revocation of data already downloaded to an offline device. A server-confirmed permission change invalidates relevant caches/live access; the acknowledgement/freshness boundary is documented and tested. Secret values never enter generated clients, logs, cache keys, error messages or public manifest bodies.

## 6. Server rendering and framework boundaries

Define a request-scoped TS server client using the existing transport/executor, not a new global client. It does not open browser WS or require DOM globals during rendering. Never share auth-bearing state across simultaneous requests or put service tokens into hydration payloads. Cookie/header integration follows the chosen framework's security model, including CSRF protection where cookies authorize writes.

A server-rendered snapshot is scoped to subject, query, descriptor revision and observation frontier. Client hydration starts/resumes the corresponding subscription only when the frontier protocol is supported; otherwise mark the initial data provisional and explicitly refetch. Do not claim an atomic SSR-to-live handoff without a no-gap test. Auth mismatch discards the snapshot. Shared caches are private or keyed by verified scope, not only SQL text.

Add one React browser example, one request-rendered TS example and one supported Flutter example. Framework-specific extras remain optional and must not inflate the base client bundle.

## 7. Query efficiency, pagination and consistency

Expose bounded keyset pagination for deterministic orders with a unique tie-breaker. Cursor identity binds query/filter/order/scope/revision; reject another account's or another query's cursor. Define whether subsequent pages use a pinned snapshot or a moving dataset; do not promise duplicate/gap-free paging while leaving this unspecified.

Live pagination/top-N is a separate capability. A memory cap over an incomplete client subset is not a correct globally maintained top-N. Either prove server-maintained membership/window semantics or return an unsupported-capability result. Multi-query dashboards are not automatically a single consistent snapshot; offer a named server query/command for one-view consistency where supported.

Named joined/projected read models prevent UI N+1 requests and retain inferred output/nullability. General one-shot SQL support does not imply arbitrary reactive joins/search. Generate index advice from actual plans or measurements; do not create indexes silently. Diagnostics expose safe query plans, rows/bytes scanned, subscription count and query/RPC round trips without leaking another tenant's data.

A command result and a live update are not necessarily delivered together. Provide an explicit committed/accepted outcome and, only where supported, a scope-bound observation token or equivalent reconciliation marker. The UI must not overwrite a newer optimistic change with an older echo or use a version from one Raft group as a global clock.

## 8. Commands, conflicts and UI mutation state

Prefer server-side business commands for booking, enrollment, invoicing and transfers. Define default isolation and atomicity separately for one statement, explicit transaction, routine and bulk command; this is enforced by the stability contract. A transaction wrapper alone does not prevent write skew or overselling.

Examples use database-enforced uniqueness, tested conditional updates or another supported serialization strategy for contested resources. Expose optimistic concurrency through a generated expected-version predicate only when enforced server-side; distinguish conflict, not-found, permission denial and uncertain outcome without disclosing protected existence.

Framework mutation helpers accept typed operations and expose per-invocation pending/result/error state. Track concurrent operations on the same key with counts/identities rather than a boolean/Set cleared by the first completion. Optimistic transforms are synchronous, replayable and side-effect-free; rollback only the failed overlay, preserve later user intent, and restore overlays after an authoritative refetch. Nothing automatically queues unsafe commands offline.

## 9. Internal operations, external work and scheduling

Server-only routine references are excluded from client manifests, but this is not the security boundary: ACL/exposure checks reject direct SQL/HTTP/native attempts as appropriate. Generated server helpers reference the same canonical identities and preserve per-function permission frames.

Classify reads, transactional commands and externally effectful jobs clearly. Only claim readonly/deterministic enforcement when the runtime actually restricts effects. Do not import another platform's transaction guarantees merely because the function is TypeScript.

Use existing topics/jobs/scheduler owners for durable external work. Prefer transactionally recording intent followed by a worker, with explicit state/retry/timeout/cancellation/dead-letter behavior and downstream idempotency. If atomic commit of the intent plus business writes is unavailable in the same supported domain, expose that gap and block the recipe rather than presenting a pseudo-atomic example. No new client DAG engine or distributed transaction coordinator.

Webhook tests use a local provider simulator: signature validation, duplicate/out-of-order events, response loss after external acceptance, replay and secret redaction. No live payment/SMS/email provider is exercised by default. Exactly-once external delivery is not promised. Scheduler/trigger capabilities that remain unfinished are explicit dependencies, not stub successes.

## 10. Files and operational diagnostics

Give typed FILE workflows for upload progress/cancel, authorized reference creation, attach, read/download and cleanup. Document object-upload/database-attach failure windows; orphan cleanup is idempotent and cannot delete another tenant's object. Enforce size/type/checksum/path/ownership policies at the actual serving boundary. Do not put base64 in the binary hot path or assume data-row deletion atomically removes external bytes.

Errors carry stable codes, safe field paths, request IDs and known/uncertain execution disposition. Logs/traces correlate SDK -> auth -> routine/query -> commit -> live delivery; sampling and label cardinality are bounded. Sensitive sentinels in tests must not appear in artifacts. Diagnostics distinguish connection liveness from readiness, correctness and restore availability.

## 11. Capability and scope discipline

The release matrix in app-readiness.md names mandatory current work, conditional capabilities and deferred features. Do not require email delivery, payments, presence, global search, vector search, custom auth UI, site hosting, branching/merge, browser persistent sync or reactive arbitrary SQL merely to complete the core SDK. For each unsupported item, documentation gives an honest integration/defer boundary. Existing implementations are reused after verification; this spec does not assert they are absent.

A fresh disposable environment is mandatory for tests. Copy-on-write branching is an optional implementation of isolation only when its catalog, files, auth and side effects are proven isolated; no preview may contact production jobs or provider secrets. New-format backups and restoration are mandatory even though migration of historical release data is not.

## 12. Executable product fixtures and DX gate

**DX-01 School:** TS React and Flutter over the same schema. Organizations, teacher assignment, parent access, enrollment, paginated lessons, one-slot booking, attendance, typed failures and live updates. Use a fake provider for invoices/notifications and an explicit atomic intent boundary. Race 100 independent one-slot booking attempts; exactly one booking succeeds, losers have a typed conflict and no orphan invoice/intent or partial enrollment. Where a required constraint/isolation mechanism is absent, this fixture is a blocker, not a mock success.

**DX-02 Helpdesk/chat:** authenticated tenant/project inbox with named joined read models, attachment lifecycle, live patch/delete/filter transitions, SSR initial read, account switch, optimistic pending state, supported Flutter offline action restoration and reconnect. Error, loading, empty and stale are distinguishable; no cross-user snapshot or file is exposed.

**DX-03 Workflow:** from clean clone/config to seeded authenticated read/write/live result, then same-epoch schema/function edit, regeneration, tests and staged deployment. Capture failed generation, bad secrets, wrong target, stale contract and handler compile failure. Use existing CLI conventions; new entry points must be implemented and discoverable before documented as commands.

**DX-04 Multi-identity testkit:** lightweight helpers in existing test/dev packages create isolated fixtures and typed clients as owner/member/teacher/parent/outsider/anonymous. They never enable production impersonation. Positive and denied access checks run against the real server; pure UI/model mocks are explicitly limited.

Measure handwritten infrastructure statements/files, duplicated contract declarations, setup actions, network round trips, payload/SDK size, retained subscriptions and time-to-first authenticated live result. Exclude generated output, UI/domain logic and test assertions from boilerplate counts; include equivalent auth/error/recovery behavior in every comparison. Target at least 50% less generic glue than the pinned current KalamDB fixture, zero manual wire codecs/routes/DTOs and one reusable descriptor per query. Human timing is recorded as observed, not invented. Compare competitor fixtures only at equivalent supported scope; no claim of outperforming every platform without evidence.

Gate APP requires DX-01 through DX-04, scope-matrix honesty, diagnostics and the stability gate. Passing compile examples alone does not establish a stable database.
