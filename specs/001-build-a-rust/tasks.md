---
description: "Task list for KalamDB - Simple Rust messaging server with RocksDB and SQL query interface"
---

# Tasks: KalamDB - Simple Rust Messaging Server (MVP)

**Input**: Design documents from `/specs/001-build-a-rust/`
**Prerequisites**: plan.md, spec.md, data-model.md, contracts/rest-api.yaml

**Scope**: This is a simplified MVP implementation focusing on:
- Basic Rust server with logging to `/logs/app.log`
- Configuration file (`config.toml`) support
- RocksDB storage for messages table
- **SQL-ONLY REST API**: Single `/api/v1/query` endpoint for ALL operations (INSERT, SELECT, UPDATE, DELETE)
- **NO JSON message endpoints**: All message operations use SQL queries
- Comprehensive tests for all functionality

**Note**: Full features (Parquet consolidation, WebSocket streaming, authentication, Admin UI) are deferred to future iterations.

## Format: `[ID] [P?] [Story] Description`
- **[P]**: Can run in parallel (different files, no dependencies)
- **[Story]**: Which user story this task belongs to (e.g., US1, US2)
- Include exact file paths in descriptions

---

## Phase 1: Setup (Project Foundation)

**Purpose**: Initialize Rust project structure and dependencies

- [x] T001 Create Rust workspace structure in `backend/` with Cargo.toml workspace configuration
- [x] T002 [P] Create `backend/crates/kalamdb-core/` crate for core storage (Cargo.toml with dependencies: rocksdb, serde, thiserror)
- [x] T003 [P] Create `backend/crates/kalamdb-api/` crate for REST API (Cargo.toml with dependencies: actix-web, serde_json, chrono, log)
- [x] T004 [P] Create `backend/crates/kalamdb-server/` binary crate for main server (Cargo.toml with dependencies: tokio, toml, env_logger, log)
- [x] T005 [P] Create `backend/config.example.toml` with example configuration (server port, log level, RocksDB path, message size limits)
- [x] T006 [P] Create `.gitignore` file (target/, logs/, *.db/, config.toml)
- [x] T007 [P] Create `backend/README.md` with build/run instructions

**Checkpoint**: Project structure ready, dependencies defined ✅

---

## Phase 2: Foundational (Core Infrastructure)

**Purpose**: Core infrastructure that MUST be complete before ANY user story can be implemented

**⚠️ CRITICAL**: No user story work can begin until this phase is complete

- [x] T008 Implement configuration module in `backend/crates/kalamdb-server/src/config.rs` (parse TOML, validate settings, struct for ServerConfig)
- [x] T009 [P] Implement logging setup in `backend/crates/kalamdb-server/src/logging.rs` (configure env_logger to write to `/logs/app.log` and console)
- [x] T010 [P] Create Message model in `backend/crates/kalamdb-core/src/models/message.rs` (struct with msg_id, conversation_id, from, timestamp, content, metadata fields, Serialize/Deserialize)
- [x] T011 Implement RocksDB wrapper in `backend/crates/kalamdb-core/src/storage/rocksdb_store.rs` (init DB, open/close, generic put/get operations)
- [x] T012 Create error types in `backend/crates/kalamdb-core/src/error.rs` (StorageError, ConfigError using thiserror)
- [x] T013 Implement Snowflake ID generator in `backend/crates/kalamdb-core/src/ids/snowflake.rs` (time-ordered unique IDs for messages)

**Checkpoint**: Foundation ready - core models, storage, config, logging all functional ✅

---

## Phase 3: User Story 1 - Store Messages with Immediate Confirmation (Priority: P1) 🎯 MVP

**Goal**: Accept SQL INSERT statements via `/api/v1/query` endpoint and store messages in RocksDB with immediate acknowledgment

**Independent Test**: Send POST requests with SQL INSERT statements, verify 200 OK response with rowsAffected and insertedId, confirm messages are persisted in RocksDB

### Tests for User Story 1 (TDD Approach)

**NOTE: Write these tests FIRST, ensure they FAIL before implementation**

