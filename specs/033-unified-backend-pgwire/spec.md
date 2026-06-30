# Feature Specification: Unified Backend Sessions, Transactions, and PostgreSQL Wire Access

**Feature Branch**: `033-unified-backend-pgwire`  
**Created**: 2026-06-30  
**Status**: Draft  
**Input**: User description: "Unify KalamDB sessions and transactions into a scalable architecture shared by pgwire, the PostgreSQL extension, and the SQL API; eliminate duplicate and obsolete session/transaction code; add PostgreSQL wire protocol access with unified authentication (username/password resolving to the same identity model used by existing API and bridge access)."

## Clarifications

### Session 2026-06-30

- Q: Should connection session listings include stateless HTTP SQL API traffic? → A: No. The HTTP SQL API is stateless and does not open connection sessions; only long-lived connection entry points (wire protocol and extension bridge) appear in session listings.
- Q: What must admins see for active connection sessions? → A: A single operational view of all open connection-based sessions at a time, with each row showing where the session was opened from (wire protocol vs extension bridge).
- Q: How aggressively should existing working code be refactored? → A: Incrementally—preserve behavior that already works correctly; consolidate duplicate session/transaction tracking so both wire protocol and extension bridge share one path, without large unrelated rewrites.

## User Scenarios & Testing *(mandatory)*

### User Story 1 - One Connection, One Transaction Block (Priority: P1)

A developer expects KalamDB to manage client connections the way PostgreSQL does: each connection maintains its own session state, and at most one explicit transaction block is open on that connection at a time. Beginning, committing, and rolling back a transaction on the connection behaves predictably across the lifetime of the connection.

**Why this priority**: This is the foundation for reliable client behavior. Without a single, Postgres-like session and transaction model, every access path invents its own rules and operators cannot reason about connection state.

**Independent Test**: Open one connection, run `BEGIN`, perform work, run `COMMIT`, then run another `BEGIN`/`COMMIT` cycle on the same connection and verify both cycles behave independently without leaking state from the first block.

**Acceptance Scenarios**:

1. **Given** a connected client with no open transaction block, **When** the client begins an explicit transaction, **Then** the connection enters an in-transaction state and subsequent statements in that block share the same transaction identity until commit or rollback.
2. **Given** a connection with an open transaction block, **When** the client attempts to begin another explicit transaction without first ending the current one, **Then** the system rejects or safely resolves the conflict according to documented connection rules and does not leave ambiguous dual-transaction state.
3. **Given** a connection completes `COMMIT` or `ROLLBACK`, **When** the client runs the next statement, **Then** the connection returns to a normal idle state ready for autocommit statements or a new explicit block.
4. **Given** two concurrent connections from different clients, **When** each opens its own transaction block, **Then** transaction state on one connection never overwrites or merges with the other.

---

### User Story 2 - Connect with Any PostgreSQL Client (Priority: P1)

An operator or application developer wants to connect to KalamDB using standard PostgreSQL client tools and drivers (for example `psql`, JDBC, or ORMs) without installing a custom extension, relying on the same authentication experience they already use for other KalamDB entry points.

**Why this priority**: Wire-protocol access removes friction for adoption, enables existing tooling, and validates that the unified session model works for real client ecosystems—not only for internal bridge protocols.

**Independent Test**: Connect with a standard PostgreSQL client using supported credentials, run `SELECT 1`, then run an explicit `BEGIN` / DML / `COMMIT` workflow and verify results persist as expected.

**Acceptance Scenarios**:

1. **Given** a user with valid credentials, **When** they connect through a PostgreSQL-compatible client, **Then** authentication succeeds using the same identity and authorization rules as the existing HTTP API and bridge access paths.
2. **Given** an authenticated wire-protocol connection, **When** the client executes read and write statements in autocommit mode, **Then** each standalone statement completes with correct success or error feedback without requiring manual transaction control.
3. **Given** an authenticated wire-protocol connection, **When** the client executes `BEGIN`, multiple statements, and `COMMIT`, **Then** all statements in the block succeed or fail atomically at commit time.
4. **Given** invalid credentials, **When** a client attempts wire-protocol login, **Then** authentication fails with a clear error and no session is created.

---

### User Story 3 - SQL API Batch Transactions in One Request (Priority: P2)

