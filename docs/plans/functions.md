# KalamDB Server Functions, Typed Contracts & Topic Triggers

**Status:** Proposed
**Version:** 0.1
**Primary design principle:** SQL is the source of truth for the complete backend contract.

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
+ Server-side application logic
+ Generated typed client SDK
```

without introducing a separate REST endpoint model.

Applications should use one programming interface:

```text
SELECT       → query
INSERT       → direct write
UPDATE       → direct write
DELETE       → direct write
SUBSCRIBE    → realtime
PUBLISH      → asynchronous event
CALL         → application/business operation
```

Server-side functions may be invoked in two ways:

```text
Client
  │
  │ CALL chat.create_message(...)
  ▼
Procedure
```

or:

```text
Topic
  │
  ▼
Trigger
  │
  ▼
Procedure
```

The same procedure implementation may be used by either invocation mechanism.

---

# 2. Core terminology

Use **Kalam Functions** as the product/feature name.

Internally distinguish three concepts.

## Procedure

A transactional business operation.

Example:

```sql
CALL chat.create_message(...);
```

A procedure may:

* read tables
* insert/update/delete
* publish topics
* call other procedures
* return values
* run under an authenticated execution context

## Function

Reserved for expression-style/read-only computation.

Example:

```sql
SELECT COSINE_DISTANCE(...);
SELECT SNOWFLAKE_ID();
```

Existing DataFusion UDFs remain unchanged.

## Trigger

Connects an event source to a procedure.

Initially only:

```text
TOPIC → PROCEDURE
```

Later:

```text
TABLE CHANGE → PROCEDURE
SCHEDULE → PROCEDURE
HTTP → PROCEDURE
```

---

# 3. SQL remains the source of truth

A Kalam project may contain:

```text
my-app/
├── kalam.toml
├── schema.sql
├── functions/
└── src/
    └── generated/
```

Or eventually:

```text
schema/
├── tables.sql
├── types.sql
├── topics.sql
├── procedures.sql
└── triggers.sql
```

These SQL files describe:

```text
Tables
Types
Topics
Procedures
Triggers
Policies
Permissions
```

The implementation code **does not redefine procedure signatures**.

---

# 4. Table row types

Every KalamDB table automatically creates a logical row type.

Example:

```sql
CREATE USER TABLE chat.messages (
    id BIGINT PRIMARY KEY,
    group_id TEXT NOT NULL,
    sender_id TEXT NOT NULL,
    body TEXT NOT NULL,
    created_at TIMESTAMP NOT NULL
);
```

This automatically creates the logical type:

```text
chat.messages
```

A procedure can therefore return one message directly:

```sql
CREATE PROCEDURE chat.create_message(
    group_id TEXT,
    body TEXT
)
RETURNS chat.messages;
```

No response structure needs to be repeated.

The implementation must return one row matching `chat.messages`.

---

# 5. Returning multiple table rows

Use PostgreSQL-like `SETOF`.

```sql
CREATE PROCEDURE chat.get_recent_messages(
    group_id TEXT,
    limit_count INT
)
RETURNS SETOF chat.messages;
```

Generated TypeScript:

```ts
type Result = Message[];
```

Client:

```ts
const messages =
    await kalam.chat.getRecentMessages({
        groupId,
        limitCount: 50
    });
```

---

# 6. Reusable response types

Many procedures will return structures that are not tables.

Do **not** repeat:

```sql
RETURNS TABLE (
   message ...,
   unread_count ...,
   ...
)
```

three times.

Instead add SQL composite types.

Example:

```sql
CREATE TYPE chat.recipient_result AS (
    user_id TEXT,
    delivered BOOLEAN
);
```

And:

```sql
CREATE TYPE chat.send_message_result AS (
    message chat.messages,
    recipients chat.recipient_result[],
    fanout_count INT
);
```

Now several procedures may reuse it:

```sql
CREATE PROCEDURE chat.create_message(
    group_id TEXT,
    body TEXT
)
RETURNS chat.send_message_result;
```

```sql
CREATE PROCEDURE chat.retry_message(
    message_id BIGINT
)
RETURNS chat.send_message_result;
```

```sql
CREATE PROCEDURE chat.forward_message(
    message_id BIGINT,
    target_group_id TEXT
)
RETURNS chat.send_message_result;
```

One type definition.

Three procedures.

---

# 7. Generated TypeScript

The SQL above generates something equivalent to:

```ts
export type Message = {
    id: bigint;
    groupId: string;
    senderId: string;
    body: string;
    createdAt: Date;
};

export interface RecipientResult {
    userId: string;
    delivered: boolean;
}

export interface SendMessageResult {
    message: Message;
    recipients: RecipientResult[];
    fanoutCount: number;
}
```

Notice:

```ts
message: Message
```

reuses the generated table type.

No duplication.

---

# 8. Supported procedure return types

Procedures should support six result categories.

## No result

```sql
RETURNS VOID
```

Generated:

```ts
Promise<void>
```

## Scalar

```sql
RETURNS BIGINT
RETURNS TEXT
RETURNS BOOLEAN
RETURNS UUID
RETURNS TIMESTAMP
```

Generated:

```ts
Promise<bigint>
Promise<string>
Promise<boolean>
```

## Table row

```sql
RETURNS chat.messages
```

Generated:

```ts
Promise<Message>
```

## Multiple table rows

```sql
RETURNS SETOF chat.messages
```

Generated:

```ts
Promise<Message[]>
```

## Named composite type

```sql
RETURNS chat.send_message_result
```

Generated:

```ts
Promise<SendMessageResult>
```

## Arbitrary JSON

```sql
RETURNS JSONB
```

Generated:

```ts
Promise<JsonValue>
```

Use arbitrary `JSONB` only where there genuinely is no stable contract.

Prefer named types whenever possible.

---

# 9. JSON response behavior

A named composite result:

```sql
RETURNS chat.send_message_result
```

is logically typed, but the HTTP SQL endpoint can serialize it naturally as JSON:

```json
{
  "message": {
    "id": "923472934",
    "groupId": "g1",
    "senderId": "u1",
    "body": "Hello"
  },
  "recipients": [
    {
      "userId": "u2",
      "delivered": true
    }
  ],
  "fanoutCount": 1
}
```

Therefore **typed object results do not need to be declared `JSONB` merely because the frontend wants JSON**.

The logical type remains:

```text
chat.send_message_result
```

while the transport encoding can be JSON.

This keeps type generation possible.

---

# 10. Nested reusable types

Types may contain:

* scalar types
* table row types
* other named composite types
* arrays
* JSON/JSONB
* nullable fields

Example:

```sql
CREATE TYPE users.user_summary AS (
    id TEXT,
    display_name TEXT,
    avatar_url TEXT
);
```

```sql
CREATE TYPE chat.message_details AS (
    message chat.messages,
    sender users.user_summary,
    reactions JSONB,
    participants users.user_summary[]
);
```

Then:

```sql
CREATE PROCEDURE chat.get_message_details(
    message_id BIGINT
)
RETURNS chat.message_details;
```

Generated:

```ts
interface MessageDetails {
    message: Message;
    sender: UserSummary;
    reactions: JsonValue;
    participants: UserSummary[];
}
```

---

# 11. Physical table columns

For V1, named composite types should primarily be **contract types** used by:

```text
Procedure parameters
Procedure returns
Topic payloads
Generated SDKs
```

Do not initially require KalamDB storage to support arbitrary nested composite columns.

That avoids substantially increasing the Arrow/RocksDB storage implementation scope.

Physical table support for composite types can be added later.

---

# 12. Procedure parameters may also use named types

Example:

```sql
CREATE TYPE chat.create_message_request AS (
    group_id TEXT,
    body TEXT,
    reply_to BIGINT
);
```

Then:

```sql
CREATE PROCEDURE chat.create_message(
    request chat.create_message_request
)
RETURNS chat.send_message_result;
```

Generated:

```ts
await kalam.chat.createMessage({
    request: {
        groupId,
        body,
        replyTo
    }
});
```

For simple functions, ordinary parameters remain preferable:

```sql
CREATE PROCEDURE chat.create_message(
    group_id TEXT,
    body TEXT
)
...
```

---

# 13. Callable procedures

Invocation uses SQL:

```sql
CALL chat.create_message(
    $1,
    $2
);
```

The existing KalamDB SQL endpoint remains the only required HTTP endpoint.

Conceptually:

```text
POST /v1/api/sql

        │
        ▼

CALL chat.create_message(...)

        │
        ▼

SqlExecutor

        │
        ▼

ProcedureHandler

        │
        ▼

Runtime

        │
        ▼

