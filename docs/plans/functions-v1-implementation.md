# Kalam Functions V1 Implementation Plan

> **Source design:** See [functions.md](functions.md) for the complete product vision, rationale, examples, deferred runtimes, actor analysis, and scheduler design.

> **For Codex:** implement this plan task-by-task using test-driven development and verification-before-completion.

**Goal:** Deliver SQL-defined, typed server procedures with generated SDKs, transactional execution, topic triggers, and production CLI deployment.

**Architecture:** SQL files compile locally into one canonical `ContractSnapshot`. That snapshot drives schema diffing, generated types, module build validation, server catalog state, and compatibility checks. Runtime artifacts are uploaded before one Meta-Raft activation atomically installs the contract metadata and active module revision.

**Initial runtime:** TypeScript on a sandboxed V8 adapter. The runtime interface must remain language-neutral so Wasm can be added later.

---

## 1. V1 product contract

SQL is the only source of truth for:

- namespaces, tables, policies, topics, and topic sources;
- named composite types, table row types (`ROW TYPE` / `FROM TABLE`), and enums;
- procedure signatures and execute permissions;
- topic-to-procedure triggers.

Implementation code supplies behavior only. It must not redefine procedure signatures.

Supported invocation paths:

```text
CALL              synchronous client or PGWire invocation
ctx.functions     nested procedure invocation in the same root execution
TOPIC TRIGGER     asynchronous, at-least-once invocation
```

Not in V1:

- scheduled procedures / `CREATE EVENT`;
- `SECURITY DEFINER`;
- procedure overloads, defaults, `OUT`, `INOUT`, or variadic parameters;
- recursive composite types, unions/interfaces, multidimensional arrays, nullable array elements, or composite table columns;
- distributed transactions across multiple data Raft groups;
- same-partition parallel trigger processing;
- Rust/Wasm, TypeScript Component/Wasm, C#, external HTTP, filesystem, TCP, subprocesses, or environment access.

These require separate follow-on plans.

---

## 2. Frozen SQL semantics

### 2.1 Composite types

```sql
CREATE TYPE chat.recipient_result AS (
    user_id TEXT NOT NULL,
    delivered BOOLEAN NOT NULL
);

CREATE TYPE chat.send_message_result AS (
    message chat.message NOT NULL,
    recipients chat.recipient_result[] NOT NULL,
    note TEXT NULL
);

CREATE TYPE chat.message_status AS ENUM (
    'sent',
    'delivered',
    'read'
);
```

Rules:

- Fields are nullable unless `NOT NULL` is present.
- `NONEMPTY` is allowed on `TEXT`, `BYTES`, and arrays. It requires `NOT NULL`.
  Text/bytes: trimmed length > 0. Arrays: cardinality >= 1. Enforced in the
  shared codec, not by generated TypeScript types alone.
- Arrays are one-dimensional. V1 array elements are non-null (`T[]` = GraphQL
  `[T!]`, `T[] NOT NULL` = `[T!]!`).
- Fields may reference scalars, enums, table row types, named composite types,
  arrays, or JSON/JSONB.
- Direct and indirect type cycles are rejected.
- `AS (...)`, `FROM TABLE`, and `AS ENUM` cannot share a name. A named type
  also may not collide with a table name.
- Named composite, enum, and row types are contract-only and cannot be
  physical table columns in V1.
- `AS UNION` / `AS INTERFACE` are reserved and rejected in V1.
- Arbitrary `CHECK`, regex, `LENGTH`, and `RANGE` are not V1.

### 2.2 Table row types

Tables are plural. Row types are explicit and singular. The table name is not a
procedure type.

```sql
CREATE USER TABLE chat.users (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL
) ROW TYPE chat.user;

CREATE TYPE chat.message FROM TABLE chat.messages;
```

`ROW TYPE` on `CREATE TABLE` and `CREATE TYPE ... FROM TABLE` are the same
binding. The type always tracks the table's canonical query schema, including
server-generated query-visible fields.

```sql
CREATE PROCEDURE chat.get_user(user_id TEXT NOT NULL)
RETURNS chat.user;

CREATE PROCEDURE chat.list_users()
RETURNS SETOF chat.user;
```

Rules:

- Same namespace as the table in V1.
- Unqualified names follow `USE` in the current file; the snapshot stores `chat.user`.
- At most one row type per table.
- No English inflection (`users` → `user`) in SQL.
- `RETURNS chat.users` is rejected; use `chat.user`.
- `CREATE OR REPLACE` cannot retarget a row type to a different table.
- `ALTER TABLE` must pass compatibility checks against procedures, topics, and triggers that use the bound row type.
- `DROP TABLE ... RESTRICT` fails while the row type exists.

### 2.3 Procedures

`CREATE PROCEDURE ... RETURNS` is an explicit KalamDB dialect extension:

```sql
CREATE PROCEDURE chat.create_message(
    group_id TEXT NOT NULL,
    body TEXT NOT NULL
)
RETURNS chat.send_message_result
SECURITY INVOKER;
```

Rules:

- One procedure per `namespace.name`; overloads are rejected.
- Parameters are `IN` only and nullable unless `NOT NULL` is present. `NONEMPTY` follows the same field rules.
- `SECURITY INVOKER` is required semantically and is the default when omitted.
- `SECURITY DEFINER` is rejected.
- Procedure DDL supports `CREATE`, `CREATE OR REPLACE`, `DROP [IF EXISTS] ... RESTRICT|CASCADE`.
- Replacement must remain compatible with the artifact being activated.

Supported returns:

```text
VOID
scalar
enum
table row
SETOF table row
SETOF named composite
SETOF enum
named composite
JSONB
```

`SETOF` is the GraphQL list of a named type. Other `SETOF` forms (scalar,
array) are rejected in V1. `T[]` as a procedure return is rejected; wrap it
in a composite or use `SETOF`.

### 2.4 Permissions

```sql
GRANT EXECUTE ON PROCEDURE chat.create_message TO user;
REVOKE EXECUTE ON PROCEDURE chat.create_message FROM PUBLIC;
```

Rules:

- Routine ACLs support `PUBLIC` and existing KalamDB roles.
- The owner, DBA, and system role may manage a routine.
- Direct `CALL` uses the caller as both actor and execution principal.
- A trigger names an existing service-role user as its execution principal.
- The execution principal drives authorization, RLS, and `CURRENT_USER`.
- The actor is immutable audit metadata and never grants permissions.

### 2.5 Typed topics and triggers

```sql
CREATE TOPIC chat.message_created
TYPE chat.message_created_event;

CREATE TRIGGER chat.process_message
ON TOPIC chat.message_created
EXECUTE PROCEDURE chat.on_message_created(PAYLOAD)
WITH (
    principal = 'function-worker',
    start = 'latest',
    retries = 5,
    retry_backoff = '1s',
    concurrency = 4
);
```

`PAYLOAD` is the only V1 SQL-level trigger binding. It is business input and must
match the target procedure parameter type. Invocation metadata is never declared
as a procedure parameter and cannot be supplied by a caller.

Every procedure implementation instead receives an immutable, host-created
context before its typed business input:

```ts
type InvocationSource =
  | {
      kind: "call";
      protocol: "http" | "pgwire" | "internal";
    }
  | {
      kind: "topic";
      topicName: string;
      eventId: string;
      partition: number;
      offset: bigint;
      attempt: number;
    };

interface ProcedureCall {
  procedure: string;
  depth: number;
}

interface ProcedureContext {
  executionId: string;
  requestId: string | null;
  principal: { id: string; role: Role };
  actor: { id: string } | null;
  source: InvocationSource;
  procedure: string;
  parent: ProcedureCall | null;
  stack: readonly ProcedureCall[];
  depth: number;
  db: ProcedureDatabase;
  functions: GeneratedProcedures;
  topics: ProcedureTopics;
  log: ProcedureLogger;
}
```

Direct `CALL` sets `source.kind = "call"` and uses the authenticated caller as
both principal and actor. A topic trigger sets `source.kind = "topic"`, uses the
configured service principal for authorization, and copies the original actor
from the event when available.

Nested `ctx.functions` calls preserve the root `source`, principal, actor,
request, transaction, cancellation, and pinned revision. The host pushes a
`ProcedureId` onto the root `SmallVec` (cap 16). `parent`, `stack`, and `depth`
are views over that vector. Names are resolved only when user code reads them or
when an error is reported. Do not clone JS stack objects on every nested invoke.

