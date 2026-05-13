# Firebase Authentication with KalamDB

KalamDB supports **Firebase Authentication** as an external OIDC identity provider.
Clients authenticate with Firebase, receive a Firebase ID token (RS256-signed JWT), and pass it directly to KalamDB as a Bearer token.
KalamDB verifies the token against Firebase's public JWKS endpoint and either looks up or auto-provisions the corresponding KalamDB user.

---

## How it works

```
Client app                 Firebase               KalamDB
   │                          │                       │
   │── sign in ──────────────►│                       │
   │◄─ Firebase ID token ─────│                       │
   │                          │                       │
   │── Authorization: Bearer <id-token> ─────────────►│
   │                          │   verify RS256 sig     │
   │                          │   via JWKS endpoint    │
   │                          │◄──────────────────────►│
   │                          │  resolve / provision   │
   │                          │  KalamDB user          │
   │◄── SQL response ─────────────────────────────────│
```

Firebase ID tokens are:
- Signed with **RS256** using Firebase's per-project key pair
- Issued with `iss = https://securetoken.google.com/{PROJECT_ID}`
- Scoped to a single Firebase project via `aud = {PROJECT_ID}`
- Short-lived (1 hour by default)

KalamDB handles key rotation automatically through the JWKS endpoint — no manual key management is required.

---

## Prerequisites

