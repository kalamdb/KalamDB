# KalamDB Transactional Table Triggers

> Normative companion to [`functions.md`](functions.md) and [`functions-v1-implementation.md`](functions-v1-implementation.md).

**Status:** Proposed for V1  
**Scope:** PostgreSQL-like `AFTER ... FOR EACH ROW` triggers that execute existing Kalam procedures in the same root transaction.

---

## 1. Goal

KalamDB already supports durable table-change routing into topics. Table triggers are a different feature.

A table trigger executes database logic immediately because a row changed:

```text
INSERT / UPDATE / DELETE
        ↓
row mutation becomes visible inside the transaction
        ↓
matching AFTER ROW trigger(s)
        ↓
existing Kalam procedure(s)
        ↓
COMMIT
```

If a trigger procedure fails, the root mutation and every trigger-side mutation roll back together.

This is intentionally different from a durable topic route:

```text
row mutation
    ↓
commit/persist durable topic event
    ↓
consumer or topic-trigger processing later
```

Use table triggers for synchronous transactional invariants and side effects. Use topics for replayable, decoupled, asynchronous work.

---

## 2. SQL syntax

Follow PostgreSQL's `CREATE TRIGGER` shape where practical.

V1 syntax:

```sql
CREATE TRIGGER app.users_after_change
AFTER INSERT OR UPDATE OR DELETE
ON app.users
FOR EACH ROW
EXECUTE PROCEDURE app.on_user_changed(OPERATION, OLD, NEW);
```

Single-event examples:

```sql
CREATE TRIGGER app.users_after_insert
AFTER INSERT
ON app.users
FOR EACH ROW
EXECUTE PROCEDURE app.on_user_inserted(NEW);
```

```sql
CREATE TRIGGER app.users_after_update
AFTER UPDATE
ON app.users
FOR EACH ROW
EXECUTE PROCEDURE app.on_user_updated(OLD, NEW);
```

```sql
CREATE TRIGGER app.users_after_delete
AFTER DELETE
ON app.users
FOR EACH ROW
EXECUTE PROCEDURE app.on_user_deleted(OLD);
```

Multiple events are joined with `OR`:

```sql
AFTER INSERT OR UPDATE OR DELETE
```

V1 supports only:

```text
AFTER
INSERT
UPDATE
DELETE
FOR EACH ROW
EXECUTE PROCEDURE
```

Deferred to later:

```text
BEFORE
INSTEAD OF
FOR EACH STATEMENT
UPDATE OF column_name
WHEN (...)
transition tables
constraint/deferred triggers
trigger bodies written directly in SQL
```

KalamDB deliberately does not add a MySQL-style `BEGIN ... END` trigger language. Trigger behavior belongs in an existing Kalam procedure so there is only one server-side application runtime.

---

## 3. Procedure binding

A table trigger invokes an ordinary procedure.

Example:

```sql
CREATE PROCEDURE app.on_user_changed(
    operation TEXT NOT NULL,
    old_row app.users,
    new_row app.users
)
RETURNS VOID;
```

Then:

```sql
CREATE TRIGGER app.users_after_change
AFTER INSERT OR UPDATE OR DELETE
ON app.users
FOR EACH ROW
EXECUTE PROCEDURE app.on_user_changed(OPERATION, OLD, NEW);
```

The trigger does not create a special second trigger-function type system.

The same procedure contract, generated SDK model, Arrow type resolver, runtime ABI, and transaction machinery used by normal `CALL` are reused by the trigger.

---

## 4. Built-in row-trigger bindings

V1 supports these trigger-only binding expressions inside `EXECUTE PROCEDURE (...)`:

```text
OLD
NEW
OPERATION
```

### `OLD`

The row before the operation.

```text
INSERT -> NULL
UPDATE -> previous row
DELETE -> deleted row
```

Its logical type is the triggering table's implicit PostgreSQL-style row type.

For:

```sql
ON app.users
```

`OLD` has type:

```text
app.users nullable
```

### `NEW`

The row after the operation.

```text
INSERT -> inserted row
UPDATE -> resulting row
DELETE -> NULL
```

Its logical type is also the triggering table row type.

### `OPERATION`

A non-null textual operation value:

```text
INSERT
UPDATE
DELETE
```

V1 may internally represent this as a compact enum/tag, but the SQL binding type is `TEXT NOT NULL` unless/until KalamDB introduces a built-in trigger-operation enum.

---

## 5. Compile-time validation

The contract compiler validates trigger bindings against the target procedure signature.

Example:

