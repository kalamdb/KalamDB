# KalamDB Functions, Typed Actions, Triggers & Durable Schedules

**Status:** Proposed  
**Version:** 0.2  
**Primary design principle:** SQL is the source of truth for the backend contract, and KalamDB remains the source of truth for durable application state.

---

# 1. Vision

Extend KalamDB from:

```text
SQL Database
+ Realtime
+ Durable Topics
```

into:

```text
SQL Database
+ Realtime
+ Durable Topics
+ Typed server-side business logic
+ Durable background execution
+ Durable schedules / cron
+ Generated typed SDKs
```

without turning KalamDB into a general-purpose actor platform or introducing a second REST endpoint model.

The core model is:

```text
Database owns durable state.
Functions own behavior.
Topics own durable asynchronous events.
Schedules decide when behavior should run.
Generated SDKs make procedures feel like typed actions.
```

Applications should use one backend programming model:

```text
SELECT       -> query
INSERT       -> direct write
UPDATE       -> direct write
DELETE       -> direct write
SUBSCRIBE    -> realtime state
PUBLISH      -> durable asynchronous event
CALL         -> synchronous business operation
EVENT        -> scheduled business operation
```

A procedure may be invoked from:

```text
Client CALL
Topic trigger
Scheduled event
Dynamic schedule
Background enqueue
Nested procedure call
```

All of them enter the same procedure runtime and the same `ExecutionContext` model.

---

# 2. What KalamDB should learn from actor runtimes

Actor systems provide several useful application-runtime ideas:

```text
Typed actions
Per-entity ordering
Durable queues
Durable timers / cron
Lifecycle and cancellation
Simple client invocation
```

KalamDB should adopt the useful semantics without adopting actor-owned state.

## Keep

### Typed action ergonomics

Generated SDKs should make a SQL procedure feel like:

```ts
await kalam.chat.createMessage({
  conversationId,
  body,
});
```

instead of requiring callers to manually build `CALL` statements.

### Durable keyed work

Background work should support an ordering key such as:

```text
conversation:123
user:42
order:9001
```

so related work can execute in order while unrelated keys execute concurrently.

### Durable schedules

One-shot timers, fixed intervals, and cron schedules must survive:

```text
server restart
node failure
cluster failover
deployment
function revision change
```

### Stable invocation identity

Retries of the same topic message or schedule occurrence should keep the same logical invocation identity so database work and external side effects can be made idempotent.

## Do not copy

### No actor-local durable state

Do not create:

```text
one process + one private database per user / room / agent
```

Application state remains normal KalamDB tables.

### No actor-per-entity runtime requirement

A chat room, agent, user, order, or document is identified by data keys, not by a permanently owned process.

### No user-visible sleep / wake lifecycle

KalamDB may internally load, unload, cache, or migrate runtimes, but application correctness must never depend on `onWake`, `onSleep`, or process-local memory.

### No critical `setTimeout` / `setInterval`

Functions must not rely on process-local timers for durable work. Use Kalam schedules.

### No ephemeral background work for required side effects

If work must happen after a request returns, it must be durably enqueued, published, or scheduled before the transaction commits.

The desired difference is:

```text
Actor model:
actor owns state + behavior

KalamDB model:
database owns state
worker temporarily owns execution
```

---

# 3. Core terminology

Use **Kalam Functions** as the product feature name.

Internally distinguish these concepts.

## Procedure

A typed business operation invoked with `CALL`.

```sql
CALL chat.create_message($1, $2);
```

A procedure may:

- read tables
- insert/update/delete
- publish topics
- call other procedures
- enqueue background procedures
- create or cancel schedules
- return typed values
- run with an authenticated execution context

Procedures are the KalamDB equivalent of a typed application action.

## Function

Reserved for expression-style computation used inside SQL expressions.

```sql
SELECT COSINE_DISTANCE(...);
SELECT SNOWFLAKE_ID();
```

Existing DataFusion UDF behavior remains separate from server procedures.

## Trigger

Connects a durable event source to a procedure.

V1:

```text
TOPIC -> PROCEDURE
```

Possible later sources:

```text
TABLE CHANGE -> PROCEDURE
```

Table changes can already route through topics, so a separate table-trigger runtime is not required initially.

## Event

A named scheduled invocation definition.

Use MySQL-familiar syntax:

```sql
CREATE EVENT jobs.daily_report
ON SCHEDULE CRON '0 9 * * *'
TIME ZONE 'UTC'
DO CALL jobs.generate_report();
```

An event is a schema object. It is not an ephemeral websocket event.

## Dynamic schedule

A durable one-shot schedule created at runtime from procedure code.

Example:

```ts
await ctx.functions.billing.expireTrial.scheduleAt({
  at: input.expiresAt,
  input: { trialId: input.trialId },
  key: `trial:${input.trialId}`,
});
```

## Background invocation

A procedure invocation durably queued to execute as a new root execution after the current transaction commits.

```ts
await ctx.functions.notifications.send.enqueue({
  input: { userId, messageId },
  key: `user:${userId}`,
});
```

---

# 4. SQL remains the source of truth

A project may contain:

```text
my-app/
├── kalam.toml
├── schema.sql
├── functions/
└── src/
    └── generated/
```

Or later:

```text
schema/
├── tables.sql
├── types.sql
├── topics.sql
├── procedures.sql
├── triggers.sql
└── events.sql
```

SQL defines:

```text
Tables
Types
Topics
Procedures
Triggers
Scheduled events
Policies
Permissions
```

Function implementation code does **not** redefine procedure signatures.

Dynamic schedule instances are runtime data, not schema definitions. Their target procedure contract still comes from SQL.

---

# 5. Table row types

Every table automatically exposes a logical row type.

```sql
CREATE USER TABLE chat.messages (
    id BIGINT PRIMARY KEY DEFAULT SNOWFLAKE_ID(),
    conversation_id TEXT NOT NULL,
    sender_id TEXT NOT NULL,
    body TEXT NOT NULL,
    created_at TIMESTAMP NOT NULL DEFAULT NOW()
);
```

The logical type is:

```text
chat.messages
```

A procedure can return it directly:

```sql
CREATE PROCEDURE chat.create_message(
    conversation_id TEXT,
    body TEXT
)
RETURNS chat.messages;
```

No duplicate response model is required.

Multiple rows use PostgreSQL-like `SETOF`:

