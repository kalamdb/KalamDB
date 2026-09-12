# REST: POST /v1/functions/{namespace}/{procedure}

Existing route. Behavior changes only.

## Success

HTTP status: 200, or the status set by the **root** procedure (`ctx.http.response.status`).

Body: the SQL return value as JSON. **No** envelope:

```json
{ "order_id": "123" }
```

Not:

```json
{ "status": "success", "result": { "order_id": "123" } }
```

## Error

Consistent Kalam error envelope (keep `status: "error"` plus stable `code`). Map via typed errors ([typed-errors.md](typed-errors.md)), never `message.contains`.

## Request

- JSON body binds to procedure parameters (existing `bind_json_args`).
- Reject client-supplied `context` / `ctx` / `source` / `actor` / `tx` keys (keep).
- Do **not** copy every header to `HashMap<String,String>` up front.
- Auth: existing session extractor. Unauthenticated → 401 unless anonymous EXECUTE granted.

## Headers exposed to JS

On demand only. Blocked by default: `Authorization`, `Proxy-Authorization`, raw `Cookie`. Allowed examples: `x-client-version`, `accept-language`, `x-request-id`, `stripe-signature`.

## Response headers from JS

Root only. Reject `Connection`, `Transfer-Encoding`, `Content-Length`, `Host`. Enforce count/name/value/total byte limits.
