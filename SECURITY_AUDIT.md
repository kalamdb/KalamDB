# KalamDB security audit

Date: 2026-09-21

This pass reviewed authentication, the PostgreSQL extension bridge, procedure HTTP responses, and audit identity. It patched the issues below and added regression tests. It is not a claim that the rest of the system is secure.

## Executive summary

The highest-impact defect was on the PostgreSQL extension gRPC bridge. After `open_session`, later RPCs trusted the client-supplied session id alone. Those ids are predictable in shape (`pg-<backend pid>-<hash>`), are visible to administrators in `system.sessions`, and authorized `execute_sql` as a DBA principal. A caller who learned a live session id could run SQL, scan or mutate rows as any `user_id` the bridge accepts, or close the session.

The bridge now fails closed when no authentication is configured, and an authenticated opener receives a random session capability that later RPCs must present. The in-repo PG client sends that capability. Procedure code can no longer set hop-by-hop or `Set-Cookie` response headers, or header values containing CR/LF. Failed bearer authentication no longer records an unverified JWT `sub` as the actor.

`cargo audit` reported 9 vulnerabilities and 12 allowed warnings. They were not upgraded in this pass because the fixes cross major versions or have no upstream fix (`rsa`).

## Critical issues found

None that were reachable on a production server that already configures bearer authentication **and** keeps session ids secret. The bridge defect below is critical if a session id leaks or the bridge is left unconfigured. Production wiring always attaches a user repository, so the unconfigured path was a misconfiguration and test default, not the normal server boot path.

## High-risk issues found

### 1. PG bridge session id was a DBA capability

**Attack path.** Authenticate once (or, previously, skip auth entirely), learn a session id, then call `ExecuteSql`, `Scan`, `Insert`, `Update`, `Delete`, transaction RPCs, or `CloseSession` with that id and no further credentials. `OperationService::execute_sql` runs bridge SQL as `Role::Dba`. Typed scans and mutations honor the client-supplied `user_id`, which is how the extension scopes end-user tables. Anyone holding the session id could choose that user id.

**Root cause.** `KalamPgService` authenticated only `open_session` and `ping`. `validate_session_handle` treated “this id was opened” as sufficient. `close_session` did not check the session at all. With no mTLS, static token, or bearer repository, `open_session` returned `Role::System` for `UserId::anonymous`, and later RPCs skipped validation.

**Fix.**

- Unconfigured bridges reject ping, open, and session RPCs. Tests that still need the old behavior call `allow_insecure_unauthenticated()`.
- When authentication is configured, `open_session` returns a UUID v4 `session_token` (protobuf field 4). It is stored in process memory and is not written into `system.sessions`.
- Later RPCs must send `x-kalam-session-token`. Comparison is constant-time. A missing session close stays idempotent so cleanup retries still succeed. A **live** session cannot be closed or used without the token.
- The PG client stores the token and attaches it on scan, mutation, SQL, transaction, and close calls, including reconnect.
- Static `pg_auth_token` comparison is constant-time.
- Activity recording no longer creates a `Role::System` session when authentication is configured and the id is unknown.

**Compatibility.** Server and `kalam-pg-client` / `pg_kalam` must ship together. An old client that ignores `session_token` can still open a session, but authenticated follow-up RPCs fail.

### 2. Procedure HTTP response header injection

**Attack path.** A procedure invoked over `POST /v1/functions/...` could set `Set-Cookie` or a value containing CR/LF. Actix drops some illegal values, but the procedure API accepted them.

**Fix.** Response header names must be RFC 7230 tokens. Values must be visible ASCII or tab. `Set-Cookie`, `Set-Cookie2`, and additional hop-by-hop names are rejected.

## Medium / low issues

### Medium: unverified JWT subject used as the audit actor

`extract_user_id_for_audit` base64-decoded the JWT payload without checking the signature and logged `sub`. A forged token could attribute a failed login to another user. Bearer and raw JWT attempts now log the anonymous user. Password logins still record the username the client typed, which is the claimed login name, not an authorization decision.

### Medium: `actix-rt` feature gap when `kalamdb-auth` is built with HTTP enabled

`actix-web` depends on `actix-rt` with default features off. Building `kalamdb-auth` with the `http` feature, without also depending on the workspace `actix-rt` (which enables `net` and `signal`), failed to compile `actix-server`. The `http` feature now enables that workspace dependency. This is a build-correctness fix, not a runtime auth bypass.