1. A Firebase project in the [Firebase Console](https://console.firebase.google.com/).
2. At least one Firebase Auth sign-in method enabled (Email/Password, Google Sign-In, GitHub, phone, anonymous, etc.).
3. KalamDB 0.5+ (the `[oauth.providers.firebase]` config key and `OAuthProvider::Firebase` variant are required).

---

## Server configuration (`server.toml`)

### Minimal setup

```toml
[authentication]
# Must include the Firebase project issuer
jwt_trusted_issuers = "https://securetoken.google.com/YOUR_PROJECT_ID"

# Auto-create KalamDB users on first successful Firebase login
auto_create_users_from_provider = true

[oauth]
enabled = true
auto_provision = true
default_role = "user"

[oauth.providers.firebase]
enabled = true
# Issuer must match the aud (project ID) in your Firebase tokens
issuer = "https://securetoken.google.com/YOUR_PROJECT_ID"
# Firebase's public JWKS endpoint (static URL, no discovery needed)
jwks_uri = "https://www.googleapis.com/service_accounts/v1/jwk/securetoken@system.gserviceaccount.com"
# client_id must equal the Firebase project ID (the aud claim in Firebase ID tokens)
client_id = "YOUR_PROJECT_ID"
```

Replace `YOUR_PROJECT_ID` with your actual Firebase project ID (visible in **Project Settings → General** in the Firebase Console).

### Multiple OAuth providers

You can trust Firebase alongside other providers:

```toml
[authentication]
jwt_trusted_issuers = "https://securetoken.google.com/my-app,https://accounts.google.com"
auto_create_users_from_provider = true

[oauth]
enabled = true
auto_provision = true

[oauth.providers.firebase]
enabled = true
issuer = "https://securetoken.google.com/my-app"
jwks_uri = "https://www.googleapis.com/service_accounts/v1/jwk/securetoken@system.gserviceaccount.com"
client_id = "my-app"

[oauth.providers.google]
enabled = true
issuer = "https://accounts.google.com"
jwks_uri = "https://www.googleapis.com/oauth2/v3/certs"
client_id = "your-google-oauth-client-id.apps.googleusercontent.com"
```

### Environment variable override

If you prefer environment variables:

```sh
KALAMDB_JWT_TRUSTED_ISSUERS="https://securetoken.google.com/YOUR_PROJECT_ID" \
  cargo run
```

---

## User provisioning

When a Firebase user authenticates for the first time KalamDB needs a corresponding local user record.
Two modes are available:

### Auto-provision (recommended for most apps)

Set `auto_create_users_from_provider = true` in `[authentication]` (or `auto_provision = true` in `[oauth]`). KalamDB will:

1. Derive a deterministic username: `oidc:fbs:{firebase-uid}` (prefix `fbs` = Firebase).
2. Create a new KalamDB user with the role specified by `[oauth].default_role` (default: `"user"`).
3. Populate `email` from the `email` claim if present in the token.

The user is created once and reused on subsequent logins.

### Manual provision

If you manage users explicitly, create the Firebase user in KalamDB before their first login:

```sql
-- As a DBA or system user:
CREATE USER WITH
  USERNAME 'oidc:fbs:FIREBASE_UID'
  EMAIL    'user@example.com'
  ROLE     user
  OAUTH;
```

The username **must** follow the `oidc:fbs:{subject}` format, where `{subject}` is the Firebase UID (`sub` claim in the ID token).

---

## Client integration

### Web (JavaScript / TypeScript)

```ts
import { getAuth, signInWithEmailAndPassword } from "firebase/auth";
import { initializeApp } from "firebase/app";

const app = initializeApp({ /* your Firebase config */ });
const auth = getAuth(app);

async function queryKalamDB(sql: string) {
  const user = auth.currentUser;
  if (!user) throw new Error("Not signed in");

  // Firebase ID tokens expire after 1 hour; getIdToken() auto-refreshes
  const idToken = await user.getIdToken();

  const response = await fetch("https://your-kalamdb-host/v1/api/sql", {
    method: "POST",
    headers: {
      "Content-Type": "application/json",
      "Authorization": `Bearer ${idToken}`,
    },
    body: JSON.stringify({ query: sql }),
  });

  return response.json();
}
```

### Flutter / Dart (using kalam-link)

```dart
import 'package:firebase_auth/firebase_auth.dart';
import 'package:kalam_link/kalam_link.dart';

Future<KalamClient> buildClient() async {
  final user = FirebaseAuth.instance.currentUser!;
  final idToken = await user.getIdToken();

  return KalamClient(
    host: 'your-kalamdb-host',
    port: 2900,
    token: idToken,
  );
}
```

### Rust

```rust
// After obtaining the Firebase ID token (e.g., via the Firebase REST API):
let client = reqwest::Client::new();
let response = client
    .post("https://your-kalamdb-host/v1/api/sql")
    .bearer_auth(&firebase_id_token)
    .json(&serde_json::json!({ "query": "SELECT * FROM myapp.items" }))
    .send()
    .await?;
```

### cURL

```sh
TOKEN="<paste-your-firebase-id-token>"

curl -X POST https://your-kalamdb-host/v1/api/sql \
  -H "Authorization: Bearer $TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"query": "SELECT 1"}'
```

---

## Token refresh

Firebase ID tokens expire after **1 hour**. Clients should call `getIdToken(/* forceRefresh */ false)` before each request; the Firebase SDK will transparently refresh the token when needed.

KalamDB does not issue refresh tokens for external OIDC providers — token lifecycle management stays entirely within Firebase.

---

## Internal username format

Firebase users are stored in KalamDB with the username:

```
oidc:fbs:{firebase-uid}
```

Examples:

| Firebase UID | KalamDB username |
|---|---|
| `abc123xyz` | `oidc:fbs:abc123xyz` |
| `UKHV8qpT...` | `oidc:fbs:UKHV8qpT...` |

The prefix `fbs` is the unique 3-character code assigned to `OAuthProvider::Firebase`. This is stable and will not change.

---

## Granting roles to Firebase users

By default auto-provisioned Firebase users receive the `user` role. A DBA can upgrade a user after their first login:

```sql
-- Promote to dba:
ALTER USER 'oidc:fbs:FIREBASE_UID' SET ROLE dba;
```

---

## Security considerations

| Concern | Mitigation |
|---|---|
| Token forgery | KalamDB fetches and caches Firebase's JWKS, verifying the RS256 signature on every request (key cache auto-rotates). |
| Expired tokens | Standard JWT `exp` claim validation rejects tokens older than 1 hour. |
| Wrong project | The `aud` claim is validated against `client_id` (your project ID), preventing tokens from other Firebase projects. |
| Compromised Firebase account | Revoke the Firebase session server-side. Tokens are short-lived (1 h); no additional KalamDB action is needed. |
| Role escalation | Firebase tokens carry no KalamDB role claim. The role is always read from the KalamDB user record. |

---

## Troubleshooting

### `403 Forbidden: issuer 'https://securetoken.google.com/...' not trusted`

The issuer is not in `jwt_trusted_issuers`. Add it:

```toml
[authentication]
jwt_trusted_issuers = "https://securetoken.google.com/YOUR_PROJECT_ID"
```

### `404 User not found` / `401 Invalid credentials`

The Firebase user has no matching KalamDB user. Either:
- Enable `auto_create_users_from_provider = true`, **or**
- Manually create the user (`CREATE USER ... OAUTH`).

### `401 Token audience mismatch`

`client_id` in `[oauth.providers.firebase]` does not match your Firebase project ID. The `aud` claim in a Firebase ID token is always the project ID (e.g., `my-app`, not `my-app.firebaseapp.com`).

### `401 JWT signature verification failed`

Rare — JWKS key rotation. KalamDB caches JWKS keys and will try to re-fetch on a cache miss. If you see this persistently, ensure the `jwks_uri` is reachable from the server network.

### Inspecting a Firebase token

```sh
# Decode the payload (no signature verification — for debugging only)
echo "<YOUR_FIREBASE_TOKEN>" | cut -d. -f2 | base64 -d 2>/dev/null | jq .
```

Expected claims:

```json
{
  "iss": "https://securetoken.google.com/YOUR_PROJECT_ID",
  "aud": "YOUR_PROJECT_ID",
  "sub": "the-firebase-uid",
  "email": "user@example.com",
  "exp": 1234567890
}
```
