# Kalam Functions HTTP API & Routine Security

> Normative companion to [`functions.md`](functions.md) and [`functions-v1-implementation.md`](functions-v1-implementation.md).

**Status:** Proposed for V1  
**Goal:** Make every Kalam procedure usable as a typed REST endpoint while preserving PostgreSQL-like routine privileges, RLS, and transaction semantics.

---

## 1. One procedure, multiple invocation transports

A procedure is defined once in SQL and implemented once in the Kalam Functions runtime.

It may be invoked through:

```text
SQL / PGWire
CALL api.create_order(...)

HTTP
POST /v1/functions/api/create_order

Nested
ctx.functions.api.createOrder(...)

Table trigger
AFTER INSERT/UPDATE/DELETE -> procedure

Topic trigger
topic message -> procedure

Scheduler
scheduled occurrence -> procedure
```

There is no separate REST controller definition.

The SQL procedure remains the authoritative contract for:

```text
name
parameters
parameter types
return type
security mode
EXECUTE permissions
```

---

## 2. Canonical REST route

Every procedure is automatically addressable through:

```text
POST /v1/functions/{schema}/{procedure}
```

Example:

```text
POST /v1/functions/api/create_order
```

The route resolves directly to:

```sql
CALL api.create_order(...);
```

The HTTP adapter must use the same:

```text
ProcedureRegistry
ContractSnapshot
argument binder
EXECUTE authorization
routine security mode
ExecutionContext
transaction machinery
runtime
return validation
```

as SQL/PGWire invocation.

Do not create a second HTTP function registry.

---

## 3. Request binding

Example SQL contract:

```sql
CREATE TYPE api.create_order_request AS (
    product_id TEXT NOT NULL,
    quantity INT NOT NULL
);

CREATE PROCEDURE api.create_order(
    request api.create_order_request
)
RETURNS api.create_order_result
SECURITY DEFINER;
```

HTTP:

```http
POST /v1/functions/api/create_order
Authorization: Bearer ...
Content-Type: application/json
X-Client-Version: 4.1

{
  "request": {
    "product_id": "p123",
    "quantity": 2
  }
}
```

The HTTP JSON body is only the **edge representation** of SQL arguments.

Internally the request is converted directly into the typed Kalam/DataFusion value model using the procedure signature.

Do not use JSON as the internal procedure object model.

Validation errors should use stable KalamDB/PostgreSQL-style errors for:

```text
missing parameter
unknown parameter
wrong SQL type
missing required struct field
unknown struct field
nullability violation
NONEMPTY violation
payload too large
```

---

## 4. HTTP request context

An HTTP-originated root execution exposes request metadata through `ctx.http`.

Do not expose the raw Actix request type to user code.

Use a stable Kalam-owned API:

```ts
interface ProcedureHttpContext {
  request: {
    method: string;
    path: string;
    headers: ReadonlyHeaders;
    query: ReadonlyQuery;
    cookies: ReadonlyCookies;
  };

  response: {
    status(code: number): void;
    header(name: string, value: string): void;
    contentType(value: string): void;
  };
}
```

Example:

```ts
export default defineProcedure(async (ctx, input) => {
  const version = ctx.http?.request.headers.get("x-client-version");
  const language = ctx.http?.request.headers.get("accept-language");

  // business logic
});
```

`ctx.http` values:

```text
HTTP REST root       -> present
SQL / PGWire root    -> null
scheduler root       -> null
table trigger root   -> null
topic trigger root   -> null
```

Nested procedures originating from an HTTP root may read the inherited request metadata.

Only the root HTTP procedure may mutate HTTP response metadata.

---

## 5. HTTP response contract

The declared SQL `RETURNS` type remains the response body contract.

Do not introduce a second `HttpResponse<T>` SQL type merely for REST.

Example:

```sql
CREATE TYPE api.create_order_result AS (
    order_id TEXT NOT NULL,
    status TEXT NOT NULL
);

CREATE PROCEDURE api.create_order(...)
RETURNS api.create_order_result;
```

Procedure:

```ts
export default defineProcedure(async (ctx, input) => {
  const order = await ctx.db.app.orders.insert(/* ... */);

  ctx.http?.response.status(201);
  ctx.http?.response.header("Location", `/orders/${order.id}`);

  return {
    orderId: order.id,
    status: "created",
  };
});
```

Response:

```http
HTTP/1.1 201 Created
Content-Type: application/json
Location: /orders/o456

{
  "order_id": "o456",
  "status": "created"
}
```

Default edge mapping:

```text
composite / row     -> application/json
SETOF               -> JSON array
JSON / JSONB        -> application/json
TEXT                -> JSON scalar by default, or explicit text content type
BYTES               -> binary content when explicitly selected
VOID                -> successful empty response
```

The procedure may change:

```text
HTTP status
response headers
content type
```

without changing the SQL return contract.

---