`parent` and `stack` are identity-only frames. They do not clone host APIs, so
reading nested context cannot impersonate a parent or rebind `db` / `functions`.

Context is not serialized inside SQL arguments, cannot be overridden by
`CALL`, and is excluded from the procedure's business signature. Generated
server entrypoints always have the shape:

```ts
defineProcedure(async (ctx, input) => { /* implementation */ });
```

V1 does not use `@Function()` decorators or user-declared TypeScript
parameter lists. SQL remains the contract. Implementations destructure the
generated `input` object (`{ groupId, body }`) when they want named arguments.
`string!` is not TypeScript; nullability comes from the generated contract.

Nested calls from JavaScript use the generated `ctx.functions` client, not
`CALL` SQL and not HTTP:

```ts
export default defineProcedure<ChatCreateMessage>(async (ctx, input) => {
  const message = await ctx.db.messages.insert({
    groupId: input.groupId,
    senderId: ctx.principal.id,
    body: input.body,
  });

  const fanout = await ctx.functions.chat.fanoutMessage({ message });
  await ctx.functions.chat.notifyMembers({
    groupId: message.groupId,
    messageId: message.id,
  });

  return {
    message,
    deliveries: fanout.deliveries,
    fanoutCount: fanout.fanoutCount,
  };
});
```

`fanout_message` and `notify_members` are ordinary SQL procedures. Inside them,
`ctx.source` is still the original `CALL chat.create_message`, `ctx.parent` is
`chat.create_message`, and a thrown error reports that host stack. All three
share one transaction.

Rules:

- Topic payloads are validated against their declared contract at publish time.
- Every message carries the topic contract hash used to encode it.
- `concurrency` means parallel partitions; each partition remains ordered.
- Stable event identity is `(TopicId, partition, offset)`.
- Delivery is at least once. Handlers must be idempotent; the server SDK provides a transactional inbox/dedup helper.
- Retry attempt, next attempt, lease owner, lease expiry, and last error are durable.
- DLQ publication and source-offset advancement are one durable operation.

---

## 3. Canonical contract and file discovery

`[schema].path` may reference one SQL file or a directory.

Directory rules:

- Recursively discover `.sql` files inside the configured directory.
- Reject symlinks and paths outside the project.
- Sort normalized relative paths for deterministic diagnostics only.
- Apply `USE` / `USE NAMESPACE` / `SET NAMESPACE` sequentially within a file.
  Each file starts from the project default namespace; `USE` does not leak
  across files.
- Unqualified types, tables, procedures, topics, triggers, `RETURNS`,
  `FROM TABLE`, `ROW TYPE`, parameters, and composite field types resolve
  against that default. Qualified names always win.
- Canonicalize to `namespace.name` before hashing and generation.
  `USE chat; CREATE PROCEDURE list_users() RETURNS SETOF user` is
  `chat.list_users` returning `chat.user`.
- Resolve symbols in a second pass, so filename order does not control
  dependencies.
- Reject duplicate objects, unresolved references, identifier/codegen
  collisions, and dependency cycles.

The compiler produces a versioned `ContractSnapshot` containing:

```text
namespaces, tables, policies, topics, topic sources,
types, procedures, grants, triggers, dependency graph,
canonical naming map, compiler version, contract hash
```

Hashing ignores whitespace, comments, and file order. It includes normalized identifiers, type aliases, nullability, field/parameter order, policies, permissions, and trigger options.

One deep compiler module must be reused by CLI generation, semantic diffing, deployment, and server validation. Do not add another ad-hoc schema parser.

---

## 4. Type and wire model

Use a recursive contract type separate from physical `KalamDataType`:

```text
ContractTypeRef =
  Scalar(KalamDataType, constraints)
  Enum(TypeId)
  TableRow(TableId, schema_hash)
  Named(TypeId)
  Array { element, array_null }
  Json
  Void
```

`constraints` in V1 are `NOT NULL` and `NONEMPTY` only. All languages share
this IR. TypeScript, Dart, and Rust generators project it; they do not extend
it.

All interfaces use the same signature-directed codec:

