# Quickstart: Unified OIDC and Local Authentication

## 1. Configure Local and OIDC Login

Use only `[auth]` in `server.toml`.

```toml
[auth]
jwt_secret = "replace-with-strong-secret"
jwt_expiration_hours = 24

[auth.local]
enabled = false
allow_initial_setup = false

[auth.oidc]
enabled = true
display_name = "Dex"
issuer = "http://127.0.0.1:5556/dex"
client_id = "kalamdb-admin"
scopes = ["openid", "email", "profile"]
auto_provision = true
default_role = "dba"
admin_redirect_uri = "http://127.0.0.1:2900/ui/oauth/callback"
cli_redirect_uri = "http://127.0.0.1:8787/callback"
# Optional when provider discovery does not expose it.
device_authorization_endpoint = "http://127.0.0.1:5556/dex/device/code"
```

For local development with both login modes:

```toml
[auth.local]
enabled = true
allow_initial_setup = true

[auth.oidc]
enabled = true
```

## 2. Start Dex for Tests

Use the existing Dex/testcontainers setup for backend and CLI tests. The implementation should pin the Dex configuration used by the test harness so browser and device-flow behavior are deterministic.

For device-flow tests, prefer Dex when the selected Dex image exposes a device authorization endpoint. If the test image does not expose that endpoint, keep Dex for normal OIDC coverage and add a local device-flow fixture that speaks the standard OAuth 2.0 Device Authorization Grant contract exercised through `openidconnect`.

## 3. Run Focused Backend Validation

Confirm `openidconnect` is present only where OIDC protocol work is implemented and that feature selection stays minimal. The preferred dependency shape is `openidconnect = { version = "4.0.1", default-features = false }` plus a redirect-disabled adapter to KalamDB's workspace `reqwest` client:

```bash
cargo tree -i openidconnect
```

```bash
cargo nextest run -p kalamdb-server --test test_misc auth::test_oidc_auto_provision
```

Expected coverage:
- one OIDC provider configured under `[auth.oidc]`
- provider metadata loaded through `CoreProviderMetadata::discover_async` or explicit endpoint overrides
- ID tokens verified through `CoreClient::id_token_verifier`
- device authorization endpoint loaded from custom provider metadata or `[auth.oidc]`
- legacy provider-specific configuration rejected
- local login rejected when disabled
- valid Dex token accepted
- invalid issuer/audience/token rejected
- repeated external login maps to the same user
- brokered device sessions expire and never expose provider device codes

## 4. Run CLI Validation

```bash
cd cli
cargo nextest run --features e2e-tests auth_oidc
```

Expected coverage:
- `kalam login --local` works only when local auth is enabled
- `kalam login --oidc` completes browser login against Dex
- `kalam login --oidc --no-browser` completes direct device-code login against Dex/provider when the CLI can reach the provider
- `kalam login --oidc --no-browser` completes brokered device-code login by talking only to KalamDB while KalamDB reaches Dex/provider
- invalid external tokens are not stored

## 5. Run Admin UI Validation

```bash
cd ui
npm exec tsc -- --noEmit
npm exec vitest run src/components/auth/LoginForm.test.tsx src/store/authSlice.test.ts
```

Expected coverage:
- login form hides username/password when local login is disabled
- login form shows OIDC login when OIDC is configured
- callback rejects invalid state/token responses
- successful OIDC callback stores an authenticated session

## 6. Run Config and Documentation Checks

```bash
cargo check -p kalamdb-configs -p kalamdb-auth -p kalamdb-api --tests
./scripts/check-auth-config-docs.sh
./scripts/check-auth-oidc-cleanup.sh
```

Also update and review:
- `backend/server.example.toml`
- `docs/architecture/oidc-authentication.md`
- `docs/security/README.md`
- any Firebase/provider-specific docs that should be removed or redirected
- `../kalamdb-skills` auth and server-configuration references

The cleanup guard must prove the old provider-family path and custom KalamDB OIDC/JWKS implementation are not active anymore: no `/auth/oauth/providers` route, no `[oauth.providers.*]` accepted config, no `OidcValidator`/`OidcConfig::discover` custom validator path, and no `reqwest::get` OIDC discovery/JWKS fetches in `backend/crates/kalamdb-auth/src/oidc/`.

## 7. Migration Acceptance

A server configuration containing any of these old sections must fail validation with a migration message:

```toml
[oauth]
[oauth.providers.google]
[oauth.providers.github]
[oauth.providers.azure]
[oauth.providers.firebase]
[oauth.providers.openid]
```

The migration message should point to `[auth.oidc]` and explain that only one OIDC provider is supported at a time.