Result
```

No endpoint needs to be dynamically created.

---

# 14. Generated client invocation

Users should normally not manually write `CALL`.

Generated TypeScript:

```ts
const result =
    await kalam.chat.createMessage({
        groupId,
        body: "Hello!"
    });
```

Internally:

```sql
CALL chat.create_message($1, $2)
```

Generated Dart:

```dart
final result =
    await kalam.chat.createMessage(
        groupId: groupId,
        body: 'Hello!',
    );
```

Generated Rust:

```rust
let result = client
    .chat()
    .create_message(group_id, "Hello!")
    .await?;
```

One server contract generates all clients.

---

# 15. User execution context

Every client procedure invocation receives KalamDB's existing authentication context.

Conceptually:

```text
ExecutionContext

user_id
role
namespace
request_id
ip_address
transaction
```

Inside a function:

```ts
ctx.user.id
ctx.user.role
ctx.requestId
```

or Rust:

```rust
ctx.user_id()
ctx.role()
```

A client procedure defaults to:

```text
SECURITY INVOKER
```

Meaning the procedure operates using the permissions of the calling user.

Example:

```sql
CREATE PROCEDURE chat.create_message(...)
RETURNS chat.messages
SECURITY INVOKER;
```

This should be the default even when omitted.

---

# 16. Transaction semantics

Every procedure invocation is transactional by default.

```text
CALL chat.create_message()

        │
        ▼

BEGIN

INSERT message

UPDATE recipient state

UPDATE unread counters

PUBLISH topic

        │
        ▼

success?
   │
 ┌─┴─┐
 │   │
yes  no
 │   │
COMMIT ROLLBACK
```

All mutations should commit atomically.

Publishing a topic from a procedure must be staged until transaction commit.

This is especially important for:

```text
database mutation
+
topic publication
```

because it prevents the classic transactional-outbox inconsistency.

---

# 17. Calling multiple procedures

The SQL-first model allows:

```sql
BEGIN;

CALL shop.create_order($1);
CALL shop.reserve_inventory($1);
CALL audit.order_created($1);

COMMIT;
```

All procedures share:

```text
same authenticated user
same request
same transaction
```

If one fails:

```text
ROLLBACK everything
```

Procedures may also invoke another procedure through the host runtime.

A recursion/depth limit must prevent accidental cycles.

Suggested default:

```text
maximum nested procedure depth = 16
```

---

# 18. Topics

KalamDB already supports durable topics, consumer groups, replay, offsets and ACK semantics.

Extend topics with optional payload typing.

Example:

```sql
CREATE TYPE chat.message_created_event AS (
    message_id BIGINT,
    group_id TEXT,
    sender_id TEXT
);
```

Then:

```sql
CREATE TOPIC chat.message_created
TYPE chat.message_created_event;
```

A publish to that topic must match the declared type.

Untyped topics remain supported:

```sql
CREATE TOPIC events.generic;
```

Their payload type is effectively:

```text
JSONB
```

---

# 19. Database-to-topic routes remain unchanged

Existing functionality such as:

```sql
ALTER TOPIC chat.message_created
ADD SOURCE chat.messages
ON INSERT
WITH (payload = 'full');
```

continues to work.

Where possible, KalamDB can derive the payload type from:

```text
chat.messages
```

If one topic has multiple heterogeneous sources, either:

* declare a common named type
* use JSONB
* use a future tagged-union type

Do not complicate V1 with union types.

---

# 20. Topic-triggered procedures

Add:

```sql
CREATE TRIGGER chat.process_message
ON TOPIC chat.message_created
EXECUTE PROCEDURE chat.on_message_created(PAYLOAD);
```

Example procedure:

```sql
CREATE PROCEDURE chat.on_message_created(
    event chat.message_created_event
)
RETURNS VOID;
```

The trigger maps:

```text
topic payload
     ↓
procedure argument
```

The procedure may also be called manually:

```sql
CALL chat.on_message_created(...);
```

if desired.

---

# 21. Trigger argument expressions

V1 should support reserved values:

```text
PAYLOAD
EVENT_ID
ACTOR_USER_ID
TOPIC_NAME
PARTITION
OFFSET
```

Example:

```sql
CREATE TRIGGER chat.process_message
ON TOPIC chat.message_created
EXECUTE PROCEDURE chat.process_message(
    PAYLOAD,
    EVENT_ID
);
```

Procedure:

```sql
CREATE PROCEDURE chat.process_message(
    event chat.message_created_event,
    event_id TEXT
)
RETURNS VOID;
```

Future versions may support expressions such as:

```sql
PAYLOAD.message_id
```

but this is not required for V1.

---

# 22. Topic consumer identity

Each trigger automatically owns a durable consumer group.

The user does **not** configure it manually.

Conceptually:

```text
trigger id:
TRG_123

consumer group:
__kalam_function_trigger_TRG_123
```

That gives every trigger its own:

* offset
* retry state
* replay position
* metrics
* lifecycle

---

# 23. Topic delivery semantics

Use KalamDB's existing:

```text
at-least-once
```

semantics.

Execution:

```text
read topic message
       ↓
invoke procedure
       ↓
begin transaction
       ↓
perform work
       ↓
commit
       ↓
ACK topic offset
```

If execution fails:

```text
no ACK
   ↓
retry
```

Suggested trigger options:

```sql
CREATE TRIGGER chat.process_message
ON TOPIC chat.message_created
EXECUTE PROCEDURE chat.on_message_created(PAYLOAD)
WITH (
    start = 'latest',
    retries = 5,
    concurrency = 4,
    retry_backoff = '1s'
);
```

After maximum retries:

```text
DLQ
```

can use an internal system topic.

---

# 24. Trigger security context

Topic-triggered execution needs two identities.

Expose:

```text
ctx.actorUser
ctx.executionPrincipal
```

`actorUser`:

> the user who originally caused the event, when available.

`executionPrincipal`:

> the function/module identity under which the background consumer executes.

Do not silently grant background functions administrator privileges.

Triggers should execute with explicitly assigned module/function permissions.

---

# 25. Procedure code project

A function deployment is a **complete application project**, not one source file per function.

Example TypeScript project:

```text
functions/
├── package.json
├── tsconfig.json
├── src/
│   ├── chat/
│   │   ├── create_message.ts
│   │   ├── delete_message.ts
│   │   └── process_message.ts
│   │
│   ├── users/
│   │   └── profile.ts
│   │
│   ├── services/
│   │   ├── fanout.ts
│   │   └── moderation.ts
│   │
│   └── common/
│       ├── validation.ts
│       └── permissions.ts
│
└── .kalam/
    └── generated/
        ├── contracts.ts
        └── registry.ts
```

Developers may use:

* subfolders
* helper functions
* classes
* modules
* npm dependencies
* shared utilities
* tests

Only public procedure entrypoints have a convention.

---

# 26. No manual SQL → code mapping

The developer must **not** write:

```text
chat.create_message = src/foo/create.ts
```

manually.

Instead:

```text
schema.sql
      ↓
kalam generate
      ↓
generated registry
```

For:

```sql
CREATE PROCEDURE chat.create_message(...)
```

Kalam knows the conventional implementation path:

```text
functions/src/chat/create_message.ts
```

If it does not exist:

```bash
kalam generate
```

prints:

```text
New procedure detected:
  chat.create_message

Created:
  functions/src/chat/create_message.ts
```

The file is scaffolded once.

It is never overwritten afterward.

---

# 27. Generated TypeScript server entrypoint

Example generated scaffold:

```ts
import {
  defineProcedure,
  type ChatCreateMessage
} from "../.kalam/generated/contracts";

export default defineProcedure<ChatCreateMessage>(
  async (ctx, input) => {

    // implementation

  }
);
```

The generated `ChatCreateMessage` contract already knows:

```ts
Input
Output
Context
```

Changing the SQL signature causes TypeScript compilation errors until the implementation is updated.

---

# 28. Using generated table types inside procedures

The server function project should reuse the same generated table definitions already used by KalamDB's TypeScript ORM.

Example:

```ts
import { messages } from "@generated/schema";

export default defineProcedure<ChatCreateMessage>(
  async (ctx, input) => {

    const message =
      await ctx.db
        .insert(messages)
        .values({
          groupId: input.groupId,
          senderId: ctx.user.id,
          body: input.body
        })
        .returningOne();

    return message;
  }
);
```

`message` already has the generated `Message` row type.

Therefore returning:

```sql
RETURNS chat.messages
```

requires no extra model class.

---

# 29. Do not use HTTP from server functions to KalamDB itself

`ctx.db` should use an in-process host API.

Do not implement:

```text
WASM/V8
   ↓
HTTP
   ↓