```text
HTTP params ↔ forwarded params ↔ ProcedureHandler ↔ V8 host ABI
           ↔ PGWire ↔ generated TypeScript/Dart/Rust SDKs
```

Transport mappings:

| SQL | TypeScript | JSON transport |
|---|---|---|
| BOOLEAN | `boolean` | boolean |
| INT/SMALLINT | `number` | number |
| BIGINT | `bigint` | decimal string |
| DECIMAL | `string` | decimal string |
| TEXT/UUID | `string` | string |
| DATE/TIME/TIMESTAMP/DATETIME | existing SDK canonical types | existing canonical wire encoding |
| BYTES | `Uint8Array` | existing bytes encoding |
| JSON/JSONB | `JsonValue` | JSON |
| nullable `T` | `T \| null` | null |
| `T[]` | `T[]` | array |
| ENUM | string-literal union | enum string |
| `NONEMPTY` | same TS type; rejected if empty at codec | omitted / error |

Generated identifiers:

- Access paths are always nested: `kalam.chat.listUsers()`, `ctx.functions.chat.listUsers()`, `ctx.db.chat.users`.
- Type/const names default to a namespace prefix: `chat.user` → `ChatUser`, `chat.create_message` → `ChatCreateMessage`.
- `[schema.targets.typescript] unqualified_names = true` drops the prefix (`User`, `CreateMessage`) and fails generate on collisions. Call paths stay nested.
- Enums generate as string-literal unions in TypeScript, enums in Dart/Rust, with the same wire strings.
- Do not auto-unqualify when a project has one namespace. `USE` does not change generated names.
- Server files stay `functions/src/<namespace>/<procedure>.ts`.

Objects reject missing required fields, unknown fields, excessive nesting, excessive array length, and oversized encoded values.

Procedure results remain compatible with the existing SQL `QueryResult`:

- scalar: one `value` column and one row;
- table row: normal table columns and one row;
- `SETOF` table: normal columns and rows;
- composite: top-level fields as columns; nested values use JSON cells;
- JSONB: one `value` JSON column;
- VOID: successful result with zero rows.

Do not add a parallel `{type: procedure, value: ...}` HTTP response envelope.

---

## 5. Execution and transaction model

Reuse the existing execution context through a deep root-execution module:

```text
ExecutionRoot (Arc)
  identity, actor, request metadata, cancellation,
  pinned deployment revision, lazy transaction lease,
  root InvocationSource,
  SmallVec<[ProcedureId; 4]> function_stack

InvocationFrame
  index into the root stack, local operation state
```

Rules:

- External `CALL` and each trigger message create a root execution.
- Nested `ctx.functions` calls push/pop one `ProcedureId` on the root and reuse the same isolate, transaction, and node.
- Maximum nested depth is 16. Exceeding it fails with a stable error that includes the host stack.
- Each frame can read the inherited root source plus its own `parent`/`stack`.
- Nested calls are host-mediated, so JS `Error.stack` cannot see parent procedures. Failed roots report the host procedure stack plus a truncated JS exception from the failing frame only.
- Do not persist execution stacks, JS heaps, or per-invocation start/end rows to RocksDB or Raft.
- The first transactional mutation begins the transaction through a single-flight path.
- A procedure inside explicit SQL `BEGIN` borrows that transaction and never commits it.
- A root-owned transaction commits only after the outer procedure returns and its return value validates.
- Any transactional failure marks the root transaction aborted, even if user code catches the error.
- V1 permits one data Raft group per transaction; cross-group, stream-table, and system-table mutations fail before staging.
- `StagedOperation` must include both table mutations and topic publications.
- Cancellation stops V8, DataFusion, host calls, and nested frames. Cancellation cannot claim rollback after durable commit begins.
- An invocation pins one module revision; activation or rollback affects only new roots.
- A root and all nested frames run on one node. If SQL routing must forward, forward the whole `CALL`, never individual nested procedures.
- In-flight executions are per-node memory. Node failure fails the CALL; trigger leases/offsets let another node retry the same event identity.
- Isolate pooling is bounded per node and shared by concurrent roots of the same revision. Nested frames do not create isolates.

The `CALL` path bypasses DataFusion planning. DataFusion remains lazy and is used for queries and scalar coercion/evaluation.

---

## 6. Runtime and sandbox

