# Typed function errors

Do not map HTTP status from error **message** substrings.

## Codes

| Code | HTTP | When |
|------|------|------|
| `PROCEDURE_NOT_FOUND` | 404 | Unknown schema/procedure |
| `PROCEDURE_NOT_IMPLEMENTED` | 404 or 501 (pick **404** for missing impl on existing contract) | `ImplementationRef::Missing` |
| `EXECUTE_DENIED` | 403 | ACL fail (authenticated) |
| `AUTHENTICATION_REQUIRED` | 401 | No session and no anonymous grant |
| `INVALID_ARGUMENTS` | 400 | Bind/type errors; client context keys |
| `RESOURCE_LIMIT` | 429 | Capacity, queue, memory budget, host-op limits |
| `PROCEDURE_TIMEOUT` | 504 | Deadline (document 504 as V1 policy) |
| `INTERNAL_RUNTIME_ERROR` | 500 | Isolate crash, unexpected host failure |
| `CONTRACT_MISMATCH` / `ABI_MISMATCH` / `STALE_REVISION` | 409 or 500 | Activation/runtime consistency; 409 if client retried during deploy |

Keep SQLSTATE on the SQL/wire path where Kalam already emits it. REST body:

```json
{
  "status": "error",
  "code": "EXECUTE_DENIED",
  "message": "EXECUTE denied on procedure api.create_order"
}
```

`FunctionsError` and `KalamDbError` must carry the code as a field, not only as English text. REST matches on the field.
