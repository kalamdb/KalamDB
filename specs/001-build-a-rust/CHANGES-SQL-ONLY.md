# Specification Changes: SQL-Only API

**Date**: October 14, 2025  
**Branch**: 001-build-a-rust  
**Change Type**: Architecture Simplification

## Summary

Modified the KalamDB specification to use **SQL-ONLY** for all data operations. The `/api/v1/messages` JSON endpoint has been **REMOVED**. All message insertion and querying now uses SQL statements via the `/api/v1/query` endpoint.

## Key Changes

### 1. API Endpoints

**BEFORE**:
- `/api/v1/messages` (POST) - Insert messages with JSON payload
- `/api/v1/query` (POST) - Query messages with QueryParams JSON

**AFTER**:
- `/api/v1/query` (POST) - **ALL operations** using SQL (INSERT, SELECT, UPDATE, DELETE)
- No separate message insertion endpoint

### 2. Message Insertion

**BEFORE**:
```bash
curl -X POST http://localhost:2900/api/v1/messages \
  -H "Content-Type: application/json" \
  -d '{
    "conversation_id": "conv_123",
    "from": "user_alice",
    "content": "Hello, world!",
    "metadata": {"role": "user"}
  }'
```

**AFTER**:
```bash
curl -X POST http://localhost:2900/api/v1/query \
  -H "Content-Type: application/json" \
  -d '{
    "sql": "INSERT INTO messages (conversation_id, from, timestamp, content, metadata) VALUES ('\''conv_123'\'', '\''user_alice'\'', 1699000000000000, '\''Hello, world!'\'', '\''{}'\'')"
  }'
```

### 3. Message Querying

**BEFORE**:
```bash
curl -X POST http://localhost:2900/api/v1/query \
  -H "Content-Type: application/json" \
  -d '{
    "conversation_id": "conv_123",
    "limit": 50
  }'
```

**AFTER**:
```bash
curl -X POST http://localhost:2900/api/v1/query \
  -H "Content-Type: application/json" \
  -d '{
    "sql": "SELECT * FROM messages WHERE conversation_id = '\''conv_123'\'' ORDER BY timestamp DESC LIMIT 50"
  }'
```

## Modified Files

### 1. `spec.md`
- Updated User Story 1 to emphasize SQL INSERT statements
- Updated User Story 2 to emphasize SQL SELECT statements
- Updated FR-001 to specify SQL INSERT via `/api/v1/query`
- Updated FR-013 to specify single SQL query endpoint
- Updated FR-015 to include SQL validation
- Updated FR-017 to expand SQL support details
- Updated FR-022 to mention SQL query scoping

### 2. `contracts/rest-api.yaml`
- **REMOVED** `/api/v1/messages` endpoint entirely
- **REMOVED** `/api/v1/media/upload` endpoint
- Updated `/api/v1/query` description to emphasize "SQL-only"
- Added SQL INSERT example to `/api/v1/query`
- Added `insertedId` field to QueryResponse schema
- **REMOVED** `MessageRequest` schema
- **REMOVED** `MessageResponse` schema
- **REMOVED** `MediaUploadResponse` schema
- Updated tags to remove "Messages" and "Media"

### 3. `tasks.md`
- Updated description to emphasize "SQL-ONLY REST API"
- Updated Phase 3 goal and tests to focus on SQL INSERT
- Updated Phase 4 goal and tests to focus on SQL SELECT
- **NEW** tasks for SQL parser and executor modules:
  - T021: Create SQL parser module
  - T022: Create SQL executor module
- **REMOVED** tasks for JSON message handlers:
  - ~~T021: Create POST /api/v1/messages handler~~
  - ~~T022: Add request validation for JSON messages~~
- Updated file organization to show `sql/` directory structure
- Updated example API usage to show SQL queries
- Updated success metrics to reflect SQL operations
- Updated notes to emphasize "SQL-ONLY API"

## Implementation Impact

### New Components Required

1. **SQL Parser** (`backend/crates/kalamdb-core/src/sql/parser.rs`)
   - Parse INSERT INTO statements
   - Parse SELECT statements with WHERE, ORDER BY, LIMIT
   - Extract table names, columns, values, conditions

2. **SQL Executor** (`backend/crates/kalamdb-core/src/sql/executor.rs`)
   - Execute parsed INSERT statements
   - Execute parsed SELECT statements
   - Generate msgId for inserts
   - Format results as columns/rows

### Removed Components

1. **JSON Message Handler** (`backend/crates/kalamdb-api/src/handlers/messages.rs`)
   - No longer needed
   - All logic moved to SQL executor

2. **QueryParams Struct** (`backend/crates/kalamdb-core/src/storage/query.rs`)
   - Replaced by SQL WHERE clause parsing

## Benefits

1. **Simplified API Surface**: Single endpoint for all operations
2. **More Flexible**: SQL allows complex queries, aggregations, joins
3. **Consistent Interface**: Same pattern for INSERT and SELECT
4. **Easier to Extend**: Adding new SQL features doesn't require new endpoints
5. **Better for Power Users**: Direct SQL access without abstraction layers

## Migration Notes

- Existing code that calls `/api/v1/messages` must be updated to use SQL INSERT
- Existing tests for JSON message insertion must be rewritten for SQL
- Client libraries/SDKs will need to generate SQL statements instead of JSON payloads

## Next Steps

1. Update existing implementation to match new specification
2. Remove `handlers/messages.rs` if already implemented
3. Implement SQL parser and executor modules
4. Update integration tests to use SQL statements
5. Update documentation and examples

## Questions?

If you have questions about these changes, refer to:
- Updated `spec.md` for functional requirements
- Updated `contracts/rest-api.yaml` for API contract
- Updated `tasks.md` for implementation roadmap