## 6. Routine access uses EXECUTE privileges

RLS is not the mechanism for deciding whether a caller may invoke a procedure.

Use PostgreSQL-like routine privileges:

```sql
REVOKE EXECUTE ON PROCEDURE api.create_order FROM PUBLIC;
GRANT EXECUTE ON PROCEDURE api.create_order TO user;
```

Authorization layers remain distinct:

```text
Routine access
  GRANT/REVOKE EXECUTE

Table/column access
  GRANT/REVOKE SELECT/INSERT/UPDATE/DELETE

Row access
  RLS / CREATE POLICY
```

Internally KalamDB may share one authorization engine, but the SQL surface remains PostgreSQL-like.

---

## 7. Recommended default

Internet-facing procedures should not accidentally become public endpoints.

Recommended KalamDB default:

> Newly created application procedures do not grant `EXECUTE` to `PUBLIC` automatically.

A public endpoint must be explicit:

```sql
GRANT EXECUTE ON PROCEDURE api.health TO PUBLIC;
```

A user endpoint:

```sql
REVOKE EXECUTE ON PROCEDURE api.create_order FROM PUBLIC;
GRANT EXECUTE ON PROCEDURE api.create_order TO user;
```

This intentionally favors secure-by-default API exposure even if PostgreSQL deployments may use different default ACL behavior.

---

## 8. SECURITY INVOKER

Default:

```sql
CREATE PROCEDURE api.get_my_orders(...)
RETURNS SETOF app.orders
SECURITY INVOKER;
```

or simply omit the clause:

```sql
CREATE PROCEDURE api.get_my_orders(...)
RETURNS SETOF app.orders;
```

Semantics:

```text
actor               = authenticated caller
execution principal = caller
```

Inside the procedure:

```text
table privileges
column privileges
RLS
CURRENT_USER
```

are evaluated using the caller's effective principal.

Use `SECURITY INVOKER` when the procedure should obey the caller's ordinary DB permissions and RLS.

---

## 9. SECURITY DEFINER

KalamDB V1 supports:

```sql
CREATE PROCEDURE api.create_order(...)
RETURNS api.create_order_result
SECURITY DEFINER;
```

Semantics:

```text
actor               = authenticated caller
execution principal = procedure owner
```

The caller still needs:

```sql
GRANT EXECUTE ON PROCEDURE api.create_order TO user;
```

but does not necessarily need direct table write privileges.

Example:

```sql
REVOKE INSERT ON TABLE app.orders FROM user;

REVOKE EXECUTE ON PROCEDURE api.create_order FROM PUBLIC;
GRANT EXECUTE ON PROCEDURE api.create_order TO user;
```

`api.create_order` can insert into `app.orders` only if the procedure owner's effective permissions allow it.

This makes the procedure a real application/API security boundary.

---

## 10. Actor vs execution principal

Keep these identities separate.

### Actor

Who initiated the root execution.

Examples:

```text
HTTP authenticated user
PGWire user
original actor copied into a topic event
scheduler/service actor where configured
```

Actor is immutable audit metadata.

### Effective execution principal

Whose DB permissions are active for the current procedure frame.

For:

```text
SECURITY INVOKER -> caller frame's principal
SECURITY DEFINER -> callee procedure owner
```

Conceptual context:

```ts
interface ProcedureContext {
  actor: Actor | null;
  principal: Principal;
  // ...
}
```

Authorization and RLS use:

```text
ctx.principal
```

Business/audit logic that needs the original user uses:

```text
ctx.actor
```

Never silently replace actor identity when entering a definer procedure.

---

## 11. Nested procedure security

Routine security is evaluated **per procedure frame**.

Example:

```text
user
 ↓
procedure A SECURITY INVOKER
 principal = user
 ↓
procedure B SECURITY DEFINER
 principal = owner_B
 ↓
procedure C SECURITY INVOKER
 principal = owner_B
 ↓
return B
 principal restored to user
```

Rules:

- nested `SECURITY INVOKER` inherits the effective principal of its caller frame;
- nested `SECURITY DEFINER` switches to the callee owner;
- returning restores the previous frame's effective principal;
- actor remains unchanged through the root execution;
- transaction, request metadata, cancellation, revision, and source remain shared.

Do not create a second transaction or root context during a security transition.

---

## 12. RLS interaction

RLS continues to operate normally against the effective principal.

For invoker:

```text
user -> procedure -> query table
                   -> RLS evaluates as user
```

For definer:

```text
user -> procedure -> query table
                   -> RLS/authorization evaluates using procedure owner's effective principal
```

Any special owner/RLS bypass semantics must be defined explicitly by KalamDB's existing RLS model; `SECURITY DEFINER` itself should not invent an extra bypass flag.

The central rule is:

> Routine security selects the effective principal; the normal authorization/RLS engine then evaluates access for that principal.

---

## 13. End-to-end example

