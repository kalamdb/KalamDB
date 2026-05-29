# OIDC Authentication Architecture

KalamDB supports two authentication paths through one auth surface:

- Local username/password login, controlled by `[auth.local]`.
- One external OpenID Connect provider, controlled by `[auth.oidc]`.

Internal KalamDB access and refresh tokens are HS256 JWTs issued by KalamDB. External identity tokens are OIDC ID tokens verified with the `openidconnect` crate against the configured issuer metadata and signing keys.

## Configuration

All authentication settings live under `[auth]` in `server.toml`. The legacy authentication alias can still deserialize existing local files, but new examples and docs should use `[auth]`. Legacy split OAuth tables and provider-specific OAuth subtables are rejected with migration guidance.

```toml
[auth]
jwt_secret = "replace-with-a-strong-random-secret-at-least-32-chars"
jwt_trusted_issuers = "kalamdb,https://idp.example.com/realms/kalamdb"
jwt_expiry_hours = 24
allow_remote_setup = false
cookie_secure = true

[auth.local]
enabled = true

[auth.oidc]
enabled = true
display_name = "Company SSO"
issuer = "https://idp.example.com/realms/kalamdb"
client_id = "kalamdb"
client_secret = "optional-confidential-client-secret"
scopes = ["openid", "email", "profile"]
auto_provision = true
default_role = "user"
broker_device_flow_enabled = true
# Optional override when provider discovery does not advertise device flow.
device_authorization_endpoint = "https://idp.example.com/realms/kalamdb/protocol/openid-connect/auth/device"
```

`auth.local.enabled` is authoritative. When it is `false`, password login and password setup are rejected server-side, and clients should hide local username/password controls.

## Public Login Metadata

Clients discover available login methods with:

```text
GET /v1/api/auth/login-options
```

The response exposes only public, no-secret metadata:

```json
{
  "local": { "enabled": true },
  "oidc": {
    "enabled": true,
    "display_name": "Company SSO",
    "issuer": "https://idp.example.com/realms/kalamdb",
    "client_id": "kalamdb",
    "authorization_endpoint": "https://idp.example.com/.../auth",
    "token_endpoint": "https://idp.example.com/.../token",
    "device_authorization_endpoint": "https://idp.example.com/.../device",
    "scopes": ["openid", "email", "profile"],
    "device_flow": {
      "direct_supported": true,
      "broker_supported": true,
      "broker_start_endpoint": "/v1/api/auth/oidc/device/start",
      "broker_poll_endpoint": "/v1/api/auth/oidc/device/poll"
    }
  }
}
```

The old plural provider endpoint is not part of the active auth surface.

## Browser Flow

The Admin UI uses Authorization Code with PKCE:

1. Fetch `/v1/api/auth/login-options`.
2. Generate PKCE verifier/challenge, nonce, and state in browser storage.
3. Redirect to the configured authorization endpoint.
4. Handle `/ui/oauth/callback`, validate state, and send the authorization code, redirect URI, and PKCE verifier to `/v1/api/auth/oidc/exchange-code`.
5. Store the returned KalamDB access and refresh tokens, then use the KalamDB access token for normal API calls.

KalamDB performs the provider token exchange server-side through `openidconnect`, verifies the external ID token, maps or provisions a KalamDB user, and returns the normal login response. Admin UI access still requires the resulting user to have `dba` or `system` role.

## CLI Flow

The CLI supports three login modes:

- Local login: `kalam login` when `[auth.local].enabled = true`.
- Browser OIDC: `kalam login --oidc` using Authorization Code with PKCE.
- Headless OIDC: `kalam login --oidc --no-browser`, using direct provider device flow when available or `--brokered` for KalamDB-brokered device flow.

Browser OIDC sends the callback code to `/v1/api/auth/oidc/exchange-code`. Direct headless device flow obtains a provider ID token and sends it to `/v1/api/auth/oidc/exchange-token`. Both modes save KalamDB access and refresh tokens, not provider credentials.

When `kalam login` runs from an interactive terminal, the CLI continues directly into the normal SQL shell after a successful local or OIDC login. When stdin or stdout is not a terminal, it keeps the prior one-shot behavior: save credentials if requested, print the login result, and exit so scripts do not hang.