```sql
CREATE PROCEDURE app.on_user_updated(
    old_row app.users NOT NULL,
    new_row app.users NOT NULL
)
RETURNS VOID;

CREATE TRIGGER app.users_update
AFTER UPDATE
ON app.users
FOR EACH ROW
EXECUTE PROCEDURE app.on_user_updated(OLD, NEW);
```

This is valid because both `OLD` and `NEW` are guaranteed for `UPDATE`.

This should be rejected:

```sql
CREATE TRIGGER app.users_change
AFTER INSERT OR UPDATE OR DELETE
ON app.users
FOR EACH ROW
EXECUTE PROCEDURE app.on_user_updated(OLD, NEW);
```

if either procedure parameter is `NOT NULL`, because `OLD` is null for INSERT and `NEW` is null for DELETE.

The compiler knows the selected event set and can prove binding nullability without waiting for runtime.

Other validation rules:

- target table must exist;
- target procedure must exist;
- `OLD`/`NEW` row types must match the triggering table row type or a compatible alias;
- `OPERATION` must bind to a compatible scalar parameter;
- trigger names are unique per table;
- duplicate event specifications are rejected;
- trigger dependency edges are recorded in `ContractSnapshot`;
- `DROP TABLE ... RESTRICT` fails while triggers depend on the table;
- `DROP PROCEDURE ... RESTRICT` fails while triggers invoke the procedure;
- `CASCADE` follows the normal dependency graph.

---

## 6. Execution semantics

`AFTER` means:

1. the row mutation has passed normal validation/constraints for that mutation;
2. the new row state is visible to SQL executed by the trigger procedure inside the same transaction;
3. the transaction has **not committed yet**;
4. the trigger procedure executes;
5. if all trigger procedures succeed, normal commit continues;
6. if any trigger procedure fails, the complete root transaction rolls back.

Conceptually:

```text
BEGIN
  UPDATE app.users ...
      ↓
  row staged/applied in root transaction
      ↓
  AFTER ROW triggers execute
      ↓
  trigger procedure SQL mutations
      ↓
COMMIT
```

Failure:

```text
UPDATE
  ↓
AFTER trigger
  ↓
procedure error
  ↓
ROLLBACK original UPDATE + trigger writes
```

There is no independent retry for an `AFTER` table trigger. It is part of the SQL transaction.

Durable topic-trigger retries remain a separate asynchronous feature.

---

## 7. One root `ExecutionContext`

A table trigger reuses the existing root `ExecutionContext`.

Do not create:

```text
new HTTP context
new DataFusion root session
new transaction
new actor
new function execution service
```

for each trigger.

Reuse:

```text
root ExecutionContext
├── authenticated principal
├── actor
├── DataFusion/session state
├── transaction
├── cancellation
├── pinned contract/module revision
├── procedure call stack
└── trigger metadata
```

The trigger procedure enters the same nested-procedure execution path used by `ctx.functions`.

This keeps trigger calls cheap and avoids serialization for `OLD`/`NEW` while execution remains in-process.

---

## 8. Invocation source metadata

Extend the procedure invocation source model with a table-trigger source.

Conceptually:

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
    }
  | {
      kind: "table";
      triggerName: string;
      tableName: string;
      operation: "INSERT" | "UPDATE" | "DELETE";
    };
```

`OLD`, `NEW`, and `OPERATION` remain typed procedure arguments.

Source metadata is host-created audit/execution information and cannot be supplied or forged by a normal client `CALL`.

---

## 9. Multi-row statements

`FOR EACH ROW` means exactly one invocation per affected row.

Example:

```sql
UPDATE app.users
SET active = false
WHERE last_login < '2025-01-01';
```

If 1,000 rows are updated:

```text
1,000 row mutations
1,000 AFTER ROW trigger invocations per matching trigger
```

All invocations belong to the same root statement/transaction unless the outer caller explicitly established a different transaction boundary.

This follows PostgreSQL row-trigger semantics and must be reflected in cost/performance expectations.

---

## 10. Trigger ordering

Multiple matching triggers on the same table must execute deterministically.

V1 rule:

> For the same timing/event/table, execute triggers in ascending trigger-name order.

This mirrors PostgreSQL's deterministic name ordering for same-kind triggers and avoids storing a separate mutable priority mechanism in V1.

Example:

```text
a_audit
b_update_stats
c_publish_internal
```

execute in that order.

A later version may introduce explicit ordering only if there is a concrete need.

---

## 11. Recursive triggers

Trigger procedures may write tables that themselves have triggers.

This is allowed, but it uses the same root execution stack and recursion/depth protection as nested procedures.

Example:

```text
UPDATE users
  ↓ users_after_update
  ↓ procedure inserts audit row
  ↓ audit_after_insert
  ↓ another procedure