- [x] T014 [P] [US1] Unit test for Message model serialization in `backend/crates/kalamdb-core/src/models/message.rs` (test module with serde JSON roundtrip)
- [x] T015 [P] [US1] Unit test for Snowflake ID generator in `backend/crates/kalamdb-core/src/ids/snowflake.rs` (test uniqueness, ordering, correct timestamp encoding)
- [x] T016 [P] [US1] Unit test for RocksDB store operations in `backend/crates/kalamdb-core/src/storage/rocksdb_store.rs` (test put, get, error cases)
- [x] T017 [US1] Integration test for message storage in `backend/tests/integration/test_message_storage.rs` (create message, store via RocksDB, retrieve, verify fields)
- [ ] T018 [US1] API contract test for POST /api/v1/query endpoint with SQL INSERT in `backend/tests/integration/test_api_query.rs` (test INSERT statement, verify insertedId, error cases)

### Implementation for User Story 1

- [x] T019 [US1] Implement MessageStore trait in `backend/crates/kalamdb-core/src/storage/message_store.rs` (insert_message, get_message methods)
- [x] T020 [US1] Implement MessageStore for RocksDB in `backend/crates/kalamdb-core/src/storage/rocksdb_store.rs` (concrete implementation using key format: `msg:{msg_id}`)
- [ ] T021 [US1] Create SQL parser module in `backend/crates/kalamdb-core/src/sql/parser.rs` (parse INSERT INTO statements, extract table name, columns, values)
- [ ] T022 [US1] Create SQL executor module in `backend/crates/kalamdb-core/src/sql/executor.rs` (execute parsed INSERT statements, generate msgId, store message)
- [ ] T023 [US1] Create POST /api/v1/query handler in `backend/crates/kalamdb-api/src/handlers/query.rs` (parse SQL from request body, execute via SQL executor, return JSON response with insertedId)
- [ ] T024 [US1] Add SQL validation in `backend/crates/kalamdb-api/src/handlers/query.rs` (validate SQL syntax, enforce message size limit from config)
- [ ] T025 [US1] Create API routes module in `backend/crates/kalamdb-api/src/routes.rs` (configure Actix routes for /api/v1/query)
- [ ] T026 [US1] Wire up dependencies in `backend/crates/kalamdb-server/src/main.rs` (load config, init logging, create RocksDB store, start Actix server)
- [ ] T027 [US1] Add error handling and logging in `backend/crates/kalamdb-api/src/handlers/query.rs` (log incoming SQL queries, errors, performance metrics)

**Checkpoint**: At this point, User Story 1 should be fully functional - messages can be inserted via SQL INSERT and are persisted in RocksDB ✅

---

## Phase 4: User Story 2 - Query Messages via SQL SELECT (Priority: P2)

**Goal**: Enable querying stored messages via POST /api/v1/query endpoint with SQL SELECT statements and flexible WHERE clauses (by conversation_id, msg_id range, limit)

**Independent Test**: Send POST requests with SQL SELECT queries (conversation_id, since_msg_id, limit), verify correct messages returned in chronological order

### Tests for User Story 2

- [ ] T028 [P] [US2] Unit test for SQL SELECT parsing in `backend/crates/kalamdb-core/src/sql/parser.rs` (test SELECT statement parsing, WHERE clause extraction, ORDER BY, LIMIT)
- [ ] T029 [US2] Integration test for message querying in `backend/tests/integration/test_query_messages.rs` (insert multiple messages via SQL INSERT, query by conversation_id via SQL SELECT, verify results)
- [ ] T030 [US2] API contract test for POST /api/v1/query endpoint with SQL SELECT in `backend/tests/integration/test_api_query.rs` (test various SELECT combinations, WHERE clauses, pagination)

### Implementation for User Story 2

- [ ] T031 [P] [US2] Extend SQL parser in `backend/crates/kalamdb-core/src/sql/parser.rs` to support SELECT statements (parse SELECT columns, FROM table, WHERE conditions, ORDER BY, LIMIT)
- [ ] T032 [US2] Implement query_messages method in `backend/crates/kalamdb-core/src/storage/message_store.rs` (trait method signature accepting parsed WHERE conditions)
- [ ] T033 [US2] Implement query_messages for RocksDB in `backend/crates/kalamdb-core/src/storage/rocksdb_store.rs` (iterate messages, apply WHERE filters, return Vec<Message>)
- [ ] T034 [US2] Extend SQL executor in `backend/crates/kalamdb-core/src/sql/executor.rs` to handle SELECT statements (execute query, format results as columns/rows)
- [ ] T035 [US2] Update POST /api/v1/query handler in `backend/crates/kalamdb-api/src/handlers/query.rs` to support SELECT statements (return JSON with columns, rows, rowCount)
- [ ] T036 [US2] Add pagination support in SQL executor (LIMIT and OFFSET enforcement, response metadata with rowCount)
- [ ] T037 [US2] Add SELECT query logging in `backend/crates/kalamdb-api/src/handlers/query.rs` (log SQL query, execution time, result count)

