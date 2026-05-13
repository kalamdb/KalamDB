# Feature Specification: Docker Container, WASM Compilation, and TypeScript Examples

**Feature Branch**: `006-docker-wasm-examples`  
**Created**: 2025-10-25  
**Status**: Draft  
**Input**: User description: "Create Docker container with config override, compile kalam-link to WASM, and build TypeScript TODO app example with CLI setup"

## Clarifications

### Session 2025-10-25

- Q: What is the exact schema definition for the TODO table? → A: `id (auto-increment), title (text), completed (boolean), created_at (timestamp)`
- Q: How should the app handle sync conflicts between localStorage and KalamDB? → A: KalamDB is always source of truth. Subscription protocol returns insert/update/delete changes. App cannot add new TODOs unless WebSocket is connected to KalamDB server. LocalStorage serves only as read cache for fast initial display.
- Q: What should happen when Docker container starts with missing environment variables? → A: Container starts with documented default values (e.g., port 2900, data dir /data/kalamdb, log level INFO)
- Q: How should the app display WebSocket connection status to users? → A: Status badge with text (e.g., "Connected" / "Disconnected") and color coding, plus disabled button state when disconnected
- Q: How should setup.sh validate KalamDB server accessibility? → A: Use kalam-cli to run a simple query (e.g., query system table). SQL statements are loaded from todo-app.sql file via kalam-cli.
- Q: What parameters does kalam-link WASM client require? → A: KalamDB server URL and user's API key are required parameters for initialization. The WASM library cannot function without these. CLI tools work on localhost without API key for development.
- Q: How should UserId be passed for authentication? → A: kalam-link WASM client accepts UserId as a parameter for basic authentication (future enhancement will add OAuth/JWT support)

## User Scenarios & Testing *(mandatory)*

### User Story 0 - API Key Authentication and Soft Delete (Priority: P0)

Backend developers and API users need a simple authentication mechanism using API keys to access user-specific tables, and deleted rows should be marked as deleted rather than permanently removed to enable data recovery and audit trails.

**Why this priority**: This is a prerequisite for the WASM client and TODO app examples to work properly. API key authentication enables the examples to function without complex auth flows, and soft delete prevents accidental data loss.

**Independent Test**: Can be tested by creating a user with an API key, making SQL API calls with X-API-KEY header to access that user's tables, deleting a row and verifying it's marked deleted (not removed), and running the updated test suite to verify all tests pass with soft delete behavior.

**Acceptance Scenarios**:

1. **Given** a user table exists, **When** a user record is created with an API key, **Then** the API key is stored and associated with that user
2. **Given** a user has an API key, **When** a SQL API request is made with X-API-KEY header containing that key, **Then** the request is authorized and executes against that user's tables
3. **Given** kalam-cli is used on localhost, **When** commands are executed without API key, **Then** the CLI works normally for local development
4. **Given** a user table with records, **When** a DELETE operation is executed on a row, **Then** the row is marked as deleted (soft delete) rather than physically removed
5. **Given** a row marked as deleted, **When** a SELECT query is executed, **Then** the deleted row is not returned in results by default
6. **Given** existing tests for user tables, **When** the test suite runs, **Then** all tests pass with the new soft delete and API key functionality

---

### User Story 1 - Docker Deployment with Configuration (Priority: P1)

DevOps engineers and developers need to deploy KalamDB in containerized environments with custom configurations without rebuilding images. They should be able to run KalamDB server with the CLI tool included, override any configuration using environment variables, and persist data across container restarts.

**Why this priority**: Docker deployment is fundamental for modern production deployments and enables easy local development setup. This is the foundation that other teams will use to run KalamDB.

**Independent Test**: Can be fully tested by running `docker-compose up`, verifying the server starts, using the CLI to create tables, stopping the container, and verifying data persists after restart. Delivers a production-ready deployment method.

**Acceptance Scenarios**:

1. **Given** a Docker host with Docker Compose installed, **When** a user runs `docker-compose up` with the provided configuration, **Then** KalamDB server starts successfully and is accessible on the configured port
2. **Given** a running KalamDB container, **When** a user executes the included CLI tool to create a namespace and table, **Then** the operations complete successfully
3. **Given** environment variables set in docker-compose.yml, **When** the container starts, **Then** configuration values are overridden according to the environment variables
4. **Given** a container with data stored, **When** the container is stopped and restarted, **Then** all data persists via the mounted volume
5. **Given** default docker-compose.yml, **When** a user modifies environment variables (e.g., port, log level), **Then** the server respects the new configuration without image rebuild

