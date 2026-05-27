# Contract: OIDC Device Broker API

## Purpose

Allow headless CLI login when the CLI host can reach KalamDB but cannot reach the OIDC provider directly. KalamDB brokers only the standard OAuth 2.0 Device Authorization Grant using `openidconnect`; it does not invent a custom provider protocol.

## Start Device Login

```text
POST /v1/api/auth/oidc/device/start
```

Authentication is not required. The endpoint is rate-limited and available only when `[auth.oidc]` is enabled and a device authorization endpoint is configured or discovered.

### Success Response

```json
{
  "device_session_id": "opaque-random-session-handle",
  "verification_uri": "https://idp.example.com/device",
  "verification_uri_complete": "https://idp.example.com/device?user_code=ABCD-EFGH",
  "user_code": "ABCD-EFGH",
  "expires_in_seconds": 600,
  "interval_seconds": 5
}
```

KalamDB stores the provider `device_code` only server-side. The response must not include client secrets, provider device codes, refresh tokens, or raw provider token responses.

## Poll Device Login

```text
POST /v1/api/auth/oidc/device/poll
```

### Request

```json
{
  "device_session_id": "opaque-random-session-handle"
}
```

### Pending Response

```json
{
  "status": "pending",
  "interval_seconds": 5
}
```

### Authorized Response

```json
{
  "status": "authorized",
  "token_type": "bearer",
  "access_token": "kalamdb-session-token-or-accepted-bearer-token",
  "expires_at": "2026-05-25T20:30:00Z",
  "user": {
    "id": "usr_...",
    "role": "dba"
  }
}
```

### Terminal Error Response

```json
{
  "status": "expired",
  "message": "Device login expired. Start a new login attempt."
}
```

Valid terminal statuses are `authorized`, `denied`, `expired`, and `failed`. Messages must be non-secret and safe to show in CLI output.

## Server Behavior

- Build the provider client with `CoreProviderMetadata::discover_async` and `CoreClient::from_provider_metadata`.
- Read `device_authorization_endpoint` from provider metadata using custom additional provider metadata, matching the upstream `okta_device_grant` example, or from `[auth.oidc].device_authorization_endpoint` when configured.
- Start provider device flow with `CoreClient::set_device_authorization_url(...).exchange_device_code().request_async(...)`.
- Poll with `CoreClient::exchange_device_access_token(&details).request_async(..., sleep_fn, timeout)` or an equivalent interval-aware loop that preserves provider `authorization_pending`, `slow_down`, denial, and expiration semantics.
- Verify the returned ID token with `client.id_token_verifier()` and the expected issuer/audience before mapping or provisioning a KalamDB user.
- Store broker sessions in bounded memory with TTL cleanup; restart may invalidate in-progress device logins.

## CLI Behavior

- Prefer brokered mode when `--no-browser` is used and the CLI cannot reach the provider directly, or when login options indicate brokered mode is the only device-flow option.
- Print `verification_uri_complete` when present; otherwise print `verification_uri` and `user_code`.
- Poll KalamDB at the returned interval and stop on authorization, denial, expiration, user interrupt, or server error.
- Store only the final KalamDB-accepted token in the existing credentials store.