# Feature Specification: Schema-First Functions Runtime (V1.1)

**Feature Branch**: `034-schema-first-functions`

**Created**: 2026-09-07

**Status**: Draft

**Input**: User description: "Evolve the existing Functions implementation (PR #380 / feat/server-functions) into a production-quality schema-first server function runtime: SQL owns the contract; a TypeScript/JavaScript project owns implementation; KalamDB owns execution, deployment, and rollback. Fix memory, nested-call, HTTP, CLI, security, and one-module-per-project issues before adding more features. Share one host-context type declaration between inline SQL bodies and project methods. Prefer a FlatBuffer zero-copy check for request/response values crossing the JavaScript runtime boundary."

## Product Principles

These are non-negotiable product rules for this feature:

1. **SQL is the backend contract. TypeScript/JavaScript is only the implementation.**
2. **A deployed functions project is one immutable module revision, not one module per procedure.**
3. **Inline SQL code is a fallback implementation that the active project revision may override.**
4. **A root invocation pins one immutable function deployment view for its entire lifetime.**
5. **Nested procedures share the root transaction and normally the same execution resources.**
6. **Do not create heavyweight query or runtime state unless it is actually used.**
7. **Memory is bounded globally, not merely per execution instance.**
8. **Keep only a very small warm pool. Idle memory is a cost, not a feature.**
9. **Never trade request isolation for pooling performance. Destroy suspicious instances.**
10. **The initiating actor is immutable; the effective principal is frame-local.**
11. **EXECUTE privilege, relation privileges, and row-level security are separate layers.**
12. **User code gets capabilities only through explicit host APIs.**
13. **The host context API is declared once and shared by inline SQL bodies and project implementations.**
14. **Values crossing the JavaScript runtime boundary prefer one in-memory FlatBuffer copy, reused when already in that form.**
15. **No filesystem, network sockets, process, environment, or native-addon environment exists inside functions.**
16. **The normal server distribution must not include unused experimental runtimes.**
17. **Optimize the database workload, not JavaScript microbenchmarks. Measure before replacing the current runtime.**

## User Scenarios & Testing *(mandatory)*

### User Story 1 - Declare Procedures as SQL Contracts (Priority: P1)

A database developer defines procedures in SQL with names, parameters, return types, composite/table types, security mode, and EXECUTE privileges. The SQL contract does **not** name source files, export names, or a per-procedure runtime. The functions project runtime belongs to project configuration (`path`, `runtime`, `module`), not to each `CREATE PROCEDURE`.

**Why this priority**: Schema-first is the product model. If SQL encodes physical source layout, every later workflow (codegen, deploy, rollback, multi-file projects) is wrong.

**Independent Test**: Create a procedure with signature, `SECURITY DEFINER`, `REVOKE`/`GRANT EXECUTE`, and no `AS 'src/...'` mapping. Confirm the catalog stores a callable contract and that invocation without an implementation returns a clear not-implemented error.

**Acceptance Scenarios**:

1. **Given** a schema-first project, **When** a developer runs `CREATE PROCEDURE api.create_order(request api.create_order_request) RETURNS api.create_order_result SECURITY DEFINER` with no source-file mapping, **Then** the procedure is a valid contract and does not require `LANGUAGE TYPESCRIPT`.
2. **Given** that procedure, **When** they `REVOKE EXECUTE ON PROCEDURE api.create_order FROM PUBLIC` and `GRANT EXECUTE ... TO user`, **Then** only granted identities may call it.
3. **Given** a project-backed procedure with no inline body, **When** anyone inspects the definition, **Then** there is no required per-procedure runtime declaration.
4. **Given** a procedure whose implementation is missing, **When** it is invoked, **Then** the caller receives a stable not-implemented error rather than a filesystem lookup failure.

---

### User Story 2 - Implement Procedures in a Normal TypeScript Project (Priority: P1)

An application developer implements business logic in a full project: `package.json`, lockfile, `tsconfig.json`, shared helpers, tests, and pure JavaScript packages. The server never installs dependencies. Compilation happens on the developer machine, CI, CLI, or a future cloud builder. The server receives one immutable compiled artifact.

**Why this priority**: Developers expect a normal project, not one generated file per isolated sandbox. The server must stay a database, not a package manager.

**Independent Test**: Create a project with two procedures sharing a helper and a pure JS dependency, build locally without a live server, and invoke both procedures from the compiled artifact.

**Acceptance Scenarios**:

1. **Given** a functions project with `src/api/`, `src/services/`, `src/utils/`, `tests/`, and package manager lockfile, **When** the developer builds, **Then** shared imports and supported pure JS packages are bundled into one artifact.
2. **Given** that project, **When** the database server starts or a procedure is invoked, **Then** the server does not run dependency installation or lifecycle scripts.
3. **Given** a supported package manager (npm, pnpm, yarn, or bun), **When** the project builds, **Then** the lockfile is hashed into the artifact manifest for reproducibility checks.
4. **Given** a native addon, Node builtin, or package that cannot be bundled into the allowed capability set, **When** the developer builds or deploys, **Then** the operation fails with a clear diagnostic.

---

### User Story 3 - Generate Bindings Once, Never Overwrite Implementations (Priority: P1)

Code generation creates a typed `procedure.<schema>.<method>` builder API plus a one-shot named-export scaffold at `functions/src/<schema>/<procedure>.ts` when that identity is not bound anywhere. Each schema procedure gets its own file. A generated registry maps qualified SQL names to implemented named exports and is the build entrypoint. Runtime lookup is contract identity → compiled registry → function. There is no filesystem lookup during invocation.

**Why this priority**: Overwriting developer files destroys work. Per-call filesystem lookup is slow and incorrect under revision pinning. One file per procedure keeps implementations independently editable.

**Independent Test**: Generate a new procedure file, edit it, regenerate, and confirm the implementation file is unchanged while the generated registry updates. Add a second procedure and confirm it is scaffolded in its own file, not appended to the first.

**Acceptance Scenarios**:

1. **Given** `CREATE PROCEDURE api.create_order(...)`, **When** generation runs for the first time, **Then** `functions/src/api/create_order.ts` is created with `export const createOrder = procedure.api.createOrder.unimplemented()`.
2. **Given** that file already exists with developer code, **When** generation runs again, **Then** the implementation file is not overwritten.
3. **Given** several implemented named exports, **When** generation or build runs, **Then** the generated registry maps qualified names such as `api.create_order` to those named exports and is the build entrypoint.
4. **Given** a running server, **When** a procedure is invoked, **Then** dispatch uses the compiled registry, not a live filesystem path.
5. **Given** generation, **When** contracts and host API declarations are written, **Then** SQL input/output types, shared `ctx` `.d.ts`, and generated `procedure` builders are written, and developer-owned implementation files are still not overwritten.
6. **Given** two bodyless procedures in the same schema, **When** generation runs for the first time, **Then** each identity is scaffolded at its own `src/<schema>/<procedure>.ts` path.

---

### User Story 4 - Write Small Procedures Inline in SQL (Priority: P1)

A developer may put JavaScript (and TypeScript, with documented limits) directly in `CREATE PROCEDURE ... AS $$ ... $$`. `LANGUAGE` is required only when an inline body exists. Inline JavaScript is immediately executable after catalog apply. Inline TypeScript is stored as source but is not executed by the server until a project build has transpiled it.

**Why this priority**: Small health/tax/greeting procedures should not require a full project file. Inline TypeScript must not force a TypeScript compiler into the database process.

**Independent Test**: Create `LANGUAGE JAVASCRIPT` inline `api.health`, `CALL` it, then create `LANGUAGE TYPESCRIPT` inline source and confirm it is stored but not executed until a project deployment provides compiled output.

**Acceptance Scenarios**:

1. **Given** `CREATE PROCEDURE api.health() RETURNS TEXT LANGUAGE JAVASCRIPT AS $$ return "ok"; $$`, **When** an authorized caller invokes it and no project override exists, **Then** the result is `"ok"`.
2. **Given** a project-backed procedure with no body, **When** it is created, **Then** `LANGUAGE` is not required.
3. **Given** inline TypeScript in SQL, **When** the statement is applied without a compiled project artifact containing that procedure, **Then** the source is stored for introspection and execution is refused until a project deployment supplies compiled JavaScript.
4. **Given** inline source, **When** an operator inspects catalog metadata, **Then** language, source, and source hash are visible; compiled bytes remain content-addressed and are not unnecessarily duplicated.