**Checkpoint**: At this point, both User Stories 1 AND 2 work - messages can be inserted via SQL INSERT and queried via SQL SELECT

---

## Phase 5: Configuration & Operational Readiness (Priority: P2)

**Goal**: Ensure the server is configurable, loggable, and production-ready

**Independent Test**: Start server with custom config, verify settings applied, check logs are written to `/logs/app.log`, graceful shutdown works

### Tests for Configuration & Operations

- [ ] T038 [P] [OPS] Unit test for config parsing in `backend/crates/kalamdb-server/src/config.rs` (test valid/invalid TOML, defaults)
- [ ] T039 [P] [OPS] Unit test for logging setup in `backend/crates/kalamdb-server/src/logging.rs` (verify log file creation, format)
- [ ] T040 [OPS] Integration test for server lifecycle in `backend/tests/integration/test_server_lifecycle.rs` (start server, send request, graceful shutdown, verify logs)

### Implementation for Configuration & Operations

- [ ] T041 [OPS] Enhance config.toml schema in `backend/config.example.toml` (add all supported options: server.host, server.port, storage.rocksdb_path, limits.max_message_size, logging.level, logging.file_path)
- [ ] T042 [OPS] Implement config validation in `backend/crates/kalamdb-server/src/config.rs` (validate port range, file paths, size limits)
- [ ] T043 [OPS] Implement log rotation setup in `backend/crates/kalamdb-server/src/logging.rs` (use env_logger with file appender, configurable levels)
- [ ] T044 [OPS] Add health check endpoint in `backend/crates/kalamdb-api/src/handlers/health.rs` (GET /health returns 200 OK with status JSON)
- [ ] T045 [OPS] Add graceful shutdown handler in `backend/crates/kalamdb-server/src/main.rs` (signal handling, flush RocksDB, close connections)
- [ ] T046 [OPS] Create startup/shutdown logging in `backend/crates/kalamdb-server/src/main.rs` (log server start with config, shutdown events)
- [ ] T047 [OPS] Create build script in `backend/build.sh` (cargo build --release, copy config.example.toml)

**Checkpoint**: Server is production-ready with configuration, logging, health checks, and graceful shutdown

---

## Phase 6: Polish & Documentation

**Purpose**: Finalize the MVP for deployment

- [ ] T048 [P] Update `backend/README.md` with comprehensive build, configuration, and usage instructions
- [ ] T049 [P] Create `backend/docs/API.md` documenting POST /api/v1/query endpoint with SQL INSERT and SELECT examples
- [ ] T050 [P] Add inline code documentation (rustdoc comments) for all public APIs in kalamdb-core and kalamdb-api crates
- [ ] T051 Run cargo clippy on all crates and fix warnings
- [ ] T052 Run cargo fmt on all crates for consistent formatting
- [ ] T053 [P] Create example SQL requests in `backend/examples/` directory (insert_message.sh, query_messages.sh using curl with SQL statements)
- [ ] T054 Verify all tests pass with `cargo test` in workspace root
- [ ] T055 [P] Create `backend/docker/Dockerfile` for containerized deployment (multi-stage build)
- [ ] T056 Create quickstart validation script based on `specs/001-build-a-rust/quickstart.md` (if it exists)

**Checkpoint**: MVP complete, documented, tested, and ready for deployment

---

## Dependencies & Execution Order

### Phase Dependencies

- **Setup (Phase 1)**: No dependencies - can start immediately
- **Foundational (Phase 2)**: Depends on Setup completion - BLOCKS all user stories
- **User Stories (Phase 3+)**: All depend on Foundational phase completion
  - User Story 1 (Phase 3) can start after Phase 2
  - User Story 2 (Phase 4) can start after Phase 2 (independent of US1, but logically after)
