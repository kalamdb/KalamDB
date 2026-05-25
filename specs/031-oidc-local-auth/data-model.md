# Data Model: Unified OIDC and Local Authentication

## Auth Configuration

Represents the complete authentication policy for a KalamDB deployment.

**Fields**:
- `local`: Local authentication policy.
- `oidc`: Optional single OIDC provider configuration.
- `token`: Internal token configuration for KalamDB-issued local sessions.
- `login_options_cache`: Public client metadata cache policy for login capability discovery.

**Validation Rules**:
- Configuration must be rooted under `[auth]`.
- At most one OIDC provider may be configured.
- Local authentication and OIDC authentication may both be enabled, only one may be enabled, or both may be disabled only when startup/setup policy explicitly allows an unauthenticated bootstrap state.
- Legacy provider-specific config keys must produce migration diagnostics.

**Relationships**:
- Owns one `Local Auth Policy`.
- Owns zero or one `OIDC Provider`.
- Produces `Auth Login Options` for Admin UI and CLI.

## Local Auth Policy

Represents whether KalamDB accepts username/password credentials directly.

**Fields**:
- `enabled`: Whether local username/password login is accepted.
- `setup_allowed`: Whether initial local admin setup is permitted.
- `password_policy`: Existing password rules for local users.

**Validation Rules**:
- If `enabled = false`, username/password login endpoints must reject attempts regardless of account state.
- If `enabled = true`, existing password hashing, lockout, rate limiting, and generic error behavior remain mandatory.

**State Transitions**:
- `enabled` to `disabled`: Local users remain stored, but cannot authenticate with passwords.
- `disabled` to `enabled`: Existing local users can authenticate again if not locked or deleted.

## OIDC Provider

Represents the single external identity provider trusted by the deployment.

**Fields**:
- `enabled`: Whether external OIDC login is available.
- `issuer`: Expected token issuer.
- `client_id`: Expected audience/client identifier.
- `scopes`: Requested scopes, including `openid`.
- `authorization_endpoint`: Optional override when discovery is unavailable.
- `token_endpoint`: Optional override when discovery is unavailable.
- `device_authorization_endpoint`: Optional endpoint for headless CLI login.
- `device_flow_mode`: Whether clients may use direct provider device flow, KalamDB-brokered device flow, or both.
- `redirect_uris`: Admin UI and CLI redirect expectations.
- `auto_provision`: Whether first-time external identities can create users automatically.
- `default_role`: Role assigned to auto-provisioned users.

**Validation Rules**:
- `issuer` and `client_id` are required when OIDC is enabled.
- `openid` scope is required for OpenID Connect login.
- Token validation must require matching issuer, expected audience, supported signing algorithm, valid lifetime, and subject claim.
- Device login is available only when a device authorization endpoint is configured or discovered.
- Direct device login requires the CLI host to reach the provider device authorization and token endpoints.
- Brokered device login requires the KalamDB server to reach the provider endpoints while the CLI reaches only the KalamDB server.

**Relationships**:
- Validated tokens map to `External Identity User` records.
- Public, non-secret fields appear in `Auth Login Options`.

## External Identity User

Represents a KalamDB user authenticated through OIDC.

**Fields**:
- `user_id`: Stable KalamDB user identifier.
- `issuer`: OIDC issuer that authenticated the user.
- `subject`: Provider subject claim.
- `email`: Optional email claim.
- `display_name`: Optional display claim.
- `role`: KalamDB role.
- `auth_type`: External/OIDC auth marker.
- `created_at`, `updated_at`, `last_login_at`: Audit timestamps.

**Validation Rules**:
- The pair `(issuer, subject)` must map to exactly one KalamDB user.
- Repeated valid logins for the same `(issuer, subject)` must reuse the same user.
- Existing local username/email collisions must not merge identities without an explicit mapping policy.

## Local Credential User

Represents a KalamDB user that can authenticate with a username/password when local authentication is enabled.

**Fields**:
- Existing local user fields: username/user ID, password hash, role, lockout state, deleted marker, timestamps.