A developer sending multiple SQL statements in a single API request needs explicit transaction control (`BEGIN`, DML, `COMMIT` or `ROLLBACK`) within that payload to behave correctly, including support for more than one sequential transaction block in the same request when each block is closed explicitly.

**Why this priority**: The HTTP SQL surface is a first-class entry point. Its transaction semantics must align with the unified transaction model, while remaining stateless with respect to connection sessions.

**Independent Test**: Send one API request containing `BEGIN; INSERT ...; COMMIT; BEGIN; INSERT ...; ROLLBACK;` and verify only the first insert persists.

**Acceptance Scenarios**:

1. **Given** a single API request containing an explicit transaction block that ends with `COMMIT`, **When** the request completes successfully, **Then** all committed changes from that block are durable and visible to other sessions after commit.
2. **Given** a single API request containing an explicit transaction block that ends with `ROLLBACK`, **When** the request completes, **Then** none of the changes from that block remain visible.
3. **Given** a single API request with multiple sequential explicit blocks each closed by `COMMIT` or `ROLLBACK`, **When** the request completes, **Then** each block is applied independently in order.
4. **Given** a single API request that ends while a transaction block is still open, **When** the request finishes, **Then** the open block is automatically rolled back and no partial durable state remains.
5. **Given** traffic arrives only through the stateless HTTP SQL API, **When** an operator inspects connection session listings, **Then** no API request appears as a connection session (explicit API transactions may still appear in transaction status views while active).

---

### User Story 4 - Unified Authentication Across Entry Points (Priority: P2)

Security and platform teams require one authentication policy for KalamDB: the same users, roles, and credential validation whether the client connects through the HTTP API, the existing PostgreSQL bridge, or the new wire-protocol listener. Username and password login must resolve to the same authenticated identity used elsewhere in the product.

**Why this priority**: Split authentication paths create security gaps, duplicate configuration, and inconsistent audit trails. Unified auth is a prerequisite for offering wire-protocol access safely.

**Independent Test**: Authenticate the same user through two different supported entry points and verify both receive equivalent authorization outcomes for the same protected operation.

**Acceptance Scenarios**:

1. **Given** valid username and password credentials accepted by the existing login flow, **When** the same credentials are used for wire-protocol login, **Then** the client receives an authenticated session with equivalent user identity and role enforcement.
2. **Given** a disabled or deleted user, **When** they attempt login on any supported entry point, **Then** authentication fails consistently with a generic failure message.
3. **Given** an authenticated session on any entry point, **When** the user attempts an operation their role cannot perform, **Then** access is denied under the same authorization rules regardless of entry point.
4. **Given** a session lease or token expiry policy configured for the product, **When** credentials expire, **Then** new operations are rejected until re-authentication on all entry points that support session renewal.

---

### User Story 5 - PostgreSQL Extension Keeps Working (Priority: P2)

Developers using the existing PostgreSQL foreign-data extension must continue to run explicit PostgreSQL transactions against KalamDB-backed tables without behavior regressions, while sharing the same connection session lifecycle as wire-protocol clients internally.

**Why this priority**: The extension is an existing production integration. Refactoring session ownership must not break atomic multi-statement workflows already validated in prior releases.

**Independent Test**: Re-run the extension’s explicit transaction scenarios (commit, rollback, read-your-writes, disconnect cleanup) and confirm identical user-visible outcomes before and after the refactor.

**Acceptance Scenarios**:

1. **Given** a PostgreSQL explicit transaction that touches KalamDB-backed foreign tables, **When** the PostgreSQL transaction commits, **Then** all staged KalamDB changes in that block become durable.
2. **Given** a PostgreSQL explicit transaction that touches KalamDB-backed foreign tables, **When** the PostgreSQL transaction rolls back, **Then** no staged KalamDB changes from that block remain visible.
3. **Given** a PostgreSQL explicit transaction that never touches KalamDB-backed tables, **When** the transaction runs only native PostgreSQL statements, **Then** KalamDB does not open unnecessary transaction overhead for that block.
4. **Given** the extension disconnects while a remote transaction block is active, **When** cleanup runs, **Then** uncommitted KalamDB changes are discarded.

---

### User Story 6 - Admin Views All Connection Sessions by Origin (Priority: P2)

An administrator troubleshooting client connectivity needs to see every active connection-based backend session at one time, including where each session was opened from—PostgreSQL wire protocol or the PostgreSQL extension bridge—without stateless HTTP API requests appearing as connection sessions.