```sql
CREATE PROCEDURE chat.get_recent_messages(
    conversation_id TEXT,
    limit_count INT
)
RETURNS SETOF chat.messages;
```

---

# 6. Reusable contract types

Use named SQL composite types for request, response, and event contracts.

```sql
CREATE TYPE chat.recipient_result AS (
    user_id TEXT,
    delivered BOOLEAN
);

CREATE TYPE chat.send_message_result AS (
    message chat.messages,
    recipients chat.recipient_result[],
    fanout_count INT
);
```

Then:

```sql
CREATE PROCEDURE chat.create_message(
    conversation_id TEXT,
    body TEXT
)
RETURNS chat.send_message_result;
```

Named types may contain:

- scalar types
- table row types
- other named composite types
- arrays
- JSON/JSONB
- nullable fields

For V1, composite types are contract types. KalamDB does not need arbitrary nested composite physical table columns yet.

---

# 7. Procedure parameters and results

Procedure parameters may use scalars or named types.

```sql
CREATE TYPE chat.create_message_request AS (
    conversation_id TEXT,
    body TEXT,
    reply_to BIGINT
);

CREATE PROCEDURE chat.create_message(
    request chat.create_message_request
)
RETURNS chat.send_message_result;
```

Supported V1 result categories:

```text
RETURNS VOID
RETURNS scalar
RETURNS table_row_type
RETURNS SETOF table_row_type
RETURNS named_composite_type
RETURNS JSONB
```

Prefer named contracts over arbitrary `JSONB` whenever the shape is stable.

Transport encoding may still be JSON. Logical typing and wire encoding are separate concerns.

---

# 8. Generated typed action API

Users should normally not write `CALL` manually.

TypeScript:

```ts
const result = await kalam.chat.createMessage({
  conversationId,
  body: 'Hello',
});
```

Dart:

```dart
final result = await kalam.chat.createMessage(
  conversationId: conversationId,
  body: 'Hello',
);
```

Rust:

```rust
let result = client
    .chat()
    .create_message(conversation_id, "Hello")
    .await?;
```

The generated SDK compiles these calls to:

```sql
CALL chat.create_message($1, $2)
```

through the existing SQL transport.

The desired developer mental model is:

```text
SQL procedure = typed action
```

not:

```text
SQL procedure = manually assembled SQL string
```

---

# 9. Generated server procedure handles

Inside a server function, generated procedure handles should support four execution modes.

## Nested synchronous call

```ts
const result = await ctx.functions.chat.fanout({
  messageId,
});
```

This reuses the same root `ExecutionContext` and transaction.

## Background enqueue

```ts
const invocation = await ctx.functions.search.reindexDocument.enqueue({
  input: { documentId },
  key: `document:${documentId}`,
});
```

This stages durable background work that starts only after the current transaction commits.

## Schedule at an exact time

```ts
const schedule = await ctx.functions.billing.expireTrial.scheduleAt({
  at: expiresAt,
  input: { trialId },
  key: `trial:${trialId}`,
});
```

## Schedule after a delay

```ts
const schedule = await ctx.functions.chat.expireTyping.scheduleAfter({
  delay: '15s',
  input: { conversationId, userId },
  key: `typing:${conversationId}:${userId}`,
});
```

The generated handle knows the procedure input contract. Incorrect inputs fail at compile time where the language supports it and at runtime on every platform.

---

# 10. Synchronous vs background execution

A synchronous `CALL` executes immediately and returns the procedure result.

A background invocation:

```text
current transaction
      |
      +-- stage invocation
      |
    COMMIT
      |
      v
internal durable invocation topic
      |
      v
new root ExecutionContext
      |
      v
procedure
```

If the current transaction rolls back, the background invocation must not become visible.

This gives the same important guarantee as transactional topic publication:

```text
database write
+
background work request
```

commit together.

Background invocation is intentionally different from spawning an arbitrary task or calling `setTimeout`.

## V1 client behavior

Client-facing SDKs should keep synchronous `CALL` as the primary API.

Applications that need durable asynchronous behavior can use:

```text
typed topic + trigger
```

or, once background invocation is exposed publicly, a generated `.enqueue()` method.

Do not require public async invocation in the first implementation phase.

---

# 11. Keyed background ordering

Actor queues are useful because they serialize work for one entity. KalamDB should provide the same outcome using topics and keys rather than an actor mailbox.

Example:

```ts
await ctx.functions.chat.processMessage.enqueue({
  input: { conversationId, messageId },
  key: `conversation:${conversationId}`,
});
```

Semantics:

```text
same ordering key
    -> same internal partition
    -> FIFO processing for that partition/key

different keys
    -> may execute concurrently
```

The implementation SHOULD reuse KalamDB's existing topic partition and consumer-group machinery.

Do not introduce a second standalone queue subsystem.

## Important limitation

V1 does **not** promise global keyed serialization for ordinary synchronous client `CALL`s.

If strict per-key ordering is required, use:

```text
background enqueue
or
typed topic + trigger
```

This avoids a distributed lock service in the synchronous request path.

---

# 12. User and execution context

Every root invocation receives a unified `ExecutionContext`.

Conceptually:

```text
user_id
role
namespace
request_id
execution_id
execution_source
attempt
idempotency_key
transaction
cancellation
```

Server SDK:

```ts
ctx.user.id
ctx.user.role
ctx.requestId
ctx.execution.id
ctx.execution.source
ctx.execution.attempt
ctx.execution.idempotencyKey
ctx.abortSignal
```

Background sources additionally expose source metadata.

Topic trigger:

```text
ctx.topic.name
ctx.topic.partition
ctx.topic.offset
ctx.topic.eventId
ctx.actorUser
ctx.executionPrincipal
```

Scheduled invocation:

```text
ctx.schedule.scheduleId
ctx.schedule.eventName
ctx.schedule.scheduledAt
ctx.schedule.occurrenceId
```

`actorUser` means the user whose action originally caused the work when known.

`executionPrincipal` means the identity whose permissions are used to execute background work.

Never silently grant administrator privileges to background functions.

---

# 13. Procedure security

Direct procedures default to:

```text
SECURITY INVOKER
```

```sql
CREATE PROCEDURE chat.create_message(...)
RETURNS chat.messages
SECURITY INVOKER;
```

Use SQL permissions:

```sql
GRANT EXECUTE ON PROCEDURE chat.create_message TO app_user;
```

Background invocations persist both:

```text
actor user, when available
execution principal
```

so an audit record can distinguish who caused the action from which service identity executed it.