The language-neutral interface is:

```text
load(revision) → validated module handle
invoke(handle, ProcedureId, InvocationFrame, RoutineValue[]) → RoutineValue
cancel(execution_id)
drain(revision)
```

The TypeScript adapter must provide:

- multi-file TypeScript bundled to one JavaScript artifact;
- async host calls for `ctx.db`, `ctx.functions`, `ctx.topics`, and `ctx.log`;
- isolate memory and deadline enforcement;
- bounded instance pooling and cold-start metrics;
- no filesystem, network, subprocess, native addon, or environment access;
- ABI version validation and panic/exception isolation;
- return contract validation before commit.

The generated manifest includes:

```text
manifest_version, abi_version, compiler_version,
module_name, runtime, runtime_target,
contract_hash, per-procedure hashes, export map,
artifact_hash, artifact_size,
capabilities, limits,
source_hash, lockfile_hash, naming map
```

---

## 7. CLI and deployment

Required configuration:

```toml
[functions]
enabled = true
module = "backend"
runtime = "typescript"
path = "functions"
artifact = "functions/.kalam/build/module.js"

[functions.build]
command = "pnpm build"

[functions.limits]
memory_mb = 64
timeout_ms = 5000
max_return_bytes = 1048576

[functions.capabilities]
http = false
filesystem = false
```

All paths are relative, remain inside the project, and are validated. V1 supports one function module per project.

Required commands:

```text
kalam generate
kalam functions build
kalam deploy --dry-run [--env NAME] [--json]
kalam deploy [--env NAME] [--json]
kalam functions status
kalam functions rollback REVISION
kalam functions logs [PROCEDURE]
```

Deployment order:

1. Discover and compile local SQL into `ContractSnapshot`.
2. Diff physical schema and require committed migrations for production-like environments.
3. Generate client/server contracts from the local snapshot. Scaffold missing
   server entrypoints once; never overwrite existing implementations.
4. Build with the configured command and inspect the generated manifest/exports.
5. Validate permissions, limits, ABI, hashes, and compatibility locally.
6. Apply pending physical schema migrations and verify the live base-schema hash.
7. Upload the immutable, content-addressed artifact to `kalamdb-filestore`.
8. Submit an idempotent activation request with expected current revision/hash.
9. In one Meta-Raft operation, reconcile contract catalog rows, insert revision metadata, and CAS the active pointer.
10. Load the revision on required nodes and verify artifact hash, contract hash, and runtime initialization.
11. Report success only after the configured health quorum is ready.

Physical table migration is not rolled back by function rollback. Artifact rollback only switches to a retained compatible revision. Failed uploads leave no live contract change; staged orphan artifacts are garbage-collected.

Repeated deployment of the same contract and artifact hashes is a no-op. Concurrent deployment with a stale expected revision fails with a conflict.

---

## 8. Catalog and storage ownership

Metadata tables:

```text
system.types, system.type_fields
system.routines, system.routine_parameters, system.routine_grants
system.triggers, system.trigger_attempts
system.function_modules, system.function_revisions, system.function_artifacts
```

Use type-safe IDs and models, one model per file. Store normalized type references, not free-form type strings.

Ownership:

- `kalamdb-dialect`: SQL syntax, ASTs, classification, canonical contract compiler.
- `kalamdb-commons`: shared typed IDs and contract value/type models.
- `kalamdb-system`: catalog models and providers.
- `kalamdb-filestore`: artifact bytes, hashes, lifecycle, retention, and GC.
- `kalamdb-core`: SQL/procedure orchestration and transaction integration.
- `kalamdb-publisher`: trigger leases, retry state, ACK/DLQ coordination.
- new `kalamdb-functions` crate: runtime interface, V8 adapter, revision cache, and sandbox host ABI.
- CLI: project discovery, generation/build orchestration, upload, activation, and UX.

Add an ADR for the new crate and activation protocol before implementing them.

Observability:

- `system.active_function_runs` is an in-memory virtual view.
- Aggregate invocation/error/cancellation/duration metrics are always recorded.
- Errors, slow calls, explicit `ctx.log`, and sampled traces use the existing observability pipeline.
- There is no automatic persistent start/end row per invocation.

---

