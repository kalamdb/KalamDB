# Contract: CLI Authentication Flows

## Commands

```text
kalam login
kalam login --local
kalam login --oidc
kalam login --oidc --no-browser
kalam logout
kalam auth status
```

Exact flag names may follow the existing CLI style, but the user-facing capability set must match this contract.

## Default Login Behavior

1. CLI fetches `GET /v1/api/auth/login-options`.
2. If local and OIDC are both enabled, CLI offers the user a choice or honors an explicit flag.
3. If only local is enabled, CLI prompts for username/password.
4. If only OIDC is enabled, CLI starts OIDC login.
5. If local login is requested while disabled, CLI explains that local login is disabled and suggests OIDC login.

## Browser OIDC Flow

1. CLI fetches login options.
2. CLI builds an `openidconnect` `CoreClient` from discovered or server-provided metadata.
3. CLI generates state, nonce, and PKCE verifier/challenge using `openidconnect` helpers.
4. CLI opens the authorization URL produced by `CoreClient::authorize_url` in the default browser when possible.
5. CLI listens on the configured loopback redirect URI.
6. Provider redirects back with an authorization code.
7. CLI exchanges the code with `CoreClient::exchange_code(...).set_pkce_verifier(...).request_async(...)`.
8. CLI verifies the ID token with `client.id_token_verifier()` and validates the accepted token with KalamDB.
9. CLI stores the accepted token in the existing credentials store.

## Headless OIDC Flow

1. CLI fetches login options.
2. If the CLI can reach the provider and direct device flow is enabled, CLI builds an `openidconnect` `CoreClient`, calls `set_device_authorization_url`, starts with `exchange_device_code().request_async(...)`, prints `verification_uri_complete` or `verification_uri` plus `user_code`, and polls with `exchange_device_access_token(&details).request_async(..., sleep_fn, timeout)`.
3. If the CLI cannot reach the provider but can reach KalamDB, CLI starts the brokered flow with `POST /v1/api/auth/oidc/device/start`, prints the returned verification URL/code, and polls `POST /v1/api/auth/oidc/device/poll` at the returned interval.
4. On success, CLI validates or receives the KalamDB-accepted token and stores it in the existing credentials store.
5. On timeout, denial, provider error, or broker error, CLI exits with a clear non-secret message.

## Local Login Flow

1. CLI fetches login options.
2. If local auth is enabled, CLI prompts for username/password and calls the existing local login endpoint.
3. If local auth is disabled, CLI does not prompt for a password and reports that local login is disabled.

## Stored Credentials

- Store bearer tokens using the existing credential storage path and permissions.
- Do not store provider passwords.
- Store enough user/server metadata for `kalam auth status` to explain the current session.
- Preserve refresh-token behavior only when the provider returns one and the storage model can protect it according to existing credential rules.

## Test Expectations

- Local disabled: `kalam login --local` fails before password prompt or after server policy rejection with a clear message.
- Browser OIDC: CLI obtains a token from Dex and authenticated commands succeed.
- Headless OIDC direct: CLI completes the provider user-code flow against Dex when the configured Dex endpoint supports device authorization and the CLI can reach Dex.
- Headless OIDC brokered: CLI completes user-code login by talking only to KalamDB while KalamDB reaches Dex/provider endpoints.
- Invalid issuer/audience/token: CLI does not store credentials.