Static scheduled events execute using the event owner / configured execution principal because there is no live client session at fire time.

---

# 14. Transaction semantics

Every root procedure invocation is transactional by default.

```text
CALL chat.create_message()
        |
        v
ExecutionContext
        |
        v
lazy transaction
        |
        +-- INSERT
        +-- UPDATE
        +-- nested CALL
        +-- PUBLISH
        +-- enqueue background function
        +-- create dynamic schedule
        |
      success?
       /   \
     yes    no
      |      |
   COMMIT  ROLLBACK
```

Topic publishes, background enqueues, and dynamic schedule creation performed through the host API should participate in the root transaction.

This prevents database/event/job dual-write problems.

---

# 15. Lazy transaction creation

Do not open a transaction just because a function started.

Initial state:

```text
TransactionScope::None
```

The first transactional mutation starts the transaction lazily.

```text
INSERT
UPDATE
DELETE
transactional topic publish
background enqueue
dynamic schedule mutation
```

A read-only procedure may complete without creating a write transaction.

---

# 16. Nested procedure calls

Nested calls reuse the same root execution.

```text
function1
  -> function2
     -> function3
```

means:

```text
one ExecutionContext
one root transaction
one cancellation token
one active execution record
one function stack
```

not:

```text
three actors
three transactions
three active execution registrations
```

Track nested procedures with compact IDs such as:

```rust
SmallVec<[ProcedureId; 4]>
```

V1 failure semantics follow PostgreSQL's strict transaction model: an unrecoverable transactional failure aborts the root transaction even if application code catches the error.

Savepoints may be added later.

---

# 17. Explicit SQL transactions

A procedure called inside an explicit SQL transaction borrows it.

```sql
BEGIN;
CALL shop.create_order($1);
INSERT INTO audit.entries ...;
CALL shop.reserve_inventory($1);
COMMIT;
```

The procedures do not commit independently.

Logical transaction states:

```rust
pub enum TransactionScope {
    None,
    Owned(TransactionId),
    Borrowed(TransactionId),
    Aborted(TransactionId),
    Committing(TransactionId),
}
```

---

# 18. Topics and typed events

KalamDB already supports durable topics, partitions, consumer groups, replay, offsets, and ACK semantics.

Extend topics with optional payload contracts.

```sql
CREATE TYPE chat.message_created_event AS (
    message_id BIGINT,
    conversation_id TEXT,
    sender_id TEXT
);

CREATE TOPIC chat.message_created
TYPE chat.message_created_event;
```

Untyped topics remain supported and behave like JSON payloads.

Database-to-topic routes remain unchanged.

```sql
ALTER TOPIC chat.message_created
ADD SOURCE chat.messages
ON INSERT
WITH (payload = 'full');
```

KalamDB should prefer live SQL queries for authoritative UI state and topics for durable asynchronous processing.

Do not add a second actor-style `broadcast` abstraction merely to duplicate realtime tables and topics.

---

# 19. Topic-triggered procedures

```sql
CREATE TRIGGER chat.message_created_handler
ON TOPIC chat.message_created
EXECUTE PROCEDURE chat.after_message_created(PAYLOAD)
WITH (
    start = 'latest',
    retries = 5,
    concurrency = 8,
    retry_backoff = '1s'
);
```

Reserved trigger arguments for V1:

```text
PAYLOAD
EVENT_ID
ACTOR_USER_ID
TOPIC_NAME
PARTITION
OFFSET
```

Each trigger owns a durable consumer group automatically.

```text
trigger id: TRG_123
consumer group: __kalam_function_trigger_TRG_123
```

Users do not need to manage that group manually.

## Ordering

The trigger runtime should preserve source partition order by default.

Different partitions may execute concurrently.

Applications that require per-entity ordering should publish with a stable topic key so all events for that entity map to the same partition.

---

# 20. Topic delivery and retry semantics

Topic-triggered execution is at-least-once.

```text
read event
   |
   v
invoke procedure
   |
   v
transaction
   |
   v
commit
   |
   v
ACK offset
```

On failure:

```text
rollback
no ACK
retry
```

After maximum retries, the trigger may route the event to an internal DLQ topic.

The source identity:

```text
topic + consumer group + partition + offset
```

must remain stable across retries and should become the invocation idempotency identity.

---

# 21. Durable scheduled functions

Scheduled work is a first-class feature.

KalamDB should support three schedule forms:

```text
AT       -> one time at an absolute timestamp
EVERY    -> fixed interval
CRON     -> calendar recurrence
```

Use MySQL-familiar `CREATE EVENT ... ON SCHEDULE ... DO CALL ...` syntax because PostgreSQL does not have a built-in scheduler, while `pg_cron` establishes familiar cron-expression behavior.

## One-time event

```sql
CREATE EVENT billing.expire_campaign
ON SCHEDULE AT TIMESTAMP '2026-10-01 00:00:00Z'
DO CALL billing.expire_campaign('fall-2026');
```

## Fixed interval

```sql
CREATE EVENT presence.cleanup
ON SCHEDULE EVERY 15 SECOND
DO CALL presence.remove_stale_sessions();
```

Intervals are anchored to scheduled deadlines and should not drift based on procedure runtime.

## Cron

```sql
CREATE EVENT reports.daily_summary
ON SCHEDULE CRON '0 9 * * *'
TIME ZONE 'Asia/Jerusalem'
DO CALL reports.generate_daily_summary();
```

V1 cron expressions use standard five-field minute-based syntax.

Time zones use IANA names. Default timezone is `UTC`.

---

# 22. Scheduled event options

A scheduled event may configure execution policy.

```sql
CREATE EVENT reports.daily_summary
ON SCHEDULE CRON '0 9 * * *'
TIME ZONE 'Asia/Jerusalem'
WITH (
    retries = 5,
    retry_backoff = '5s',
    overlap = 'skip',
    misfire = 'run_once'
)
DO CALL reports.generate_daily_summary();
```

Recommended policies:

## `overlap`

```text
skip   -- default; if previous occurrence is still running, do not start another
queue  -- preserve every occurrence and run later
allow  -- overlapping occurrences may run concurrently
```

Default:

```text
skip
```

This is safer for cron-style maintenance work.

## `misfire`

Controls what happens when KalamDB was unavailable when an occurrence should have fired.

```text
run_once  -- default; run one recovery occurrence after restart
skip      -- ignore missed occurrences
catch_up  -- enqueue missed occurrences up to a configured bound
```