**Why this priority**: With two connection-based entry points, operators must distinguish client sources quickly. Mislabeling or duplicating sessions makes incident response unreliable.

**Independent Test**: Open one wire-protocol connection and one extension-bridge connection concurrently, query the admin session view as an authorized administrator, and verify both sessions appear once each with the correct origin label.

**Acceptance Scenarios**:

1. **Given** active wire-protocol and extension-bridge connections, **When** an authorized administrator queries the connection session view, **Then** every open connection-based session is listed with a clear origin label distinguishing wire protocol from extension bridge.
2. **Given** only stateless HTTP SQL API traffic with no open connection sessions, **When** an administrator queries the connection session view, **Then** the result is empty or contains only connection-based sessions—not individual API requests.
3. **Given** a connection session with an open transaction block, **When** an administrator inspects session and transaction views, **Then** transaction identifiers and block state match between the two views for that connection.
4. **Given** a connection closes, **When** an administrator refreshes the connection session view, **Then** that session no longer appears within the configured cleanup window.

---

### User Story 7 - Observable, Lean Connection State (Priority: P3)

Operators monitoring KalamDB need connection and transaction status views that are accurate, non-duplicated, and memory-efficient as connection-based entry points scale.

**Why this priority**: Duplicate session/transaction tracking increases memory use, complicates troubleshooting, and creates reconciliation bugs. Clean observability and bounded resource use are required for production scale.

**Independent Test**: Open several connection-based sessions with and without active transaction blocks, query operational views, and verify each connection and block appears exactly once with consistent state labels.

**Acceptance Scenarios**:

1. **Given** multiple active connection-based sessions, **When** an operator inspects connection and transaction status views, **Then** each connection appears once with accurate block state (idle, in transaction, or failed block).
2. **Given** a connection with no open transaction block, **When** listed in connection status views, **Then** no stale transaction identifiers from prior blocks are shown.
3. **Given** a connection closes or times out, **When** cleanup completes, **Then** its connection record and any associated open block are removed from active status views within the configured cleanup window.
4. **Given** a baseline load test with at least 1,000 concurrent idle connection-based sessions, **When** measured over 15 minutes, **Then** average memory per idle connection stays within the agreed efficiency target defined in success criteria.

---

### User Story 8 - Consolidate Duplicate Session Logic Without Breaking What Works (Priority: P3)

Maintainers need duplicate connection-session and transaction tracking removed so wire protocol and the extension bridge share one implementation, without rewriting large portions of already-correct extension, API, or transaction commit behavior.

**Why this priority**: The product now relies on connections for both extension bridge and wire protocol access. Parallel session implementations will drift; large rewrites risk regressions in paths that already work.

**Independent Test**: Compare extension and API transaction acceptance results before and after refactor; they must match. Code review confirms duplicate session lifecycle code is removed while commit, rollback, and autocommit paths remain behaviorally unchanged.

**Acceptance Scenarios**:

1. **Given** the unified connection session model is live, **When** reviewers inspect lifecycle code, **Then** wire protocol and extension bridge use one shared connection session implementation.
2. **Given** existing extension and HTTP SQL batch transaction tests passed before migration, **When** rerun after migration, **Then** all pass without behavioral changes.
3. **Given** deprecated duplicate session or transaction helpers identified during migration, **When** the feature is marked complete, **Then** those helpers are removed—not left alongside the new model.
4. **Given** release notes for this feature, **When** operators upgrade, **Then** any temporary compatibility shim is documented with a clear removal window.

---

### Edge Cases

