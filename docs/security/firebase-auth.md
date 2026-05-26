# Firebase Authentication Through OIDC

Firebase can be used with KalamDB through the standard OIDC path. KalamDB does not keep Firebase-specific provider code in the active auth model; Firebase is configured as the single `[auth.oidc]` issuer for the server.

## Configuration

Configure the Firebase project issuer and client values under `[auth.oidc]`:

```toml
[auth]
jwt_secret = "replace-with-a-strong-random-secret-at-least-32-chars"
jwt_trusted_issuers = "kalamdb,https://securetoken.google.com/my-project-id"
jwt_expiry_hours = 24
allow_remote_setup = false
cookie_secure = true

[auth.local]
enabled = true

[auth.oidc]
enabled = true
display_name = "Firebase"
issuer = "https://securetoken.google.com/my-project-id"
client_id = "my-project-id"
scopes = ["openid", "email", "profile"]
auto_provision = true
default_role = "user"
```

Replace `my-project-id` with the Firebase project ID. Include `kalamdb` in `jwt_trusted_issuers` so internally issued KalamDB tokens continue to verify.

## Login Discovery

Clients should discover the active login policy with:

```text
GET /v1/api/auth/login-options
```

The response tells the Admin UI and CLI whether local login is enabled and which OIDC authorization, token, and device-flow endpoints are available. Do not hard-code provider lists in clients.

## Browser Login

The Admin UI uses Authorization Code with PKCE:

1. Fetch login options from KalamDB.
2. Redirect the user to the configured Firebase/OIDC authorization endpoint.
3. Receive the callback at `/ui/oauth/callback`.
4. Send the authorization code, redirect URI, and PKCE verifier to `/v1/api/auth/oidc/exchange-code`.
5. Store the returned KalamDB access and refresh tokens for the session.

KalamDB exchanges the code server-side, then verifies issuer, audience, signature, and expiry with the `openidconnect` crate before mapping the identity to a KalamDB user.

## User Mapping

KalamDB uses the Firebase ID token `sub` claim directly as the KalamDB `user_id`. The same value is used for SQL identity, `system.users.user_id`, and PG extension `EXECUTE AS USER '<user_id>'` workflows.

Regular Firebase/OIDC users with role `user` can be stateless when `[auth.oidc].auto_provision = true`: KalamDB checks for an existing `system.users` row first, then authenticates an absent row as a regular user without creating one. Persisted `system.users` rows are needed for explicit local policy, such as service, DBA, or system roles, deleted tombstones, or deployments with auto-provisioning disabled.

The stored auth data shape for those overrides is generic OIDC data:

```json
{
  "issuer": "https://securetoken.google.com/my-project-id",
  "subject": "firebase-user-subject"
}
```

If auto-provisioning is disabled, or if the user needs a non-`user` role, create the user before first login. The user id must match the Firebase `sub` claim:

```sql
CREATE USER 'firebase-user-subject'
  WITH OIDC '{"issuer":"https://securetoken.google.com/my-project-id","subject":"firebase-user-subject"}'
  ROLE service
  EMAIL 'alice@example.com';
```

## Security Notes

- Keep `[auth.oidc].auto_provision` disabled unless every user in the Firebase project should be allowed to access KalamDB as a regular user.
- Use `default_role = "user"` unless the deployment has a tightly controlled IdP group; elevated OIDC users should be explicit local overrides.
- Set `allow_remote_setup = false` after bootstrap.
- Use TLS and `cookie_secure = true` in production.
- Rotate `jwt_secret` if it was ever committed or shared outside the deployment secret store.
