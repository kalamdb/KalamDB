# ADR-019: Subject-Scoped User Identity

**Status**: Accepted  
**Date**: 2026-05-01  
**Related**: ADR-009 (Three-Layer Architecture), docs/reference/sql.md, docs/api/api-reference.md

## Context

USER tables store rows under a subject user identity. Earlier role rules allowed privileged roles to widen scans implicitly. That made DBA, service, or system sessions capable of observing or mutating rows outside their own subject scope without an explicit delegation boundary.

The security model now requires a stronger invariant: authenticated users must never share USER-table row visibility or DML effects through role privilege alone.

## Decision

Ordinary USER-table and STREAM-table access is always subject-scoped to the
authenticated user.

- No role receives implicit all-user USER-table or STREAM-table reads.
- Explicit `EXECUTE AS '<user_id>'` uses a role hierarchy: system can target system/dba/service/user, DBA can target dba/service/user, service can target service/user, and regular users cannot target another user.
- Target role classification for `EXECUTE AS '<user_id>'` and USER-table file `user_id` downloads comes from a small in-memory map of service, DBA, and system users maintained by `system.users`; IDs absent from that privileged map are treated as regular users.
- Self-targeted `EXECUTE AS '<user_id>'` is allowed as a no-op identity boundary.
- APIs that accept an optional `user_id` for USER-table resources must authorize cross-user values through the same role hierarchy used by `EXECUTE AS '<user_id>'`.
- Shared tables remain shared through their table access policy, not through user impersonation.

## Rationale

Role privilege should control administrative capabilities, not silently merge user data planes. Keeping ordinary USER-table and STREAM-table DML and SELECT behavior tied to the authenticated subject avoids accidental cross-user reads, deletes, updates, file downloads, and subscriptions. When a background service must act for a user, `EXECUTE AS '<user_id>'` provides an explicit, audited boundary governed by the role hierarchy.

Self-targeted `EXECUTE AS '<user_id>'` remains valid so existing wrapper syntax can be parsed and audited without introducing a second behavior for the caller's own identity.

The privileged-role map avoids fetching high-cardinality regular user rows in impersonation hot paths. `system.users` warms the map from its role index at provider construction and updates it after successful create, role update, and soft-delete operations. Soft-deleted IDs whose persisted role is still service, DBA, or system remain classified as privileged so deletion cannot silently downgrade a target ID into regular-user permissions.

## Consequences

### Positive

- DBA, service, system, and regular users all see the same per-subject isolation on USER and STREAM tables.
- Privileged DML can act on another user's USER-table or STREAM-table rows only through authorized, audited `EXECUTE AS '<user_id>'`.
- File download `user_id` query parameters cannot bypass the caller identity or role matrix.
- Impersonation authorization no longer performs a per-request `system.users` lookup for regular targets.
- Tests assert both isolation surfaces: direct USER-table and STREAM-table access is scoped, and explicit `EXECUTE AS '<user_id>'` follows the role matrix.

### Negative

- Background workflows that need to create data for another user must use an account whose role can target that user through `EXECUTE AS '<user_id>'`.
- Historical specs and demos that described implicit privileged USER-table scans are superseded by this policy.

### Operational implications

- A running server must be restarted after this policy change; old binaries may still allow cross-user `EXECUTE AS '<user_id>'`.
- Smoke tests should be run against a fresh server when validating this behavior locally.