- What happens when a client disconnects mid-transaction? Uncommitted work is rolled back and connection state is reclaimed.
- What happens when a statement fails inside an explicit block? The connection enters a failed-block state until the client issues `ROLLBACK` (wire-protocol and SQL semantics aligned with PostgreSQL expectations).
- What happens when transaction idle timeout fires? The open block is aborted, resources are released, and the connection reflects the aborted state or is closed per product policy.
- What happens when authentication succeeds but authorization denies a statement? The statement fails without mutating data; session remains usable for permitted operations where policy allows.
- What happens when the same user opens many concurrent connections? Each connection maintains independent session and block state without cross-connection leakage.
- What happens when an API batch request mixes valid and invalid statements inside one open block? The block is rolled back according to unified error-handling rules and the client receives a deterministic error outcome.
- What happens when an administrator queries sessions during heavy API-only traffic? Connection session listings remain limited to wire and extension-bridge connections; active API request transactions appear only in transaction status views if applicable.

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001**: The system MUST model each long-lived client connection (wire protocol or extension bridge) as a backend session that outlives any single transaction block on that connection.
- **FR-002**: The system MUST allow at most one open explicit transaction block per backend session at a time.
- **FR-003**: The system MUST provide explicit begin, commit, and rollback operations that apply consistently to connection-based backend sessions and to request-scoped API transaction blocks through the same underlying transaction authority.
- **FR-004**: The system MUST expose a PostgreSQL wire-protocol listener that allows standard PostgreSQL clients to connect, authenticate, and execute SQL.
- **FR-005**: The system MUST support autocommit execution on wire-protocol connections for standalone statements that do not open an explicit block.
- **FR-006**: The system MUST support explicit multi-statement transaction blocks on wire-protocol connections using standard `BEGIN`, `COMMIT`, and `ROLLBACK` semantics.
- **FR-007**: The system MUST continue to support explicit transaction blocks inside a single HTTP SQL request payload, including multiple sequential closed blocks within that request.
- **FR-008**: The system MUST automatically roll back any still-open explicit transaction block when an HTTP SQL request ends without a closing `COMMIT` or `ROLLBACK`.
- **FR-009**: The system MUST preserve existing PostgreSQL extension transaction behavior for explicit PostgreSQL transactions that touch KalamDB-backed data, including commit, rollback, read-your-writes, and disconnect cleanup.
- **FR-010**: The system MUST avoid opening KalamDB transaction overhead for PostgreSQL explicit transactions that never touch KalamDB-backed data.
- **FR-011**: The system MUST authenticate wire-protocol clients using the same user identity and role model as the existing HTTP API and bridge access paths.
- **FR-012**: The system MUST accept username and password during wire-protocol login and validate them through the same credential authority used for existing password-based access, producing an authenticated session equivalent to other entry points.
- **FR-013**: The system MUST enforce authorization consistently after authentication regardless of entry point.
- **FR-014**: The system MUST maintain a single authoritative source of truth for active explicit transaction state; connection metadata MUST NOT contradict that source in operational views.
- **FR-015**: The system MUST roll back uncommitted explicit transaction blocks on connection close, session timeout, or unrecoverable client disconnect.
- **FR-016**: The system MUST abort long-running explicit transaction blocks that exceed the configured timeout and release associated resources.
- **FR-017**: The system MUST provide an operational connection session view listing all active connection-based backend sessions.
- **FR-018**: Each row in the connection session view MUST include the session origin, distinguishing at minimum PostgreSQL wire protocol from PostgreSQL extension bridge.
- **FR-019**: The system MUST NOT register stateless HTTP SQL API requests as connection sessions in the connection session view.
- **FR-020**: Authorized administrators MUST be able to query all open connection-based sessions at a time through the connection session view.
- **FR-021**: The system MUST provide a transaction status view for active explicit transactions, including request-scoped API transactions when applicable, without duplicating connection session rows.
- **FR-022**: The system MUST remove duplicated connection-session lifecycle implementations used separately for extension bridge and future wire protocol access, consolidating to one shared path.
- **FR-023**: The refactor MUST preserve existing correct behavior for extension bridge transactions, HTTP SQL batch transactions, and durable commit/rollback paths; changes MUST be limited to deduplication, shared session lifecycle, and wire-protocol enablement unless a defect is being fixed.
- **FR-024**: The system MUST keep non-transactional request paths on a low-overhead autocommit path that does not allocate full transaction block resources unless an explicit block is opened.
- **FR-025**: The system MUST document intentional differences between connection-based sessions (wire and extension bridge) and stateless HTTP SQL request transactions without duplicating core transaction logic.

### Key Entities

