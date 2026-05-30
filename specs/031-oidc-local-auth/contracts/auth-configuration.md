# Contract: Unified `[auth]` Configuration

## Purpose

Define the user-facing server configuration contract for local username/password authentication and one OIDC provider.

## Accepted Shape

```toml
[auth]
jwt_secret = "replace-with-strong-secret"
jwt_expiration_hours = 24

[auth.local]
enabled = true
allow_initial_setup = true

[auth.oidc]
enabled = true
display_name = "Company SSO"
issuer = "https://idp.example.com/realms/kalamdb"
client_id = "kalamdb-admin"
scopes = ["openid", "email", "profile"]
auto_provision = true
default_role = "user"

# Optional overrides when discovery is unavailable or incomplete.
authorization_endpoint = "https://idp.example.com/realms/kalamdb/protocol/openid-connect/auth"
token_endpoint = "https://idp.example.com/realms/kalamdb/protocol/openid-connect/token"
device_authorization_endpoint = "https://idp.example.com/realms/kalamdb/protocol/openid-connect/auth/device"

# Registered with the provider.
admin_redirect_uri = "https://kalamdb.example.com/ui/oauth/callback"
cli_redirect_uri = "http://127.0.0.1:8787/callback"
```

`[auth.local]` may be omitted only if the implementation keeps the existing default local-auth behavior. New public examples and environment overrides must use `[auth.local]`.

`[auth.oidc]` may be omitted when a deployment uses local authentication only.

## Rejected Shapes

These old provider-specific shapes are no longer accepted as active configuration:

```toml
[oauth]

[oauth.providers.google]
[oauth.providers.github]
[oauth.providers.azure]
[oauth.providers.firebase]
[oauth.providers.openid]
```

Configuration validation must produce a migration diagnostic that points to `[auth.oidc]`.

## Validation Rules

- `auth.local.enabled = false` disables all username/password login attempts.
- `auth.oidc.enabled = true` requires `issuer`, `client_id`, and a scope list containing `openid`.
- `auth.oidc.device_authorization_endpoint` is optional, but `kalam login --oidc --no-browser` is available only when this endpoint is configured or discovered from provider metadata.
- Only one OIDC provider may be configured.
- Secrets must not be exposed through public login metadata.
- Old provider-specific sections must not silently remain active.
- Environment variable overrides must map to the unified `[auth]` keys only.

## Migration Notes

- Provider-specific issuer and audience values move into `[auth.oidc]`.
- Existing local users remain stored; the local policy only controls whether password login is accepted.
- Existing OIDC users remain mapped by issuer and subject.