**Validation Rules**:
- Password hashes remain required for local login.
- Local users cannot authenticate with passwords when local authentication is disabled.
- Security-sensitive local login failures must remain generic.

## Auth Login Options

Public client-facing view of available login methods.

**Fields**:
- `local.enabled`: Whether username/password login should be shown.
- `oidc.enabled`: Whether external login should be shown.
- `oidc.display_name`: Provider display label.
- `oidc.issuer`: Provider issuer.
- `oidc.client_id`: Public client identifier.
- `oidc.authorization_endpoint`: Browser login endpoint.
- `oidc.token_endpoint`: Public-client token endpoint when available.
- `oidc.device_authorization_endpoint`: Device-flow endpoint when available.
- `oidc.device_flow`: Public device-flow modes and KalamDB broker endpoints.
- `oidc.scopes`: Requested scopes.
- `oidc.redirect_uris`: Client redirect expectations.

**Validation Rules**:
- Must not expose secrets, private keys, password hashes, or server-only signing configuration.
- Must reflect server-side policy exactly.

## CLI Login Session

Represents an in-progress CLI OIDC login.

**Fields**:
- `mode`: Browser or device.
- `state`: CSRF protection value for browser flow.
- `pkce_verifier`: Browser-flow PKCE verifier.
- `device_code`: Provider-issued device code for headless flow.
- `broker_session_id`: Random KalamDB-issued handle for brokered device flow.
- `user_code`: Provider-issued user code shown to the user.
- `verification_uri`: URL shown to the user.
- `verification_uri_complete`: URL containing the user code when provided by the provider.
- `expires_at`: Provider-issued expiration.
- `poll_interval`: Provider-issued polling interval.

**State Transitions**:
- `Created` -> `WaitingForProvider` -> `Authenticated` when token exchange succeeds.
- `Created` -> `WaitingForProvider` -> `Expired` when provider expiration is reached.
- `Created` -> `WaitingForProvider` -> `Cancelled` when the user aborts.
- `WaitingForProvider` -> `Failed` on denied access, invalid state, or invalid token.

## OIDC Device Broker Session

Represents a short-lived server-side device authorization attempt for CLI hosts that cannot reach the IdP directly.

**Fields**:
- `session_id`: Random opaque handle returned to the CLI.
- `provider_device_code`: Provider-issued device code, stored only server-side.
- `user_code`: Provider-issued user code displayed by the CLI.
- `verification_uri`: Provider verification URI displayed by the CLI.
- `verification_uri_complete`: Provider verification URI with embedded code when available.
- `expires_at`: Provider-issued expiration.
- `poll_interval`: Provider-issued minimum polling interval.
- `last_poll_at`: Last KalamDB-side provider poll time.
- `status`: Pending, authorized, denied, expired, or failed.
- `token_result`: KalamDB-issued session result after successful OIDC token validation and identity mapping.

**Validation Rules**:
- Broker sessions are stored in bounded in-memory state with TTL cleanup; they are not durable credentials.
- `provider_device_code` is never returned to the CLI or logs.
- Polling honors provider `interval`, `slow_down`, expiration, and denial responses.
- Successful sessions return a KalamDB session token or accepted bearer token according to the final auth token strategy, not a provider password or raw secret bundle.

**State Transitions**:
- `Pending` -> `Authorized` when provider token exchange succeeds and KalamDB identity mapping succeeds.
- `Pending` -> `Denied` when provider reports denied access.
- `Pending` -> `Expired` when provider expiration is reached.
- `Pending` -> `Failed` on unrecoverable provider or validation errors.

## Admin UI Login Session

Represents an in-progress browser OIDC login from the Admin UI.

**Fields**:
- `state`: CSRF protection value.
- `nonce`: OIDC replay protection value.
- `pkce_verifier`: PKCE verifier stored only for the login attempt.
- `return_to`: Safe post-login route.
- `expires_at`: Local expiration for the login attempt.

**State Transitions**:
- `Created` -> `RedirectedToProvider` -> `Authenticated` when callback and token validation succeed.
- `RedirectedToProvider` -> `Failed` on invalid state, provider error, invalid token, or timeout.
