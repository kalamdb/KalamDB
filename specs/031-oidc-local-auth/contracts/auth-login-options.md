# Contract: Auth Login Options API

## Endpoint

```text
GET /v1/api/auth/login-options
```

## Authentication

No authentication required. The response contains only public login capability metadata.

## Success Response

```json
{
  "local": {
    "enabled": true
  },
  "oidc": {
    "enabled": true,
    "display_name": "Company SSO",
    "issuer": "https://idp.example.com/realms/kalamdb",
    "client_id": "kalamdb-admin",
    "authorization_endpoint": "https://idp.example.com/realms/kalamdb/protocol/openid-connect/auth",
    "token_endpoint": "https://idp.example.com/realms/kalamdb/protocol/openid-connect/token",
    "device_authorization_endpoint": "https://idp.example.com/realms/kalamdb/protocol/openid-connect/auth/device",
    "device_flow": {
      "direct_provider": true,
      "server_brokered": true,
      "broker_start_endpoint": "/v1/api/auth/oidc/device/start",
      "broker_poll_endpoint": "/v1/api/auth/oidc/device/poll"
    },
    "scopes": ["openid", "email", "profile"],
    "admin_redirect_uri": "https://kalamdb.example.com/ui/oauth/callback",
    "cli_redirect_uri": "http://127.0.0.1:8787/callback"
  }
}
```

## Local-Only Response

```json
{
  "local": {
    "enabled": true
  },
  "oidc": {
    "enabled": false
  }
}
```

## OIDC-Only Response

```json
{
  "local": {
    "enabled": false
  },
  "oidc": {
    "enabled": true,
    "display_name": "Company SSO",
    "issuer": "https://idp.example.com/realms/kalamdb",
    "client_id": "kalamdb-admin",
    "authorization_endpoint": "https://idp.example.com/realms/kalamdb/protocol/openid-connect/auth",
    "token_endpoint": "https://idp.example.com/realms/kalamdb/protocol/openid-connect/token",
    "device_authorization_endpoint": null,
    "device_flow": {
      "direct_provider": false,
      "server_brokered": false,
      "broker_start_endpoint": null,
      "broker_poll_endpoint": null
    },
    "scopes": ["openid", "email", "profile"],
    "admin_redirect_uri": "https://kalamdb.example.com/ui/oauth/callback",
    "cli_redirect_uri": "http://127.0.0.1:8787/callback"
  }
}
```

## Error Behavior

- If provider discovery is temporarily unavailable, the endpoint returns the last valid cached public metadata when available.
- If no valid metadata exists, OIDC is reported as unavailable with a non-secret operator-safe error code.
- The endpoint must never expose client secrets, JWT signing secrets, password hashes, private keys, or provider refresh tokens.

## Client Expectations

- Admin UI hides username/password controls when `local.enabled = false`.
- Admin UI shows external login when `oidc.enabled = true` and `authorization_endpoint` is available.
- CLI browser login requires `oidc.enabled = true`, `authorization_endpoint`, `token_endpoint`, and `cli_redirect_uri`.
- CLI direct headless login requires `oidc.enabled = true`, `device_flow.direct_provider = true`, `device_authorization_endpoint`, and `token_endpoint`, and it requires the CLI host to reach the provider.
- CLI no-IdP-egress headless login requires `oidc.enabled = true`, `device_flow.server_brokered = true`, `broker_start_endpoint`, and `broker_poll_endpoint`; the CLI only talks to KalamDB while the server talks to the provider.