KalamDB
```

Implement:

```text
WASM/V8
   ↓
Kalam Host API
   ↓
existing executor / transaction layer
```

The Drizzle-like API may compile to host calls internally.

---

# 30. Runtime abstraction

A deployed module should not expose runtime implementation details to callers.

KalamDB understands:

```text
Procedure contract
        +
Module artifact
```

Suggested runtime interface:

```text
KalamModuleRuntime

load(module)
invoke(procedure, context, args)
shutdown()
```

Implementations:

```text
WasmRuntime
TypescriptRuntime
```

---

# 31. Rust and compiled languages

Rust implementation:

```text
Rust project
   ↓
Wasm Component
   ↓
Wasmtime
```

The whole Rust project may contain unlimited source files/modules and dependencies compatible with the WASI/Wasm environment.

One compiled module may export many Kalam procedures.

---

# 32. Full TypeScript project support

Yes, KalamDB can support a **real multi-file TypeScript project**.

Two runtime strategies are viable.

## Option A — TypeScript + V8

Recommended for first-class TypeScript support.

Build:

```text
TypeScript project
      ↓
tsc/esbuild
      ↓
bundled JavaScript
      ↓
V8 isolate
```

Advantages:

* real TypeScript syntax
* normal classes/modules
* npm ecosystem
* async/await
* familiar JavaScript semantics
* smaller conceptual jump for web developers

This is similar to the architecture SpacetimeDB 2.0 currently uses for TypeScript modules.

## Option B — TypeScript → JavaScript → Wasm Component

Build:

```text
TypeScript
    ↓
transpile
    ↓
JavaScript
    ↓
componentize-js
    ↓
WebAssembly Component
    ↓
Wasmtime
```

This is technically possible today.

It keeps one runtime technology in KalamDB:

```text
Wasmtime
```

but has tradeoffs:

* JS engine is embedded in the component
* larger artifacts
* Node compatibility is incomplete
* not every npm package works
* native Node addons do not work
* filesystem/network APIs require explicit capabilities

---

# 33. Recommended TypeScript decision

Support:

```toml
[functions]
runtime = "typescript"
path = "functions"
```

The public contract should not say whether TypeScript internally uses V8 forever.

For the initial implementation I recommend:

```text
TypeScript → V8
Rust       → Wasmtime
```

because it gives users genuine TypeScript rather than a TypeScript-like language.

Later optionally add:

```text
typescript_component
```

for users who explicitly want Wasm isolation.

---

# 34. Important npm limitation

"Complete TypeScript project" should mean:

* multiple files
* subfolders
* package.json
* pure JavaScript/TypeScript npm packages
* classes
* async code
* bundling
* tests

It should **not** mean unrestricted Node.js.

Functions must not automatically receive:

```text
fs
net
child_process
process.env
native .node addons
```

Those would bypass the KalamDB sandbox.

Capabilities must be explicit.

---

# 35. Function host capabilities

Initial server functions receive:

```text
ctx.db.query()
ctx.db.execute()

ctx.topics.publish()

ctx.user
ctx.request

ctx.log
```

No default:

```text
filesystem
arbitrary TCP
process spawning
environment variables
HTTP
```

External HTTP can be added later as an explicit capability.

Example:

```toml
[functions.capabilities]
http = false
filesystem = false
```

---

# 36. Module/project configuration

Extend `kalam.toml`.

Example:

```toml
[project]
name = "masky"

[schema]
mode = "sql"
path = "schema.sql"

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

[schema.targets.typescript]
output = "src/generated/kalam.ts"

[schema.targets.dart]
output = "lib/generated/kalam.dart"
```

This configures the **module project**, not individual functions.

There is still no per-procedure mapping.

---

# 37. Generated artifacts

One command:

```bash
kalam generate
```

should generate all client/server contracts.

Conceptually:

```text
schema.sql
    │
    ▼
Kalam Schema Compiler
    │
    ├── tables
    ├── types
    ├── topics
    ├── procedures
    └── triggers
          │
          ▼
     Schema IR
          │
 ┌────────┼────────┬─────────┐
 ▼        ▼        ▼         ▼
TS      Dart     Rust     Server contracts
```

---

# 38. TypeScript client generation

Generated:

```ts
export interface SendMessageResult {
   message: Message;
   recipients: RecipientResult[];
   fanoutCount: number;
}

export interface KalamProcedures {
   chat: {
      createMessage(
        input: CreateMessageInput
      ): Promise<SendMessageResult>;
   };
}
```

User:

```ts
const result =
    await kalam.chat.createMessage({
        groupId,
        body
    });
```

---

# 39. Existing Drizzle integration

Continue generating tables through the existing KalamDB ORM integration.

Conceptually generated output contains both:

```ts
// database schema
export const messages = ...

export type Message =
    typeof messages.$inferSelect;
```

and:

```ts
// application operations
export interface SendMessageResult {
    message: Message;
    fanoutCount: number;
}