`catch_up` must always have a maximum bound to prevent unbounded restart storms.

## `retries`

Scheduled executions use the same retry/backoff infrastructure as topic-triggered procedures.

---

# 23. Scheduled event lifecycle SQL

Support familiar DDL operations:

```sql
ALTER EVENT reports.daily_summary ENABLE;
ALTER EVENT reports.daily_summary DISABLE;
DROP EVENT reports.daily_summary;
```

Later `ALTER EVENT` can support replacing schedule expressions and options.

Schema deployment treats named events idempotently. Reapplying the same schema must not create duplicate schedules.

---

# 24. Schedule invocation metadata

Every occurrence receives stable metadata.

Reserved schedule values:

```text
SCHEDULED_AT
FIRED_AT
OCCURRENCE_ID
EVENT_NAME
```

Example:

```sql
CREATE PROCEDURE reports.generate_daily_summary(
    scheduled_at TIMESTAMP,
    occurrence_id TEXT
)
RETURNS VOID;

CREATE EVENT reports.daily_summary
ON SCHEDULE CRON '0 9 * * *'
TIME ZONE 'UTC'
DO CALL reports.generate_daily_summary(
    SCHEDULED_AT,
    OCCURRENCE_ID
);
```

`OCCURRENCE_ID` must be deterministic for the logical occurrence and remain unchanged across retries.

For recurring events it can be derived from:

```text
event_id + scheduled_at
```

---

# 25. Dynamic one-shot schedules

Static `CREATE EVENT` is ideal for schema-defined recurring work.

Applications also need millions of entity-specific timers such as:

```text
expire a session
send a reminder
end a trial
release a reservation
retry a delayed workflow
```

Do not create a schema object for every user timer.

Use dynamic schedule rows.

Generated server API:

```ts
const schedule = await ctx.functions.billing.expireTrial.scheduleAt({
  at: input.expiresAt,
  input: {
    trialId: input.trialId,
  },
  key: `trial:${input.trialId}`,
});
```

Relative delay:

```ts
await ctx.functions.chat.expireTyping.scheduleAfter({
  delay: '15s',
  input: { conversationId, userId },
  key: `typing:${conversationId}:${userId}`,
});
```

Cancel:

```ts
await ctx.schedules.cancel(schedule.id);
```

Optional upsert key:

```ts
await ctx.functions.billing.expireTrial.scheduleAt({
  at: input.expiresAt,
  input: { trialId },
  name: `trial-expiry:${trialId}`,
  replace: true,
});
```

A named dynamic schedule may replace an existing pending schedule instead of creating duplicates.

---

# 26. Dynamic schedules are transactional

Creating or cancelling a dynamic schedule inside a procedure participates in that procedure transaction.

```text
BEGIN

INSERT trial
SCHEDULE expire_trial

COMMIT
```

If the transaction rolls back:

```text
trial not created
schedule not created
```

This is significantly safer than application-managed process timers.

---

# 27. Schedule runtime architecture

Do not create one OS timer, actor, or Tokio task per schedule.

Use durable rows plus a small scheduling subsystem.

```text
system.schedules / system.scheduled_invocations
              |
              v
      Scheduler ownership
              |
        near-term heap/index
              |
              v
       due occurrence
              |
              v
internal durable invocation topic
              |
              v
      Function Runtime
```

The scheduler owns *when to enqueue* work. It does not own the application state or execute the procedure itself.

In a cluster, exactly one scheduler owner must claim each schedule partition using existing cluster leadership or leases.

Execution is distributed through the normal durable invocation path.

This keeps scheduler coordination lightweight and allows function execution to scale independently.

---

# 28. Schedule durability and recovery

The durable schedule store is authoritative.

In-memory timers/heaps are caches only.

On restart or failover:

```text
load due / near-future schedules
apply misfire policy
enqueue occurrences with stable occurrence IDs
continue
```

A schedule must survive function module replacement because the schedule points to the SQL procedure contract, not directly to a source file or runtime object.

Deployment must reject a revision that removes or incompatibly changes a procedure still referenced by an enabled event.

---

# 29. Idempotent execution identity

At-least-once delivery is unavoidable around crashes and retries.

KalamDB should make idempotency easy by giving every background root execution a stable identity.

Examples:

```text
topic trigger:
  topic/group/partition/offset

scheduled event:
  occurrence_id

background enqueue:
  invocation_id
```

Expose:

```ts
ctx.execution.idempotencyKey
```

The key remains stable across retries of the same logical work.

## Internal database effects

KalamDB SHOULD maintain a durable completion marker for background invocation identities.

If a committed invocation is redelivered because an ACK or dispatcher response was lost:

```text
same identity detected
  -> do not repeat committed KalamDB mutations
  -> acknowledge / mark completed
```

This can provide effectively-once database effects even though transport remains at-least-once.

## External side effects

HTTP calls, email, payment APIs, and other external systems cannot be rolled back with the KalamDB transaction.

Functions must pass the stable idempotency key to external systems whenever duplicate effects would be harmful.

---

# 30. Direct client idempotency

Generated clients should optionally support an idempotency key for mutation procedures.

```ts
await kalam.payments.createCharge(
  { orderId, amount },
  { idempotencyKey: `charge:${orderId}` },
);
```

This is opt-in rather than mandatory for every `CALL`.

A successful result may be cached durably for a configured TTL under:

```text
principal + procedure + idempotency key
```

Repeated calls return the committed result instead of running the procedure again.

Do not add this persistence cost to procedures that do not request idempotency.

---

# 31. Procedure implementation project

A function deployment is a complete application project, not one isolated source file per function.

```text
functions/
├── package.json
├── tsconfig.json
├── src/
│   ├── chat/
│   │   ├── create_message.ts
│   │   └── process_message.ts
│   ├── billing/
│   │   └── expire_trial.ts
│   ├── services/
│   └── common/
└── .kalam/
    └── generated/
        ├── contracts.ts
        └── registry.ts
```

Developers may use:

- multiple files
- helper modules
- classes
- npm dependencies compatible with the runtime
- shared utilities
- tests

Only public procedure entrypoints follow Kalam conventions.

---

# 32. No manual SQL-to-code mapping

For:

```sql
CREATE PROCEDURE chat.create_message(...)
```

Kalam can use the conventional implementation path:

```text
functions/src/chat/create_message.ts
```

`kalam generate` may scaffold a missing implementation once.