---

### User Story 2 - WASM Compilation for Browser/Node.js Use (Priority: P2)

JavaScript/TypeScript developers need to use KalamDB client library in browser applications and Node.js projects without native dependencies. The kalam-link library must be compiled to WebAssembly to enable usage in any JavaScript runtime.

**Why this priority**: Enables web application integration and expands the ecosystem to JavaScript developers. Required before the TypeScript example can be built.

**Independent Test**: Can be tested by compiling kalam-link to WASM, importing it in a minimal Node.js script, establishing a connection to KalamDB server, and executing a simple query. Delivers a usable JavaScript/TypeScript client.

**Acceptance Scenarios**:

1. **Given** the kalam-link Rust crate, **When** compilation to WASM is triggered, **Then** a valid WASM module and TypeScript bindings are generated
2. **Given** a TypeScript project with the WASM module installed, **When** the module is imported, **Then** it loads without errors in Node.js environment
3. **Given** a TypeScript project with the WASM module installed, **When** the module is imported in a browser environment, **Then** it loads without errors
4. **Given** an initialized WASM client, **When** a connection is established to KalamDB server with USER-ID authentication, **Then** the connection succeeds
5. **Given** a connected WASM client, **When** basic operations (insert, query, subscribe) are performed, **Then** they execute correctly

---

### User Story 3 - TypeScript TODO App Example (Priority: P3)

Developers evaluating KalamDB need a complete, working example demonstrating real-time features with a familiar use case. The TODO app example is a React frontend application that shows how to insert, delete, and subscribe to changes using the WASM client, with localStorage persistence for offline capability and automatic syncing when reconnecting.

**Why this priority**: Provides a concrete reference implementation that accelerates developer onboarding. Demonstrates the value proposition of real-time subscriptions combined with offline-first capabilities in a practical context.

**Independent Test**: Can be tested by running setup.sh to create tables, starting the React app, adding TODOs through the UI, deleting TODOs, observing real-time updates across browser tabs, closing/reopening the app to verify localStorage persistence and sync behavior. Delivers a complete working example that developers can clone and extend.

**Acceptance Scenarios**:

1. **Given** a running KalamDB server, **When** setup.sh script is executed using kalam-cli, **Then** all required tables are created using "CREATE IF NOT EXISTS" statements
2. **Given** tables are created, **When** the React TODO app is started in a browser, **Then** it connects to KalamDB successfully with USER-ID authentication
3. **Given** a connected TODO app, **When** a new TODO is inserted via the UI, **Then** the TODO appears in the list immediately and is persisted to both KalamDB and localStorage
4. **Given** a TODO exists in the database, **When** it is deleted via the UI, **Then** the TODO is removed from the list immediately in both KalamDB and localStorage
5. **Given** multiple browser tabs/windows with the TODO app open, **When** a TODO is added or deleted in one tab, **Then** all tabs receive subscription updates and reflect the change in real-time
6. **Given** TODOs stored in localStorage with a highest ID, **When** the app is reopened, **Then** it loads TODOs from localStorage first and subscribes to changes starting from that last ID to sync any new data from KalamDB
7. **Given** the app is offline with TODOs in localStorage, **When** connection is restored, **Then** the app syncs with KalamDB to get any missed updates since the last known ID
8. **Given** the setup.sh script, **When** it is run multiple times, **Then** it completes successfully without errors (idempotent behavior)

---

### Edge Cases