export const chat = {
    createMessage: ...
};
```

Drizzle owns table/query ergonomics.

KalamDB owns procedure/topic contracts.

Do not modify Drizzle itself to understand Kalam procedures.

---

# 40. Local development

Extend:

```bash
kalam dev
```

to watch:

```text
schema.sql
functions/src/**
package.json
```

Development loop:

```text
schema changes
      ↓
parse/validate
      ↓
regenerate types
      ↓
rebuild function project
      ↓
validate module contract
      ↓
hot activate new revision
```

Function source change:

```text
source changes
      ↓
rebuild
      ↓
new local revision
      ↓
hot swap
```

---

# 41. Deployment

```bash
kalam deploy
```

should perform:

```text
1. Parse SQL schema
2. Generate canonical contract
3. Build function project
4. Inspect built module exports
5. Validate implementation against SQL
6. Validate permissions/capabilities
7. Apply schema migration (types, routines, triggers → catalog rows)
8. Upload module artifact bytes
9. Insert revision + artifact catalog rows
10. Activate revision (point module at new revision)
11. Run health validation
```

A contract mismatch prevents deployment.

Deploy is transactional with respect to catalog state: either the new types/routines/revision/artifact rows become the active contract together, or none of them do.

---

# 42. Module versions

Version the **whole function module**, not every individual source file.

Example:

```text
backend

revision 41
revision 42
revision 43 ← active
```

Revision metadata is stored as rows in `system.function_revisions` (see §49), including:

```text
revision_id
module_name
artifact_id
artifact_hash
source_hash
schema_hash
runtime
created_at
created_by
status
```

Each revision is immutable. Artifact file bytes are retained for every non-purged revision so rollback can switch the active pointer without rebuilding.

---

# 43. Rollback

```bash
kalam functions rollback backend 42
```

switches:

```text
revision 43
    ↓
revision 42
```

without rebuilding.

Rollback updates the active revision pointer on `system.function_modules`; it does not rewrite historical revision or artifact rows.

Rollback is allowed only if the old module is compatible with the current procedure/schema contract.

---

# 44. Source code management

KalamDB should not replace Git.

Recommended:

```text
Git
 ↓
TypeScript/Rust source code
 ↓
kalam build
 ↓
deployable artifact
 ↓
KalamDB catalog + artifact store
```

KalamDB stores in the **system catalog** (and linked artifact storage):

* compiled artifact file bytes (`system.function_artifacts`)
* revision metadata (`system.function_revisions`)
* module active pointer (`system.function_modules`)
* types and type fields (`system.types`, `system.type_fields`)
* procedures and parameters (`system.routines`, `system.routine_parameters`)
* triggers (`system.triggers`)
* manifest
* schema digest
* source commit/hash
* optionally source archive (as an additional artifact row)

Git remains source control.

A hosted Kalam Cloud service could later offer remote builds from source.

---

# 45. Build manifest

Build output includes an automatically generated manifest.

Example:

```json
{
  "module": "backend",
  "schemaHash": "...",
  "procedures": {
    "chat.create_message": {
      "input": "...",
      "output": "chat.send_message_result"
    },
    "chat.process_message": {
      "input": "...",
      "output": "void"
    }
  }
}
```

The user never writes this manually.

---

# 46. Runtime validation

Static type generation is not enough.

KalamDB validates:

```text
arguments
return value
topic payload
```

at runtime.

Example:

SQL says:

```sql
RETURNS chat.messages
```

but implementation returns:

```json
{
  "foo": "bar"
}
```

Invocation fails with:

```text
PROCEDURE_RETURN_TYPE_MISMATCH
```

before returning invalid data to the client.

---

# 47. SQL response model

Existing SQL HTTP endpoint should return procedure results through the standard result format.

For:

```sql
CALL chat.create_message(...);
```

table result:

```json
{
  "type": "procedure",
  "columns": [...],
  "rows": [...]
}
```

Named composite results may be represented naturally as an object:

```json
{
  "type": "procedure",
  "value": {
    "message": {...},
    "fanoutCount": 5
  }
}
```

The SDK normalizes these wire representations automatically.

---

# 48. PGWire behavior

For simple results:

```text
scalar → one column
table row → normal columns
SETOF → normal rows
```

For nested composite types, V1 may encode nested values as JSON/JSONB rather than immediately implementing PostgreSQL composite OIDs.

The generated Kalam SDK hides this implementation detail.

Native PostgreSQL composite type protocol support can be added later.

---

# 49. Procedure catalog and artifact storage

Function contracts, types, revisions, and deployable files are first-class **system catalog** data — the same durability and queryability model as existing `system.*` metadata tables.

Add system metadata tables:

```text
system.types
system.type_fields
system.routines
system.routine_parameters
system.triggers
system.function_modules
system.function_revisions
system.function_artifacts
system.function_runs
```

`information_schema.routines` (and related type views where useful) should expose standards-compatible routine information where possible.

## Catalog ownership

| Concern | Catalog |
|---|---|
| Named composite / request / response / event types | `system.types`, `system.type_fields` |
| Procedure signatures and return contracts | `system.routines`, `system.routine_parameters` |
| Topic → procedure bindings | `system.triggers` |
| Deployable module identity + active revision | `system.function_modules` |
| Immutable module revisions | `system.function_revisions` |
| Compiled wasm / JS bundle / optional source archive bytes | `system.function_artifacts` |
| Execution history / observability | `system.function_runs` |

Types declared with `CREATE TYPE` are inserted as catalog rows at schema apply time, next to tables/topics — not only generated into client SDKs.

## Suggested row shapes

### `system.types` / `system.type_fields`

```text
system.types
  type_id
  namespace
  type_name
  kind            -- composite | enum | alias (V1: composite)
  schema_hash
  created_at
  updated_at

system.type_fields
  type_id
  field_ordinal
  field_name
  field_type      -- scalar, table row ref, nested type, array
  is_nullable
```

### `system.routines` / `system.routine_parameters`

```text
system.routines
  routine_id
  namespace
  routine_name
  routine_kind    -- procedure | function
  return_kind     -- scalar | row | setof | composite | void | jsonb
  return_type_id  -- nullable FK → system.types / table type
  module_name     -- implementing function module
  security_mode
  created_at
  updated_at

system.routine_parameters
  routine_id
  param_ordinal
  param_name
  param_mode      -- in | out | inout
  param_type
  is_nullable
  default_expr    -- optional
```

### `system.function_modules` / `system.function_revisions` / `system.function_artifacts`

```text
system.function_modules
  module_name
  runtime         -- typescript | wasm | ...
  active_revision_id
  status          -- active | disabled
  created_at
  updated_at

system.function_revisions
  revision_id
  module_name
  revision_number
  artifact_id     -- FK → system.function_artifacts
  artifact_hash
  source_hash
  schema_hash
  manifest_json
  created_at
  created_by
  status          -- pending | active | inactive | failed

system.function_artifacts
  artifact_id
  module_name
  revision_id
  kind            -- compiled_module | source_archive | manifest
  content_type    -- application/wasm | application/javascript | ...
  content_hash
  size_bytes
  storage_backend -- catalog_blob | filestore (implementation choice)
  storage_key     -- opaque locator when bytes live in filestore
  created_at
```

Artifact **bytes** may be stored either:

1. inline / in the catalog blob store behind `system.function_artifacts`, or
2. in the existing filestore / object-store path, with `storage_key` + hash recorded in the catalog row.

Either way, the **catalog row is authoritative**: deploy, activate, rollback, and load always resolve files through `system.function_artifacts` joined to the active revision.

## Deploy writes catalog rows

On successful `kalam deploy`:

```text
1. Upsert CREATE TYPE definitions → system.types / system.type_fields
2. Upsert CREATE PROCEDURE definitions → system.routines / system.routine_parameters
3. Upsert CREATE TRIGGER definitions → system.triggers
4. Insert compiled artifact → system.function_artifacts (+ bytes)
5. Insert immutable revision → system.function_revisions
6. Point system.function_modules.active_revision_id at the new revision
```

Runtime load path:

```text
CALL / trigger
  → system.routines (contract)
  → system.function_modules.active_revision_id
  → system.function_revisions
  → system.function_artifacts
  → ModuleRuntime.load(artifact bytes)
```

## Durability rules

* Revision and artifact rows are immutable after insert.
* Activating or rolling back only updates `system.function_modules.active_revision_id`.
* Dropping a type/procedure/trigger updates catalog rows through normal schema DDL; active revisions must remain compatible with the live contract or deploy/rollback is rejected.
* Purging old inactive revisions may delete artifact bytes only after an explicit retention/GC policy; catalog history should remain queryable for retained revisions.

---


# 50. Observability

Every procedure execution receives:

```text
execution_id
request_id
procedure
module_revision
user
start_time
duration
status
error
```

Topic-triggered execution additionally records:

```text
topic
partition
offset
event_id
retry_count
```

CLI:

```bash
kalam functions logs
kalam functions logs chat.create_message
kalam functions runs chat.create_message
```

---

# 51. Resource limits

Every runtime invocation must enforce:

```text
timeout
memory
maximum return size
maximum nested procedure depth
maximum DB operations
maximum rows
```

Wasm additionally:

```text
fuel / epoch interruption
```

V8:

```text
isolate memory limit
execution deadline
termination
```

One broken user procedure must never compromise the KalamDB server.

---

# 52. Recommended `create_message` example

Schema:

```sql
CREATE USER TABLE chat.messages (
    id BIGINT PRIMARY KEY DEFAULT SNOWFLAKE_ID(),
    group_id TEXT NOT NULL,
    sender_id TEXT NOT NULL,
    body TEXT NOT NULL,
    created_at TIMESTAMP DEFAULT NOW()
);

CREATE TYPE chat.delivery_result AS (
    user_id TEXT,
    delivered BOOLEAN
);

CREATE TYPE chat.send_message_result AS (
    message chat.messages,
    deliveries chat.delivery_result[],
    fanout_count INT
);

CREATE TYPE chat.message_created_event AS (
    message_id BIGINT,
    group_id TEXT,
    sender_id TEXT
);

CREATE TOPIC chat.message_created
TYPE chat.message_created_event;

CREATE PROCEDURE chat.create_message(
    group_id TEXT,
    body TEXT
)
RETURNS chat.send_message_result
SECURITY INVOKER;

CREATE PROCEDURE chat.after_message_created(
    event chat.message_created_event
)
RETURNS VOID;

CREATE TRIGGER chat.message_created_handler
ON TOPIC chat.message_created
EXECUTE PROCEDURE chat.after_message_created(PAYLOAD)
WITH (
    retries = 5,
    concurrency = 4
);
```

---

# 53. TypeScript implementation

Generated implementation file:

```text
functions/src/chat/create_message.ts
```

User writes:

```ts
import { defineProcedure } from "@kalamdb/server";
import type {
  ChatCreateMessage
} from "../.kalam/generated/contracts";

export default defineProcedure<ChatCreateMessage>(
  async (ctx, input) => {

    const message =
      await ctx.db.messages.insert({
        groupId: input.groupId,
        senderId: ctx.user.id,
        body: input.body
      });

    const members =
      await ctx.db.groupMembers
        .where({ groupId: input.groupId });

    const deliveries = [];

    for (const member of members) {

      await ctx.db.userUpdates.upsert({
        userId: member.userId,
        groupId: input.groupId,
        lastSeq: message._seq
      });

      deliveries.push({
        userId: member.userId,
        delivered: true
      });
    }

    await ctx.topics.publish(
      "chat.message_created",
      {
        messageId: message.id,
        groupId: message.groupId,
        senderId: message.senderId
      }
    );

    return {
      message,
      deliveries,
      fanoutCount: deliveries.length
    };
  }
);
```

All operations participate in one KalamDB transaction.

---

# 54. Client usage

The generated frontend API becomes:

```ts
const result =
    await kalam.chat.createMessage({
        groupId,
        body: text
    });

console.log(result.message);
console.log(result.fanoutCount);
```

No:

```text
REST controller
DTO
API route
fetch()
manual JSON model
topic fanout worker
```

is required.

---

# 55. Proposed implementation architecture

```text
                         schema.sql
                             │
                             ▼
                     Schema Compiler
                             │
                         Contract IR
                             │
              ┌──────────────┼──────────────┐
              ▼              ▼              ▼
         Table types     Procedures      Topics
              │              │              │
              └──────────────┼──────────────┘
                             │
                      Code Generation
                             │
             ┌───────────────┴──────────────┐
             ▼                              ▼
        Client SDK                  Server Contracts


Client
   │
   │ CALL procedure
   ▼
SqlExecutor
   │
   ▼
ProcedureHandler
   │
   ▼
ProcedureRegistry
   │
   ▼
ModuleRuntime
   │
 ┌─┴─────────────┐
 ▼               ▼
Wasmtime         V8
   │               │
   └──────┬────────┘
          ▼
     Kalam Host API
          │
  ┌───────┼────────┐
  ▼       ▼        ▼
 DB     Topics   Context
```

Topic execution enters at:

```text
Topic Store
    │
    ▼
Trigger Dispatcher
    │
    ▼
ProcedureRegistry
```

and then uses exactly the same procedure runtime.

---

# 56. Implementation phases

## Phase 1 — Contract system

Implement:

```text
CREATE TYPE
CREATE PROCEDURE
system.types / system.type_fields catalog rows
system.routines / system.routine_parameters catalog rows
schema parser
schema IR
```

Support:

```text
scalar returns
table row returns
SETOF table
composite returns
JSONB
VOID
```

## Phase 2 — Callable procedures

Implement:

```text
CALL namespace.procedure(...)
ExecutionContext propagation
transactions
result conversion
nested procedure calls
```

Start with Rust/Wasm or TypeScript runtime.

## Phase 3 — Code generation

Extend:

```bash
kalam schema gen
```

into:

```bash
kalam generate
```

Generate:

```text
Drizzle tables
custom types
procedure request/response types
client methods
server contracts
entrypoint stubs
```

## Phase 4 — Topic triggers

Implement:

```text
CREATE TRIGGER ... ON TOPIC
durable trigger consumers
retry
ACK
DLQ
trigger execution context
```

Reuse existing topic offsets/consumer machinery.

## Phase 5 — TypeScript projects

Add:

```text
functions/package.json
functions/tsconfig.json
multi-file build
V8 sandbox
generated server SDK
hot reload
```

## Phase 6 — Deployment/versioning

Implement:

```text
system.function_modules / function_revisions / function_artifacts
immutable revisions with catalog rows
artifact file storage linked from catalog
type + routine catalog upserts on deploy
compatibility checks
activation / rollback via active_revision_id
logs
metrics
```

## Phase 7 — Additional runtimes

Add:

```text
Rust/Wasm
TypeScript Component/Wasm
possibly C#
```

all behind the same procedure ABI.

---

# 57. Final product model

KalamDB project:

```text
                      schema.sql
                          │
               Backend source of truth
                          │
       ┌──────────────────┼──────────────────┐
       │                  │                  │
     Tables            Procedures          Topics
       │                  │                  │
       │                  └─────────┐        │
       │                            ▼        │
       │                     Functions code │
       │                     TS / Rust      │
       │                            │        │
       │                            ▼        │
       │                        Runtime      │
       │                            │        │
       └────────────────────────────┼────────┘
                                    │
                              kalam generate
                                    │
                   ┌────────────────┼────────────────┐
                   ▼                ▼                ▼
              TypeScript           Dart            Rust
                   │
                   ▼
             Application

await kalam.chat.createMessage(...)
```

The developer owns:

```text
1. SQL contract
2. Business logic source code
```

KalamDB owns everything else:

```text
type generation
client bindings
function registration
topic consumption
authentication context
transactions
deployment
versioning
runtime isolation
retries
observability
```

---

# 58. Key design decisions

**SQL is the only backend contract source of truth.**

**Tables are automatically reusable row types.**

**`CREATE TYPE` provides reusable request/response/event objects.**

**`CALL` is used for side-effecting business logic.**

**Topic triggers invoke the exact same procedure runtime.**

**No manual procedure-to-source mapping is required.**

**A function module is a complete multi-file codebase, not a collection of isolated snippets.**

**TypeScript should be supported as a real TypeScript project.**

**For maximum TypeScript/npm compatibility, use a sandboxed V8 runtime; for Rust and other compiled languages, use Wasmtime.**

**Generated SDKs make `CALL` feel like a normal typed method call.**

The resulting abstraction is effectively:

> **SQL defines your backend. Code implements its behavior. KalamDB generates and runs the API.**


# 59. Unified Execution Model

KalamDB SHALL use a single root execution abstraction for SQL requests, callable procedures and topic-triggered functions.

The existing:

```rust
ExecutionContext
```

SHALL be extended rather than introducing a parallel `FunctionExecutionContext`.

Conceptually:

```text
                     ExecutionContext
                            │
          ┌─────────────────┼──────────────────┐
          │                 │                  │
          ▼                 ▼                  ▼
      Identity        TransactionScope      Control
  user / role / ns                           │
                                             │
                       ┌─────────────────────┼────────────┐
                       ▼                     ▼            ▼
                   Functions            DataFusion      Topics
                       │                    lazy
                       ▼
                 nested functions
```

The same context flows through the entire root execution.

---

# 60. ExecutionContext Responsibilities

Extend the existing KalamDB `ExecutionContext` to conceptually contain:

```rust
pub struct ExecutionContext {
    // Existing
    auth_session: AuthSession,
    namespace_id: Option<NamespaceId>,

    base_session_context: Arc<SessionContext>,
    session_context_cache: Arc<OnceCell<SessionContext>>,

    // New
    transaction_scope: TransactionScope,

    execution_control: Option<Arc<ExecutionControl>>,

    function_stack: SmallVec<[ProcedureId; 4]>,
}
```

The actual implementation may optimize or separate fields internally.

The semantic ownership SHALL remain:

```text
ExecutionContext
    ├── identity
    ├── namespace
    ├── transaction scope
    ├── cancellation
    ├── function nesting
    └── lazy DataFusion context
```

The existing DataFusion design already lazily creates a per-user `SessionContext` from a shared base session and caches it in `OnceCell`, so this model extends the current architecture rather than replacing it.

---

# 61. Root Execution

A **root execution** is created for any independently executing server operation that requires lifecycle tracking.

Examples:

```text
CALL procedure
topic-triggered procedure
PGWire procedure invocation
scheduled procedure — future
```

A normal lightweight SQL query MAY continue using the existing ephemeral `ExecutionContext` without registering itself as a tracked execution.

This prevents execution observability from adding unnecessary overhead to the ordinary SQL fast path.

---

# 62. Execution Runtime Is Not DataFusion

DataFusion is an execution engine used by an `ExecutionContext`.

It is not the root execution abstraction.

For example:

```text
CALL chat.create_message(...)
       │
       ▼
ExecutionContext
       │
       ▼
TypeScript / Wasm
       │
       ├── INSERT
       ├── Topic publish
       │
       └── ctx.db.query()
                  │
                  ▼
              DataFusion
```

A server function may complete without ever creating a DataFusion `SessionContext`.

Therefore:

```text
ExecutionContext
```

owns:

```text
DataFusion
```

rather than the reverse.

---

# 63. Lazy DataFusion Creation

Preserve the current lazy DataFusion behavior.

Initially:

```text
ExecutionContext

DataFusion SessionContext:
NOT CREATED
```

If execution requires:

```text
SELECT
JOIN
aggregation
DataFusion expression evaluation
```

the existing:

```rust
session_context_cache:
    OnceCell<SessionContext>
```

creates the per-user DataFusion session.

Subsequent queries inside the same root execution reuse it.

The current implementation already shares heavy DataFusion state and clones mostly `Arc`-backed `SessionState`; KalamDB's current source estimates the per-request clone in the low-microsecond range.

---

# 64. Automatic Function Transaction

A root procedure invocation SHALL behave as one transactional unit by default.

Example:

```sql
CALL app.function1();
```

where:

```text
function1
    │
    ├── INSERT A
    ├── UPDATE B
    │
    └── function2
           │
           ├── INSERT C
           └── UPDATE D
```

SHALL behave as:

```text
                 Root Execution EX100
                         │
                         ▼
                    Transaction TX1
                         │
        ┌────────────────┼─────────────────┐
        ▼                ▼                 ▼
    INSERT A         UPDATE B          function2
                                         │
                                  ┌──────┴──────┐
                                  ▼             ▼
                              INSERT C       UPDATE D

                         │
                         ▼
                    all successful?
                       /     \
                     yes      no
                      │        │
                   COMMIT   ROLLBACK
```

If `function2` fails:

```text
A rolled back
B rolled back
C rolled back
D rolled back
```

---

# 65. Lazy Transaction Creation

A procedure SHALL NOT necessarily create a database transaction immediately.

Example:

```typescript
return {
    userId: ctx.user.id
};
```

may require no database transaction.

Initial state:

```text
TransactionScope::None
```

The first transactional mutation:

```text
INSERT
UPDATE
DELETE
transactional topic publish
```

starts a transaction lazily.

Example:

```text
CALL create_message()

TransactionScope = NONE

        │
        ▼

INSERT message

        │
        ▼

BEGIN TX42

TransactionScope = OWNED(TX42)
```

All later transactional work uses `TX42`.

---

# 66. TransactionScope

Introduce a logical abstraction:

```rust
pub enum TransactionScope {
    None,

    Owned(TransactionId),

    Borrowed(TransactionId),

    Aborted(TransactionId),

    Committing(TransactionId),
}
```

## Owned

The root procedure created the transaction.

It is responsible for:

```text
COMMIT
ROLLBACK
```

## Borrowed

The procedure is executing inside an existing SQL transaction.

It participates in that transaction but does not own its final commit.

---

# 67. Procedure Inside Explicit SQL Transaction

Example:

```sql
BEGIN;

CALL app.function1();

INSERT INTO ...;

CALL app.function3();

COMMIT;
```

The transaction is:

```text
SQL transaction TX55
        │
        ├── function1
        ├── INSERT
        └── function3
```

Each function receives:

```text
TransactionScope::Borrowed(TX55)
```

The procedure MUST NOT commit `TX55`.

Only:

```sql
COMMIT
```

ends the transaction.

---

# 68. Nested Function Calls

Nested procedure calls SHALL reuse the same root `ExecutionContext`.

Example:

```text
function1()
   ↓
function2()
   ↓
function3()
```

SHALL NOT normally create:

```text
3 actors
3 transactions
3 execution registries
```

Instead:

```text
                    Root Execution
                          │
                    ExecutionContext
                          │
                    Transaction TX1
                          │
                   Function Stack
                          │
               function1 → function2 → function3
```

This keeps nested calls extremely cheap.

---

# 69. Function Stack

Track nested functions using IDs rather than allocated names.

Recommended representation:

```rust
SmallVec<[ProcedureId; 4]>
```

Example:

```text
EX100

chat.create_message
   → chat.fanout
      → notifications.enqueue
```

The stack may be exposed for debugging.

Procedure names are resolved from catalog metadata only when required.

---

# 70. Nested Failure Semantics

For V1, a database/runtime failure inside any nested procedure SHALL abort the transaction.

Example:

```typescript
try {
    await ctx.functions.billing.charge();
} catch (e) {
    // application catches error
}
```

If `billing.charge()` caused a transactional failure:

```text
TransactionScope = ABORTED
```

The caller cannot later commit the transaction.

This follows a strict transactional model similar to PostgreSQL failed transactions.

Future versions may support:

```text
SAVEPOINT
```

or isolated child transactions.

Not required for V1.

---

# 71. Topic Trigger Transactions

A topic-triggered function gets the exact same execution model.

```text
Topic event
    │
    ▼
ExecutionContext
    │
    ▼
Procedure
    │
    ▼
lazy Transaction
```

Success:

```text
COMMIT
  ↓
ACK topic offset
```

Failure:

```text
ROLLBACK
  ↓
NO ACK
  ↓
retry according to trigger policy
```

This integrates naturally with KalamDB's existing durable topic consumer and ACK model.

---

# 72. Topic Publication Inside Transactions

A function may perform:

```typescript
await ctx.topics.publish(...);
```

If the root execution has a transactional scope, publication SHOULD participate in that transaction.

Example:

```text
BEGIN

INSERT message
UPDATE inbox
PUBLISH messages.created

COMMIT
```

If execution fails:

```text
message       not committed
inbox update  not committed
topic event   not published
```

This avoids the traditional database/event dual-write problem.

---

# 73. Root Execution Cancellation

Tracked executions receive an:

```text
ExecutionControl
```

attached to the existing `ExecutionContext`.

Do not create another full execution context.

Suggested lightweight form:

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

Implementation should prefer numeric IDs/enums over strings.

---

# 74. ActiveExecutionRegistry

Introduce:

```text
ActiveExecutionRegistry
```

rather than a heavyweight:

```text
FunctionExecutionManager
```

The registry contains only **currently active tracked executions**.

Conceptually:

```rust
DashMap<ExecutionId, Arc<ExecutionControl>>
```

The registry MUST NOT persist execution start/end events to RocksDB or Raft.

---

# 75. Registry Cost Model

Function invocation rate may be much higher than concurrent function count.

Example:

```text
10,000 invocations/sec
average duration = 2 ms
```

Expected average active executions:

```text
≈ 20
```

Therefore memory usage is primarily based on:

```text
concurrency
```

rather than:

```text
requests per second
```

The key hot-path cost is insert/remove churn.

Implementation must minimize:

```text
allocations
locking
string construction
logging
UUID formatting
```

---

# 76. Execution IDs

Use compact numeric execution IDs.

Preferred:

```rust
u64
```

Options:

```text
per-node AtomicU64
Snowflake-compatible distributed ID
node bits + local sequence
```

Avoid allocating UUID strings on the hot path.

Human-readable formatting happens only at the API/view boundary.

---

# 77. RAII Execution Registration

Execution tracking SHOULD use an RAII guard.

Conceptually:

```rust
let guard =
    active_executions.register(...);
```

When execution completes:

```text
drop guard
    ↓
remove from registry
```

This guarantees cleanup on:

```text
success
failure
early return
cancellation
panic boundary
```

---

# 78. `system.function_runs`

Retain:

```sql
SELECT *
FROM system.function_runs;
```

as a virtual in-memory view.

However, its implementation SHALL read from:

```text
ActiveExecutionRegistry
```

and enrich IDs from:

```text
procedure catalog
module catalog
transaction coordinator
```

only when the view is queried.

Normal function execution should not allocate all display strings.

---

# 79. Suggested Active Execution Fields

Core registry fields should remain minimal.

The view may expose:

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

topic
partition
offset

current_operation
started_at
age_ms

node_id
```

Not every field must physically exist inside `ExecutionControl`.

For example:

```text
procedure_id → procedure_name
```

is resolved from the catalog when rendering.

---

# 80. Current Operation

Use a compact enum.

Example:

```rust
#[repr(u8)]
enum ExecutionOperation {
    Runtime = 0,
    DbQuery = 1,
    DbWrite = 2,
    TopicPublish = 3,
    FunctionCall = 4,
    Commit = 5,
}
```

Store:

```text
AtomicU8
```

instead of strings.

Convert to:

```text
db_query
db_write
topic_publish
```

only when displaying system views.

---

# 81. No Per-Invocation Persistent Logging

High-frequency functions MUST NOT automatically emit:

```text
FUNCTION START
FUNCTION END
```

logs.

At:

```text
10,000 invocations/sec
```

that would create:

```text
20,000 log messages/sec
```

for no useful purpose.

Instead maintain low-cost metrics:

```text
function_invocations_total
function_errors_total
function_cancellations_total
function_duration_histogram
```

Detailed logs should be produced for:

```text
explicit ctx.log()
errors
slow executions
sampled traces
```

---

# 82. Procedure CALL Fast Path

A procedure call SHOULD bypass DataFusion planning when possible.

For:

```sql
CALL chat.create_message($1, $2);
```

execution becomes:

```text
SQL parse/classify
      │
      ▼
CALL procedure
      │
      ▼
ProcedureHandler
      │
      ▼
Function Runtime
```

Do not generate:

```text
DataFusion LogicalPlan
Optimizer
PhysicalPlan
```

unless the function itself performs a DataFusion-backed query.

---

# 83. CALL Plan Caching

Generated SDKs will repeatedly issue identical calls.

Example:

```sql
CALL chat.create_message($1, $2);
```

KalamDB SHOULD cache resolution:

```text
SQL
 ↓
ProcedureCallPlan

procedure_id
signature_id
argument bindings
return type
```

Subsequent calls:

```text
cache lookup
   ↓
bind arguments
   ↓
invoke procedure
```

KalamDB already has SQL cache/plan-cache infrastructure, so procedure resolution SHOULD reuse or extend the existing cache rather than introducing an unrelated cache.

---

# 84. DataFusion Cancellation Context

When DataFusion is created from an execution, inject execution cancellation information into the DataFusion session extensions.

Current KalamDB already injects user context into DataFusion's session state.

Add conceptually:

```text
SessionUserContext
ExecutionCancellationContext
TransactionQueryContext
```

Thus:

```sql
KILL EXECUTION 100
```

can interrupt:

```text
Function
   ↓
DataFusion query
```

using the same root cancellation state.

---

# 85. Kill Execution

Expose:

```sql
KILL EXECUTION 100;
```

Flow:

```text
KILL EXECUTION
      │
      ▼
ActiveExecutionRegistry
      │
      ▼
ExecutionControl
      │
      ▼
CancellationToken
      │
      ├── Wasmtime
      ├── V8
      ├── DataFusion
      ├── host DB calls
      └── nested functions
```

Then:

```text
rollback owned transaction
```

when safe.

---

# 86. Commit Cancellation Boundary

Once execution enters durable transaction commit:

```text
COMMITTING
```

KalamDB must not claim cancellation if the transaction may already be in the Raft commit path.

The current transaction coordinator moves transaction state to `Committing` before handing staged mutations to the durable apply path.

Therefore:

```sql
KILL EXECUTION 100;
```

may return:

```text
EXECUTION_COMMIT_IN_PROGRESS
```

rather than reporting a false rollback.

---

# 87. Actor Model

KalamDB MAY use an actor framework to implement execution/runtime ownership, but the actor framework SHALL remain an implementation detail.

The semantics SHALL be defined by:

```text
ExecutionContext
TransactionScope
ExecutionControl
ActiveExecutionRegistry
```

rather than by a specific actor library.

---

# 88. Recommended Actor Usage

Use actors primarily for long-lived infrastructure components.

Example:

```text
                    Runtime Supervisor
                           │
        ┌──────────────────┼──────────────────┐
        ▼                  ▼                  ▼
 Topic Dispatcher    Runtime Manager    Trigger Manager
      actor               actor              actor
```

High-frequency root executions may run as ordinary Tokio tasks:

```text
HTTP / Topic
     │
     ▼
Tokio Task
     │
     ▼
ExecutionContext
```

This avoids mailbox/address allocation on every trivial request.

---

# 89. Optional Root Execution Actors

KalamDB MAY optionally implement every tracked root execution as a lightweight actor if benchmarks demonstrate acceptable overhead.

Conceptually:

```rust
struct ExecutionActor {
    context: ExecutionContext,
}
```

Actor identity:

```text
ActorId = ExecutionId
```

This provides:

```text
lifecycle
cancellation
visibility
panic isolation
monitoring
```

without changing transaction semantics.

---

# 90. Actor Framework Selection

No specific framework is mandated by the contract.

Candidates include:

```text
Kameo
Ractor
Actix actors
plain Tokio tasks
```

The implementation SHOULD benchmark at least:

```text
Tokio ExecutionScope

vs

actor-per-root-execution
```

before deciding.

A framework such as Kameo is attractive because it provides actor IDs, supervision and monitoring, but KalamDB must not couple SQL/function semantics to actor-library behavior.

---

# 91. Required Actor Benchmark

Benchmark:

```text
10,000 CALL/sec
50,000 CALL/sec
100,000 CALL/sec
```

with function durations:

```text
1 ms
10 ms
100 ms
```

Compare:

```text
plain Tokio task
actor-per-execution
```

Measure:

```text
p50 latency
p95 latency
p99 latency
allocations/invocation
CPU
RSS
registry churn
actor/mailbox memory
spawn/despawn throughput
cancellation latency
```

Actor-per-execution should only become the default if overhead is acceptably small.

---

# 92. Actor Supervision Rules

Transactional executions MUST NOT automatically restart after a crash.

This is unsafe:

```text
function crashes
     ↓
actor restart
     ↓
run function again automatically
```

because the system may not know whether side effects reached a commit boundary.

For client `CALL`:

```text
failure
  ↓
rollback if possible
  ↓
terminate execution
  ↓
return error
```

For topic triggers:

```text
failure
  ↓
rollback
  ↓
no ACK
  ↓
durable topic retry policy decides retry
```

Actor supervision MAY restart long-lived infrastructure workers.

---

# 93. ExecutionRuntime Abstraction

Keep execution scheduling independent from the actor implementation.

Conceptually:

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

Potential implementations:

```text
TokioExecutionRuntime
KameoExecutionRuntime
```

This lets KalamDB benchmark and change implementations without changing SQL semantics.

---

# 94. Function Runtime Relationship

The execution runtime and language runtime are separate.

```text
              ExecutionRuntime

               /          \
            Tokio        Actor
               │
               ▼
         ExecutionContext
               │
      ┌────────┴─────────┐
      ▼                  ▼
 WasmtimeRuntime       V8Runtime
```

This separation is important.

Actors manage:

```text
lifecycle
ownership
cancellation
```

Language runtimes execute:

```text
Rust/Wasm
TypeScript
```

---

# 95. Automatic Transaction Example

Given:

```sql
CREATE PROCEDURE app.function1()
RETURNS app.result;
```

Implementation:

```typescript
async function function1(ctx) {

    await ctx.db.insert(...);

    await ctx.db.update(...);

    await ctx.functions.function2();

    return {...};
}
```

and:

```typescript
async function function2(ctx) {

    await ctx.db.insert(...);

    await ctx.db.update(...);
}
```

Runtime:

```text
CALL app.function1()
        │
        ▼
Execution EX100
        │
        ▼
ExecutionContext
        │
        ▼
TransactionScope::None
        │
        │ first INSERT
        ▼
TransactionScope::Owned(TX77)
        │
        ├── INSERT
        ├── UPDATE
        │
        └── function2
              │
              ├── INSERT
              └── UPDATE

                success
                   │
                   ▼
               COMMIT TX77
```

If anything fails:

```text
ROLLBACK TX77
```

---

# 96. Database Calls From Functions

Database operations executed through:

```text
ctx.db
```

SHALL automatically inherit:

```text
user identity
namespace
transaction scope
execution cancellation
DataFusion context
```

The function author never manually passes:

```text
transaction_id
execution_id
user_id
```

between operations.

---

# 97. Nested Function API

Generated server SDKs SHOULD allow:

```typescript
await ctx.functions.chat.fanout({
    messageId: message.id
});
```

The SDK internally invokes the procedure with the same:

```text
ExecutionContext
TransactionScope
CancellationToken
```

rather than routing through HTTP or generating another root execution.

---

# 98. Root vs Nested Invocation

These are intentionally different.

External:

```sql
CALL chat.fanout(...);
```

creates:

```text
new root ExecutionContext
```

Internal:

```typescript
ctx.functions.chat.fanout(...)
```

uses:

```text
existing root ExecutionContext
```

This distinction prevents unnecessary actor/task creation and preserves transaction atomicity.

---

# 99. Function Return Does Not Commit Nested Calls

A nested procedure returning successfully does not commit anything.

Example:

```text
function2 returns
    │
    ▼
writes remain staged in TX77
```

Only the root transaction owner commits.

This guarantees:

```text
root success → commit everything
root failure → rollback everything
```

---

# 100. Updated High-Level Runtime

The final runtime architecture becomes:

```text
                       Client / Topic
                              │
                              ▼
                       Root Execution
                              │
                              ▼
                      ExecutionContext
                              │
          ┌───────────────────┼───────────────────┐
          │                   │                   │
          ▼                   ▼                   ▼
    ExecutionControl    TransactionScope      Function Stack
          │                   │                   │
          │                   │                   ▼
          │                   │              nested CALLs
          │                   │
          │          ┌────────┴────────┐
          │          ▼                 ▼
          │       DB Writes        Topic Publish
          │
          ▼
      Cancellation

                       DataFusion
                          │
                       lazy only

                Language Runtime
                 /             \
              Wasm              V8
```

---

# 101. Updated Core Requirements

The feature SHALL follow these requirements:

**The existing KalamDB `ExecutionContext` becomes the unified root runtime context.**

**Do not create a second heavyweight per-function execution context.**

**DataFusion remains lazily initialized inside the root context.**

**A root procedure invocation automatically provides one transaction boundary.**

**Transactions begin lazily on the first mutation.**

**Nested procedure calls reuse the root transaction.**

**Nested procedure calls reuse the root execution rather than spawning new execution actors.**

**A function invoked inside an explicit SQL transaction borrows that transaction rather than committing independently.**

**Any unrecoverable nested transactional error aborts the root transaction in V1.**

**Topic-triggered functions use the same execution and transaction model.**

**Topic ACK occurs only after successful function transaction commit.**

**Transactional topic publishes commit or rollback with database mutations.**

**Only currently running tracked executions are indexed in memory.**

**`system.function_runs` remains a virtual snapshot view.**

**The active execution registry uses compact IDs and enum/atomic state rather than allocated strings.**

**Detailed function start/end logging is not enabled by default.**

**Procedure `CALL` should bypass DataFusion planning where possible.**

**Procedure resolution should be cached.**

**Actors are optional execution infrastructure, not part of the SQL contract.**

**Long-lived runtime subsystems are good actor candidates.**

**Actor-per-root-execution is allowed only after benchmark validation.**

**Plain Tokio and actor-backed implementations should share one `ExecutionRuntime` contract.**

**Actor crashes must not automatically replay client transactional functions.**

**The root execution owns commit or rollback for all nested business logic.**

---

# 102. Scheduled Procedures

Scheduling extends the existing Kalam Functions model without introducing a new application runtime abstraction.

The rule is simple:

```text
Scheduler decides when
        ↓
CALL existing procedure
        ↓
existing ExecutionContext
        ↓
existing transaction/runtime
```

A scheduled operation is therefore still a normal KalamDB procedure defined by SQL:

```sql
CREATE TYPE reports.daily_summary_request AS (
    report_date DATE
);

CREATE PROCEDURE reports.generate_daily_summary(
    request reports.daily_summary_request
)
RETURNS VOID;
```

The scheduler does not own business state and does not create a second function implementation model.

---

# 103. SQL Schedule Syntax

Use a MySQL-familiar `CREATE EVENT` statement because scheduling is a database concern and should remain declarative SQL.

Support three schedule forms:

```text
AT      one-time execution
EVERY   fixed interval
CRON    calendar schedule
```

## One-time execution

```sql
CREATE EVENT billing.expire_campaign
ON SCHEDULE AT TIMESTAMP '2026-10-01 00:00:00Z'
DO CALL billing.expire_campaign('fall-2026');
```

## Fixed interval

```sql
CREATE EVENT sessions.cleanup
ON SCHEDULE EVERY INTERVAL '15 minutes'
DO CALL sessions.remove_expired();
```

## Cron

```sql
CREATE EVENT reports.daily_summary
ON SCHEDULE CRON '0 9 * * *'
TIME ZONE 'Asia/Jerusalem'
DO CALL reports.generate_daily_summary(
    ROW(CURRENT_DATE())::reports.daily_summary_request
);
```

The exact parser representation may differ internally, but the user-facing model SHALL remain SQL-first.

Cron should initially use standard five-field minute-based expressions.

Default time zone:

```text
UTC
```

Named time zones should use IANA identifiers.

---

# 104. Schedule Lifecycle

Scheduled events are schema objects and should support normal DDL lifecycle operations.

```sql
ALTER EVENT reports.daily_summary ENABLE;
ALTER EVENT reports.daily_summary DISABLE;
DROP EVENT reports.daily_summary;
```

Reapplying the same schema must not create duplicate schedules.

`kalam dev` and `kalam deploy` should treat event definitions in the SQL schema the same way they treat procedures, topics and triggers:

```text
parse
validate referenced procedure
apply catalog change
activate schedule
```

A schedule cannot reference an unknown procedure or an incompatible procedure signature.

---

# 105. Schedule Execution Semantics

Every scheduled occurrence creates a normal root execution.

```text
schedule becomes due
        ↓
ProcedureRegistry
        ↓
ExecutionContext
        ↓
Procedure
        ↓
lazy transaction
```

This means scheduled procedures automatically reuse the design already defined in this document:

```text
ExecutionControl
TransactionScope
Function Stack
lazy DataFusion
runtime limits
cancellation
observability
```

A scheduled procedure is not a special function type.

It is the same procedure invoked from another SQL-defined source.

Scheduled execution should expose source metadata such as:

```text
event_name
scheduled_at
started_at
retry_count
```

inside the root execution metadata and `system.function_runs` view where useful.

---

# 106. Schedule Reliability, Retry and Overlap

Scheduled work must survive:

```text
server restart
node failure
cluster failover
function module reload
```

The durable schedule definition is authoritative. In-memory timers are only an optimization.

Recommended event options:

```sql
CREATE EVENT reports.daily_summary
ON SCHEDULE CRON '0 9 * * *'
TIME ZONE 'UTC'
DO CALL reports.generate_daily_summary(...)
WITH (
    retries = 3,
    retry_backoff = '5s',
    overlap = 'skip',
    misfire = 'run_once'
);
```

Recommended `overlap` values:

```text
skip   previous occurrence still running → skip the new occurrence
queue  preserve the occurrence and run it later
allow  permit concurrent occurrences
```

Recommended default:

```text
skip
```

Recommended `misfire` values:

```text
run_once  after recovery run one missed occurrence
skip      ignore occurrences missed while unavailable
catch_up  run missed occurrences up to a configured bound
```

Recommended default:

```text
run_once
```

`catch_up` must always have a maximum bound.

Retries re-invoke the same logical scheduled occurrence. They must not create a new independent schedule occurrence.

---

# 107. Schedule Catalog and Runtime

Add schedule metadata as normal system catalog state.

Suggested table:

```text
system.schedules
  schedule_id
  namespace
  event_name
  schedule_kind       -- at | every | cron
  schedule_expression
  timezone
  procedure_id
  arguments
  enabled
  retries
  retry_backoff
  overlap_policy
  misfire_policy
  next_run_at
  last_run_at
  created_at
  updated_at
```

The scheduler must not create one process, actor, or OS timer for every schedule.

Recommended architecture:

```text
system.schedules
       ↓
query/index upcoming schedules
       ↓
small scheduler service
       ↓
due event
       ↓
ProcedureRegistry
       ↓
normal root execution
```

Use an index ordered by `next_run_at` so the scheduler does not scan every schedule on every tick.

In a cluster, only one owner should dispatch a specific scheduled occurrence. Ownership may use the existing cluster leadership/lease mechanisms.

The procedure execution itself remains normal KalamDB function execution and may run on the appropriate execution node.

---

# 108. Scheduled Procedures and Transactions

Each scheduled procedure invocation gets the same automatic transaction behavior as a direct `CALL`.

Example:

```text
09:00 schedule fires
      ↓
CALL reports.generate_daily_summary(...)
      ↓
BEGIN lazily
      ↓
INSERT summary
UPDATE counters
PUBLISH report.completed
      ↓
COMMIT
```

On failure:

```text
ROLLBACK
   ↓
retry according to schedule policy
```

If the procedure publishes a topic, the publication participates in the same transaction as already defined in §72.

The scheduler must only mark the occurrence successful after the procedure transaction commits.

---

# 109. Topics and Consumers Remain First-Class

Scheduled procedures and topic-triggered procedures do **not** replace KalamDB topics or the consumer APIs.

The existing model remains available:

```text
KalamDB Topic
     ↓
consumer group
     ↓
external service / agent / worker
```

Users must still be able to consume events using the existing topic APIs and SDKs, including explicit consume/ACK semantics.

Conceptually:

```text
@kalamdb/consumer
Rust consumer API
Python consumer API
other external systems
```

remain valid first-class integration paths.

A topic-triggered procedure is only a convenience when the desired handler should execute **inside KalamDB's function runtime**.

An external consumer is preferred when the event should be handled by:

```text
another service
another runtime
an existing worker fleet
an integration platform
an external AI agent
```

The same topic may have both:

```text
internal function trigger consumer group
+
external application consumer group
```

because consumer groups keep offsets independently.

Therefore the model remains:

```text
CALL             synchronous function execution
CREATE EVENT     scheduled function execution
TOPIC TRIGGER    internal event-driven function execution
CONSUME / ACK    external event consumption
```

These are complementary capabilities, not replacements for one another.

---

# 110. Scheduler Design Requirements

The scheduler addition SHALL follow these requirements:

**Scheduling is defined through SQL.**

**Scheduled work always targets an existing `CREATE PROCEDURE` contract.**

**`CREATE TYPE`, table row types, typed procedure parameters and typed return values remain unchanged.**

**`CREATE EVENT ... ON SCHEDULE ... DO CALL ...` is the primary scheduling abstraction.**

**Support one-time `AT`, interval `EVERY`, and `CRON` schedules.**

**A scheduled occurrence enters the same `ExecutionContext`, transaction and runtime used by direct and topic-triggered procedure calls.**

**Do not create another function runtime specifically for scheduled work.**

**Do not make process-local timers authoritative.**

**Schedule metadata is durable catalog state.**

**Schedule dispatch must work after restart and cluster failover.**

**The scheduler should query/index upcoming work rather than allocating one runtime object per scheduled event.**

**Retries occur only after failed procedure execution and follow explicit schedule policy.**

**Overlap and misfire behavior are explicit and deterministic.**

**Topics, consumer groups, `CONSUME`, ACK and external consumer SDKs remain first-class and are not replaced by scheduled functions or topic triggers.**

**KalamDB remains a SQL database with server functions; scheduling only adds another SQL-defined way to invoke those functions.**