- **Configuration & Operations (Phase 5)**: Can run in parallel with User Stories, depends on Phase 2
- **Polish (Phase 6)**: Depends on all previous phases being complete

### Within Each Phase

**Phase 2 (Foundational)**:
- T008 (config) before T026 (main.rs)
- T009 (logging) before T026 (main.rs)
- T010 (Message model) before T019 (MessageStore)
- T011 (RocksDB wrapper) before T020 (MessageStore implementation)
- T012 (error types) before T019, T020
- T013 (Snowflake ID) before T022 (SQL executor)

**Phase 3 (User Story 1 - SQL INSERT)**:
- Tests T014-T018 MUST be written and FAIL before implementation
- T019 (MessageStore trait) before T020 (implementation)
- T020 (MessageStore impl) before T022 (SQL executor)
- T021 (SQL parser) before T022 (SQL executor)
- T022 (SQL executor) before T023 (query handler)
- T023 (query handler) before T025 (routes)
- T025 (routes) before T026 (main.rs wiring)

**Phase 4 (User Story 2 - SQL SELECT)**:
- Tests T028-T030 MUST be written and FAIL before implementation
- T031 (extend SQL parser) before T034 (extend SQL executor)
- T032 (query_messages trait method) before T033 (implementation)
- T033 (query_messages impl) before T034 (SQL executor SELECT support)
- T034 (SQL executor SELECT) before T035 (update query handler)

### Parallel Opportunities

**Phase 1 (Setup)**: ALL tasks (T001-T007) can run in parallel

**Phase 2 (Foundational)**: 
- T009 (logging) || T010 (Message model) || T012 (error types) can run in parallel
- After T010: T013 (Snowflake ID) can run in parallel with T011 (RocksDB wrapper)

**Phase 3 Tests**: 
- T014 || T015 || T016 can run in parallel (unit tests, different modules)
- T017 (integration test) depends on T014-T016
- T018 (API contract test) can run in parallel with T017

**Phase 3 Implementation**:
- T021 (SQL parser) || T019 (MessageStore trait) can run in parallel initially

**Phase 4 Tests**:
- T028 || T029 can run in parallel

**Phase 5 Tests**:
- T038 || T039 can run in parallel

**Phase 6 (Polish)**:
- T048 || T049 || T050 || T053 || T055 can all run in parallel

---

## Parallel Example: Foundational Phase

```bash
# Launch in parallel (different files, no dependencies):
Terminal 1: "Create Message model in backend/crates/kalamdb-core/src/models/message.rs"
Terminal 2: "Create error types in backend/crates/kalamdb-core/src/error.rs"
Terminal 3: "Implement logging setup in backend/crates/kalamdb-server/src/logging.rs"

# Wait for completion, then next batch:
Terminal 1: "Implement RocksDB wrapper in backend/crates/kalamdb-core/src/storage/rocksdb_store.rs"
Terminal 2: "Implement Snowflake ID generator in backend/crates/kalamdb-core/src/ids/snowflake.rs"
```

---

## Implementation Strategy

### MVP First (Core Message Storage + Query)

1. Complete Phase 1: Setup → Project structure ready
2. Complete Phase 2: Foundational (CRITICAL) → Core components ready
3. Complete Phase 3: User Story 1 → Messages can be inserted
4. **VALIDATE**: Test message insertion independently
5. Complete Phase 4: User Story 2 → Messages can be queried
6. **VALIDATE**: Test full insert → query flow
7. Complete Phase 5: Configuration & Operations → Production-ready
8. Complete Phase 6: Polish → Deploy/demo ready

### Testing Strategy (TDD)

- Write ALL tests for a phase FIRST (they should FAIL)
- Implement features until tests PASS
- Run `cargo test` after each implementation task
- Never move to next phase until current phase tests are GREEN

### File Organization