The brokered device flow keeps the provider device code server-side. The CLI only polls KalamDB until the broker returns KalamDB access and refresh tokens.

## Token Verification

Bearer authentication peeks at the JWT algorithm and issuer without trusting the token yet:

1. If issuer is `kalamdb`, the token must use HS256 and is verified with `auth.jwt_secret`.
2. If issuer is external, it must be configured in `[auth.oidc]` and trusted by `auth.jwt_trusted_issuers`.
3. External tokens must use an asymmetric OIDC algorithm supported by `openidconnect`.
4. Provider metadata and verifiers are discovered through `openidconnect::CoreProviderMetadata::discover_async` and cached per issuer.
5. Audience, issuer, signature, expiry, and nonce checks are handled by `openidconnect` verifier APIs.

Refresh tokens are never accepted as API access tokens.

## User Identity

External users use the OIDC subject claim directly as the KalamDB `user_id`. The same value appears in `system.users.user_id`, `CURRENT_USER()`, and PG extension `EXECUTE AS USER '<user_id>'` workflows.

The subject must be a valid KalamDB `UserId`: ASCII letters, digits, `_`, or `-`, up to 128 characters. Providers that emit subjects with spaces, slashes, quotes, or other unsafe characters need a stable subject transform at the IdP layer before they are used with KalamDB.

Changing the configured OIDC issuer or provider can produce different subject values. In that case KalamDB treats the login as a different user because the authenticated `sub` changed.

External-token authentication checks for an existing `system.users` row with `user_id = sub` before using any stateless regular-user fallback. If a local password user has the same ID, OIDC authentication is rejected instead of silently crossing auth modes.

`system.users.auth_data` stores the linked OIDC issuer and subject for persisted OIDC users. For persisted OIDC users, the `subject` must match the row's `user_id`:

```json
{
  "issuer": "https://idp.example.com/realms/kalamdb",
  "subject": "provider-subject"
}
```

There is no provider-family enum in the active identity model. A specific IdP such as Dex, Keycloak, Firebase, Okta, or Entra ID is just an OIDC issuer.

OIDC invitations are stored in `system.users` as pending rows with `auth_type = "oidc_invite"`, a synthetic `invite_<hash>` `user_id`, the invited email, requested role, `invite_expires_at`, and `invited_by`. They are not valid login users. During OIDC login, if no persisted `user_id = sub` row exists, KalamDB checks the token email against active pending invites. A matching unexpired invite creates a real `auth_type = "oidc"` user with `user_id = sub`, copies the invited role and storage preferences, records the issuer/subject link in `auth_data`, and soft-deletes the invite row.

## Auto-Provisioning

When `[auth.oidc].auto_provision = true` and `[auth.oidc].default_role = "user"`, valid external users authenticate as regular users without creating per-user rows. A persisted row is still checked first so deleted OIDC users, elevated OIDC users, and same-ID local password users keep their explicit local policy.

If the default role is elevated, auto-provisioning creates a persisted OIDC user row with `user_id = sub`. If auto-provisioning is disabled, users must already have a persisted OIDC row or an active email invite before first login.

When auto-provisioning is disabled, either create the OIDC user explicitly before first login or create an email invite. Canonical SQL uses `WITH OIDC`; `WITH OAUTH` remains only as a compatibility alias.

```sql
CREATE USER 'provider-subject'
  WITH OIDC '{"issuer":"https://idp.example.com/realms/kalamdb","subject":"provider-subject"}'
  ROLE service
  EMAIL 'alice@example.com';

CREATE USER INVITE 'alice@example.com'
  ROLE dba
  EXPIRES_AT 1770000000000;
```

## Operational Notes

- Set a strong `auth.jwt_secret` in every non-local deployment.
- Include `kalamdb` and the configured OIDC issuer in `auth.jwt_trusted_issuers`.
- Configure the IdP redirect URI for the Admin UI callback: `https://your-host/ui/oauth/callback`.
- Enable TLS at the edge and set `auth.cookie_secure = true` in production.
- Use the cleanup guards to keep legacy provider sections and custom JWKS code out of active paths.