---

### User Story 5 - Project Files Override Inline Fallbacks (Priority: P1)

A routine may have SQL inline implementation, project implementation, or both. At activation time the server resolves: active deployed project export wins; otherwise inline; otherwise not-implemented. Rollback to a revision without the override restores inline behavior without DDL.

**Why this priority**: This is the intended evolution path from inline stubs to a real project, and the reason inline must not be modeled as a separate deployed module per routine.

**Independent Test**: Create inline `api.health` returning `"basic"`, deploy a project file returning `"project"`, confirm the project result, rollback, and confirm `"basic"` returns.

**Acceptance Scenarios**:

1. **Given** an inline implementation and no matching export in the active revision, **When** the procedure is called, **Then** the inline implementation runs.
2. **Given** an active revision whose registry exports that procedure, **When** it is called, **Then** the project implementation runs even though inline source still exists in the catalog.
3. **Given** rollback to a revision that does not export the procedure, **When** it is called, **Then** the inline implementation is active again without any `CREATE OR REPLACE`.
4. **Given** neither project export nor inline source, **When** it is called, **Then** the caller receives `PROCEDURE_NOT_IMPLEMENTED`.
5. **Given** any of the above, **When** a call is in flight, **Then** resolution is not re-checked via filesystem per nested call.

---

### User Story 6 - Deploy One Module Containing Many Procedures (Priority: P1)

Operators deploy a named functions module (for example `backend`) as one revision containing one artifact and many procedures. The system does not create a module or revision per routine. Inline routines are catalog metadata plus optional compiled inline artifacts, not normal project modules.

**Why this priority**: Hundreds of procedures must not create hundreds of modules, caches, or activation records.

**Independent Test**: Deploy a project with many procedures and confirm a single module revision is created and all exported procedures share that revision.

**Acceptance Scenarios**:

1. **Given** a functions project configured as module `backend`, **When** it is deployed, **Then** one function module revision and one artifact are created covering all exported procedures.
2. **Given** that revision, **When** an operator lists revisions, **Then** they see module-level rows, not one row per procedure.
3. **Given** inline procedures, **When** they are created, **Then** they do not each become a separate project module.
4. **Given** activation, **When** the active set is published, **Then** each routine maps to project implementation, inline implementation, or missing—without a routine-id-to-module-id identity.

---

### User Story 7 - In-Flight Calls Survive Deploy and Rollback (Priority: P1)

When a request starts, it pins the currently active function deployment. A deploy or rollback that happens during the request does not change which implementations nested calls see. New root requests use the new active set. Historical artifacts remain durable for rollback; they are not kept resident merely because history exists.

**Why this priority**: Mixed revisions inside one request corrupt transactions and make rollback unsafe.

**Independent Test**: Start a slow request on revision 41, activate 42, start a second request, and confirm the first finishes entirely on 41 including nested calls while the second uses 42.

**Acceptance Scenarios**:

1. **Given** revision 41 active and a long-running root request A, **When** revision 42 is activated, **Then** A and all of its nested procedures continue using 41.
2. **Given** that activation, **When** request B starts, **Then** B uses 42.
3. **Given** a rollback 43 → 42, **When** in-flight roots from 43 finish, **Then** they complete on 43 and new roots use 42.
4. **Given** an inactive revision with no pinned roots, **When** memory pressure or idle policy applies, **Then** its warm instances and compiled caches are eligible for release while catalog/filestore history remains.

---

### User Story 8 - One Root Request, One Transaction, Nested Calls Stay Inside It (Priority: P1)

SQL `CALL`, REST, wire protocol, table triggers, topic triggers, and scheduled calls each create one root execution. Nested procedure calls do not create another root, do not join a separate admission queue, and do not consume another concurrency slot. They share the root transaction, cancellation, deadline, and pinned deployment. Admission is based on root executions.

**Why this priority**: Today nested calls can re-enter general engine admission and land on another worker. That wastes memory and breaks request-local guarantees.

**Independent Test**: Call A which calls B which calls C; confirm one transaction, one root, nested depth accounting, and that nested calls succeed even when root concurrency is already at the configured maximum.

**Acceptance Scenarios**:

1. **Given** a REST, SQL, wire, trigger, or scheduled invocation, **When** it starts, **Then** exactly one root execution is created with identity, origin, namespace, deadline, cancellation, pinned deployment, and transaction scope.
2. **Given** procedure A calling B calling C, **When** those calls run, **Then** they share A's root, transaction, and execution resources.
3. **Given** any failure in A, B, C, a table trigger, or an explicit topic publish in that root, **When** the failure occurs, **Then** the entire root transaction rolls back.
4. **Given** a read-only procedure that never mutates, **When** it completes, **Then** no write transaction is allocated unless existing transactional guarantees require one.
5. **Given** the first mutation in that root (SQL change, nested write, trigger write, or topic publish), **When** it occurs, **Then** one write transaction is begun and reused.
6. **Given** nesting beyond the configured small maximum (default 16), **When** another nested call is attempted, **Then** the call is rejected with a resource-limit error.
7. **Given** host procedure dispatch, **When** JavaScript requests a nested call, **Then** ACL, security mode, lookup, transaction, and revision pinning are enforced by the host—not by a local JavaScript function reference that bypasses checks, and not by an HTTP or SQL string roundtrip.

---

### User Story 9 - Actor Stays Fixed; Principal Follows Security Mode (Priority: P1)

The person or service that started the root request is the immutable actor. Each procedure frame has an effective principal: invoker uses the caller frame principal; definer uses the procedure owner. JavaScript cannot supply either value. `EXECUTE` is checked before entering the target under the caller. Row-level security uses the frame's effective principal. Definer frames must not inherit an attacker-controlled name search path.

**Why this priority**: Confused actor/principal is a privilege-escalation class of bug. Search-path shadowing is a classic definer attack.

**Independent Test**: User calls a definer procedure that calls an invoker procedure; confirm actor never changes, principal switches to owner then restores, EXECUTE is enforced at each hop, and RLS sees the effective principal.

**Acceptance Scenarios**:

1. **Given** `SECURITY DEFINER` procedure owned by `api_owner` called by `user_123`, **When** it runs, **Then** actor is `user_123` and principal is `api_owner`.
2. **Given** a nested `SECURITY INVOKER` call from that frame, **When** it runs, **Then** principal is `api_owner` (caller frame principal) and actor remains `user_123`.
3. **Given** return from the nested call, **When** the outer frame continues, **Then** principal is restored to the outer frame's principal.
4. **Given** missing `EXECUTE` on the target, **When** a nested or root call is attempted, **Then** the call is denied before entering the routine.
5. **Given** generated database object access inside a definer frame, **When** it runs, **Then** objects resolve by canonical identity (or a safe deterministic search path) and cannot be shadowed by a caller-writable schema.
6. **Given** JavaScript, **When** it attempts to set actor or principal, **Then** there is no capability to do so.

---

### User Story 10 - Function Memory and Concurrency Stay Inside a Global Budget (Priority: P1)

Operators configure a global functions memory ceiling plus concurrency and queue limits. Per-instance heap limits must not multiply into a multi-gigabyte implicit envelope. The runtime starts empty. Workers default conservatively (not half of all CPUs). Warm instances stay few, revision-aware, and idle-evicted. Admission respects both concurrency and memory. Queues are bounded and hold lightweight references, not copied artifacts.

**Why this priority**: Current defaults can theoretically allow tens of large heaps at once inside a database process. That is unacceptable for a lightweight server.

**Independent Test**: Configure a modest global budget, drive concurrent invocations, confirm admission/backpressure, confirm idle instances drop, and confirm RSS growth tracks the budget rather than `max concurrent × max heap`.

**Acceptance Scenarios**:

1. **Given** a server that has never invoked a function, **When** it is idle, **Then** no function execution instances are allocated.
2. **Given** default configuration, **When** the functions runtime starts, **Then** the number of dedicated function workers is conservative and configurable (starting from the lesser of available CPUs and 4 unless benchmarks justify another default).
3. **Given** configured `max_memory`, `max_active`, and `max_queued`, **When** load exceeds those limits, **Then** new root requests receive backpressure/capacity errors rather than unbounded growth.
4. **Given** a completed invocation whose instance is still healthy and below the soft heap threshold, **When** pooling policy allows, **Then** the instance may be reused.
5. **Given** an instance above the soft threshold after cleanup, or whose cleanup cannot be proven, **When** the invocation ends, **Then** the instance is destroyed and not returned to the pool.
6. **Given** an active revision, **When** warm pooling occurs, **Then** idle instances are keyed by revision and default to at most one warm instance per worker/revision unless measurement justifies more.
7. **Given** a superseded revision with no pinned roots, **When** idle policy runs, **Then** its idle instances are dropped and its source is eligible for cache eviction.
8. **Given** a queued root request, **When** it waits, **Then** the queue entry does not duplicate artifacts or full request context; oversized inputs are rejected before enqueue by an explicit input-size limit.
9. **Given** nested calls, **When** they run, **Then** they do not consume additional root concurrency slots or additional global heap budgets as if they were new roots.

---

### User Story 11 - Pooled Executions Cannot Leak Request State (Priority: P1)

Request-specific mutable JavaScript state must not leak across unrelated requests. Module-level `let counter = 0` is not durable application state. Before reuse, an instance must drop host pointers, request context, transactions, timers, response state, and pending work. If cleanup is not provable, destroy the instance. Timeouts cancel JavaScript, host work, database work, nested calls, and queued work.

**Why this priority**: Pooling without isolation is a security and correctness defect.

**Independent Test**: Run a procedure that stores the actor id in a module-level variable and returns the previous value; consecutive unrelated calls must not observe the prior actor. Repeat after a timeout.

**Acceptance Scenarios**:

1. **Given** a procedure that assigns module-global state from the current actor, **When** two unrelated requests run (possibly on a pooled instance), **Then** the second request does not observe the first request's actor or other request data.
2. **Given** request A times out, **When** request B later runs, **Then** B sees no state from A; if A's instance cannot be proven clean, it is destroyed rather than reused.
3. **Given** any pending promise, timer, host operation, or transaction reference after a request, **When** pooling is considered, **Then** the instance is destroyed unless those are fully drained/rejected and verified.
4. **Given** a timeout, **When** it fires, **Then** JavaScript is terminated, host operations are cancelled, the transaction is aborted, and an unhealthy instance is discarded.

---

### User Story 12 - HTTP Context Is Compact, Least-Privilege, and Response-Safe (Priority: P2)

HTTP-origin roots expose method, path, selected headers, and query parameters on demand. Credential headers are not exposed by default. Cookies have explicit semantics rather than raw header dumping. Nested procedures may read the root request and must not mutate the root response. Response headers are validated and size-limited. Non-HTTP roots expose no HTTP context.

**Why this priority**: Eager string conversion of all headers copies credentials and wastes allocations. Unrestricted response headers are a smuggling hazard.

**Independent Test**: Invoke over REST with `Authorization` and `x-client-version`; confirm the latter is readable, the former is not, nested code cannot change status, and hop-by-hop response headers are rejected.

**Acceptance Scenarios**:

1. **Given** a REST invocation, **When** a procedure reads `ctx.http.request.method`, path, `headers.get("x-client-version")`, and `query.get("lang")`, **Then** those values are available without converting every incoming header to a string up front.
2. **Given** `Authorization` or `Proxy-Authorization`, **When** a procedure asks for them, **Then** access is denied unless an explicit capability/allowlist exists (none by default).
3. **Given** a non-HTTP origin (SQL `CALL`, trigger, schedule), **When** a procedure inspects HTTP context, **Then** it is absent/null.
4. **Given** a root HTTP procedure, **When** it sets status, a safe header such as `Location`, and content type, **Then** those apply to the HTTP response.
5. **Given** a nested procedure, **When** it attempts to mutate the root HTTP response, **Then** the attempt is rejected.
6. **Given** unsafe/hop-by-hop names such as `Connection`, `Transfer-Encoding`, `Content-Length`, or `Host`, **When** a procedure sets them, **Then** they are rejected.
7. **Given** too many or too large response headers, **When** they are set, **Then** the operation fails under documented limits.

---

### User Story 13 - REST Returns the Procedure Result Directly with Typed Errors (Priority: P2)

The dedicated functions REST endpoint uses the SQL return type as the HTTP body. Success is not wrapped in `{ "status": "success", "result": ... }`. Failures use a consistent error envelope and stable error codes mapped to HTTP status. Status is never chosen by searching error message text.

**Why this priority**: The procedure contract already is the API. String matching for HTTP status is brittle and leaks internals.

**Independent Test**: Call a procedure returning `{ order_id: "123" }` and confirm the body is that object. Call with no grant, missing procedure, bad args, timeout, and unauthenticated access and confirm 403/404/400/timeout/401 respectively via typed codes.

**Acceptance Scenarios**:

1. **Given** a successful REST call whose SQL return type is an object, **When** the response is returned, **Then** the HTTP body is that object with no success envelope.
2. **Given** procedure not found, **When** REST is called, **Then** the status is 404 with a stable error code.
3. **Given** missing `EXECUTE`, **When** REST is called, **Then** the status is 403.
4. **Given** unauthenticated access to a non-public procedure, **When** REST is called, **Then** the status is 401.
5. **Given** invalid arguments, **When** REST is called, **Then** the status is 400.
6. **Given** a resource/concurrency limit, **When** REST is called, **Then** the status is 429 or the documented runtime limit status.
7. **Given** a procedure timeout, **When** REST is called, **Then** the status follows the documented 504/408 policy.
8. **Given** an internal runtime failure, **When** REST is called, **Then** the status is 500 and the body does not rely on substring matching of messages.

---

### User Story 14 - Function Code Has a Small, Explicit Capability Set (Priority: P1)

V1 function code is administrator-deployed application code, not a hostile multi-tenant sandbox. Still, by default there is no filesystem, TCP/UDP, child process, environment, native addon, Node builtins, `process`, `require`, or unrestricted dynamic import. String-based code generation (`eval`, `new Function`) is disabled when compatible with the bundle; otherwise deploy fails rather than silently weakening isolation. Host APIs (`db`, `functions`, `topics`, `http`, `log`) are thin and native. Nested procedure arguments stay on the typed in-process path; they are not JSON-encoded.

**Why this priority**: Developers will assume Node.js. The product must fail closed. Future untrusted multi-tenant isolation is explicitly out of scope.

**Independent Test**: Attempt eval, filesystem, network, and a native package; all fail. Call a nested procedure with typed arguments and confirm no JSON roundtrip. Confirm a pure calculation procedure does not initialize unused database query machinery.

**Acceptance Scenarios**:

1. **Given** default configuration, **When** function code attempts filesystem, sockets, process, environment, or native addons, **Then** the capability is unavailable.
2. **Given** a bundle that requires runtime `eval`/`new Function`, **When** it is deployed, **Then** deployment fails with a clear diagnostic.
3. **Given** a Node builtin or `.node` addon dependency, **When** the project is built, **Then** it is rejected unless Kalam provides an equivalent explicit capability.
4. **Given** nested procedure calls, **When** arguments and returns flow, **Then** they use the SQL/typed value boundary, not JSON stringify/parse.
5. **Given** a procedure that only adds two numbers or reads HTTP metadata, **When** it runs, **Then** it does not instantiate unused query-engine session state.
6. **Given** host database calls for a given effective principal, **When** several queries run in that frame, **Then** the derived execution identity/session is reused rather than rebuilt per query.
7. **Given** host operations, **When** a request exceeds configured counts or byte limits (SQL text, result rows/bytes, nested depth, topic publishes/payload, logs, HTTP headers, return bytes, input bytes), **Then** the request fails closed without bypassing the memory budget via Rust-side accumulation.

---

### User Story 15 - Unauthenticated HTTP Access Requires an Explicit Grant (Priority: P1)

Having a URL does not make a procedure anonymously callable. Invocation always passes `EXECUTE`. If an anonymous role exists, unauthenticated API procedures require an explicit grant. The secure default is no anonymous execute.

**Why this priority**: Accidental public APIs are a high-severity default.

**Independent Test**: Create a procedure, expose the REST route, call without credentials, and confirm denial until an explicit anonymous/public grant exists.

**Acceptance Scenarios**:

1. **Given** a procedure with a REST path and no anonymous `EXECUTE` grant, **When** an unauthenticated client calls it, **Then** the call is denied.
2. **Given** an explicit grant to the anonymous/public role, **When** an unauthenticated client calls it, **Then** the call is allowed under that principal and still subject to relation privileges and RLS.
3. **Given** a granted authenticated user, **When** they call the same path, **Then** `EXECUTE` is still checked for that user.