## 9. Dependency-ordered implementation tasks

### Task 1: Freeze dialect and contract models

**Files:** `backend/crates/kalamdb-dialect/src/`, `backend/crates/kalamdb-commons/src/models/`, `docs/architecture/decisions/`

**Acceptance:** Typed ASTs and IDs exist for composites, `FROM TABLE` / `ROW TYPE`, enums, `NOT NULL`, `NONEMPTY`, and reserved `UNION`/`INTERFACE` errors; unsupported V1 syntax fails with stable errors; the ADR records crate and activation ownership.

**Verification:** `cargo nextest run -p kalamdb-dialect -p kalamdb-commons`

### Task 2: Build the canonical contract compiler

**Files:** `backend/crates/kalamdb-dialect/src/contracts/`, `cli/crates/kalam-schema-diff/src/`

**Acceptance:** Multi-file SQL resolves independent of file order; `USE chat` plus unqualified `TYPE` / `PROCEDURE` / `RETURNS` canonicalizes to `chat.*` and hashes equal to the fully qualified spelling; cycles/collisions/missing refs fail; canonical hashes ignore formatting and comments; diff includes creates, changes, and removals.

**Verification:** Golden compiler/diff tests plus `cargo nextest run -p kalamdb-dialect -p kalam-schema-diff`

### Task 3: Add catalog models and providers

**Files:** `backend/crates/kalamdb-commons/src/system_tables.rs`, `backend/crates/kalamdb-system/src/providers/`, `backend/crates/kalamdb-system/src/registry.rs`

**Acceptance:** Every contract object persists through typed `EntityStore` models and is introspectable; dependency-restricted drop behavior is tested.

**Verification:** `cargo nextest run -p kalamdb-system`

### Checkpoint A

- Compiler, diff, catalog, and lifecycle tests pass.
- Review the public SQL semantics before runtime work.

### Task 4: Generate local client and server contracts

**Files:** `cli/src/workflow/schema/`, `cli/src/workflow/project/config.rs`, `cli/assets/`, TypeScript/Dart/Rust generator tests

**Acceptance:** Generation requires no running server; all targets use the same snapshot/hash; nullability, nested types, table rows, naming collisions, and codecs have golden tests; default type names are namespace-prefixed; `unqualified_names = true` emits short type names and fails on collisions without flattening `kalam.chat.*`; a new `CREATE PROCEDURE` scaffolds `functions/src/<ns>/<name>.ts` once, regenerates the frontend client method, and fails build if the export is missing; existing implementation files are not overwritten.

**Verification:** CLI unit tests and target compile/type tests pass.

### Task 5: Implement runtime interface and V8 spike

**Files:** new `backend/crates/kalamdb-functions/`, root workspace manifests, ADR update

**Acceptance:** A bundled fixture loads and invokes through the versioned ABI; timeout, memory, cancellation, forbidden capabilities, exceptions, and invalid returns are tested; runtime dependency choice is benchmarked and recorded.

**Verification:** `cargo nextest run -p kalamdb-functions`; report cold-start and warm invocation seconds.

### Task 6: Implement staged artifact storage and activation

**Files:** `backend/crates/kalamdb-filestore/src/`, `backend/crates/kalamdb-system/src/providers/`, Meta-Raft command/application code, `kalamdb-functions`

**Acceptance:** Upload is content-addressed and verified; activation is idempotent and CAS-protected; contract rows and active pointer change atomically; interruption leaves the old revision active; old revisions remain loadable.

**Verification:** Unit and cluster integration tests for upload failure, stale CAS, same-hash no-op, activation failure, restart, rollback, and GC.

### Checkpoint B

- A revision can be built, uploaded, activated, loaded, and rolled back without `CALL`.
- Failure injection confirms the previous revision remains active.

### Task 7: Implement direct typed `CALL`

**Files:** dialect classifier/parser, `kalamdb-core` SQL handlers, HTTP params/response adapters, PGWire adapter, forwarded SQL protobuf/model

**Acceptance:** Scalar, table, setof, composite, JSONB, and VOID calls work through HTTP and PGWire; composite/array params forward losslessly; every implementation receives a trusted direct-call context with `source.kind = "call"`, `parent = null`, and a one-frame `stack`; client attempts to pass or override context are rejected; execute permissions and stable error codes are enforced; existing query response compatibility remains.