```text
New procedure detected:
  chat.create_message

Created:
  functions/src/chat/create_message.ts
```

Generated stubs are never overwritten after creation.

The generated registry maps SQL contracts to implementation exports automatically.

---

# 33. Generated TypeScript server entrypoint

```ts
import {
  defineProcedure,
  type ChatCreateMessage,
} from '../.kalam/generated/contracts';

export default defineProcedure<ChatCreateMessage>(
  async (ctx, input) => {
    // implementation
  },
);
```

The generated contract knows:

```text
input
output
context
nested procedure handles
```

Changing the SQL signature causes compile-time errors until implementation code is updated.

---

# 34. Reuse generated table types and Drizzle

Functions should reuse existing `@kalamdb/orm` table definitions.

```ts
import { messages } from '@generated/schema';

export default defineProcedure<ChatCreateMessage>(
  async (ctx, input) => {
    const [message] = await ctx.db
      .insert(messages)
      .values({
        conversationId: input.conversationId,
        senderId: ctx.user.id,
        body: input.body,
      })
      .returning();

    return message;
  },
);
```

Drizzle owns table/query ergonomics.

KalamDB owns procedure, topic, trigger, schedule, and execution contracts.

---

# 35. Do not call KalamDB over HTTP from server functions

Never implement internal DB access as:

```text
V8 / Wasm
  -> HTTP
  -> KalamDB
```

Use:

```text
V8 / Wasm
  -> Kalam Host API
  -> existing executor / transaction layer
```

The host API automatically inherits:

```text
identity
namespace
transaction
cancellation
execution ID
```

No server function manually passes transaction IDs or user IDs between internal operations.

---

# 36. Host capabilities

Initial function capabilities:

```text
ctx.db
ctx.functions
ctx.topics
ctx.schedules
ctx.user
ctx.request
ctx.execution
ctx.log
ctx.abortSignal
```

Do not automatically expose:

```text
filesystem
arbitrary TCP
process spawning
raw environment variables
native Node addons
```

External HTTP should be an explicit capability.

```toml
[functions.capabilities]
http = false
filesystem = false
```

When HTTP is enabled, KalamDB should still enforce timeout, cancellation, and optional hostname allowlists.

---

# 37. Runtime abstraction

Keep language runtime independent of function contracts.

```text
KalamModuleRuntime

load(module)
invoke(procedure, context, args)
shutdown()
```

Possible implementations:

```text
TypescriptRuntime
WasmRuntime
```

Recommended initial language strategy:

```text
TypeScript -> bundled JavaScript -> sandboxed V8 isolate
Rust       -> Wasm Component     -> Wasmtime
```

Do not expose V8 or Wasmtime implementation details in generated clients.

---

# 38. TypeScript project support

A complete TypeScript project means:

```text
multiple files
subfolders
package.json
pure JS/TS npm packages
classes
async/await
bundling
tests
```

It does not mean unrestricted Node.js.

The runtime sandbox remains capability-based.

Configuration:

```toml
[functions]
enabled = true
path = "functions"
runtime = "typescript"

[functions.build]
command = "pnpm build"

[functions.limits]
memory_mb = 64
timeout_ms = 5000

[functions.capabilities]
http = false
filesystem = false
```

---

# 39. Unified `ExecutionContext`

Do not introduce a second heavyweight `FunctionExecutionContext`.

Extend the existing KalamDB `ExecutionContext`.

Conceptually:

```rust
pub struct ExecutionContext {
    auth_session: AuthSession,
    namespace_id: Option<NamespaceId>,

    base_session_context: Arc<SessionContext>,
    session_context_cache: Arc<OnceCell<SessionContext>>,

    transaction_scope: TransactionScope,
    execution_control: Option<Arc<ExecutionControl>>,
    function_stack: SmallVec<[ProcedureId; 4]>,

    source_context: ExecutionSourceContext,
}
```

The semantic ownership is:

```text
ExecutionContext
  |- identity
  |- namespace
  |- transaction scope
  |- cancellation
  |- function nesting
  |- source metadata
  `- lazy DataFusion context
```

---

# 40. Root execution sources

A root execution is created for independently tracked business work.

```text
client CALL
PGWire CALL
topic-triggered procedure
scheduled event
dynamic schedule
background invocation
```

Ordinary lightweight SQL queries may continue using the existing ephemeral context without active execution registration.

This keeps observability out of the normal SQL fast path.

---

# 41. DataFusion remains lazy

DataFusion is an execution engine used by `ExecutionContext`; it is not the root runtime abstraction.

A procedure may complete without creating a DataFusion session.

If it needs:

```text
SELECT
JOIN
aggregation
DataFusion expression evaluation
```

reuse the current lazy per-user `SessionContext` cache.

```text
ExecutionContext
  -> function runtime
      -> host DB query
          -> lazy DataFusion SessionContext
```

---

# 42. Procedure CALL fast path

For:

```sql
CALL chat.create_message($1, $2);
```

execution should bypass DataFusion planning where possible.

```text
SQL parse/classify
      |
      v
ProcedureCallPlan
      |
      v
ProcedureHandler
      |
      v
Function Runtime
```

Only queries issued by the function need DataFusion.

Generated SDKs will repeatedly issue identical `CALL` shapes, so procedure resolution should reuse existing SQL/plan cache infrastructure.

Cached plan data can contain:

```text
procedure_id
signature_id
argument bindings
return type
```

---

# 43. Execution cancellation

Tracked root executions receive lightweight `ExecutionControl`.

```rust
pub struct ExecutionControl {
    id: ExecutionId,
    procedure_id: ProcedureId,
    started_at: Instant,
    state: AtomicU8,
    operation: AtomicU8,
    cancellation: CancellationToken,
}
```

Expose:

```sql
KILL EXECUTION 100;
```

Cancellation should propagate to:

```text
V8
Wasmtime
DataFusion
host DB calls
nested procedures
HTTP capability
```

Once a transaction enters durable commit, cancellation must not falsely report rollback.

Return a clear PostgreSQL-style cancellation/commit-in-progress error instead.

---

# 44. Active execution registry

Use a lightweight in-memory registry of currently active tracked executions.

```rust
DashMap<ExecutionId, Arc<ExecutionControl>>
```

Memory cost should scale with concurrent execution count, not invocation rate.

Use compact numeric IDs and enums rather than allocated strings on the hot path.

RAII registration is preferred so cleanup happens on success, failure, cancellation, early return, or panic boundary.

---

# 45. Active run view and history

Avoid the previous ambiguity between active executions and historical logs.

Use two concepts.

## `system.function_runs`

A virtual in-memory snapshot of currently active root executions.

Possible columns:

```text
execution_id
procedure_name
module_name
module_revision
source
state
user_id
request_id
transaction_id
current_operation
started_at
age_ms
node_id