```

V1 rules:

- use the existing root procedure/trigger depth counter;
- cap total nested procedure/trigger depth at the existing V1 execution cap (16 unless changed globally);
- exceeding the cap aborts the transaction with a stable recursion-depth error;
- do not build a separate trigger recursion engine.

Recommended error shape:

```text
ERROR: maximum trigger/procedure nesting depth exceeded
DETAIL: execution depth 17 exceeds the configured maximum of 16.
```

---

## 12. Transaction-group limitation

The existing V1 rule still applies: KalamDB does not introduce distributed transactions across unrelated Raft data groups merely because a trigger exists.

If a trigger procedure attempts a write that would require an unsupported cross-group transaction, return the normal transaction-scope error and roll back the root mutation.

Table triggers must reuse existing transaction routing; they do not bypass it.

---

## 13. Interaction with current topic sources

A table can have both:

```text
AFTER ROW table triggers
and
durable topic source routes
```

They serve different purposes and must remain separate catalog objects.

Example:

```sql
CREATE TRIGGER app.users_audit
AFTER UPDATE
ON app.users
FOR EACH ROW
EXECUTE PROCEDURE app.audit_user_change(OLD, NEW);
```

and independently:

```sql
ALTER TOPIC app.user_changes
ADD SOURCE app.users
ON UPDATE
WITH (payload = 'full');
```

Do not implement topic sources by internally creating hidden SQL triggers.

Do not implement table triggers by internally publishing temporary topics.

The direct transactional path should remain direct:

```text
row mutation -> procedure
```

The durable asynchronous path should remain:

```text
row mutation -> topic log -> consumer/topic trigger
```

This avoids unnecessary serialization, RocksDB writes, offsets, ACK state, and retries for synchronous table-trigger work.

---

## 14. Commit visibility and topic publication

A failed table trigger must not leave behind a durable topic event claiming that the original row mutation succeeded.

Required semantic invariant:

```text
if root transaction rolls back
    => no committed topic event for that rolled-back mutation
```

Therefore the transaction/publisher integration must ensure topic source publication observes the same commit outcome as the originating row mutation.

If the existing topic publisher path cannot yet guarantee this for transactional writes, implementation of table triggers must include the required transaction integration rather than weakening trigger atomicity.

---

## 15. Catalog model

Table triggers should be represented in the canonical contract/catalog, not as parser-only objects.

Conceptually:

```text
system.triggers
  trigger_id
  schema_name
  trigger_name
  source_kind       = table | topic
  table_id          nullable
  topic_id          nullable
  timing            = after
  events            bitset/list: insert/update/delete
  granularity       = row
  procedure_id
  bindings
  enabled
```

The existing trigger catalog used for topic triggers should be generalized rather than creating unrelated `system.table_triggers` and `system.topic_triggers` implementations if one clean model can represent both.

The behavior remains source-specific, but lifecycle/dependency/introspection machinery is shared.

---

## 16. PostgreSQL-like introspection

Where practical, expose table trigger metadata through PostgreSQL-compatible catalog/information-schema projections used by JDBC/Studio tooling.

At minimum KalamDB's own system catalog must expose:

```text
trigger name
source table
AFTER timing
event set
ROW granularity
target procedure
enabled state
```

This should be derived from the same canonical trigger record used by execution.

---

## 17. DDL lifecycle

Support:

```sql
CREATE TRIGGER ...;
CREATE OR REPLACE TRIGGER ...;
DROP TRIGGER trigger_name ON table_name;
DROP TRIGGER IF EXISTS trigger_name ON table_name;
```

`CREATE OR REPLACE` follows the current PostgreSQL shape and replaces the trigger definition while preserving dependency validation.

V1 may omit `ALTER TRIGGER` unless needed for rename/enable operations; a drop/create cycle is acceptable initially.

---

## 18. Errors

Prefer PostgreSQL-like messages.

Missing trigger procedure:

```text
ERROR: procedure app.on_user_changed does not exist
```

Wrong binding type:

```text
ERROR: trigger argument NEW is incompatible with procedure parameter new_row
DETAIL: NEW has type app.users but parameter new_row expects app.account.
```

Impossible non-null binding:

```text
ERROR: trigger argument OLD may be NULL
DETAIL: trigger app.users_change includes INSERT, where OLD is not available.
HINT: make the procedure parameter nullable or remove INSERT from the trigger events.
```

Duplicate trigger:

```text
ERROR: trigger "users_after_change" for relation "app.users" already exists
```

---

## 19. Implementation ownership

Reuse existing crates and avoid a new trigger runtime crate.

Recommended ownership:

```text
kalamdb-dialect
  CREATE/DROP TRIGGER parser + typed AST