**Verification:** Backend integration tests plus TypeScript client call tests.

### Task 8: Add automatic and nested transactions

**Files:** `backend/crates/kalamdb-core/src/sql/context/`, transaction coordinator/write-set, `kalamdb-functions` host calls

**Acceptance:** Root-owned and borrowed transactions behave correctly; nested calls inherit root source/principal/actor/request/cancellation while receiving a derived procedure frame with readable `parent` and `stack`; nested calls reuse the root, isolate, node, and pinned revision; a nested exception returns the full host procedure stack in SQL/HTTP/PGWire details and in the thrown parent error; concurrent first mutations cannot create two transactions; caught transactional failures still prevent commit; cross-group/stream/system writes fail clearly.

**Verification:** Transaction integration tests covering success, nested failure, return mismatch, cancellation, explicit SQL transaction, commit boundary, nested-context fields (`source`, `parent`, `stack`, `depth`) on both root and child frames, and a three-deep failure whose error details list every procedure in the chain.

### Task 9: Add transactional topic publish

**Files:** transaction staged operations, Raft data commands, `kalamdb-publisher`, runtime topic host calls

**Acceptance:** Table writes and explicit topic publication commit together; rollback emits neither; publish failure cannot be silently ignored.

**Verification:** Raft/apply integration tests with failure injection before and during commit.

### Checkpoint C

- Generated client → `CALL` → nested DB/topic work → typed result succeeds end-to-end.
- Rollback, cancellation, permissions, and one-group limits are proven.

### Task 10: Implement durable trigger delivery

**Files:** dialect trigger AST/handler, publisher trigger dispatcher, system trigger/attempt providers, `kalamdb-functions`

**Acceptance:** Per-partition order is preserved; trigger implementations receive trusted topic context plus typed payload input (`source.kind = "topic"`, `parent = null`); nested calls from a trigger keep that topic source while exposing the trigger procedure as `parent`; retries preserve the same source identity and survive restart/failover; leases expire safely; event IDs are stable; ACK follows commit; DLQ transition is atomic; disabling/dropping/redeploying a trigger preserves defined offset behavior.

**Verification:** Multi-partition, poison message, crash-before-ACK, crash-after-commit, lease expiry, DLQ replay, and schema-version backlog tests.

### Task 11: Complete CLI development and deployment flows

**Files:** `cli/src/args/workflow.rs`, `cli/src/workflow/deploy/`, `cli/src/workflow/dev/`, project templates, CLI e2e tests

**Acceptance:** Required commands/config work; `dev` watches SQL, function sources, package manifest, and lockfile; adding a procedure prints the scaffold path and new client method; unimplemented stubs warn and throw `PROCEDURE_NOT_IMPLEMENTED`; dry-run is mutation-free; JSON output and exit codes are stable; production deploy requires exact committed migration coverage.

**Verification:** `cargo build -p kalam-cli --bin kalam` then `cargo nextest run -p kalam-cli-e2e` against a running server.

### Task 12: Security, observability, and final documentation

**Files:** auth/permission handlers, observability metrics/views, canonical SDK/CLI/SQL docs, `../kalamdb-skills` mirrors

**Acceptance:** Sandbox escape and privilege tests pass; secrets never enter manifests/logs; active-run view and aggregate metrics work; failed invocations emit one structured error with host procedure stack and truncated JS exception, not per-frame start/end logs; public docs describe limits, retries, idempotency, rollback, stack traces, and wire mappings.

**Verification:** Security review, production-like server build, CLI smoke tests, and `python3 scripts/versions.py verify` when SDK versions change.

---

## 10. Final release gate

- All task acceptance criteria and checkpoints pass.
- Backend and CLI smoke tests pass against a running server.
- Production-like server build succeeds.
- Direct calls, nested calls, nested error stack traces, rollback, trigger retry/DLQ, restart, and cluster failover pass end-to-end.
- Runtime limits and security checklist are verified.
- Relevant runtime and trigger performance tests report seconds and show no unacceptable regression.
- SDK and canonical skill documentation are updated.

Implementation must stop at each checkpoint for review. Do not begin schedules or additional runtimes as part of V1.
