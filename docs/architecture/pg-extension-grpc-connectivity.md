# PostgreSQL Extension gRPC Connectivity

## Overview

The `pg_kalam` PostgreSQL extension talks to KalamDB through the gRPC `PgService`, not through the HTTP SQL API. The client lives in `pg/crates/kalam-pg-client`, the extension-side state/cache lives in `pg/src/remote_state.rs`, and the server-side service lives in `backend/crates/kalamdb-pg/src/service.rs`.

The gRPC service is mounted on KalamDB's shared RPC listener, which is the same listener used by the Raft RPC and cluster message services. In practice, the PostgreSQL extension connects to `host:port` from `CREATE SERVER ... OPTIONS (...)`, and that `host:port` must point at the shared RPC listener, not the HTTP listener.

## Endpoint and Security Model

### Endpoint source

The extension reads these foreign-server options:

- `host`
- `port`
- `timeout`
- `auth_header`
- `ca_cert`
- `client_cert`
- `client_key`

Those values are parsed into `RemoteServerConfig` and used to build a tonic `Endpoint`.

### Transport protocol

- Protocol: gRPC over HTTP/2
- Scheme: `http://` when TLS is not configured, `https://` when `ca_cert` is configured
- Call shape: unary RPCs only
- Current RPC surface: `Ping`, `OpenSession`, `CloseSession`, `Scan`, `Insert`, `Update`, `Delete`, `BeginTransaction`, `CommitTransaction`, `RollbackTransaction`, `ExecuteSql`, `ExecuteQuery`

There is no streaming RPC between the extension and KalamDB today.

### Authentication

Two auth modes exist today:

1. Pre-shared token
   The extension sends the `auth_header` foreign-server option as gRPC `authorization` metadata. The server compares it against `auth.pg_auth_token` when configured.

2. mTLS
   When RPC TLS is enabled and client certificates are required, the server authenticates the peer certificate CN. PostgreSQL extension clients must present a CN with the `kalamdb-pg-{name}` pattern.

### Deployment note

KalamDB now uses the unified Raft executor in both standalone and cluster mode. That means the PostgreSQL bridge rides the shared RPC listener in both modes.

One important operational detail is that single-node `RaftManagerConfig::for_single_node()` defaults to `127.0.0.1:0`, which means the OS assigns an ephemeral RPC port. For a stable PostgreSQL extension endpoint, operators should expose an explicit RPC address, typically through `[cluster].rpc_addr` in `server.toml`.

## Connection vs Session

There are two distinct layers:

1. Transport connection
   A tonic `Channel` stored in `RemoteKalamClient`. This is the reusable gRPC client transport.

2. Logical session
   A server-side `BackendSessionManager` entry keyed by a session id such as `pg-<pid>-<config-hash>`. This is what appears in `system.sessions`.

These are related, but they are not the same thing.

- The transport is about getting bytes to the server.
- The shared backend session is about tracking PostgreSQL backend identity,
  origin, transaction block state, cleanup, and recent activity.

`kalamdb-pg::SessionRegistry` remains as a compatibility adapter for extension
RPC authentication, leases, and legacy handle validation. It is not the durable
transaction authority when `BackendSessionManager` is present.

## Lifecycle

### 1. First use in a PostgreSQL backend

On the first FDW scan, FDW modify, DDL bridge call, or `kalam_exec(...)` call, the extension does the following:

1. Parse foreign-server options into `RemoteServerConfig`.
2. Call `ensure_remote_extension_state(config)`.
3. If no state exists for this PostgreSQL backend process and config pair:
   - build a single-thread Tokio runtime
   - connect a tonic `Endpoint`
   - compute a session id: `pg-<backend-pid>-<config-hash>`
   - send `OpenSession`
4. Cache the resulting `RemoteExtensionState` for reuse.

That cache is per PostgreSQL backend process and per remote config, not per statement.

### 2. Reuse across statements

For the same PostgreSQL backend process and the same foreign-server config:

- the same `RemoteExtensionState` is reused
- the same session id is reused
- the same `RemoteKalamClient` is reused
- each RPC goes through the shared client call helper in `RemoteKalamClient`, which clones the reusable tonic channel, applies the correct request metadata boundary, and maps gRPC status errors consistently

The extension test suite explicitly verifies that the same config reuses the same state and only opens one remote session.