- What happens when API key is invalid or missing in X-API-KEY header? (Return 401 Unauthorized with clear error message)
- How does the system handle queries that try to show deleted rows? (By default exclude deleted rows; optionally support INCLUDE DELETED clause for admin/recovery scenarios)
- What happens when the same API key is used by multiple concurrent requests? (All requests are authorized independently)
- What happens when Docker container starts without required environment variables? (Uses documented defaults: port 2900, data dir /data/kalamdb, log level INFO)
- How does the system handle WASM module loading failures in older browsers? (Display browser compatibility error)
- What happens when subscription connection is lost while TODOs are being modified? (Disable add/delete buttons, show "Disconnected" status badge with red color)
- How does setup.sh handle partial table creation (e.g., some tables exist, others don't)? (CREATE IF NOT EXISTS ensures idempotent behavior)
- What happens when multiple TODO app instances try to delete the same TODO simultaneously? (Subscription broadcasts delete to all instances)
- How does the Docker container behave when the data volume has permission issues? (Fail to start with permission error in logs)
- What happens when localStorage is full or disabled in the browser? (App still works but no caching - slower initial load)
- How does the app handle sync conflicts when localStorage has different data than KalamDB? (KalamDB is source of truth - subscription updates overwrite localStorage)
- What happens when the app reconnects after being offline for an extended period? (Subscribe from last known ID, receive all missed insert/update/delete events)

## Requirements *(mandatory)*

### Functional Requirements

#### API Key Authentication and Soft Delete (Story 0)

- **FR-001**: System MUST add an `apikey` field to the user table schema for storing API keys
- **FR-002**: System MUST accept X-API-KEY header in SQL API requests for authentication
- **FR-003**: System MUST authenticate requests by matching X-API-KEY header value to user's apikey field
- **FR-004**: System MUST execute SQL operations in the context of the authenticated user's tables when valid API key is provided
- **FR-005**: System MUST return 401 Unauthorized response when X-API-KEY header is missing or invalid
- **FR-006**: System MUST allow kalam-cli to function on localhost connections without requiring API key for local development
- **FR-007**: System MUST implement soft delete for user table rows by marking them as deleted instead of physically removing them
- **FR-008**: System MUST add a `deleted` boolean field (or deleted_at timestamp) to user tables to track deletion status
- **FR-009**: System MUST exclude soft-deleted rows from SELECT query results by default
- **FR-010**: System MUST update all existing tests to work with soft delete behavior
- **FR-011**: System MUST ensure DELETE operations mark rows as deleted rather than removing them from storage

#### Docker Container (Story 1)

- **FR-012**: System MUST provide a Dockerfile that builds a working KalamDB server image
- **FR-013**: System MUST include kalam-cli tool in the Docker image and make it accessible via container exec
- **FR-014**: System MUST provide a docker-compose.yml file that defines KalamDB service with volume mounts for data persistence
- **FR-015**: System MUST support overriding any config.toml setting via environment variables in docker-compose.yml, with documented default values used when variables are not provided
- **FR-016**: Docker image MUST expose the KalamDB server port and allow external connections
- **FR-017**: System MUST persist RocksDB data across container restarts via mounted volume
- **FR-018**: Docker documentation MUST include examples of common configuration overrides (port, log level, data directory) and list all default values used when environment variables are not provided

#### WASM Compilation (Story 2)

- **FR-019**: System MUST compile kalam-link Rust crate to WebAssembly using wasm-pack or similar tool
- **FR-020**: System MUST generate TypeScript type definitions for the WASM module
- **FR-020a**: System MUST organize WASM output as a multi-language SDK structure in `link/sdks/`
- **FR-020b**: TypeScript SDK MUST be self-contained at `link/sdks/typescript/` with build.sh, tests, docs, and package.json
- **FR-020c**: Each language SDK MUST be independently buildable and publishable to respective package managers
- **FR-021**: WASM module MUST require KalamDB server URL as initialization parameter
- **FR-022**: WASM module MUST require user's API key as initialization parameter
- **FR-023**: WASM module MUST fail to initialize with clear error message if server URL or API key is missing
- **FR-024**: WASM module MUST support insert operations for table records
- **FR-025**: WASM module MUST support delete operations for table records (soft delete)
- **FR-026**: WASM module MUST support subscribing to table changes and receiving real-time updates with insert/update/delete change types
- **FR-027**: WASM module MUST work in both Node.js and browser environments
- **FR-028**: System MUST provide build instructions for compiling kalam-link to WASM

#### TypeScript TODO Example (Story 3)

- **FR-029**: System MUST provide a complete React frontend application in `/examples/simple-typescript` directory
- **FR-030**: Example MUST include setup.sh script that uses kalam-cli to create required tables
- **FR-031**: Example MUST provide todo-app.sql file containing all SQL statements (CREATE TABLE IF NOT EXISTS, etc.) needed for the app to function
- **FR-032**: Setup script MUST load SQL statements from todo-app.sql file via kalam-cli
- **FR-033**: Setup script MUST validate that KalamDB server is accessible by running a simple query via kalam-cli before attempting table creation
- **FR-034**: Example application MUST initialize kalam-link WASM client with KalamDB server URL and API key
- **FR-035**: Example application MUST demonstrate inserting TODO items using WASM client with persistence to both KalamDB and localStorage
- **FR-036**: Example application MUST demonstrate deleting TODO items using WASM client with updates to both KalamDB and localStorage (soft delete)
- **FR-037**: Example MUST include README with setup instructions including how to obtain/configure API key
- **FR-038**: Example application MUST demonstrate subscribing to TODO changes and updating UI in real-time across multiple browser tabs/windows
- **FR-039**: Example MUST include package.json with all required dependencies including React testing library
- **FR-040**: Application MUST store TODOs in browser localStorage as read-only cache for fast initial display
- **FR-041**: Application MUST track the highest TODO ID in localStorage for sync purposes
- **FR-042**: Application MUST load TODOs from localStorage on startup before fetching from KalamDB
- **FR-043**: Application MUST subscribe to changes starting from the last known ID to sync only new/updated data
- **FR-044**: Application MUST handle reconnection by syncing missed updates since last known ID
- **FR-045**: Application MUST disable add/delete TODO operations when WebSocket connection to KalamDB is not active
- **FR-046**: Application MUST process subscription events (insert/update/delete) and update both UI and localStorage accordingly
- **FR-047**: Application MUST display WebSocket connection status via a visible status badge showing "Connected" or "Disconnected" with color coding (e.g., green for connected, red for disconnected)

### Key Entities

- **User Record**: Entity with `apikey` field for API-based authentication, enabling access to user-specific tables
- **API Key**: Authentication token passed via X-API-KEY header to identify and authorize user requests
- **Soft Delete Marker**: Boolean `deleted` field or `deleted_at` timestamp on user table rows indicating deletion status without physical removal
- **Docker Container**: Encapsulates KalamDB server and CLI tool with configurable runtime environment
- **Volume Mount**: Persistent storage area for RocksDB data that survives container lifecycle
- **Environment Variables**: Key-value pairs in docker-compose.yml that override config.toml settings
- **WASM Module**: Compiled WebAssembly binary of kalam-link with JavaScript/TypeScript bindings, requires KalamDB server URL and API key for initialization
- **Multi-Language SDK Architecture**: Organizational pattern where `link/sdks/{language}/` contains complete, self-contained packages for each target language
- **TypeScript SDK**: Self-contained package at `link/sdks/typescript/` with build.sh, package.json, tests, docs, and compiled WASM artifacts
- **SDK Build System**: Language-specific build scripts (e.g., build.sh) that compile Rust source from `link/` to target SDK directory
- **TODO Item**: Entity in the example app with fields: `id` (auto-increment integer primary key), `title` (text, required), `completed` (boolean, default false), `created_at` (timestamp, auto-generated)
- **Subscription**: Active WebSocket connection listening for changes to TODO table with real-time notification capability. Returns insert/update/delete change types and can filter by ID range for syncing. Required for write operations.
- **Setup Script**: Bash script using kalam-cli to execute SQL statements from todo-app.sql file, creates database schema with idempotent behavior
- **LocalStorage Cache**: Browser-based read-only cache containing TODOs and last synced ID for fast initial display
- **Sync Point**: The highest TODO ID known to the client, used to request only newer data from server
- **SQL Definition File**: todo-app.sql containing CREATE TABLE IF NOT EXISTS and other SQL statements required for the TODO app

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: API key authentication works for 100% of valid keys, rejecting invalid keys with 401 status
- **SC-002**: Soft delete operations complete successfully with deleted rows excluded from default queries
- **SC-003**: All existing tests pass with soft delete behavior enabled
- **SC-004**: Users can start KalamDB in Docker with persistent storage using a single `docker-compose up` command
- **SC-005**: Users can override any configuration setting without modifying Dockerfile or rebuilding image
- **SC-006**: Data persists across container restarts with 100% reliability when using volume mounts
- **SC-007**: WASM module successfully compiles and loads in both Node.js and modern browsers (Chrome, Firefox, Safari latest versions)
- **SC-008**: React TODO app setup completes in under 1 minute from clone to running application
- **SC-009**: Real-time subscription updates appear in TODO app UI within 100 milliseconds of database change across all open browser tabs
- **SC-010**: Setup script completes successfully on repeated runs without errors (idempotent)
- **SC-011**: Example application code is under 500 lines total, demonstrating simplicity and clarity
- **SC-012**: Documentation enables a developer unfamiliar with KalamDB to run all four components in under 30 minutes
- **SC-013**: App loads TODOs from localStorage and displays them within 50 milliseconds on startup
- **SC-014**: App successfully syncs missed updates when reconnecting after being offline, with sync completing in under 2 seconds for up to 1000 TODOs
- **SC-015**: Multiple browser tabs stay synchronized with less than 100 milliseconds latency between operations

## Assumptions

- Docker and Docker Compose are already installed on user's system
- KalamDB server is accessible at a known host/port for the TypeScript example
- Users have Node.js and npm/yarn installed for running TypeScript examples
- wasm-pack or equivalent tooling is available for WASM compilation
- Current KalamDB server already supports CREATE TABLE IF NOT EXISTS syntax
- API keys are generated and managed outside this feature scope (manual creation for now)
- Soft delete is sufficient for user tables; hard delete/purge is out of scope for this feature
- kalam-cli supports loading SQL statements from file
- kalam-cli works on localhost without API key for development/setup purposes
- WASM client will be used for remote/web access and requires API key authentication

## Dependencies

- Existing working KalamDB server with user tables, shared tables, and stream subscriptions
- Existing working kalam-cli tool for database operations
- Rust toolchain with WASM target support (wasm32-unknown-unknown)
- React and React testing library for frontend development
- Node.js and npm/yarn for building and running the React application
- Modern browser with localStorage and WebAssembly support

## Out of Scope

- OAuth/JWT authentication (using API key for now; OAuth/JWT is future enhancement)
- API key generation UI or management endpoints (manual creation acceptable)
- Hard delete/purge functionality for soft-deleted rows
- Audit trail or history tracking for deleted rows beyond soft delete marker
- Multi-container orchestration beyond single docker-compose setup
- Kubernetes deployment configurations
- Advanced WASM optimization or size reduction
- Complex TODO app features (tags, priorities, due dates, categories, search, filtering)
- Backend API layer (example is frontend-only using WASM client directly)
- Mobile app examples
- Performance benchmarking tools
- CI/CD pipeline configuration
- Security hardening and vulnerability scanning
- Load balancing or clustering setup
- Offline write capability (app requires active WebSocket connection for add/delete operations)
- Conflict resolution UI (KalamDB subscription is source of truth)
- IndexedDB or other advanced client storage mechanisms (localStorage is sufficient for read cache)

---

## Known Issues & Planned Fixes

### Critical: PostgreSQL-Compatible Row Count Behavior

**Issue**: UPDATE and DELETE operations currently return "Updated 0 row(s)" and "Deleted 0 row(s)" even when rows are successfully modified.

**Root Cause**: The row count tracking in `backend/crates/kalamdb-core/src/sql/executor.rs` is not properly incrementing counters after successful operations.

**Impact**: 
- User confusion (operations appear to fail when they succeed)
- Integration testing difficulties (cannot verify operation success by row count)
- Non-standard SQL behavior (PostgreSQL returns actual affected row counts)

**Planned Fix (Phase 2.5)**:
1. Research PostgreSQL behavior: Does it return count when values don't actually change (UPDATE with same values)?
2. Fix UPDATE handler to properly increment and return `updated_count`
3. Fix DELETE handler to properly increment and return `deleted_count` for soft deletes
4. Add integration tests verifying correct row counts
5. Document any behavioral differences from PostgreSQL in SQL syntax guide

**Tasks**: T011A through T011H

---

### Enhancement: DELETE with LIKE Pattern Support

**Issue**: DELETE operations with LIKE patterns (e.g., `DELETE FROM table WHERE name LIKE 'test%'`) are not currently supported. Only simple equality conditions work (e.g., `col='value'`).

**Root Cause**: The `parse_simple_where()` function in executor only handles exact equality matching.

**Impact**: Limited DELETE capabilities compared to standard SQL

**Planned Fix (Phase 2.5)**:
- Either: Implement LIKE pattern support in WHERE clause parser
- Or: Document this limitation clearly in error messages and documentation

**Task**: T011H

---

### Project Structure: Move kalam-link to /link

**Rationale**: The `cli/kalam-link/` library serves dual purposes:
1. Linked as a dependency by `kalam-cli` for CLI functionality
2. Compiled to WASM as a standalone library for TypeScript/JavaScript and other languages

**Current Issue**: The current path `cli/kalam-link/` suggests it's only for CLI use, which is misleading.

**Planned Change (Phase 2.5)**:
- Move `cli/kalam-link/` → `/link/kalam-link/`
- Update all import paths in `kalam-cli`
- Update all documentation and build scripts
- Update WASM compilation paths

**Benefits**:
- Clearer separation of concerns
- Better project organization
- Explicit indication that this is a multi-purpose library

**Tasks**: T011I through T011R

---