---

### User Story 16 - `functions build` Produces a Real Artifact and Manifest (Priority: P2)

`kalam functions build` compiles the contract snapshot, generates contracts and registry, validates implementations, runs configured typecheck/build, bundles an immutable artifact, writes a manifest, and validates exports against the contract. It does not require a live server. It is not schema generation alone.

**Why this priority**: Today's build only generates schema artifacts, so deploy cannot be honest.

**Independent Test**: Run `kalam functions build` offline against a project with a missing export and an extra unknown export; both fail. A valid project produces artifact + manifest hashes.

**Acceptance Scenarios**:

1. **Given** a valid project, **When** `kalam functions build` runs without a server, **Then** it produces a compiled artifact and a manifest including module name, runtime, ABI version, contract hash, artifact hash, source hash, lockfile hash, and per-procedure implementation kind (`module` vs `inline`).
2. **Given** a SQL procedure with neither inline body nor project file, **When** build runs, **Then** it fails as a missing export unless the procedure is documented as catalog-only inline-or-missing (missing is allowed at build only when the contract permits later not-implemented; default is fail-on-missing-for-project-backed routines).
3. **Given** a project export that does not correspond to a SQL procedure, **When** build runs, **Then** it fails as an unknown export.
4. **Given** inline fallbacks and project overrides, **When** the manifest is generated, **Then** it records which procedures are module vs inline.

---

### User Story 17 - Deploy Is an Atomic Revision Activation; Dry-Run Validates Locally (Priority: P2)

`kalam deploy` compiles the local contract, generates types, validates schema diff/migrations, builds functions, validates the manifest, uploads a content-addressed artifact, applies schema/catalog changes, creates an immutable revision, atomically activates it, optionally warms one instance, and health-checks activation. Existing revision artifacts are never mutated. Activation refuses mismatched contract hash, signature hashes, ABI, or export set. Dry-run performs all non-mutating local work and shows the planned revision without upload, catalog change, or activation.

**Why this priority**: Schema and artifact must not diverge. Dry-run that skips build is not useful.

**Independent Test**: Run `--dry-run` and confirm generate/typecheck/build/hash/plan output with no server mutation. Run a real deploy and confirm CAS activation and that old revisions remain immutable.

**Acceptance Scenarios**:

1. **Given** a successful deploy, **When** activation completes, **Then** catalog contract and function artifact hashes match and a single active pointer swap published the new set.
2. **Given** a contract/artifact/ABI/export mismatch, **When** activation is attempted, **Then** it is refused and the previous active revision remains.
3. **Given** `--dry-run`, **When** it completes, **Then** parse, diff, generate, typecheck, build, export validation, hashing, and planned revision display have run, and no migration, upload, catalog write, or activation occurred.
4. **Given** an existing revision, **When** a new deploy occurs, **Then** the old artifact bytes are not mutated.
5. **Given** activation success, **When** a subsequent invocation occurs, **Then** it does not re-read artifact bytes from durable storage per request; the active revision is shared.

---

### User Story 18 - Operators Can List, Inspect, and Roll Back Revisions (Priority: P2)

Operators use `kalam functions status`, `revisions`, `rollback <revision>`, and `logs [procedure]`. Rollback is an active-pointer swap after validating that the target artifact exists, ABI is supported, and the contract is compatible with the current schema. It does not rebuild. In-flight roots stay on the old revision; new roots use the rolled-back revision.

**Why this priority**: Rollback is currently unimplemented, which makes revision history unusable.

**Independent Test**: Deploy twice, list revisions with active/ready status, rollback, and confirm behavior returns to the prior implementation.

**Acceptance Scenarios**:

1. **Given** several deployments, **When** `kalam functions revisions` runs, **Then** it lists module, revision, status (`active`/`ready`), contract identity, and created time.
2. **Given** `kalam functions status`, **When** it runs, **Then** it shows the active module revision and whether the runtime is healthy.
3. **Given** `kalam functions rollback 42`, **When** 42's artifact exists, ABI is supported, and the contract is compatible, **Then** the active pointer becomes 42 without rebuilding.
4. **Given** an incompatible target revision, **When** rollback is attempted, **Then** it fails and the active revision is unchanged.
5. **Given** `kalam functions logs`, **When** it runs, **Then** operators can inspect recent structured function errors for a procedure without secret material.

---

### User Story 19 - Local Dev Rebuilds Functions Without Restarting the Database (Priority: P2)

`kalam dev` watches schema SQL, function sources, package manifest, lockfile, and TypeScript config. Schema changes recompile the contract, generate contracts, scaffold missing procedures, and typecheck. Function changes incrementally bundle, create a local revision, and activate it. The database process is not restarted for function edits.

**Why this priority**: Restarting the database on every TypeScript edit makes the workflow unusable.

**Independent Test**: Run `kalam dev`, add a new procedure SQL file, confirm scaffold, implement it, save, and invoke the new behavior without restarting the server.

**Acceptance Scenarios**:

1. **Given** `kalam dev` running, **When** schema SQL changes, **Then** contracts regenerate, missing procedure files are scaffolded once, and the project typechecks.
2. **Given** `kalam dev` running, **When** function source or dependencies change, **Then** an incremental bundle produces a new local revision and activates it.
3. **Given** those updates, **When** the developer invokes the procedure, **Then** the new behavior is live without a database restart.

---

### User Story 20 - Inline Procedures Are Not Auto-Overridden by Scaffolding (Priority: P2)

If a procedure has inline SQL source, generation does not automatically create an override file. Status output shows `implementation: inline`. An explicit `kalam functions override <name>` scaffolds `functions/src/<schema>/<procedure>.ts`, optionally seeding from the inline body. Once present, project builds treat it as an override.

**Why this priority**: Auto-creating files for inline procedures fights the fallback model.

**Independent Test**: Create inline `api.health`, run generate/dev, confirm no `src/api/health.ts`, then run override and confirm the file appears and subsequent builds override inline.

**Acceptance Scenarios**:

1. **Given** an inline procedure and no override file, **When** generate or `kalam dev` runs, **Then** no implementation file is created automatically and status reports inline.
2. **Given** `kalam functions override api.health`, **When** it runs, **Then** `functions/src/api/health.ts` is created and may be initialized from the inline body when practical.
3. **Given** that file, **When** the project is built and deployed, **Then** the module implementation is active.

---

### User Story 21 - Operators Can Observe Function Health Without Logging Secrets (Priority: P3)

Operators see low-cardinality metrics for invocations, errors, timeouts, memory, queue wait, instance create/destroy, host DB time, and nested depth. Structured invocation records include execution identity, procedure, module, revision, actor, origin, outcome, duration, and stable error code. V8 `console.*` and `ctx.log.*` output is stored on the same log with `outcome=log`. Credentials, cookies, bodies, secrets, and inputs are not logged by default. Active runs are in-memory only and visible through a virtual `system.active_procedure_runs` view while the root is live. Invocation history is disk-backed in rotating `{data_path}/functions/runtime/<procedure_id>/logs/procedures.jsonl` files and queried through `system.procedure_logs`. Resident V8 isolates appear in `system.module_instances`.

**Why this priority**: Without this, memory bugs and leaks cannot be operated. Logging secrets is a security incident.

**Independent Test**: Invoke, fail, and timeout procedures; scrape metrics; query active runs during a slow call; confirm labels exclude user ids and inputs; confirm logs omit Authorization and bodies.

**Acceptance Scenarios**:

1. **Given** function traffic, **When** metrics are scraped, **Then** counts and histograms exist for invocations, errors, timeouts, out-of-memory, duration, queue wait, active/idle instances, instance create/destroy, heap bytes, revision-cache bytes, host DB duration, host call duration, and nested depth.
2. **Given** those metrics, **When** labels are inspected, **Then** they do not include user id, request id, or arbitrary procedure input.
3. **Given** a function error, **When** it is logged, **Then** it includes execution id, request id, routine, revision, actor, effective principal, invocation source, procedure stack, and stable error code.
4. **Given** default configuration, **When** a request with `Authorization`, cookies, and a body fails, **Then** those values are not automatically logged.
5. **Given** an in-flight root, **When** an operator queries `system.active_procedure_runs`, **Then** the run appears, and it disappears as soon as the root finishes. No durable row is written per invocation.
6. **Given** a completed CALL, **When** an operator queries `system.procedure_logs`, **Then** a structured row exists with procedure identity and outcome (`ok` or `error`), V8 `console.*`/`ctx.log.*` lines appear as `outcome=log` with channel and the isolate text, uncaught exceptions/unhandled rejections appear as `outcome=error` with the JavaScript message, and records do not contain request bodies, arguments, results, tokens, or source.