SQL:

```sql
CREATE TYPE api.create_order_request AS (
    product_id TEXT NOT NULL,
    quantity INT NOT NULL
);

CREATE TYPE api.create_order_result AS (
    order_id TEXT NOT NULL,
    status TEXT NOT NULL
);

CREATE PROCEDURE api.create_order(
    request api.create_order_request
)
RETURNS api.create_order_result
SECURITY DEFINER;

REVOKE INSERT ON TABLE app.orders FROM user;
REVOKE EXECUTE ON PROCEDURE api.create_order FROM PUBLIC;
GRANT EXECUTE ON PROCEDURE api.create_order TO user;
```

TypeScript:

```ts
export default defineProcedure(async (ctx, input) => {
  const clientVersion =
    ctx.http?.request.headers.get("x-client-version");

  const order = await ctx.db.app.orders.insert({
    productId: input.request.productId,
    quantity: input.request.quantity,
    createdBy: ctx.actor!.id,
  });

  ctx.http?.response.status(201);
  ctx.http?.response.header("Location", `/orders/${order.id}`);

  return {
    orderId: order.id,
    status: "created",
  };
});
```

HTTP:

```http
POST /v1/functions/api/create_order
Authorization: Bearer ...
X-Client-Version: 4.1
Content-Type: application/json

{
  "request": {
    "product_id": "p123",
    "quantity": 2
  }
}
```

Response:

```http
HTTP/1.1 201 Created
Content-Type: application/json
Location: /orders/o456

{
  "order_id": "o456",
  "status": "created"
}
```

---

## 14. Error behavior

No execute permission:

```text
ERROR: permission denied for procedure api.create_order
```

Unknown procedure:

```text
ERROR: procedure api.create_order does not exist
```

Invalid REST argument:

```text
ERROR: invalid argument for procedure api.create_order
DETAIL: field request.quantity expects INT but received TEXT.
```

Definer owner missing/invalid:

```text
ERROR: cannot execute SECURITY DEFINER procedure api.create_order
DETAIL: procedure owner is not a valid execution principal.
```

HTTP maps stable KalamDB error codes into appropriate 4xx/5xx statuses without losing the underlying SQL-style error identity.

Do not leak internal stack traces, authorization internals, bearer tokens, cookies, or sensitive headers.

---

## 15. Catalog requirements

`system.routines` must include at minimum:

```text
procedure_id
schema_name
procedure_name
owner_id
security_mode       invoker | definer
return_type
active_revision
```

`system.routine_grants` records:

```text
procedure_id
grantee
privilege = EXECUTE
grantor
```

REST route resolution reads the same canonical routine catalog/registry.

Do not persist a parallel `system.http_functions` registry.

---

## 16. Implementation requirements

### Dialect

Support and retain:

```sql
SECURITY INVOKER
SECURITY DEFINER
GRANT EXECUTE ON PROCEDURE ... TO ...
REVOKE EXECUTE ON PROCEDURE ... FROM ...
```

### Contract compiler

Include in canonical hash/diff:

```text
routine owner
security mode
routine ACLs
```

### HTTP adapter

Implement:

```text
POST /v1/functions/{schema}/{procedure}
```

and map request JSON through the existing signature-aware typed binder.

### Execution context

Store immutable root actor/request metadata and frame-local effective principal/security mode.

### Runtime host API

Expose stable `ctx.http` request metadata and root-only response controls.

### Authorization

Order for a direct HTTP/SQL procedure call:

```text
1. authenticate root caller
2. resolve procedure
3. check caller has EXECUTE
4. create root actor/session identity
5. select effective principal from routine security mode
6. invoke procedure
7. all DB operations use normal authorization + RLS for effective principal
8. validate return
9. commit
10. format edge response
```

---

## 17. Required tests

Cover:

```text
POST route resolves schema/procedure
REST scalar arguments
REST composite arguments
REST nested arguments
REST return JSON
HTTP header read
query/cookie read
custom response status
custom response header
custom content type
nested procedure reads inherited request
nested procedure cannot mutate root response
no ctx.http for PGWire/trigger/scheduler
PUBLIC denied by default
GRANT EXECUTE permits call
REVOKE EXECUTE denies call
SECURITY INVOKER obeys caller table grants
SECURITY INVOKER obeys caller RLS
SECURITY DEFINER uses owner privileges
SECURITY DEFINER preserves actor
nested invoker -> definer -> invoker principal transitions
frame principal restored after nested return
transaction rollback restores no partial writes
request credentials never auto-log
```

---

## 18. Final design rule

> **KalamDB procedures are the backend API contract. `POST /v1/functions/{schema}/{procedure}` is only a transport adapter over the same procedure runtime. `EXECUTE` controls who may enter a procedure, `SECURITY INVOKER/DEFINER` controls whose privileges execute it, and the existing table/column/RLS engine controls what that effective principal may access.**