topic
partition
offset

event_name
schedule_id
scheduled_at
occurrence_id
attempt
```

Names are resolved from catalog IDs only when the view is queried.

## `system.function_run_history`

Optional persistent/sampled execution history for:

```text
errors
slow executions
scheduled runs
topic retry diagnostics
explicit tracing
```

Do not persist start/end rows for every high-frequency successful call by default.

---

# 46. Metrics and logging

Low-cost metrics:

```text
function_invocations_total
function_errors_total
function_cancellations_total
function_duration_histogram
function_background_queue_depth
function_background_lag
schedule_due_total
schedule_errors_total
schedule_lag_ms
trigger_retry_total
```

Detailed logs are emitted for:

```text
explicit ctx.log()
errors
slow executions
sampled traces
schedule failures
DLQ transitions
```

Do not automatically write `FUNCTION START` and `FUNCTION END` logs for every invocation.

---

# 47. Actor frameworks are implementation details

KalamDB may use actors internally for long-lived infrastructure components such as:

```text
Runtime Supervisor
Topic Trigger Dispatcher
Schedule Dispatcher
Module Manager
```

High-frequency root executions should default to ordinary Tokio tasks unless benchmarks show that actor-per-execution overhead is acceptable.

Do not define SQL/function semantics in terms of a particular actor framework.

Possible internal implementations:

```text
plain Tokio
Kameo
Ractor
Actix actors
```

The contract remains:

```rust
pub trait ExecutionRuntime {
    async fn execute(
        &self,
        request: ExecutionRequest,
    ) -> Result<ExecutionResult, KalamDbError>;

    fn cancel(
        &self,
        execution_id: ExecutionId,
    ) -> Result<(), KalamDbError>;
}
```

Transactional client calls must never be automatically replayed merely because an actor/task crashed.

Topic and schedule retry policies decide background replay.

---

# 48. Procedure runtime errors

Where an equivalent PostgreSQL error exists, keep PostgreSQL-like SQLSTATE behavior.

Examples:

```text
42883  undefined routine / procedure
42501  insufficient privilege
57014  execution canceled
40001  serialization failure
```

Runtime contract mismatches should use stable KalamDB error codes in addition to SQLSTATE where appropriate.

Example:

```text
PROCEDURE_RETURN_TYPE_MISMATCH
```

Generated SDK errors should preserve:

```text
message
sqlState
code
details
executionId
```

---

# 49. Runtime validation

KalamDB validates at runtime:

```text
procedure arguments
procedure return value
topic payload
schedule input payload
background invocation payload
```

Example:

SQL:

```sql
RETURNS chat.messages
```

implementation returns:

```json
{"foo":"bar"}
```

Invocation fails before invalid data reaches the caller.

---

# 50. SQL and PGWire result behavior

The HTTP SQL endpoint returns procedure results using the normal SQL response model.

Simple scalar:

```text
one value
```

Table row:

```text
normal columns
```

`SETOF`:

```text
normal rows
```

Nested composite values may use JSON encoding in V1 rather than implementing PostgreSQL composite OIDs immediately.

Generated Kalam SDKs hide transport details.

---

# 51. Catalog model

Add or extend these system objects:

```text
system.types
system.type_fields
system.routines
system.routine_parameters
system.triggers
system.schedules
system.scheduled_invocations
system.function_modules
system.function_revisions
system.function_artifacts
system.function_idempotency
system.function_runs            -- virtual active view
system.function_run_history     -- optional persistent/sampled
```

`information_schema.routines` should expose standards-compatible routine metadata where possible.

---

# 52. Suggested routine catalog

```text
system.routines
  routine_id
  namespace
  routine_name
  routine_kind
  return_kind
  return_type_id
  module_name
  security_mode
  created_at
  updated_at

system.routine_parameters
  routine_id
  param_ordinal
  param_name
  param_mode
  param_type
  is_nullable
  default_expr
```

---

# 53. Suggested trigger catalog

```text
system.triggers
  trigger_id
  namespace
  trigger_name
  source_type          -- topic
  source_id
  procedure_id
  consumer_group
  start_position
  retries
  retry_backoff_ms
  concurrency
  status
  owner_principal
  created_at
  updated_at
```

---

# 54. Suggested schedule catalog

Static recurring events:

```text
system.schedules
  schedule_id
  namespace
  event_name
  schedule_kind        -- at | every | cron
  schedule_expr
  timezone
  procedure_id
  args_blob
  owner_principal
  enabled
  overlap_policy
  misfire_policy
  retry_limit
  retry_backoff_ms
  next_fire_at
  last_fire_at
  created_at
  updated_at
```

Dynamic one-shot invocations:

```text
system.scheduled_invocations
  schedule_id
  procedure_id
  actor_user_id
  execution_principal
  input_blob
  ordering_key
  name
  fire_at
  status               -- pending | claimed | running | completed | failed | canceled
  occurrence_id
  attempt
  created_at
  updated_at
```

A durable index on:

```text
status + fire_at
```

is required.

Do not scan all schedule rows on every scheduler tick.

---

# 55. Function module catalog

```text
system.function_modules
  module_name
  runtime
  active_revision_id
  status
  created_at
  updated_at

system.function_revisions
  revision_id
  module_name
  revision_number
  artifact_id
  artifact_hash
  source_hash
  schema_hash
  manifest_json
  created_at
  created_by
  status

system.function_artifacts
  artifact_id
  module_name
  revision_id
  kind
  content_type
  content_hash
  size_bytes
  storage_backend
  storage_key
  created_at
```

Revision and artifact rows are immutable.

Rollback changes the active revision pointer rather than rewriting history.

---

# 56. Idempotency catalog

Optional direct-call idempotency and background completion markers may use:

```text
system.function_idempotency
  identity_key
  procedure_id
  principal_id
  status
  result_blob
  created_at
  expires_at