---

### User Story 22 - Developers Complete Init-to-Rollback Without Surprise Defaults (Priority: P2)

A developer can initialize a project, write types and procedures, run `kalam dev`, implement TypeScript with helpers and a pure package, build, call via REST and SQL, deploy, inspect revisions, change behavior, deploy again, rollback, and see the previous behavior.

**Why this priority**: This is the end-to-end product promise. Partial CLI (build = generate, rollback = error, deploy without artifact rollout) fails this journey.

**Independent Test**: Execute the full journey in a clean workspace against a running server and record pass/fail of each step.

**Acceptance Scenarios**:

1. **Given** `kalam init` plus `CREATE TYPE`/`CREATE PROCEDURE`, **When** `kalam dev` runs, **Then** the scaffold exists and is not overwritten later.
2. **Given** a TypeScript implementation importing a helper file and a pure JS package, **When** `kalam functions build` and `kalam deploy` run, **Then** REST `POST /v1/functions/api/create_order` and SQL `CALL api.create_order(...)` both return the typed result.
3. **Given** a second deploy with changed behavior, **When** rollback runs, **Then** the previous behavior returns.

---

### User Story 23 - One Host Context API for Inline and Project Code (Priority: P1)

Inline SQL procedure bodies and project-deployed methods use the same host context: `ctx.actor`, `ctx.principal`, `ctx.db`, `ctx.functions`, `ctx.topics`, `ctx.http`, `ctx.log`, and generated `procedure.<schema>.<method>` builders. That surface is declared once in shared type-declaration files (`.d.ts`). Developers add a host method in one place; both inline and project code see it. SQL contract input/output types remain generated from the schema; they are not duplicated into the host API file.