### 3. Transaction lifecycle

Remote transactions are not started for every statement automatically.

- Autocommit reads do not open a remote transaction.
- Explicit PostgreSQL transaction blocks lazily open a remote transaction on the first FDW operation.
- PostgreSQL xact callbacks then drive remote `BeginTransaction`, `CommitTransaction`, and `RollbackTransaction`.

This means the current design is:

- long-lived logical session per PostgreSQL backend/config pair
- on-demand unary RPC traffic
- transaction handles created only when PostgreSQL transaction semantics require them

### 4. Backend exit

The extension registers `on_proc_exit` and attempts to send `CloseSession` for every cached remote state owned by that PostgreSQL backend process.

On the server side, `CloseSession` rolls back any active backend session block
through `BackendSessionManager`, clears bridge compatibility state, and removes
the row from `system.sessions`.

### 5. Failure cases

If the transport breaks mid-transaction, the next RPC usually discovers it and returns a transport-style error such as:

- `Unavailable`
- `transport error`
- `Connection reset`
- `broken pipe`

The test suite includes proxy-failure cases that exercise exactly this behavior.

Server-side transaction bookkeeping is reconciled by the shared backend manager
when terminal transaction state is detected. Session lifetime is driven by
explicit `CloseSession` plus manager cleanup for stale idle sessions and
disconnect paths.

## Is the Connection Always Open?

Not in the sense of constant background traffic.

What exists today is a long-lived reusable client/channel object plus a long-lived logical session. Actual network traffic is request-driven.

So the right mental model is:

- not "open a brand-new TCP connection for every query"
- not "maintain a constantly active heartbeat stream"
- instead: "keep a reusable gRPC client per PostgreSQL backend, send unary RPCs when needed, and keep a logical remote session open across statements"

When the system is idle, there is no application-level heartbeat traffic between the extension and KalamDB.

## Does It Have Ping/Pong?

### What exists today

There is a unary `Ping` RPC returning `PingResponse { ok: true }`.

### What does not exist today

- no periodic ping task in the PostgreSQL extension
- no session heartbeat renewal loop
- no bidirectional ping/pong stream
- no tonic client keepalive configuration in the extension
- no tonic server keepalive configuration on the shared RPC listener

This is an important distinction:

- `Ping` exists as a callable health-style RPC
- `Ping` is not used automatically as a keepalive mechanism

Also note that `RemoteKalamClient::connect()` by itself is not a complete proof that the remote endpoint is healthy. The repository test suite already treats a first real RPC or `Ping` as the authoritative connectivity check.

## How To Verify Whether It Is Still Open

### Best transport-level verification

Issue a real RPC.

Practical choices:

- call `Ping`
- run `SELECT kalam_exec('SELECT 1')`
- run a normal FDW `SELECT 1`-style query against a Kalam-backed table

If the channel is broken, the next RPC is what will usually detect it.

### Best server-side visibility

Query `system.sessions`:

```sql
SELECT
  session_id,
  backend_pid,
  state,
  transaction_id,
  transaction_state,
  client_addr,
  transport,
  opened_at,
  last_seen_at,
  last_method
FROM system.sessions
ORDER BY last_seen_at DESC;
```

What this tells you:

- whether the server still has a logical extension or wire session entry
- which PostgreSQL backend opened it
- which origin opened it
- the last RPC method seen
- the last time the server observed activity

What this does not tell you:

- whether the underlying TCP or HTTP/2 transport is still healthy right now

`last_seen_at` is activity-based, not heartbeat-based. Cleanup can remove stale
idle rows, but a fresh row still does not prove the underlying TCP or HTTP/2
transport is healthy right now.

### Transaction-level verification

If you care about remote transaction state as well, also query `system.transactions`.

## Current Strengths

- Reuses one remote client/session per PostgreSQL backend and config instead of reconnecting for every statement.
- Separates logical session lifecycle from transaction lifecycle.
- Cleans up on normal PostgreSQL backend exit.
- Avoids blind retries for normal write paths, which is safer than duplicating writes.
- Exposes server-side observability through the shared `system.sessions` view.

## Current Gaps

### 1. No automatic keepalive

There is no configured HTTP/2 keepalive, TCP keepalive policy, or logical session heartbeat.

