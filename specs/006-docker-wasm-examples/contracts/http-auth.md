# API Contract: HTTP Authentication

**Feature**: 006-docker-wasm-examples  
**Component**: kalamdb-api  
**Purpose**: Define X-API-KEY header authentication for SQL API

---

## Authentication Endpoint

### SQL Query Execution

**Endpoint**: `POST /sql`

**Headers**:
```http
Content-Type: application/json
X-API-KEY: <user_api_key>
```

**Request Body**:
```json
{
  "query": "SELECT * FROM todos WHERE completed = false"
}
```

**Success Response** (200 OK):
```json
{
  "columns": ["id", "title", "completed", "created_at"],
  "rows": [
    [1, "Buy groceries", false, "2025-10-25T10:00:00Z"],
    [2, "Write documentation", false, "2025-10-25T11:30:00Z"]
  ],
  "row_count": 2
}
```

**Error Response** (401 Unauthorized):
```json
{
  "error": "Unauthorized",
  "message": "Invalid or missing API key"
}
```

**Error Response** (400 Bad Request):
```json
{
  "error": "BadRequest",
  "message": "Invalid SQL syntax"
}
```

---

## Authentication Flow

1. Client sends SQL query with `X-API-KEY` header
2. Middleware extracts header value
3. Middleware queries user table: `SELECT * FROM users WHERE apikey = ?`
4. If not found → 401 Unauthorized
5. If found → Inject User context into request
6. Route handler executes query in user's namespace
7. Return results

---

## Localhost Exception

**Special Case**: Requests from 127.0.0.1 (localhost)

- If connection originates from localhost (127.0.0.1 or ::1)
- AND X-API-KEY header is missing
- THEN skip authentication (for kalam-cli local development)

**Security Note**: This exception is only for development. Production deployments should disable localhost bypass or use network isolation.

---

## API Key Format

**Valid Formats**:
- UUID v4: `550e8400-e29b-41d4-a716-446655440000`
- Hex string (32 chars): `a1b2c3d4e5f6g7h8i9j0k1l2m3n4o5p6`

**Invalid Examples**:
- Too short: `abc123`
- Invalid characters: `my-api-key!`
- Empty string: `""`

---

## Rate Limiting

**Not Implemented** in this feature (future enhancement)

Suggested limits for future:
- 1000 requests/minute per API key
- 10,000 requests/hour per API key

---

## Example Usage

### cURL

```bash
# With API key
curl -X POST http://localhost:2900/sql \
  -H "Content-Type: application/json" \
  -H "X-API-KEY: 550e8400-e29b-41d4-a716-446655440000" \
  -d '{"query": "SELECT * FROM todos"}'

# Localhost (no API key needed)
curl -X POST http://127.0.0.1:2900/sql \
  -H "Content-Type: application/json" \
  -d '{"query": "SELECT * FROM todos"}'
```

### JavaScript/TypeScript

```typescript
const response = await fetch('http://localhost:2900/sql', {
  method: 'POST',
  headers: {
    'Content-Type': 'application/json',
    'X-API-KEY': 'your-api-key-here'
  },
  body: JSON.stringify({
    query: 'SELECT * FROM todos WHERE completed = false'
  })
});

const data = await response.json();
```

---

## Security Considerations

1. **Transport Security**: Use HTTPS in production (API key transmitted in header)
2. **Key Storage**: Never commit API keys to version control
3. **Key Rotation**: Manual rotation for now (future: automated rotation)
4. **Localhost Bypass**: Disable in production or use Docker network isolation
5. **Logging**: Do NOT log API key values (log `X-API-KEY: ***` instead)
