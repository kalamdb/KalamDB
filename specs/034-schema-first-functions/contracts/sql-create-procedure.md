# SQL: CREATE PROCEDURE (schema-first)

## Project-backed (no body)

```sql
CREATE PROCEDURE api.create_order(
    request api.create_order_request
)
RETURNS api.create_order_result
SECURITY DEFINER;

REVOKE EXECUTE ON PROCEDURE api.create_order FROM PUBLIC;
GRANT EXECUTE ON PROCEDURE api.create_order TO user;
```

- No `AS 'src/...'`.
- No `LANGUAGE` clause.
- Runtime comes from project `[functions]`.

## Inline JavaScript (executable)

```sql
CREATE PROCEDURE api.health()
RETURNS TEXT
LANGUAGE JAVASCRIPT
AS $$
    return "ok";
$$;
```

`LANGUAGE` is required because a body exists. Body uses the same `ctx` / `input` as project methods ([host-api.md](host-api.md)).

CREATE parses the wrapped JavaScript in V8 and rejects host-API misuse (`ctx.log(...)` instead of `ctx.log.info(...)`, `console.log`, calling `ctx.db` as a function). It does not invoke the procedure, so it will not catch logic errors or missing arguments. Success reports `created` vs `replaced` plus the inline artifact id. It does not create a function module revision.

## Inline TypeScript (stored, not executed by server)

```sql
CREATE PROCEDURE api.greeting(name TEXT)
RETURNS TEXT
LANGUAGE TYPESCRIPT
AS $$
    return `Hello ${input.name}`;
$$;
```

V1: catalog stores source. Execution requires a compiled project/inline JS artifact. Direct CALL without compiled JS → clear diagnostic, not a runtime TS parse.

## Parser rules

| Input | Result |
|-------|--------|
| Body present, no LANGUAGE | Error |
| LANGUAGE present, no body | Error |
| `AS 'path', 'export'` | Error (removed mapping) |
| SECURITY omitted | INVOKER |

## Resolution at CALL (pinned active set)

1. Active project export → module implementation  
2. Else compiled inline JS → inline  
3. Else → `PROCEDURE_NOT_IMPLEMENTED`