- **Backend Session**: A long-lived connection context for wire-protocol or extension-bridge clients only; holds identity, session origin, connection metadata, session settings (such as active schema/search path), activity timestamps, and current block state. Not used for stateless HTTP SQL requests.
- **Session Origin**: The connection entry point that opened the backend session—`wire_protocol` or `extension_bridge`.
- **Transaction Block**: The client-visible explicit transaction scope on a backend session or API request, with at most one open block per backend session, ending in commit, rollback, timeout, or disconnect/request cleanup.
- **Explicit Transaction**: The server-side unit that stages work, enforces read-your-writes and isolation rules, and durably commits or discards changes for an open block.
- **Entry Point**: A supported client access surface—wire protocol, HTTP SQL API, or PostgreSQL extension bridge—that adapts wire or request formats to shared transaction authority and, where applicable, the unified connection session model.
- **Authenticated Identity**: The user, role, and authorization context established at login and attached to a backend session or request-scoped execution context.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: In acceptance testing, 100% of explicit multi-statement transaction scenarios (commit, rollback, read-your-writes) pass identically across wire-protocol clients, HTTP SQL batch requests, and the PostgreSQL extension bridge after migration.
- **SC-002**: Operators can connect with at least three common PostgreSQL client tools and complete login plus `SELECT 1` on first attempt without custom client patches.
- **SC-003**: For the same user and password, authentication outcome (success or generic failure) matches across HTTP API, bridge, and wire-protocol login in 100% of tested credential pairs.
- **SC-004**: Under a load test of 1,000 concurrent idle connection-based sessions for 15 minutes, average memory per idle connection is no more than 50 KB above the pre-feature baseline measured on the same hardware profile.
- **SC-005**: Under a load test of 100 concurrent connections each executing 10 explicit transaction blocks per minute for 10 minutes, p95 time to begin or end a block increases by no more than 10% versus the pre-refactor extension bridge baseline.
- **SC-006**: After feature completion, operational session and transaction views show zero cases of mismatched transaction identifiers for the same active connection in automated reconciliation tests.
- **SC-007**: Architecture review confirms a single shared connection session lifecycle for wire protocol and extension bridge, with no remaining duplicate implementations except documented temporary shims scheduled for removal within one release.
- **SC-008**: At least 90% of maintainers surveyed in feature review agree the session and transaction code paths are easier to extend to new entry points than before the refactor.
- **SC-009**: In admin acceptance testing, 100% of concurrently open wire-protocol and extension-bridge sessions appear in the connection session view with the correct origin label, and zero HTTP SQL requests appear as connection sessions.
- **SC-010**: Regression suite for feature 027 extension and SQL batch transaction scenarios shows zero behavioral diffs attributable to this refactor before feature sign-off.

## Assumptions

- HTTP SQL access remains stateless: it does not open connection backend sessions and does not appear in connection session listings; explicit transaction blocks in API payloads remain request-scoped and may appear in transaction status views while active.
- PostgreSQL extension access may continue lazy opening of remote transaction blocks until KalamDB-backed data is first touched inside a PostgreSQL explicit transaction.
- Wire-protocol authentication uses username and password presented at login; internally this reuses the same credential validation and identity issuance model already used by existing API and bridge login flows.
- Connection session administration uses the same authorized administrator access model already applied to existing operational system views.
- Savepoints, nested transactions, and cross-shard distributed transactions remain out of scope for this feature unless added in a follow-up specification.
- Obsolete code removal is scoped to duplicated connection-session and transaction lifecycle logic; unrelated legacy cleanup and large rewrites of working commit paths are not required.
- Refactor strategy is incremental: preserve proven extension and API behavior, consolidate duplicates, and add wire-protocol support on the shared connection session path.
- Standard PostgreSQL client compatibility targets commonly used connection modes needed for operational tooling and application drivers; exotic replication or copy protocol features may be deferred to later specifications.
- Existing role-based access rules and audit expectations for authenticated users remain unchanged unless separately specified.

## Dependencies

- Existing explicit transaction commit and rollback behavior delivered in feature 027-pg-transactions remains the functional baseline for extension and SQL batch paths.
- Existing authentication and user management capabilities from feature 007-user-auth (and related auth integration work) provide the credential authority reused by wire-protocol login.
- Operational session and transaction visibility requirements build on views and monitoring patterns already exposed to operators today.

## Out of Scope

- Full PostgreSQL SQL and protocol compatibility beyond the connection, authentication, query, and explicit transaction workflows required for supported clients in this feature.
- Persistent HTTP session tokens for multi-request explicit transactions across separate API calls.
- Listing stateless HTTP SQL requests in connection session views.
- Large-scale rewrites of working transaction commit, rollback, or extension bridge behavior unrelated to session deduplication.
- Savepoints and subtransactions.
- Multi-group or cross-cluster atomic transactions.
- Rewriting unrelated legacy subsystems except where they duplicate session or transaction lifecycle logic targeted by this feature.