### Low: static shared-secret and session-token checks

Both comparisons now walk the full inputs. Length still affects the loop bound.

### Reviewed without a code change

These areas were inspected and did not show an immediate bypass. That is not a certification.

- HTTP `/v1/api/sql` uses `AuthSessionExtractor`. Malformed, expired, and signature-invalid tokens return generic `INVALID_CREDENTIALS` (expired tokens still use `TOKEN_EXPIRED`). There is no anonymous fallback on parse failure.
- Internal HS256 tokens are pinned to the `kalamdb` issuer. External tokens must use an asymmetric algorithm and a configured OIDC issuer. Claimed roles are checked against the stored user. Refresh tokens are rejected as access tokens. Deleted users fail like bad credentials.
- Bearer sessions are cached, and user updates invalidate that cache.
- User-table file downloads default to the authenticated user. Another `user_id` requires an impersonation role that is allowed to target that account. Path components reject `..`, slashes, and NUL.
- Live subscriptions require an authenticated socket, reject joins and subqueries in the subscription shape, and block non-admins from `system` namespaces. Shared tables fail closed when the RLS binder is missing.
- Function host SQL runs as the invoker or definer principal, rejects transaction control, and the engine has heap, timeout, depth, and result limits.
- Raft peers on non-localhost multi-node clusters are required to enable RPC TLS. Incoming RPCs also check cluster id, node id, and source address. Without mTLS, those headers are not cryptographic; the IP check is the remaining control.
- `current_setting` returns a fixed allow-list and an empty string for unknown names. It does not read process environment variables.

## Files changed

- `backend/crates/kalamdb-pg/src/service.rs`
- `backend/crates/kalamdb-pg/src/session_registry.rs`
- `backend/crates/kalamdb-pg/tests/service_auth_tx.rs`
- `backend/crates/kalamdb-pg/tests/support/mod.rs`
- `backend/crates/kalamdb-auth/Cargo.toml`
- `backend/crates/kalamdb-auth/src/services/unified/audit.rs`
- `backend/crates/kalamdb-auth/src/services/unified/mod.rs`
- `backend/crates/kalamdb-core/src/functions/host.rs`
- `pg/crates/kalam-pg-client/src/lib.rs`
- `pg/crates/kalam-pg-client/tests/connectivity.rs`
- `pg/crates/kalam-pg-client/tests/support/mod.rs`
- `pg/crates/kalam-pg-client/tests/statement_round_trip.rs`
- `pg/src/remote_state.rs`
- `backend/crates/kalamdb-raft/tests/pg_rpc_mount.rs`

## Security architecture improvements

- Bridge authentication is deny-by-default. The insecure mode is an explicit constructor, not the production path.
- Session ids are no longer sufficient credentials when the bridge is authenticated. The capability is not stored in `system.sessions`.
- Authenticated bridges do not materialize a system session for an unknown id during activity tracking.
- Procedure HTTP responses cannot set cookie or hop-by-hop headers.
- Audit logs do not treat an unverified JWT payload as identity.

## Tests added

- `unconfigured_pg_rpc_rejects_ping_and_open_session`
- `authenticated_session_id_without_capability_cannot_run_or_close` (missing token, forged token, valid token, close without token, idempotent close)
- `blocked_response_headers_are_rejected` now covers `Set-Cookie`, CR/LF, and illegal header names
- Audit tests expect anonymous user ids for forged JWT payloads

Existing `kalamdb-pg` `service_auth_tx` (30) and `kalam-pg-client` connectivity (6) tests passed after the client was updated.

Not run in this pass: full `cargo test --workspace`, full `cargo clippy --workspace --all-targets --all-features`, fuzz targets, and CLI/e2e smoke. Those suites are large and were outside the patched crates’ direct tests.

## Dependency findings

`cargo audit` (2026-09-21): **9 vulnerabilities**, **12 allowed warnings**. `cargo deny` is not configured.

Vulnerabilities:

| Crate | Advisory | Notes |
| --- | --- | --- |
| `h2` 0.3.27 and 0.4.15 | RUSTSEC-2026-0258 unbounded empty DATA frames | Fix is `h2 >= 0.4.16`. The 0.3 line is still pulled transitively. |
| `pyo3` 0.28.3 | RUSTSEC-2026-0176, RUSTSEC-2026-0177 | Fix is 0.29.0. Likely extension/tooling, not the server hot path. |
| `quick-xml` 0.39.2 | RUSTSEC-2026-0194, RUSTSEC-2026-0195 | DoS. Fix is 0.41.0. |
| `quinn-proto` 0.11.14 | RUSTSEC-2026-0185 | Memory exhaustion. Fix is 0.11.15. |
| `rkyv` 0.7.46 | RUSTSEC-2026-0235 | OOB read. Fix is 0.8.17, a major bump. |
| `rsa` 0.9.10 | RUSTSEC-2023-0071 Marvin timing attack | No fixed upgrade. |

Warnings include unmaintained `dotenv`, `paste`, `serde_cbor`, and `term_size`, plus unsoundness notes for `cxx` and `event-listener`.

These were not bumped here. `h2` 0.4 and `quinn-proto` are the most plausible small upgrades; `rsa` and `rkyv` need a planned migration.

## Remaining risks

- A valid bridge capability is still a DBA SQL channel. Compromise of the PostgreSQL extension host, or theft of the token from that process, is full bridge access. Typed RPCs still accept a client `user_id` because that is how the extension names the end user. The token does not replace end-user authorization inside PostgreSQL.
- Session capabilities are in-memory on the node that opened the session. They are not replicated. Leader redirect re-opens the session and issues a new token; that path was not penetration-tested across a real cluster.
- `allow_insecure_unauthenticated()` remains available to tests and any embedder who calls it. Production `AppContext` does not.
- Raft authentication without mTLS is cluster id + node id + source IP. A host that can connect from a peer’s address and knows those ids can speak the internal RPC. Non-localhost clusters are required to enable TLS; localhost clusters are not.
- WebSocket origin checks allow every origin when `allowed_origins` is empty or `*`. Non-localhost HTTP already requires an origin policy at config validation. Strict missing-Origin rejection is off unless `strict_ws_origin_check` is set.
- JWT access tokens do not check `aud`. Internal tokens are HS256 and issuer-pinned. External tokens rely on the OIDC verifier.
- Bearer cache keys are the raw token string, so process memory retains bearer tokens until idle expiry (5 minutes) or invalidation.
- Password authentication uses bcrypt. The `rsa` crate remains in the tree for other JWT/OIDC algorithms and has a known timing advisory with no upstream fix.
- Function sandbox limits exist (heap, timeout, depth, result size). This pass did not try to escape V8 or exhaust the host from inside a procedure.
- Cold-storage versus hot-storage authorization equivalence was not re-proved with a flush/read attack.
- Subscription delivery after a role revocation mid-connection was not re-tested.

## Areas that require manual penetration testing

- Mixed-version PG extension against this server (confirm old clients fail closed, new clients succeed).
- Bridge token theft from a live `postgres` backend and replay from another host.
- Leader forwarding: session opened on node A, RPC retried on node B.
- Raft RPC from a non-member who can spoof or share a peer source address, with TLS off (localhost) and on.
- OIDC auto-provision, invite acceptance, and role claim mismatch against a real issuer.
- `EXECUTE AS USER` from HTTP SQL, PG wire, and procedure `ctx.db` for user A targeting user B, including USER, SHARED, and STREAM tables.
- Subscribe, replay (`from` sequence), and initial snapshot for user A against user B’s rows, then revoke A mid-connection.
- Function timeout, memory, and nested SQL while a request transaction is open.
- File download and storage object names with encoded `..`, overlong UTF-8, and shared-table paths.
- Oversized HTTP bodies, WebSocket frames, and PG wire startup messages.

## Recommended next security work

1. Upgrade `h2` 0.4 to >=0.4.16 and `quinn-proto` to >=0.11.15. Plan the `h2` 0.3 removal with the crate that still pins it.
2. Bind bridge `execute_sql` to the authenticated bridge role explicitly, and audit every typed `user_id` override so only the extension’s authenticated end-user identity can be selected.
3. Re-check live subscriptions when role or deletion changes, and drop the socket.
4. Add a network fuzz target for PG bridge framing and PG wire startup, not only SQL parsing.
5. Replace or isolate `rsa` 0.9 if OIDC verification can move fully onto `jsonwebtoken` asymmetric verification without the Marvin-vulnerable implementation.
6. Add `cargo deny` with a reviewed advisory allow-list so the 12 warnings and new CVEs fail CI on purpose.
