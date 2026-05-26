# Authentication & Authorization Test Scenarios [CATEGORY:AUTH]

This document outlines the test scenarios for authentication, authorization, and multi-tenant isolation in KalamDB.

## 1. Multi-Factor Auth Lifecycle
**Goal**: Validate various authentication methods and token management.
- **Scenario A**: Basic Auth (Username/Password).
  1. Register a new user with a secure password.
  2. Authenticate using HTTP Basic Auth header.
  3. Verify access is granted.
- **Scenario B**: JWT Flow.
  1. Login via `/api/v1/auth/login`.
  2. Receive a JWT token.
  3. Use Bearer token for Subsequent requests.
  4. Verify token expiration and invalidation.
- **Scenario C**: OIDC Integration (Simulated).
  1. Initiate OIDC flow with issuer (Google/Keycloak/Entra/etc.).
  2. Exchange provider ID token for a KalamDB session.
- **Agent Friendly**: Check `backend/tests/misc/auth/test_jwt_auth.rs`, `backend/tests/misc/auth/test_oauth.rs` (legacy file name), and `backend/tests/misc/auth/test_oidc_auto_provision.rs`.

## 2. RBAC (Role-Based Access Control)
**Goal**: Enforce permissions across system roles.
- **Step 1**: Create users with roles: `user`, `service`, `dba`, `system`.
- **Step 2**: Verify `user` can only access their own data.
- **Step 3**: Verify `dba` can manage schemas but not necessarily read user data (depending on policy).
- **Step 4**: Verify `system` role has full override capabilities.
- **Agent Friendly**: Check `backend/tests/misc/auth/test_rbac.rs`.

## 3. Multi-Tenant User Isolation
**Goal**: Invariants of USER-type tables.
- **Step 1**: Create User A and User B.
- **Step 2**: Create a USER table `messages`.
- **Step 3**: User A inserts data; User B inserts data.
- **Step 4**: Verify SELECT by User A *never* returns User B's rows, even without a WHERE clause.
- **Agent Friendly**: Check `backend/tests/testserver/tables/test_user_tables_http.rs`.

## 4. EXECUTE AS USER Role Matrix
**Goal**: Direct USER-table and STREAM-table access stays subject-scoped, while explicit `EXECUTE AS USER` follows the role hierarchy.
- **Step 1**: Authenticate as `user`, `service`, `dba`, and `system` role users.
- **Step 2**: Attempt allowed and denied cross-user `EXECUTE AS USER` SELECT/INSERT/UPDATE/DELETE.
- **Step 3**: Verify allowed role edges operate under the target user ID for USER and STREAM tables, and denied edges leave target rows unchanged.
- **Step 4**: Verify self-targeted `EXECUTE AS USER` is a no-op and remains scoped to the actor.
- **Agent Friendly**: Check `backend/tests/misc/auth/test_as_user_impersonation.rs`.

## 5. Security Policies
**Goal**: Password complexity and timing-safe checks.
- **Scenario**: Verify password hashing (bcrypt) and rejection of weak passwords.
- **Agent Friendly**: Check `backend/tests/misc/auth/test_password_security.rs`.

## Coverage Analysis
- **Duplicates**: User isolation is tested in both `test_user_tables_http.rs` and most `scenarios/`.
- **Gaps**: Session revocation across multiple nodes is minimally tested.