**Why this priority**: Two context shapes (today's inline wrapper vs project stubs) teach two products. A single declaration file is how we keep inline fallbacks and project overrides interchangeable.

**Independent Test**: Implement `api.health` inline and `api.create_order` in the project against the same generated `.d.ts`. Typecheck both. Change a host method in that file only, regenerate, and confirm both implementations typecheck against the new surface while developer-owned files are not overwritten.

**Acceptance Scenarios**:

1. **Given** a functions project, **When** generation runs, **Then** a non-developer-owned host API declaration file is written (for example `functions/src/generated/runtime.d.ts`) describing context and host methods, plus `procedure.d.ts` builders.
2. **Given** those files, **When** a project procedure uses `procedure.<schema>.<method>(handler)` and `ctx`, **Then** the TypeScript project infers `ctx` and SQL input/output types — not a second handwritten context type.
3. **Given** an inline `LANGUAGE JAVASCRIPT` or `LANGUAGE TYPESCRIPT` body that uses `ctx`, **When** `kalam functions build` or `kalam dev` typechecks it, **Then** it is checked against the same host API declarations (via a generated shim if the SQL body cannot import files directly).
4. **Given** a new host method added to the shared declarations and native host, **When** an inline body and a project file both call it, **Then** both compile against the same types and both execute the same runtime behavior.
5. **Given** regeneration, **When** host API `.d.ts` files are rewritten, **Then** developer-owned `src/**` files are not overwritten; only generated declaration/contract files change.
6. **Given** no functions project yet, **When** an inline JavaScript procedure runs, **Then** it still receives the same runtime `ctx` surface; the `.d.ts` is the developer contract, not a second runtime.

---

### User Story 24 - Request and Response Cross the Runtime as One FlatBuffer Copy (Priority: P1)

Arguments and return values that enter or leave the JavaScript runtime use a FlatBuffer in-memory representation. Before encoding, the runtime checks whether the value is already that buffer (or a view over it). If it is, the same memory is reused — no second copy, no JSON roundtrip. If it is not, the value is written once into an internal FlatBuffer and that single copy is what the runtime and the host share. Inline and project implementations use this path. Nested in-process calls pass the same buffer when the callee can consume it.

**Why this priority**: Request/response serdes is on every invocation. Cloning object graphs into JSON or a persistence codec, then again into JavaScript, wastes memory and fights the global budget. FlatBuffers exist to be traversed without rebuilding a parallel tree.

**Independent Test**: Invoke a procedure with a structured input that is already a FlatBuffer and confirm no extra payload clone is taken. Invoke with a value that is not yet a FlatBuffer, confirm one internal encode, then a nested call that reuses that buffer. Confirm REST JSON at the HTTP edge is converted once into this form rather than JSON→objects→JSON→JS.

**Acceptance Scenarios**:

1. **Given** a root invocation whose input is already an in-memory FlatBuffer for the procedure's contract, **When** it is passed into the JavaScript runtime, **Then** the runtime reuses that buffer (or a zero-copy view) instead of encoding again.
2. **Given** a value that is not already a FlatBuffer (SQL argument, freshly built host value, or HTTP JSON after the network parse), **When** it crosses into the runtime, **Then** it is written once to an internal FlatBuffer and that copy is the crossing representation.
3. **Given** a procedure return that is already a FlatBuffer view produced by the runtime, **When** the host sends REST or SQL results, **Then** it reads from that buffer without first rebuilding a duplicate object graph or JSON-encoding for the in-process hop.
4. **Given** a nested procedure call, **When** arguments are already the FlatBuffer form, **Then** the callee receives the same memory rather than a newly encoded copy.
5. **Given** inline and project implementations of the same contract, **When** either runs, **Then** they use the same request/response crossing rules.
6. **Given** the HTTP or SQL network/session edge, **When** JSON or typed SQL values arrive, **Then** that edge conversion happens once; the JavaScript boundary does not perform a second JSON stringify/parse.
7. **Given** durable table/catalog storage, **When** function values are persisted, **Then** storage still uses the existing central serialization ownership — the FlatBuffer crossing is an in-memory runtime transfer format, not a second on-disk row codec.

---

### Edge Cases

- Nested call while a deploy/rollback races: children must use the root's pinned set, never a newly activated set.
- Nested call when root concurrency is already at `max_active`: nested work still proceeds on the existing root.
- Procedure with both inline source and a project file that is not in the *active* revision: inline remains active.
- Inline TypeScript stored in catalog but never built: execution refused with a clear diagnostic, not a runtime TypeScript parse.
- Rollback target missing artifact bytes: rollback fails closed.
- Rollback target ABI newer/older than the running server supports: rollback fails closed.
- Rollback target contract incompatible with current schema: rollback fails closed.
- Schema migrated without a compatible function artifact (or the reverse): activation/deploy fails; the previous consistent pair remains active.
- Isolate/instance cleanup after timeout, rejected host promise, or cancellation cannot be proven: destroy, do not reuse.
- Module-level JavaScript variables across pooled requests: treated as leak; V1 requires fresh-request semantics.
- Huge arguments, huge database results, huge return values, or huge topic payloads: rejected by explicit limits before they can exceed the global memory budget.
- CPU-bound infinite loop: deadline/termination still fires; the instance is not returned to the pool unless proven healthy.
- Unauthenticated REST hit on a URL that exists: denied unless anonymous `EXECUTE` was granted.
- Nested procedure tries to set HTTP status: rejected.
- Package with install/lifecycle scripts: those scripts never run inside the database server.
- Function worker saturation with queued work: queue bound is enforced; excess roots fail with capacity errors.
- Historical revisions in catalog: must not keep heaps or source resident solely because they exist.
- Inline SQL body cannot `import` a `.d.ts` file: typecheck uses a generated shim that references the shared host declarations; runtime still injects the same `ctx`.
- Host API `.d.ts` drift from native host methods: build/typecheck must fail when generated declarations and the runtime ABI do not match.
- Value is already a FlatBuffer but for a different contract/schema hash: do not zero-copy; re-encode or reject as a contract mismatch.
- Value is a JavaScript object the procedure mutated: do not assume the original input buffer is still the return; write a new internal FlatBuffer from the return value.
- Zero-copy into the runtime is unsafe because buffer lifetime, alignment, or writability cannot be guaranteed: encode once into a runtime-owned FlatBuffer and use that single copy rather than falling back to JSON.
- Nested call needs only a subset of a large input: still pass the shared buffer or a zero-copy view; do not serialize a second full tree unless a new value was constructed.
- REST JSON body larger than the input-bytes limit: reject before FlatBuffer encode and before queue admission.

## Requirements *(mandatory)*

### Functional Requirements

#### Contract and project model

- **FR-001**: SQL MUST own procedure names, parameters, return types, composite and table row types, security mode, `EXECUTE` privileges, triggers, schedules, and optional inline fallback source.
- **FR-002**: The functions project MUST own business logic, helpers, supported dependencies, tests, package manifest, TypeScript config, and lockfile.
- **FR-003**: SQL MUST NOT require a source-file or export-name mapping such as `AS 'src/api/orders.ts', 'createOrder'`.
- **FR-004**: Project runtime (`path`, `runtime`, `module`) MUST be configured at the functions-module level, not per procedure.
- **FR-005**: Project-backed procedures with no inline body MUST NOT require a `LANGUAGE` clause.
- **FR-006**: `LANGUAGE` MUST be required only when an inline body is present.

#### Generation and dispatch

- **FR-007**: First-time generation MUST create `functions/src/<schema>/<procedure>.ts` with a named `procedure.<schema>.<method>.unimplemented()` export only when that procedure identity is not already bound in any project file.
- **FR-008**: Generation MUST NEVER overwrite an existing developer-owned implementation file.
- **FR-009**: Generation MUST emit a registry mapping qualified procedure names to implemented named exports discovered from `procedure.<schema>.<method>(handler)` bindings; that registry MUST be the build entrypoint. `.unimplemented()` bindings MUST be omitted from the registry.
- **FR-010**: Invocation dispatch MUST use the compiled registry and MUST NOT perform filesystem lookup per call.
- **FR-084**: Generation MUST emit shared host-context type-declaration files (`.d.ts`) as the single source of types for `ctx`, host methods, and generated `procedure` builders.
- **FR-085**: Project implementation files MUST typecheck against those shared host declarations plus generated SQL contract types; they MUST NOT define a parallel `ctx` API.
- **FR-086**: Inline procedure bodies MUST use the same host method names and semantics as project methods, and MUST be typechecked against the same `.d.ts` files whenever a project/typecheck runs.
- **FR-087**: Host API type-declaration files MUST be generated and non-developer-owned; adding a host capability MUST update that declaration once, the native host once, and then be available to both inline and project code.
- **FR-088**: Generated SQL contract types (`input`/`output` composites) MUST remain separate from the host API `.d.ts` so schema types are not duplicated into the runtime declarations.

#### Inline vs project resolution

- **FR-011**: The system MUST support inline JavaScript bodies in `CREATE PROCEDURE`.
- **FR-012**: The system MUST store inline TypeScript bodies, but V1 MUST NOT execute TypeScript inside the server; execution requires a compiled project artifact.
- **FR-013**: Active implementation resolution MUST be: active project export, else inline, else not-implemented.
- **FR-014**: Resolution MUST occur at activation/publish of the active set, not via per-call filesystem checks.
- **FR-015**: Catalog routine metadata MUST record inline language, source, and source hash without modeling each inline routine as a project module.
- **FR-016**: Compiled inline and project artifacts MUST remain content-addressed; raw inline source MAY remain catalog-visible for introspection.

#### Active set, roots, and nesting

- **FR-017**: The system MUST publish one immutable active function set containing generation identity, contract identity, optional module revision, and the resolved implementation for every routine.
- **FR-018**: Every root invocation MUST take a pinned view of that active set and MUST use it for all nested calls.
- **FR-019**: Root invocations MUST be created for REST, SQL `CALL`, wire-protocol `CALL`, table triggers, topic triggers, and scheduled calls.
- **FR-020**: Nested procedure calls MUST NOT create a new root, MUST NOT re-enter root admission, and MUST NOT consume another root concurrency slot.
- **FR-021**: Nested calls MUST share the root transaction, deadline, cancellation, namespace, and pinned deployment.
- **FR-022**: Nested call depth MUST be bounded by a small configured maximum (default 16).
- **FR-023**: Nested dispatch MUST enforce `EXECUTE`, security mode, lookup, transaction, and revision pinning in the host.

#### Identity, ACL, and transactions

- **FR-024**: Actor identity MUST be immutable for the root and MUST NOT be settable from function code.
- **FR-025**: Effective principal MUST be caller-frame principal for `SECURITY INVOKER` and procedure owner for `SECURITY DEFINER`.
- **FR-026**: `EXECUTE` MUST be checked before entering a target routine under the caller frame; nested calls repeat the same sequence.
- **FR-027**: Relation privileges and RLS MUST remain separate from `EXECUTE` and MUST use the frame's effective principal for RLS.
- **FR-028**: Definer frames MUST resolve generated database objects by canonical identity or a safe deterministic search path; they MUST NOT inherit an attacker-controlled search path.
- **FR-029**: Unauthenticated/anonymous `EXECUTE` MUST be denied by default even if a REST URL exists.
- **FR-030**: One root MUST own one transactional scope; nested procedures, table triggers, and explicit topic publishes in that root MUST participate in it.
- **FR-031**: Failure anywhere in that scope MUST roll back everything.
- **FR-032**: Write transactions MAY be started lazily on first mutation only if existing guarantees are preserved.

#### Runtime, memory, and pooling

- **FR-033**: Function runtime initialization MUST be lazy: no execution instances until needed.
- **FR-034**: Process-level runtime platform initialization MUST happen once, not per worker, revision, or request.
- **FR-035**: Dedicated function workers MUST default conservatively (lesser of available CPUs and 4, configurable) rather than half of all CPUs without measurement.
- **FR-036**: Operators MUST be able to configure global functions memory budget, active root concurrency, queue depth, per-instance soft recycle threshold, and per-instance hard heap limit.
- **FR-037**: Admission MUST enforce both concurrency and global memory budgets.
- **FR-038**: `max_active × max heap` MUST NOT be treated as an acceptable implicit memory envelope.
- **FR-039**: Warm pools MUST be small, revision-keyed, and idle-evicted; they MUST NOT be pre-created up to `max_active`.
- **FR-040**: After invocation, instances below the soft threshold MAY be reused; instances still above it, or unproven-clean, MUST be destroyed.
- **FR-041**: Revision cache MUST be byte-weighted, strongly retain the active revision and revisions with in-flight roots, and LRU-evict everything else.
- **FR-042**: Artifact bytes MUST be loaded once per revision and shared; they MUST NOT be re-read from durable storage for every procedure call.
- **FR-043**: When a revision becomes inactive, the system MUST stop assigning new roots, drain pinned roots, drop idle instances, and allow compiled/source caches to evict.
- **FR-044**: Queue entries MUST be bounded in count and MUST NOT enqueue duplicated artifacts; large inputs count against an explicit limit before admission.
- **FR-045**: Timeouts and cancellation MUST cover JavaScript execution, host futures, database queries, nested procedures, topic operations, and queued requests.

#### HTTP and REST

- **FR-046**: REST function requests MUST NOT eagerly convert all headers, including credentials, into allocated strings.
- **FR-047**: Credential-bearing headers (`Authorization`, `Proxy-Authorization`) MUST be blocked from function code by default.
- **FR-048**: Cookie access MUST use explicit security semantics rather than exposing the raw `Cookie` header by default.
- **FR-049**: Ordinary application headers (for example `x-client-version`, `accept-language`, `x-request-id`, `stripe-signature`) MUST be readable on demand.
- **FR-050**: HTTP context MUST be absent for non-HTTP roots.
- **FR-051**: Root HTTP procedures MAY set status, safe headers, and content type; nested procedures MUST NOT mutate the root response.
- **FR-052**: Hop-by-hop and unsafe response headers (`Connection`, `Transfer-Encoding`, `Content-Length`, `Host`) MUST be rejected unless a documented exception exists.
- **FR-053**: Response header count, name length, value length, and total bytes MUST be limited.
- **FR-054**: The dedicated functions REST success body MUST be the declared SQL return value with no `{ status, result }` wrapper.
- **FR-055**: REST errors MUST use stable typed error codes mapped to HTTP status and MUST NOT choose status by searching error-message strings.

#### Capabilities, host APIs, and ABI

- **FR-056**: Function code MUST NOT have filesystem, TCP/UDP, child process, environment, native addon, Node builtin, `process`, `require`, or unrestricted dynamic import capabilities by default.
- **FR-057**: `eval` and `new Function` MUST be disabled when compatible with the bundle; otherwise deployment MUST fail closed.
- **FR-058**: Host APIs MUST remain thin (`db`, `functions`, `topics`, `http`, `log`) and MUST match the shared `.d.ts` surface; the runtime MUST NOT inject a large JavaScript framework into every instance.
- **FR-059**: Nested in-process calls MUST NOT JSON-encode and MUST NOT pass through the durable storage codec. They MUST pass the in-memory FlatBuffer form or a zero-copy view of it when the value is already in that form.
- **FR-089**: Before encoding a request or response for the JavaScript runtime, the system MUST check whether the value is already an in-memory FlatBuffer for the current contract; if yes, it MUST reuse that memory rather than copying.
- **FR-090**: If the value is not already that FlatBuffer, the system MUST write it once into an internal FlatBuffer and use that single copy as the host↔runtime crossing representation.
- **FR-091**: REST/SQL network-edge parsing (for example HTTP JSON or SQL arguments) MAY occur once at that edge; the JavaScript boundary MUST NOT perform an additional JSON stringify/parse of the procedure body.
- **FR-092**: Procedure returns that are already the runtime FlatBuffer MUST be read by REST/SQL result encoding from that buffer without rebuilding a parallel object graph for the in-process hop.
- **FR-093**: Zero-copy MUST be skipped when contract hash, lifetime, alignment, or mutability makes reuse unsafe; the fallback is one new internal FlatBuffer copy, never JSON.
- **FR-094**: Durable persistence remains owned by the existing central serialization path; function FlatBuffers are an in-memory transfer format and MUST NOT become a second on-disk row codec.
- **FR-060**: The production function ABI MUST be a single async host-operation ABI; dual long-term ABIs MUST NOT remain.
- **FR-061**: Database host calls MUST NOT block the general async worker model; JavaScript on a function worker MAY suspend while a database future is pending.
- **FR-062**: Function invocation MUST NOT occupy a general-purpose blocking thread for the entire call as the normal execution mode; JavaScript MAY suspend while host database work runs asynchronously.
- **FR-063**: Derived database execution state MUST be cached per effective principal on the root and reused; it MUST NOT be rebuilt on every host SQL call.
- **FR-064**: Explicit host-operation limits MUST exist for host call count, SQL text length, DB result bytes/rows, nested depth, topic publish count/payload, log bytes, HTTP response headers, return bytes, and input bytes.
- **FR-065**: Package installation and lifecycle scripts MUST NEVER run inside the database server.
- **FR-066**: Pure JS/TS packages that bundle MAY be supported; Node builtin and native addon packages MUST be rejected.

#### CLI and deployment

- **FR-067**: `kalam functions build` MUST perform contract compile, codegen, implementation validation, configured typecheck/build, artifact bundle, manifest generation, and contract validation without requiring a live server.
- **FR-068**: The build manifest MUST include enough information to validate missing exports, unknown exports, inline fallbacks, overrides, contract hash, runtime ABI, artifact hash, and reproducibility hashes.
- **FR-069**: `kalam deploy` MUST upload a content-addressed artifact, apply schema/catalog changes, create an immutable revision, atomically activate it, and optionally warm/health-check one instance.
- **FR-070**: `kalam deploy --dry-run` MUST perform all non-mutating local validation/build work and MUST NOT apply migrations, upload, change catalog, or activate.
- **FR-071**: `kalam functions status`, `revisions`, `rollback <revision>`, and `logs [procedure]` MUST be implemented.
- **FR-072**: Rollback MUST be an active-pointer swap after artifact/ABI/contract compatibility checks, without rebuilding.
- **FR-073**: `kalam dev` MUST watch schema and function project files, regenerate/scaffold on schema change, incrementally bundle/activate on function change, and MUST NOT restart the database for those updates.
- **FR-074**: Inline procedures MUST NOT auto-scaffold override files; `kalam functions override <procedure>` MUST be the explicit path.
- **FR-075**: Activation MUST refuse inconsistent pairs of schema contract and function artifact.

#### Observability and operations

- **FR-076**: The system MUST expose the low-cardinality metrics listed in User Story 21.
- **FR-077**: Structured function error logs MUST include the fields in User Story 21 and MUST NOT automatically log credentials, cookies, bodies, secrets, or function inputs.
- **FR-078**: Active procedure runs MUST be tracked in memory only and exposed via `system.active_procedure_runs` while the root is live. Invocation history MUST be written to node-local rotating `{data_path}/functions/runtime/<procedure_id>/logs/procedures.jsonl` files and exposed via `system.procedure_logs`, including V8 `console.*`/`ctx.log.*` output per procedure and uncaught JavaScript exceptions/unhandled Promise rejections. Resident V8 isolates MUST be exposed via `system.module_instances` joined to module revisions, not per-procedure instance rows.
- **FR-079**: The core database orchestration MUST NOT depend on a specific JavaScript engine type; a minimal internal runtime boundary MUST exist so a later runtime can be evaluated without rewriting function semantics.
- **FR-080**: This feature MUST NOT ship additional JavaScript/WASM engines in the standard server binary. Unused experimental runtimes currently compiled into the default functions component MUST be removed from the default build and kept optional for experiments/tests only.
- **FR-081**: Alternative runtimes MAY be benchmarked in experimental, non-default builds; production runtime choice for this feature remains the current JavaScript engine after memory/security refactor, unless measurements later justify a separate change.
- **FR-082**: Common immutable runtime bootstrap MAY be snapshotted/reused after the refactored baseline is measured; request-specific state MUST NEVER be snapshotted. User-code snapshots MUST NOT be implemented unless measurement shows substantial gain.
- **FR-083**: User module compilation MUST NOT restart from source on every request when reusable compile/preparation state for a revision is available.

### Key Entities

- **SQL Contract Snapshot**: Canonical, hashable description of types, procedures, signatures, security mode, and privileges. Independent of physical source layout.
- **Catalog Routine**: Stored procedure contract plus optional inline language, inline source, inline source hash, and optional compiled inline artifact identity.
- **Functions Project**: Developer-owned tree (manifest, lockfile, sources, tests) that implements procedures. Never executed as raw files by the server.
- **Function Module**: Named deployment unit (for example `backend`) containing many procedures. Not one module per routine.
- **Function Artifact**: Immutable compiled bundle plus manifest (module, runtime, ABI, hashes, per-procedure implementation kind).
- **Function Revision**: Immutable, content-addressed history record pointing at one artifact and one contract hash.
- **Active Function Set**: Immutable in-memory view published by atomic swap: generation, contract identity, module revision, and resolved implementation per routine (module export, inline, or missing).
- **Function Execution Root**: One invocation from REST/SQL/wire/trigger/schedule: actor, origin, namespace, deadline, cancellation, pinned active set, transaction scope, optional HTTP context, and cached principal sessions.
- **Procedure Frame**: Lightweight nested-call record: routine, effective principal, security mode. Small bounded stack.
- **Runtime Instance**: Pooled or freshly created execution instance owned by a root lease for the request lifetime; revision-keyed; destroyed if unclean or over soft heap after cleanup.
- **HTTP Invocation Context**: Compact, shared request metadata with on-demand header/query access, blocked credentials by default, and root-only response mutation.
- **Build Manifest**: Machine-readable record used to validate exports, hashes, ABI, and inline vs module implementation before activation.
- **Host API Declarations**: Generated `.d.ts` files that define `ctx` and `procedure` builders once for inline bodies and project files. Distinct from SQL contract types.
- **Runtime Value Buffer**: Single in-memory FlatBuffer (or zero-copy view) used to move a procedure request or response across the JavaScript boundary.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: A developer can go from SQL procedure contract to a working REST and SQL call using a TypeScript project file, without putting source paths in SQL, in a single documented workflow.
- **SC-002**: Regenerating contracts never overwrites an existing implementation file; 100% of existing developer-owned procedure files remain unchanged.
- **SC-003**: Deploying a project with at least 50 procedures creates exactly one module revision, not 50 modules.
- **SC-004**: After deploy of a project override, then rollback to a revision without that override, inline fallback behavior returns with no DDL; 100% of sampled procedures follow the documented resolution order.
- **SC-005**: While a slow request A runs on revision N, activating N+1 never causes A's nested calls to observe N+1; concurrent request B uses N+1. This holds in 100% of race tests.
- **SC-006**: Nested procedure chains of depth 5 share one root transaction: a failure at any depth rolls back all writes from that root.
- **SC-007**: Nested calls do not consume additional root concurrency slots; a nest of depth 3 succeeds even when active roots are already at the configured maximum (provided the original root was admitted).
- **SC-008**: An idle server that has never invoked functions shows no material functions-runtime instance memory beyond process baseline (no pre-created instance pool).
- **SC-009**: Under load, functions memory (instances + caches) remains within the configured global budget; it does not grow toward `max_active × max heap` and does not grow unbounded with revision history.
- **SC-010**: Default worker count is at most 4 on machines with more than 4 CPUs unless an operator explicitly raises it.
- **SC-011**: Unauthenticated REST calls are denied by default; 100% of procedures without an explicit anonymous grant reject anonymous execute.
- **SC-012**: Consecutive pooled requests cannot observe prior request actor/principal/HTTP/transaction/module-global mutation under V1 fresh-context tests, including after timeouts.
- **SC-013**: Successful REST responses return the SQL result as the HTTP body with no success wrapper; typed errors map to the documented statuses without message-substring matching.
- **SC-014**: `kalam deploy --dry-run` performs generate, typecheck, build, and export validation locally and performs zero catalog/artifact mutations.
- **SC-015**: `kalam functions rollback` restores the prior active revision's behavior without rebuilding; in-flight roots on the previous revision complete on that revision.
- **SC-016**: `kalam functions build` fails closed on missing required exports and unknown exports, and succeeds offline for a valid project.
- **SC-017**: Changing a TypeScript file under `kalam dev` becomes invokable without restarting the database.
- **SC-018**: After baseline measurement of the current implementation, no-op warm invocation regresses by no more than 10%, and database-heavy throughput regresses by no more than 5%, unless a documented, accepted tradeoff is recorded.
- **SC-019**: Nested calls are materially cheaper than admitting another root execution instance (no second instance allocation in the common path).
- **SC-020**: The standard server distribution's functions component does not include unused experimental runtimes; binary-size comparison versus the current default-enabled extra runtime is recorded and reduced.
- **SC-021**: Credential headers and request bodies are absent from default function logs in 100% of security log tests.
- **SC-022**: Operators can list active in-flight function roots through the system view during a slow call and see them disappear immediately after completion, with no durable per-invocation write.
- **SC-023**: Inline and project implementations typecheck against one shared host-context declaration set; adding a host method does not require a second handwritten `ctx` type in developer files.
- **SC-024**: 100% of sampled crossings into or out of the JavaScript runtime either reuse an existing contract-matching FlatBuffer or perform exactly one internal FlatBuffer write — never a JSON roundtrip at that boundary.
- **SC-025**: Nested calls whose arguments are already the runtime FlatBuffer do not allocate a second encoded payload copy in the common path.

## Assumptions

- This feature **evolves the existing Functions work** (PR #380 / `feat/server-functions`). It does not design Functions from scratch. Foundations to preserve: SQL contract snapshot, generated contracts/scaffolding/registry, content-addressed artifacts, compare-and-swap activation, immutable revision history, process-global runtime init, hard timeout and termination, typed value boundary, `EXECUTE` ACL, invoker/definer model, transaction coordinator, central serialization ownership, REST `/v1/functions/{namespace}/{procedure}`, and topic-trigger transaction semantics.
- The following current behaviors are **not** compatibility constraints if replacement tests pass: routine-id-as-module-id for project code; one module/revision per routine; nested calls re-entering general admission; nested calls moving to an arbitrary worker; implicit `max_active × max heap` memory; rebuilding effective query context on every host DB call; eager stringification of all HTTP headers; HTTP status via message substring matching; success REST wrapper; `functions build` as schema generation only; rollback returning unimplemented; extra experimental runtime enabled in the default build; two long-term execution ABIs.
- V1 function code is **administrator/DBA-deployed trusted application code**. In-process isolation is defense-in-depth, not a complete hostile multi-tenant sandbox. Process isolation, OS sandboxing, and shipping extra runtimes for untrusted cloud tenants are out of scope.
- Inline TypeScript in raw `CREATE PROCEDURE` is stored but requires project deployment before execution (no TypeScript compiler in the server for V1). Inline JavaScript remains immediately executable.
- The server never runs `npm install` or dependency lifecycle scripts.
- Default anonymous execute is off.
- Cookie raw-header access is off by default; a later explicit cookie API may be added.
- Conservative runtime defaults are a starting point and may be retuned after the required baseline benchmarks; they must not be raised without measurement.
- Common runtime bootstrap reuse (startup snapshot / code cache) is an optimization **after** the architecture refactor is measured. It is in scope to investigate and implement if beneficial; it is not a substitute for the memory/admission/module-model fixes.
- Alternative runtimes may be bench-only. This feature does not switch production runtime.
- Write-transaction laziness is adopted only if it does not weaken current transactional guarantees.
- Host-operation numeric limits will be given concrete defaults during planning after inspecting current limit code; the requirement is that they exist, are enforced, and cannot bypass the global memory budget.
- For project-backed routines, `functions build` fails on missing implementations. Catalog-only inline routines without a project file are valid and recorded as inline in the manifest when a project is built.
- Existing Meta-Raft/CAS activation remains the activation mechanism.
- Benchmarks must capture current-implementation baseline (tests, function benchmark, idle RSS, warm RSS, binary size) before architectural edits, and again after.
- Shared host API declarations live under generated output (for example `.kalam/generated/runtime.d.ts`) and may be referenced by `tsconfig`; they are regenerated, not hand-edited.
- Inline bodies are typechecked with a shim that binds `ctx` to those declarations because dollar-quoted SQL cannot import files.
- FlatBuffers at the function boundary are an in-memory transfer format. They do not replace the central durable serialization crate's storage objects, and callers still must not choose a storage codec per table row.
- Zero-copy means reusing the same bytes or a read-only view when contract, lifetime, and mutability allow. It does not mean JavaScript may mutate host memory through an unbounded raw pointer.
- If a procedure returns a newly constructed JavaScript object, that return is encoded once to FlatBuffer; the original input buffer is not treated as the response.

## Out of Scope

- Designing Functions from a blank slate or discarding working PR #380 foundations listed above.
- Shipping QuickJS, Deno, or Wasmtime as a function runtime.
- Treating in-process isolation as sufficient for untrusted multi-tenant customer code on a shared server.
- Server-side dependency installation.
- Server-side TypeScript compilation for ad-hoc DDL in V1.
- Per-procedure source mapping in SQL.
- Durable per-invocation run rows in the storage engine.
- Automatic scaffolding of override files for inline procedures.
- User-revision snapshots as a first optimization (static bootstrap reuse may be investigated after measurement).
- Introducing extra services, actors, IPC, or distributed components for function execution. KalamDB remains one database server.
- Weakening request isolation in order to keep more warm instances.

## Delivery Sequence *(planning constraint)*

Implementation MUST follow this order so measurement and safety are not skipped:

0. **Baseline**: existing tests, function benchmark, idle/warm RSS, binary size; commit the numbers.
1. **Freeze schema-first semantics**: project-backed syntax, inline syntax, override precedence, catalog fields, active-set model; parser/catalog tests before runtime refactor.
2. **Context/security**: root execution, frames, actor/principal, principal session cache, HTTP context, typed HTTP errors, sensitive headers, shared host API `.d.ts` aligned to that `ctx`.
3. **Runtime/admission**: root lease, same-root nested execution, smaller worker defaults, global memory budget, revision-aware pool, idle eviction; memory tests before snapshot work.
4. **Project revision deployment**: real build, manifest, one module revision, registry, shared runtime `.d.ts`, upload, CAS activation, pinned active set. Include request/response FlatBuffer reuse at the JavaScript boundary (zero-copy check, single internal encode fallback).
5. **Inline fallback + project override**: inline artifacts, override resolution, rollback fallback; no per-procedure module hack.
6. **CLI revision UX**: build, status, revisions, rollback, deploy dry-run, actual artifact rollout.
7. **Startup optimization** (only after measured refactor baseline): static bootstrap snapshot, code cache, thinner JS bootstrap if numbers justify it.
8. **Final gate**: backend tests, CLI e2e, function benchmarks, memory benchmark, binary-size comparison, pool leak tests, revision tests. Unit tests alone are not completion.
