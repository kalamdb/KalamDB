# Quickstart: Unified OIDC and Local Authentication

## 1. Configure Local and OIDC Login

Use only `[auth]` in `server.toml`.

```toml
[auth]
jwt_secret = "replace-with-strong-secret"
jwt_expiry_hours = 24
jwt_trusted_issuers = "kalamdb,http://127.0.0.1:5556"
allow_remote_setup = false

[auth.local]
enabled = false

[auth.oidc]
enabled = true
display_name = "Dex"
issuer = "http://127.0.0.1:5556"
client_id = "client"
scopes = ["openid", "email", "profile"]
auto_provision = true
default_role = "dba"
# Optional when provider discovery does not expose it.
device_authorization_endpoint = "http://127.0.0.1:5556/device/code"
broker_device_flow_enabled = true
```

For local development with both login modes:

```toml
[auth.local]
enabled = true

[auth.oidc]
enabled = true
```

## 2. Start Dex for Tests

Use the existing Dex/testcontainers setup for backend and CLI tests. The implementation should pin the Dex configuration used by the test harness so browser and device-flow behavior are deterministic.

For device-flow tests, prefer Dex when the selected Dex image exposes a device authorization endpoint. If the test image does not expose that endpoint, keep Dex for normal OIDC coverage and add a local device-flow fixture that speaks the standard OAuth 2.0 Device Authorization Grant contract exercised through `openidconnect`.

## 3. Run Focused Backend Validation

Confirm `openidconnect` is present only where OIDC protocol work is implemented and that feature selection stays minimal. The preferred dependency shape is `openidconnect = { version = "4.0.1", default-features = false, features = ["reqwest", "rustls-tls"] }`:

```bash
cargo fmt --all --check
cargo tree -i openidconnect
cargo check -p kalamdb-auth -p kalamdb-api -p kalamdb-system -p kalamdb-commons -p kalamdb-handlers-user --tests
```

```bash
for filter in test_oidc_token_validation test_dex_fresh_tokens_do_not_duplicate_user test_login_options_; do
	cargo nextest run -p kalamdb-server "$filter" --no-fail-fast
done
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
cargo test -p kalam-cli --features e2e-tests --test auth --no-fail-fast oidc_
```

Expected coverage:
- `kalam login --local` works only when local auth is enabled
- `kalam login --oidc` completes browser login against Dex and saves KalamDB access/refresh tokens from `/v1/api/auth/oidc/exchange-code`
- `kalam login --oidc --no-browser` completes direct device-code login against Dex/provider and saves KalamDB access/refresh tokens from `/v1/api/auth/oidc/exchange-token`
- `kalam login --oidc --no-browser` completes brokered device-code login by talking only to KalamDB while KalamDB reaches Dex/provider
- invalid external tokens are not stored

## 5. Run Admin UI Validation

```bash
cd ui
npm exec tsc -- --noEmit
npm exec vitest run src/components/auth/LoginForm.test.tsx src/lib/oauth.test.ts src/pages/OAuthCallback.test.tsx
npm run test:e2e -- tests/e2e/oidc-admin-auth.spec.ts
```

Expected coverage:
- login form hides username/password when local login is disabled
- login form shows OIDC login when OIDC is configured
- callback rejects invalid state/token responses
- successful OIDC callback exchanges through KalamDB and stores an authenticated session
- Playwright exercises the Admin UI button, local Dex login form, `/ui/oauth/callback`, and backend exchange contract

## 6. Run Config and Documentation Checks

```bash
./scripts/check-auth-config-docs.sh
./scripts/check-auth-oidc-cleanup.sh
```

Also update and review:
- `backend/server.example.toml`
- `docs/architecture/oidc-authentication.md`
- `docs/security/README.md`
- any Firebase/provider-specific docs that should be removed or redirected
- `../kalamdb-skills` auth and server-configuration references

The cleanup guard must prove the old provider-family path and custom KalamDB OIDC/JWKS implementation are not active anymore: no old plural provider metadata route, no provider-specific OAuth config accepted, no custom validator/discovery path, and no raw OIDC discovery/JWKS fetches in `backend/crates/kalamdb-auth/src/oidc/`.

## 7. Migration Acceptance

A server configuration containing the old split OAuth section or any old provider-specific OAuth section must fail validation with a migration message. The migration message should point to `[auth.oidc]` and explain that only one OIDC provider is supported at a time.