```
backend/
├── Cargo.toml                                    # Workspace config (T001)
├── config.example.toml                           # Example config (T005)
├── README.md                                     # Documentation (T007, T048)
├── build.sh                                      # Build script (T047)
├── crates/
│   ├── kalamdb-core/                            # Core library crate (T002)
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── models/
│   │       │   └── message.rs                   # T010, T014
│   │       ├── storage/
│   │       │   ├── rocksdb_store.rs            # T011, T016, T020, T033
│   │       │   └── message_store.rs            # T019, T032
│   │       ├── sql/
│   │       │   ├── parser.rs                   # T021, T031 (SQL parsing)
│   │       │   └── executor.rs                 # T022, T034 (SQL execution)
│   │       ├── ids/
│   │       │   └── snowflake.rs                # T013, T015
│   │       └── error.rs                        # T012
│   │
│   ├── kalamdb-api/                            # API crate (T003)
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── handlers/
│   │       │   ├── query.rs                    # T023, T024, T027, T035, T037
│   │       │   └── health.rs                   # T044
│   │       └── routes.rs                       # T025
│   │
│   └── kalamdb-server/                         # Server binary (T004)
│       ├── Cargo.toml
│       └── src/
│           ├── main.rs                         # T026, T045, T046
│           ├── config.rs                       # T008, T041, T042
│           └── logging.rs                      # T009, T043
│
├── tests/                                       # Integration tests
│   └── integration/
│       ├── test_message_storage.rs             # T017
│       ├── test_api_query.rs                   # T018, T030
│       ├── test_query_messages.rs              # T029
│       └── test_server_lifecycle.rs            # T040
│
├── docs/
│   └── API.md                                   # T049
│
├── examples/
│   ├── insert_message.sh                       # T053 (SQL INSERT example)
│   └── query_messages.sh                       # T053 (SQL SELECT example)
│
└── docker/
    └── Dockerfile                               # T055
```

---

## Success Metrics

After completing all phases, the following should work:

1. **Configuration**: Server reads `config.toml` and applies settings ✅
2. **Logging**: All operations logged to `/logs/app.log` with configurable levels ✅
3. **Message Insertion**: POST /api/v1/query with SQL INSERT accepts messages and returns immediate confirmation with insertedId ✅
4. **Message Persistence**: Messages survive server restart (RocksDB durability) ✅
5. **Message Querying**: POST /api/v1/query with SQL SELECT returns filtered messages with pagination ✅
6. **Health Check**: GET /health returns server status ✅
7. **Graceful Shutdown**: Server shuts down cleanly without data loss ✅
8. **Test Coverage**: All core functionality covered by unit + integration tests ✅

---

## Notes

- This is a **simplified MVP** focusing on core message storage and querying via SQL
- **SQL-ONLY API**: No JSON message endpoints - all operations use SQL queries
- **Future enhancements** (not in this task list):
  - Parquet consolidation (FR-011, User Story 5)
  - WebSocket real-time subscriptions (User Story 3)
  - JWT authentication (FR-022)
  - Admin web UI (User Story 8)
  - Full SQL engine with DataFusion (User Story 6)
  - Conversation metadata tracking (User Story 7)
- All tasks include exact file paths for clarity
- [P] markers indicate parallelizable tasks (different files)
- TDD approach: tests written first, should FAIL, then implementation makes them PASS
- Commit after each task or logical group of tasks
- Run `cargo test` frequently to catch regressions early

---

## Quick Reference

**Start Development**:
```bash
cd backend
cargo build
cp config.example.toml config.toml
# Edit config.toml with your settings
cargo run
```

**Run Tests**:
```bash
cargo test                    # All tests
cargo test --test test_*      # Integration tests only
cargo test message            # Tests matching "message"
```

**Example API Usage** (after server is running):
```bash
# Insert a message using SQL INSERT
curl -X POST http://localhost:2900/api/v1/query \
  -H "Content-Type: application/json" \
  -d '{
    "sql": "INSERT INTO messages (conversation_id, from, timestamp, content, metadata) VALUES ('\''conv_123'\'', '\''user_alice'\'', 1699000000000000, '\''Hello, world!'\'', '\''{}'\'')"
  }'

# Query messages using SQL SELECT
curl -X POST http://localhost:2900/api/v1/query \
  -H "Content-Type: application/json" \
  -d '{
    "sql": "SELECT * FROM messages WHERE conversation_id = '\''conv_123'\'' ORDER BY timestamp DESC LIMIT 50"
  }'
```
  }'
```