```

The implementation may use a more compact internal key/value representation if this table becomes too expensive as a physical row store.

The semantic contract matters more than the exact physical layout.

---

# 57. Build manifest

Build output includes a generated manifest.

```json
{
  "module": "backend",
  "schemaHash": "...",
  "procedures": {
    "chat.create_message": {
      "input": "chat.create_message_request",
      "output": "chat.send_message_result"
    },
    "billing.expire_trial": {
      "input": "billing.expire_trial_request",
      "output": "void"
    }
  }
}
```

Users never edit the manifest manually.

---

# 58. Local development

Extend `kalam dev` to watch:

```text
schema.sql
functions/src/**
package.json
```

Schema change loop:

```text
parse / validate schema
regenerate contracts
build function project
validate module exports
apply local schema
activate new local revision
refresh static schedules
```

Function-only source change:

```text
rebuild
validate exports
activate local revision
```

Static scheduled events should be visible during development without requiring an external cron service.

For testing a scheduled procedure, developers can call the underlying procedure directly.

---

# 59. Code generation

`kalam generate` should generate:

```text
Drizzle tables
custom SQL types
procedure input/output contracts
client action methods
server procedure contracts
nested procedure handles
background enqueue handles
schedule handles
entrypoint stubs
```

Conceptually:

```text
schema.sql
   |
   v
Schema Compiler
   |
   v
Contract IR
   |
   +-- tables
   +-- types
   +-- topics
   +-- procedures
   +-- triggers
   `-- scheduled events
        |
        v
    generators
   /    |     \
 TS   Dart   Rust
```

---

# 60. Deployment

`kalam deploy` should eventually perform:

```text
1. Parse SQL schema
2. Generate canonical contract
3. Build function project
4. Inspect exports
5. Validate implementation against SQL
6. Validate permissions / capabilities
7. Validate triggers and scheduled-event targets
8. Apply schema migration
9. Upload module artifact
10. Insert immutable revision metadata
11. Activate revision
12. Apply / update named schedules idempotently
13. Run health validation
```

A contract mismatch blocks deployment.

Catalog activation should be atomic where possible so the active procedure contract, module revision, triggers, and schedules cannot point at incompatible versions.

---

# 61. Deployment compatibility rules

Reject deployment when:

```text
an enabled trigger references a removed procedure
an enabled event references a removed procedure
an active dynamic schedule references an incompatible procedure signature
rollback revision does not satisfy current SQL contracts
```

Changing only implementation code does not require rewriting schedule rows because schedules reference the stable procedure contract.

When a dynamic schedule already stores an old serialized input contract, schema evolution must either:

```text
remain backward compatible
migrate pending schedule payloads
or block deployment
```

Do not silently deserialize old payloads into a new incompatible contract.

---

# 62. Rollback

```bash
kalam functions rollback backend 42
```

switches the active module revision without rebuilding.

Rollback is allowed only when the old module still satisfies the current SQL procedure contracts.

Static and dynamic schedules continue targeting procedure IDs, so they automatically use the rolled-back compatible implementation.

---

# 63. Example: chat message with background work

Schema:

```sql
CREATE USER TABLE chat.messages (
    id BIGINT PRIMARY KEY DEFAULT SNOWFLAKE_ID(),
    conversation_id TEXT NOT NULL,
    sender_id TEXT NOT NULL,
    body TEXT NOT NULL,
    created_at TIMESTAMP NOT NULL DEFAULT NOW()
);

CREATE TYPE chat.create_message_request AS (
    conversation_id TEXT,
    body TEXT
);

CREATE TYPE chat.message_created_event AS (
    message_id BIGINT,
    conversation_id TEXT,
    sender_id TEXT
);

CREATE TOPIC chat.message_created
TYPE chat.message_created_event;

CREATE PROCEDURE chat.create_message(
    request chat.create_message_request
)
RETURNS chat.messages
SECURITY INVOKER;

CREATE PROCEDURE chat.summarize_conversation(
    conversation_id TEXT
)
RETURNS VOID;
```

Implementation:

```ts
export default defineProcedure<ChatCreateMessage>(
  async (ctx, input) => {
    const [message] = await ctx.db
      .insert(messages)
      .values({
        conversationId: input.request.conversationId,
        senderId: ctx.user.id,
        body: input.request.body,
      })
      .returning();

    await ctx.topics.publish('chat.message_created', {
      messageId: message.id,
      conversationId: message.conversationId,
      senderId: message.senderId,
    });

    await ctx.functions.chat.summarizeConversation.enqueue({
      input: {
        conversationId: message.conversationId,
      },
      key: `conversation:${message.conversationId}`,
    });

    return message;
  },
);
```

If the message transaction rolls back, neither the topic event nor the background summary invocation becomes visible.

---

# 64. Example: topic-triggered AI worker

```sql
CREATE PROCEDURE chat.on_message_created(
    event chat.message_created_event
)
RETURNS VOID;

CREATE TRIGGER chat.message_created_handler
ON TOPIC chat.message_created
EXECUTE PROCEDURE chat.on_message_created(PAYLOAD)
WITH (
    retries = 5,
    concurrency = 8,
    retry_backoff = '1s'
);
```

The topic key should be the conversation ID when strict conversation ordering is required.

Then:

```text
conversation A events -> partition X -> ordered
conversation B events -> partition Y -> may run concurrently
```

No actor per conversation is required.

---

# 65. Example: daily cron

```sql
CREATE PROCEDURE analytics.rollup_daily_usage(
    scheduled_at TIMESTAMP,
    occurrence_id TEXT
)
RETURNS VOID;

CREATE EVENT analytics.daily_usage_rollup
ON SCHEDULE CRON '5 0 * * *'
TIME ZONE 'UTC'
WITH (
    retries = 3,
    retry_backoff = '10s',
    overlap = 'skip',
    misfire = 'run_once'
)
DO CALL analytics.rollup_daily_usage(
    SCHEDULED_AT,
    OCCURRENCE_ID
);
```

No Kubernetes CronJob, external scheduler, or application `setInterval` is required.

---

# 66. Example: per-user reminder without actors

Millions of user reminders should be durable rows, not millions of sleeping processes.

```ts
await ctx.functions.reminders.sendReminder.scheduleAt({
  at: input.remindAt,
  input: {
    userId: ctx.user.id,
    reminderId: input.reminderId,
  },
  key: `user:${ctx.user.id}`,
});
```

Runtime:

```text
scheduled row
   |
   | time arrives
   v
internal invocation topic
   |
   v
worker
   |
   v
CALL reminders.send_reminder(...)
   |
   v
KalamDB tables / topics / live queries
```

The compute process may disappear immediately after execution because durable state remains in KalamDB.

---

# 67. Example: trial expiry replacement

A user changes their plan, so the existing expiry should move.

```ts
await ctx.functions.billing.expireTrial.scheduleAt({
  name: `trial-expiry:${trialId}`,
  replace: true,
  at: newExpiry,
  input: { trialId },
  key: `trial:${trialId}`,
});
```

The named schedule behaves like an upsert key.

This avoids duplicate timers and is safer than retaining an application timer ID in process memory.

---

# 68. Recommended implementation phases

## Phase 1 - Contract system

Implement:

```text
CREATE TYPE
CREATE PROCEDURE
system.types
system.type_fields
system.routines
system.routine_parameters
schema IR
```

Support all V1 return categories.

## Phase 2 - Synchronous callable procedures

Implement:

```text
CALL
ExecutionContext propagation
lazy transaction
nested calls
result conversion
security
CALL fast path
procedure plan caching
```

## Phase 3 - Code generation

Generate:

```text
Drizzle table types
procedure request/response types
typed client methods
server contracts
nested procedure handles
```

## Phase 4 - Topic triggers

Implement:

```text
CREATE TRIGGER ... ON TOPIC
automatic consumer groups
transaction + ACK semantics
retry / backoff
DLQ
stable invocation identity
```

## Phase 5 - Static scheduled events

Implement:

```text
CREATE EVENT
AT
EVERY
CRON
TIME ZONE
ENABLE / DISABLE
retries
overlap policy
misfire policy
system.schedules
scheduler ownership / failover
```

## Phase 6 - Dynamic schedules

Implement:

```text
scheduleAt
scheduleAfter
cancel
named replacement
system.scheduled_invocations
transactional schedule writes
```

## Phase 7 - Durable background invocation

Implement generated:

```text
procedure.enqueue()
ordering key
internal invocation topic
stable invocation ID
completion marker
```

Reuse topics instead of adding another queue subsystem.

## Phase 8 - TypeScript runtime

Add:

```text
multi-file TS project
V8 sandbox
capability host API
hot reload
```

## Phase 9 - Deployment and versioning

Implement:

```text
module artifacts
immutable revisions
activation / rollback
trigger compatibility
schedule compatibility
observability
```

## Phase 10 - Additional runtimes

Add:

```text
Rust / Wasm
TypeScript Component / Wasm
possibly C# later
```

behind the same procedure ABI.

---

# 69. Performance requirements

Kalam Functions may be invoked thousands of times per second.

The implementation must avoid turning every function into heavyweight infrastructure.

Required principles:

```text
no per-call database connection
no per-call DataFusion session unless needed
no actor/mailbox allocation unless benchmarks justify it
no UUID strings on hot path when numeric IDs work
no persistent start/end logs by default
no HTTP loopback to KalamDB
no timer task per schedule
```

Benchmark at minimum:

```text
10k CALL/sec
50k CALL/sec
100k CALL/sec
```

with:

```text
1 ms
10 ms
100 ms
```

procedure durations.

Measure:

```text
p50 / p95 / p99
allocations per invocation
CPU
RSS
active registry churn
cancellation latency
DataFusion session creation rate
background queue lag
schedule dispatch lag
```

---

# 70. Final architecture

```text
                           schema.sql
                               |
                               v
                        Schema Compiler
                               |
                         Contract IR
                               |
          +--------------------+--------------------+
          |                    |                    |
          v                    v                    v
       Tables              Procedures            Topics
          |                    |                    |
          |                    +----------+         |
          |                               |         |
          |                               v         |
          |                       Function Runtime  |
          |                        TS / Wasm        |
          |                               |         |
          +-------------------------------+---------+
                                          |
                                          v
                                      Kalam Host API
                                          |
                       +------------------+------------------+
                       |                  |                  |
                       v                  v                  v
                      DB                Topics            Schedules
                       |                  |                  |
                       +------------------+------------------+
                                          |
                                          v
                                   Durable KalamDB state
```

Background execution:

```text
Topic / Schedule / Enqueue
          |
          v
 durable partitioned invocation
          |
          v
     Root Execution
          |
          v
    ExecutionContext
          |
          v
      Procedure
```

---

# 71. Final design decisions

**SQL is the backend contract source of truth.**

**KalamDB tables remain the durable application state; functions do not own actor-local state.**

**Procedures are typed application actions generated into TypeScript, Dart, Rust, and future SDKs.**

**`CALL` is synchronous and transactional.**

**Nested calls reuse one root `ExecutionContext` and one transaction.**

**Background work is durable, not an ephemeral task.**

**Background procedure enqueue reuses KalamDB topics rather than introducing another queue implementation.**

**Ordering keys provide actor-like per-entity sequencing without actor-per-entity processes.**

**Strict keyed ordering is provided on durable async paths, not by distributed locks on normal synchronous `CALL`.**

**Topic triggers use the same procedure runtime and preserve durable source identity across retries.**

**Scheduled functions are first-class schema objects using `CREATE EVENT ... ON SCHEDULE ... DO CALL ...`.**

**Schedules support one-time `AT`, fixed `EVERY`, and `CRON` expressions with IANA time zones.**

**Dynamic one-shot schedules are durable rows and may be created transactionally from functions.**

**No process-local timer is authoritative.**

**Recurring schedules define overlap and misfire policies explicitly.**

**Every background invocation has a stable idempotency identity.**

**KalamDB should deduplicate already-committed background invocations where possible while keeping transport semantics at-least-once.**

**External side effects remain the function author's responsibility and should receive the stable idempotency key.**

**The existing `ExecutionContext` is extended instead of adding a second function context.**

**DataFusion remains lazy and is created only when a function actually needs query planning/execution.**

**Procedure `CALL` should bypass DataFusion planning when possible and reuse plan-cache infrastructure.**

**Only active tracked executions live in the hot in-memory registry.**

**Persistent function history is sampled/error-focused rather than one row per successful call.**

**Actors may be used internally for long-lived infrastructure, but actor semantics are not part of the KalamDB programming model.**

**Function code talks to KalamDB through an in-process host API, never through loopback HTTP.**

**TypeScript should be a real multi-file project with a sandboxed runtime and explicit capabilities.**

**Function revisions are immutable artifacts and can be rolled back when compatible with the live SQL contract.**

The resulting abstraction is:

> **SQL defines the state and contract. Code implements behavior. Topics and schedules decide when behavior runs. KalamDB keeps it durable, transactional, realtime, typed, and resumable.**