ContractSnapshot / commons contract models
  canonical trigger definition + dependency edges

kalamdb-system
  persisted trigger catalog/provider + introspection

kalamdb-core / table mutation orchestration
  detect matching AFTER ROW triggers and invoke them

ExecutionContext / functions runtime
  execute target procedure in the existing root transaction

kalamdb-publisher
  unchanged direct durable topic-source routing
```

Do not put procedure execution logic into `kalamdb-publisher`.

---

## 20. Implementation tasks

### Task 1 - Dialect

Add typed AST support for:

```sql
CREATE [OR REPLACE] TRIGGER name
AFTER event [OR event ...]
ON table
FOR EACH ROW
EXECUTE PROCEDURE procedure_name(binding [, ...]);
```

and:

```sql
DROP TRIGGER [IF EXISTS] name ON table;
```

Reject unsupported `BEFORE`, `STATEMENT`, `WHEN`, and other future clauses with stable diagnostics rather than silently accepting them.

### Task 2 - Contract compiler

Resolve:

```text
trigger name
table ID
event set
procedure ID
OLD/NEW/OPERATION bindings
binding types/nullability
dependency edges
```

into `ContractSnapshot`.

### Task 3 - Catalog

Persist and introspect table-trigger definitions using the generalized trigger catalog.

### Task 4 - Mutation integration

For each inserted/updated/deleted row:

1. stage/apply row mutation in the root transaction;
2. resolve matching AFTER ROW triggers from a precompiled/in-memory index;
3. execute triggers in deterministic trigger-name order;
4. bind `OLD`, `NEW`, `OPERATION` without serialization;
5. invoke target procedures through the existing nested execution path;
6. propagate any error to the root transaction;
7. continue commit only after all triggers succeed.

Do not query the system trigger table for every changed row. Maintain a compiled/cacheable lookup such as:

```text
(table_id, operation) -> ordered trigger definitions
```

### Task 5 - Transaction/topic consistency

Verify that rollback caused by a trigger also prevents durable topic publication for the rolled-back row mutation.

### Task 6 - Tests

Cover:

```text
AFTER INSERT
AFTER UPDATE
AFTER DELETE
multi-event trigger
OLD/NEW/OPERATION values
multi-row UPDATE fires once per row
multiple trigger deterministic ordering
trigger procedure mutation commits atomically
trigger failure rolls back root mutation
nested triggers
recursion limit
unsupported BEFORE syntax
binding nullability compiler errors
DROP ... RESTRICT dependency behavior
root rollback leaves no durable topic event
```

---

## 21. V1 acceptance examples

### Audit update

```sql
CREATE PROCEDURE audit.user_updated(
    old_row app.users NOT NULL,
    new_row app.users NOT NULL
)
RETURNS VOID;

CREATE TRIGGER app.users_audit_update
AFTER UPDATE
ON app.users
FOR EACH ROW
EXECUTE PROCEDURE audit.user_updated(OLD, NEW);
```

### All mutations

```sql
CREATE PROCEDURE audit.user_changed(
    operation TEXT NOT NULL,
    old_row app.users,
    new_row app.users
)
RETURNS VOID;

CREATE TRIGGER app.users_audit_change
AFTER INSERT OR UPDATE OR DELETE
ON app.users
FOR EACH ROW
EXECUTE PROCEDURE audit.user_changed(OPERATION, OLD, NEW);
```

### Side-effecting transactional procedure

```sql
CREATE PROCEDURE app.order_inserted(
    new_order app.orders NOT NULL
)
RETURNS VOID;

CREATE TRIGGER app.orders_after_insert
AFTER INSERT
ON app.orders
FOR EACH ROW
EXECUTE PROCEDURE app.order_inserted(NEW);
```

If `app.order_inserted` inserts into another supported table and later fails, both the inserted order and trigger-side writes roll back together.

---

## 22. Final architecture

```text
                         TABLE MUTATION
                              |
                 +------------+-------------+
                 |                          |
                 v                          v
          AFTER ROW TRIGGER          TOPIC SOURCE ROUTE
                 |                          |
                 v                          v
        existing Kalam procedure       durable TopicMessage
                 |                          |
          same transaction          replay / consume / ack
                 |                          |
          success or rollback       optional topic trigger
```

The core rule is:

> **A KalamDB table trigger is a lightweight transactional binding from a row mutation to an existing typed procedure. It is not a second programming language, not a hidden topic, and not a second execution runtime.**