### 2. Cleanup is activity-based

Server-side cleanup removes stale idle rows and rolls back active blocks on close
or disconnect, but it is not a periodic wire-level heartbeat.

### 3. `system.sessions` is not a full liveness signal

It is a logical registry view, not a socket-health view.

### 4. Stable endpoint exposure is awkward in single-node mode

Single-node mode defaults to an ephemeral RPC port unless the operator configures a fixed RPC address.

### 5. Retry behavior is narrow

Today the client only reconnects and retries cleanup-oriented operations such as `CloseSession` and `RollbackTransaction`. Normal query and write RPCs do not retry automatically.

That is conservative and generally correct for writes, but it means operator-facing health recovery depends on the next caller-visible RPC.

## Recommended Improvements

The current per-backend reusable channel model is directionally correct and is common for database bridges and FDWs. The main improvements should be around liveness, leases, and operability rather than switching to per-query connections.

### 1. Keep the per-backend reusable channel model

This is the part that already matches common practice:

- one long-lived client per local PostgreSQL backend process
- reuse across statements
- explicit logical session identity

Opening a new transport for every query would add latency and create more failure surface.

### 2. Add transport keepalive on both sides

Configure tonic HTTP/2 keepalive and TCP keepalive for the shared RPC listener and the extension client.

That is the most standard improvement for gRPC-based systems because it helps detect half-open connections without waiting for the next business RPC.

### 3. Add a logical session lease

Introduce a server-enforced idle timeout for PG sessions, renewed either by:

- normal RPC activity, or
- an explicit lightweight `RenewSession` or `Ping` heartbeat loop

This would let KalamDB garbage-collect orphaned sessions after PostgreSQL crashes, network partitions, or missed `CloseSession` calls.

### 4. Add a standard health-check surface

Keep the existing `Ping` RPC, but also add one or both of:

- the standard gRPC health checking service
- a SQL-visible helper such as `kalam_ping()` that maps to `Ping`

That gives operators a first-class connectivity check instead of using `kalam_exec('SELECT 1')` as a probe.

### 5. Make session observability more explicit

Useful additions would be:

- a session age / idle age column
- a flag for "lease expired" or "closing"
- metrics for failed `CloseSession`
- metrics for orphaned or stale sessions

### 6. Keep retries conservative for writes

For common database systems, automatic retry is normal for:

- health probes
- close/cleanup
- idempotent reads when policy allows

Automatic retry is usually not safe for write or commit paths unless the protocol has an explicit idempotency story. KalamDB should keep that discipline.

If broader retry support is desired later, it should come with:

- request identifiers or idempotency keys
- clear retry classification by RPC type
- duplicate suppression server-side

### 7. Expose a stable RPC address for PG deployments

The PG extension needs a predictable gRPC target. A more operator-friendly shape would be to expose a stable top-level RPC bind/advertise setting for the PG bridge even in single-node mode, instead of relying on a single-node ephemeral default.

## Recommended Target Shape

If we want behavior that is common for similar gRPC-backed database bridges, the target design should be:

1. One reusable gRPC channel per PostgreSQL backend process.
2. One logical remote session per backend/config pair.
3. Unary request/response RPCs for normal operations.
4. HTTP/2 and TCP keepalive enabled at the transport layer.
5. Session lease or idle timeout on the server.
6. Standard health check RPC plus simple SQL-visible probe.
7. No blind automatic retry for write and commit paths without idempotency.

That keeps the good parts of the current design while fixing the biggest operational gaps.

## Relevant Implementation Files

- `pg/crates/kalam-pg-client/src/lib.rs`
- `pg/crates/kalam-pg-common/src/config.rs`
- `pg/crates/kalam-pg-fdw/src/server_options.rs`
- `pg/src/remote_state.rs`
- `pg/src/fdw_xact.rs`
- `pg/src/fdw_scan.rs`
- `pg/src/fdw_modify.rs`
- `pg/src/pgrx_entrypoint.rs`
- `backend/crates/kalamdb-pg/src/service.rs`
- `backend/crates/kalamdb-pg/src/session_registry.rs`
- `backend/crates/kalamdb-views/src/sessions.rs`
- `backend/crates/kalamdb-core/src/app_context.rs`
- `backend/crates/kalamdb-raft/src/network/service.rs`
