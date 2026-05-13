# KalamDB Authentication Quickstart Guide

**Feature**: User Authentication & Authorization  
**Version**: 1.0.0  
**Last Updated**: October 28, 2025

---

## Table of Contents

1. [Overview](#overview)
2. [Database Initialization](#database-initialization)
3. [User Management](#user-management)
4. [Authentication Methods](#authentication-methods)
5. [Role-Based Access Control (RBAC)](#role-based-access-control-rbac)
6. [CLI Authentication](#cli-authentication)
7. [API Integration Examples](#api-integration-examples)
8. [Troubleshooting](#troubleshooting)

---

## Overview

KalamDB provides a comprehensive authentication and authorization system with:

- **Multiple Authentication Methods**: HTTP Basic Auth, JWT tokens, OAuth 2.0
- **Role-Based Access Control (RBAC)**: Four roles with hierarchical permissions
- **SQL-Based User Management**: `CREATE USER`, `ALTER USER`, `DROP USER`
- **Secure Password Storage**: bcrypt hashing (cost factor 12)
- **Shared Table Access Control**: Public, private, and restricted access levels
- **System User Isolation**: Localhost-only default with optional remote access

**Key Concepts**:
- **Authentication**: Verify identity (username + password → JWT token)
- **Authorization**: Check permissions (JWT token → user_id + role → access decision)
- **System User**: Special user created during initialization (localhost-only by default)
- **User Roles**: `user` (standard), `service` (automation), `dba` (admin), `system` (internal)

---

## Database Initialization

### First-Time Setup

When you first start KalamDB server or use the CLI, a **system user** is automatically created for localhost access:

**Server Startup**:
```bash
# Start KalamDB server
cargo run --bin kalamdb-server

# Output:
# [INFO] KalamDB server starting on http://127.0.0.1:2900
# [INFO] System user 'system' created with ID: sys_system
# [INFO] System user accessible from localhost without password
```

**CLI Initialization**:
```bash
# Initialize database and create system user
cargo run --bin kalam

# Output:
# KalamDB CLI v0.1.0
# System user 'system' initialized (localhost-only)
# Connected as: system (role: system)
kalam>
```

**System User Characteristics**:
- **Username**: `system`
- **User ID**: `sys_system`
- **Role**: `system` (full permissions)
- **Access**: Localhost only (127.0.0.1, ::1)
- **Password**: Not required for localhost connections
- **Remote Access**: Disabled by default (can be enabled with password)

### Verify System User

```sql
-- Check system user exists
SELECT user_id, username, role, auth_type, allow_remote, deleted_at 
FROM system.users 
WHERE username = 'system';

-- Expected output:
-- ┌─────────────┬──────────┬────────┬───────────┬──────────────┬────────────┐
-- │ user_id     │ username │ role   │ auth_type │ allow_remote │ deleted_at │
-- ├─────────────┼──────────┼────────┼───────────┼──────────────┼────────────┤
-- │ sys_system  │ system   │ system │ internal  │ false        │ NULL       │
-- └─────────────┴──────────┴────────┴───────────┴──────────────┴────────────┘
```

---

## User Management

### Creating Users

**All user management operations are SQL-based**. There are **NO REST API endpoints** for user CRUD operations.

#### Standard User Account

```sql
-- Create a regular user
CREATE USER 'alice' 
  WITH PASSWORD 'SecurePass123!' 
  ROLE 'user';

-- Output:
-- User 'alice' created with ID: usr_alice
```

**Password Requirements**:
- Minimum length: 8 characters
- Maximum length: 72 characters (bcrypt limitation)
- No complexity requirements (recommended: uppercase, lowercase, digits, symbols)
- Stored as bcrypt hash (cost factor 12)

#### Service Account (Automation)

```sql
-- Create a service account for automated processes
CREATE USER 'backup_service'
  WITH PASSWORD 'ServiceKey456!'
  ROLE 'service'
  EMAIL 'backup@company.com'
  METADATA '{"purpose": "nightly_backups", "team": "ops"}';

-- Output:
-- User 'backup_service' created with ID: svc_backup_service
```

#### Database Administrator

```sql
-- Create a DBA account with elevated privileges
CREATE USER 'db_admin'
  WITH PASSWORD 'AdminPass789!'
  ROLE 'dba'
  EMAIL 'admin@company.com';

-- Output:
-- User 'db_admin' created with ID: dba_db_admin
```

#### OAuth User (Google/GitHub/Azure)

```sql
-- Create user with OAuth authentication
CREATE USER 'john_doe'
  WITH OAUTH PROVIDER 'google'
  SUBJECT 'john.doe@example.com'
  ROLE 'user';

-- Output:
-- User 'john_doe' created with ID: usr_john_doe (OAuth: google)
```

**Supported OAuth Providers**:
- `google` - Google Workspace / Gmail
- `github` - GitHub accounts
- `azure` - Microsoft Azure AD

#### Internal System User (Localhost-Only)

```sql
-- Create internal system user (no password, localhost only)
CREATE USER 'compaction_job'
  WITH INTERNAL
  ROLE 'system';

-- Output:
-- User 'compaction_job' created with ID: sys_compaction_job (localhost-only)
```

#### System User with Remote Access

```sql
-- Emergency admin with remote access capability
CREATE USER 'emergency_admin'
  WITH PASSWORD 'EmergencyAccess123!'
  ROLE 'system'
  ALLOW_REMOTE true;

-- Output:
-- User 'emergency_admin' created with ID: sys_emergency_admin (remote access enabled)
```

**⚠️ Security Note**: System users with `ALLOW_REMOTE true` **MUST** have a password set.

### Modifying Users

#### Change Password

```sql
-- Update user password
ALTER USER 'alice' SET PASSWORD 'NewSecurePass456!';

-- Output:
-- User 'alice' password updated
```

#### Change Role

```sql
-- Promote user to service role
ALTER USER 'alice' SET ROLE 'service';

-- Output:
-- User 'alice' role updated to: service
```

#### Enable Remote Access (System Users Only)

```sql
-- Enable remote access for system user (requires password)
ALTER USER 'compaction_job' 
  SET PASSWORD 'RemoteJobKey789!' 
  SET ALLOW_REMOTE true;

-- Output:
-- User 'compaction_job' updated: remote access enabled
```

### Deleting Users

```sql
-- Soft delete user (mark as deleted, preserve records)
DROP USER 'alice';

-- Output:
-- User 'alice' deleted (soft delete)

-- Verify deletion
SELECT user_id, username, role, deleted_at 
FROM system.users 
WHERE username = 'alice';

-- Expected output:
-- ┌───────────┬──────────┬──────┬─────────────────────┐
-- │ user_id   │ username │ role │ deleted_at          │
-- ├───────────┼──────────┼──────┼─────────────────────┤
-- │ usr_alice │ alice    │ user │ 2025-10-28 14:23:45 │
-- └───────────┴──────────┴──────┴─────────────────────┘
```

**Soft Delete Behavior**:
- User record preserved in `system.users` table
- `deleted_at` timestamp set to current time
- Authentication fails immediately (same error as "invalid credentials")
- No distinction between deleted and non-existent users (security measure)

---

## Authentication Methods

### HTTP Basic Auth

**Use Case**: Quick testing, CLI authentication, legacy integrations

**Format**: `Authorization: Basic <base64(username:password)>`

**Example (curl)**:
```bash
# Login with HTTP Basic Auth
curl -X POST http://localhost:2900/v1/auth/login \
  -H "Authorization: Basic $(echo -n 'alice:SecurePass123!' | base64)" \
  -H "Content-Type: application/json"

# Response (200 OK):
{
  "token": "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9...",
  "user_id": "usr_alice",
  "role": "user",
  "expires_at": 1698852000
}
```

**Error Response (401 Unauthorized)**:
```json
{
  "status": "error",
  "error": "Invalid username or password",
  "timestamp": 1698765432000
}
```

**Security Notes**:
- Credentials transmitted in every request (use HTTPS in production)
- Generic error messages prevent username enumeration
- Timing-attack resistant password comparison (bcrypt)

### JWT Token Authentication

**Use Case**: Web applications, mobile apps, SPAs (recommended)

**Workflow**:
1. Client sends username/password to `/v1/auth/login`
2. Server validates credentials, returns JWT token
3. Client includes token in subsequent requests: `Authorization: Bearer <token>`
4. Server validates token signature and expiration

**Step 1: Login (Obtain JWT)**

```bash
# Login with username/password in request body
curl -X POST http://localhost:2900/v1/auth/login \
  -H "Content-Type: application/json" \
  -d '{
    "username": "alice",
    "password": "SecurePass123!"
  }'

# Response (200 OK):
{
  "token": "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJ1c2VyX2lkIjoidXNyX2FsaWNlIiwicm9sZSI6InVzZXIiLCJleHAiOjE2OTg4NTIwMDB9.signature",
  "user_id": "usr_alice",
  "role": "user",
  "expires_at": 1698852000
}
```

**Step 2: Validate Token (Optional)**

```bash
# Validate JWT token and retrieve user context
curl -X POST http://localhost:2900/v1/auth/validate \
  -H "Authorization: Bearer eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9..." \
  -H "Content-Type: application/json"

# Response (200 OK):
{
  "user_id": "usr_alice",
  "role": "user",
  "exp": 1698852000
}
```

**Step 3: Authenticated SQL Query**

```bash
# Execute SQL query with JWT token
curl -X POST http://localhost:2900/v1/sql \
  -H "Authorization: Bearer eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9..." \
  -H "Content-Type: application/json" \
  -d '{
    "query": "SELECT id, content FROM app.messages LIMIT 10"
  }'

# Response (200 OK):
{
  "columns": ["id", "content"],
  "rows": [
    [1, "Hello, World!"],
    [2, "Second message"]
  ],
  "row_count": 2
}
```

**JWT Token Structure**:
```
Header (algorithm and type):
{
  "alg": "HS256",
  "typ": "JWT"
}

Payload (claims):
{
  "user_id": "usr_alice",
  "role": "user",
  "exp": 1698852000  // Expiration timestamp (Unix epoch)
}

Signature:
HMACSHA256(
  base64UrlEncode(header) + "." + base64UrlEncode(payload),
  secret
)
```

**Token Expiration**:
- Default: 24 hours from login
- Configurable via server config
- Expired tokens return `401 Unauthorized` with error: "Token has expired"

### OAuth 2.0 Authentication

**Use Case**: Enterprise SSO, third-party identity providers

**Supported Providers**:
- Google Workspace / Gmail
- GitHub
- Microsoft Azure AD

**Setup** (example: Google OAuth):

```sql
-- 1. Create OAuth user
CREATE USER 'john_doe'
  WITH OAUTH PROVIDER 'google'
  SUBJECT 'john.doe@example.com'
  ROLE 'user';
```

**OAuth Flow** (simplified):
1. Client redirects to OAuth provider (Google/GitHub/Azure)
2. User authenticates with provider
3. Provider redirects back with authorization code
4. Client exchanges code for access token with provider
5. Client sends provider token to KalamDB `/v1/auth/login`
6. KalamDB validates token with provider, matches `subject` to user
7. KalamDB returns JWT token for subsequent requests

**Example (Google OAuth Token Validation)**:
```bash
# Login with Google OAuth token
curl -X POST http://localhost:2900/v1/auth/login \
  -H "Content-Type: application/json" \
  -d '{
    "provider": "google",
    "oauth_token": "ya29.a0AfH6SMBx..."
  }'

# Response (200 OK):
{
  "token": "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9...",
  "user_id": "usr_john_doe",
  "role": "user",
  "expires_at": 1698852000
}
```

---

## Role-Based Access Control (RBAC)

KalamDB implements a **four-role hierarchy** with distinct permissions:

### Role Hierarchy

```
system   (highest privilege)
  ↓
dba      (database administration)
  ↓
service  (automation and shared data)
  ↓
user     (standard access)
```

### Role Capabilities

| Operation | user | service | dba | system |
|-----------|------|---------|-----|--------|
| **User Tables** (own) | ✅ Full | ✅ Full | ✅ Full | ✅ Full |
| **User Tables** (other user's) | ❌ No | ❌ No | ❌ No | ✅ Read/Write |
| **Shared Tables** (public) | ✅ Read | ✅ Read/Write | ✅ Full | ✅ Full |
| **Shared Tables** (private) | ❌ No | ✅ Read/Write | ✅ Full | ✅ Full |
| **Stream Tables** | ✅ Read | ✅ Read/Write | ✅ Full | ✅ Full |
| **Create User** | ❌ No | ❌ No | ✅ Yes | ✅ Yes |
| **Alter User** | ❌ No | ❌ No | ✅ Yes | ✅ Yes |
| **Drop User** | ❌ No | ❌ No | ✅ Yes | ✅ Yes |
| **Manage Shared Table Access** | ❌ No | ✅ Yes | ✅ Yes | ✅ Yes |
| **Create Namespace** | ❌ No | ❌ No | ✅ Yes | ✅ Yes |
| **Drop Namespace** | ❌ No | ❌ No | ✅ Yes | ✅ Yes |

### User Role Examples

#### Regular User (role: user)

```sql
-- Login as alice (user role)
-- Authorization: Bearer <alice_jwt_token>

-- ✅ ALLOWED: Create own user table
CREATE USER TABLE app.messages (
  id BIGINT DEFAULT SNOWFLAKE_ID(),
  content TEXT
);

-- ✅ ALLOWED: Insert into own user table
INSERT INTO app.messages (content) VALUES ('Hello, World!');

-- ✅ ALLOWED: Read from public shared table
SELECT * FROM shared.announcements WHERE access_level = 'public';

-- ❌ FORBIDDEN: Read from private shared table
SELECT * FROM shared.internal_metrics WHERE access_level = 'private';
-- Error (403): Access denied to private shared table

-- ❌ FORBIDDEN: Access another user's table
SELECT * FROM bob.messages;
-- Error (403): Cannot access another user's table

-- ❌ FORBIDDEN: Create new user
CREATE USER 'charlie' WITH PASSWORD 'pass' ROLE 'user';
-- Error (403): DBA or system role required for user management
```

#### Service Account (role: service)

```sql
-- Login as backup_service (service role)
-- Authorization: Bearer <service_jwt_token>

-- ✅ ALLOWED: Read from private shared table
SELECT * FROM shared.internal_metrics WHERE access_level = 'private';

-- ✅ ALLOWED: Create shared table
CREATE SHARED TABLE shared.analytics (
  event_name TEXT,
  count BIGINT
) ACCESS_LEVEL 'private';

-- ✅ ALLOWED: Modify shared table access level
ALTER TABLE shared.analytics SET ACCESS_LEVEL 'public';

-- ❌ FORBIDDEN: Access user's private table
SELECT * FROM alice.messages;
-- Error (403): Cannot access another user's table

-- ❌ FORBIDDEN: Create new user
CREATE USER 'charlie' WITH PASSWORD 'pass' ROLE 'user';
-- Error (403): DBA or system role required for user management
```

#### Database Administrator (role: dba)

```sql
-- Login as db_admin (dba role)
-- Authorization: Bearer <dba_jwt_token>

-- ✅ ALLOWED: Create new user
CREATE USER 'charlie' 
  WITH PASSWORD 'SecurePass789!' 
  ROLE 'user';

-- ✅ ALLOWED: Modify user role
ALTER USER 'charlie' SET ROLE 'service';

-- ✅ ALLOWED: Delete user
DROP USER 'charlie';

-- ✅ ALLOWED: Create namespace
CREATE NAMESPACE production;

-- ✅ ALLOWED: Manage shared tables
CREATE SHARED TABLE shared.config (key TEXT, value TEXT) ACCESS_LEVEL 'private';
ALTER TABLE shared.config SET ACCESS_LEVEL 'public';

-- ❌ FORBIDDEN: Access another user's table (system role only)
SELECT * FROM alice.messages;
-- Error (403): Cannot access another user's table
```

#### System User (role: system)

```sql
-- Login as system user (system role)
-- Authorization: Bearer <system_jwt_token>

-- ✅ ALLOWED: Full access to all user tables
SELECT * FROM alice.messages;
INSERT INTO bob.events (type, data) VALUES ('system_event', '{}');

-- ✅ ALLOWED: All DBA operations
CREATE USER 'maintenance' WITH INTERNAL ROLE 'system';
CREATE NAMESPACE staging;
DROP NAMESPACE test;

-- ✅ ALLOWED: System-level operations
-- (internal cleanup, compaction, replication, etc.)
```

### Shared Table Access Levels

Shared tables support **three access levels**:

| Access Level | Description | Who Can Access? |
|--------------|-------------|-----------------|
| **public** | Readable by all authenticated users | user, service, dba, system |
| **private** | Restricted access | service, dba, system |
| **restricted** | Reserved for future use | Currently same as private |

**Example: Managing Access Levels**

```sql
-- Create public shared table (accessible to all users)
CREATE SHARED TABLE shared.announcements (
  id BIGINT DEFAULT SNOWFLAKE_ID(),
  message TEXT,
  created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP()
) ACCESS_LEVEL 'public';

-- Regular user (alice) can read
-- Authorization: Bearer <alice_jwt_token> (role: user)
SELECT * FROM shared.announcements;
-- ✅ Succeeds

-- Create private shared table (service/dba/system only)
CREATE SHARED TABLE shared.internal_metrics (
  metric_name TEXT,
  value DOUBLE,
  collected_at TIMESTAMP
) ACCESS_LEVEL 'private';

-- Regular user (alice) CANNOT read
-- Authorization: Bearer <alice_jwt_token> (role: user)
SELECT * FROM shared.internal_metrics;
-- ❌ Error (403): Access denied to private shared table

-- Service account CAN read
-- Authorization: Bearer <service_jwt_token> (role: service)
SELECT * FROM shared.internal_metrics;
-- ✅ Succeeds

-- Modify access level (requires service/dba/system role)
-- Authorization: Bearer <service_jwt_token> (role: service)
ALTER TABLE shared.announcements SET ACCESS_LEVEL 'private';
-- ✅ Succeeds
```

---

## CLI Authentication

The KalamDB CLI (`kalam`) automatically authenticates as the **system user** when connecting to localhost.

### Localhost Connection (Default)

```bash
# Start CLI (auto-connects as system user)
cargo run --bin kalam

# Output:
# KalamDB CLI v0.1.0
# Connected to: http://127.0.0.1:2900
# Authenticated as: system (role: system)

kalam> SELECT CURRENT_USER();
-- ┌────────────┐
-- │ user_id    │
-- ├────────────┤
-- │ sys_system │
-- └────────────┘

kalam> CREATE USER 'alice' WITH PASSWORD 'pass' ROLE 'user';
-- User 'alice' created with ID: usr_alice

kalam> \q
-- Goodbye!
```

### Remote Connection (Password Required)

```bash
# Connect to remote server with username/password
cargo run --bin kalam -- --host https://kalamdb.example.com --user db_admin

# Output:
# Password for db_admin: [hidden input]
# Connected to: https://kalamdb.example.com
# Authenticated as: db_admin (role: dba)

kalam> SELECT CURRENT_USER();
-- ┌─────────────┐
-- │ user_id     │
-- ├─────────────┤
-- │ dba_db_admin│
-- └─────────────┘
```

### Credential Storage

The CLI stores credentials in `~/.kalam/credentials.toml`:

```toml
# ~/.kalam/credentials.toml

[[connections]]
name = "localhost"
url = "http://127.0.0.1:2900"
user = "system"
auth_type = "internal"  # No password needed

[[connections]]
name = "production"
url = "https://kalamdb.example.com"
user = "db_admin"
auth_type = "password"
# Password stored encrypted (or prompted each time)
```

---

## API Integration Examples

### JavaScript/TypeScript (fetch)

```typescript
// 1. Login and obtain JWT token
async function login(username: string, password: string): Promise<string> {
  const response = await fetch('http://localhost:2900/v1/auth/login', {
    method: 'POST',
    headers: {
      'Content-Type': 'application/json',
    },
    body: JSON.stringify({ username, password }),
  });

  if (!response.ok) {
    const error = await response.json();
    throw new Error(error.error);
  }

  const { token } = await response.json();
  return token;
}

// 2. Execute SQL query with JWT token
async function executeQuery(token: string, query: string): Promise<any> {
  const response = await fetch('http://localhost:2900/v1/sql', {
    method: 'POST',
    headers: {
      'Authorization': `Bearer ${token}`,
      'Content-Type': 'application/json',
    },
    body: JSON.stringify({ query }),
  });

  if (!response.ok) {
    const error = await response.json();
    throw new Error(error.error);
  }

  return response.json();
}

// Usage example
async function main() {
  try {
    const token = await login('alice', 'SecurePass123!');
    console.log('Logged in successfully');

    const result = await executeQuery(
      token,
      'SELECT id, content FROM app.messages LIMIT 10'
    );
    console.log('Query result:', result);
  } catch (error) {
    console.error('Error:', error.message);
  }
}
```

### Python (requests)

```python
import requests
import json

BASE_URL = "http://localhost:2900"

# 1. Login and obtain JWT token
def login(username: str, password: str) -> str:
    response = requests.post(
        f"{BASE_URL}/v1/auth/login",
        json={"username": username, "password": password}
    )
    response.raise_for_status()
    return response.json()["token"]

# 2. Execute SQL query with JWT token
def execute_query(token: str, query: str) -> dict:
    response = requests.post(
        f"{BASE_URL}/v1/sql",
        headers={"Authorization": f"Bearer {token}"},
        json={"query": query}
    )
    response.raise_for_status()
    return response.json()

# Usage example
if __name__ == "__main__":
    try:
        token = login("alice", "SecurePass123!")
        print("Logged in successfully")
        
        result = execute_query(token, "SELECT id, content FROM app.messages LIMIT 10")
        print(f"Query result: {json.dumps(result, indent=2)}")
    except requests.HTTPError as e:
        print(f"Error: {e.response.json()['error']}")
```

### Rust (reqwest)

```rust
use reqwest::Client;
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize)]
struct LoginRequest {
    username: String,
    password: String,
}

#[derive(Debug, Deserialize)]
struct LoginResponse {
    token: String,
    user_id: String,
    role: String,
}

#[derive(Debug, Serialize)]
struct QueryRequest {
    query: String,
}

async fn login(client: &Client, username: &str, password: &str) -> Result<String, Box<dyn std::error::Error>> {
    let response = client
        .post("http://localhost:2900/v1/auth/login")
        .json(&LoginRequest {
            username: username.to_string(),
            password: password.to_string(),
        })
        .send()
        .await?;

    let login_response: LoginResponse = response.json().await?;
    Ok(login_response.token)
}

async fn execute_query(client: &Client, token: &str, query: &str) -> Result<String, Box<dyn std::error::Error>> {
    let response = client
        .post("http://localhost:2900/v1/sql")
        .bearer_auth(token)
        .json(&QueryRequest {
            query: query.to_string(),
        })
        .send()
        .await?;

    let result = response.text().await?;
    Ok(result)
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let client = Client::new();

    let token = login(&client, "alice", "SecurePass123!").await?;
    println!("Logged in successfully");

    let result = execute_query(&client, &token, "SELECT id, content FROM app.messages LIMIT 10").await?;
    println!("Query result: {}", result);

    Ok(())
}
```

---

## Troubleshooting

### Common Errors and Solutions

#### 1. **401 Unauthorized: "Invalid username or password"**

**Possible Causes**:
- Wrong username or password
- User account deleted (soft delete)
- User does not exist

**Solutions**:
```sql
-- Verify user exists and is not deleted
SELECT user_id, username, role, deleted_at 
FROM system.users 
WHERE username = 'alice';

-- If deleted_at is NOT NULL, user was deleted
-- If no rows returned, user doesn't exist

-- Reset password (if you have DBA access)
ALTER USER 'alice' SET PASSWORD 'NewPassword123!';
```

**Security Note**: The error message is intentionally generic to prevent username enumeration. "User not found" and "wrong password" return the same error.

#### 2. **401 Unauthorized: "Token has expired"**

**Cause**: JWT token validity period exceeded (default: 24 hours)

**Solution**:
```bash
# Re-authenticate to obtain new token
curl -X POST http://localhost:2900/v1/auth/login \
  -H "Content-Type: application/json" \
  -d '{"username": "alice", "password": "SecurePass123!"}'
```

#### 3. **401 Unauthorized: "Invalid token signature"**

**Possible Causes**:
- Token manually modified
- JWT secret changed on server (restart with different config)
- Token from different KalamDB instance

**Solution**:
- Obtain fresh token via login
- Verify server JWT secret configuration

#### 4. **401 Unauthorized: "Missing Authorization header"**

**Cause**: Request missing `Authorization` header

**Solution**:
```bash
# ❌ WRONG: No Authorization header
curl -X POST http://localhost:2900/v1/sql \
  -H "Content-Type: application/json" \
  -d '{"query": "SELECT * FROM app.messages"}'

# ✅ CORRECT: Include Authorization header
curl -X POST http://localhost:2900/v1/sql \
  -H "Authorization: Bearer eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9..." \
  -H "Content-Type: application/json" \
  -d '{"query": "SELECT * FROM app.messages"}'
```

#### 5. **403 Forbidden: "Insufficient permissions"**

**Cause**: User role lacks permission for operation

**Examples**:
```sql
-- Regular user trying to create another user
-- Authorization: Bearer <alice_jwt_token> (role: user)
CREATE USER 'charlie' WITH PASSWORD 'pass' ROLE 'user';
-- ❌ Error (403): DBA or system role required for user management

-- Regular user trying to access private shared table
SELECT * FROM shared.internal_metrics WHERE access_level = 'private';
-- ❌ Error (403): Access denied to private shared table
```

**Solution**:
- Verify your role: `SELECT CURRENT_USER();`
- Request DBA/service/system role if needed
- Check table access level: `SELECT table_name, access_level FROM system.shared_tables;`

#### 6. **403 Forbidden: "Cannot access another user's table"**

**Cause**: User/service/dba role attempting to access another user's private table

**Example**:
```sql
-- Alice (role: user) trying to access Bob's table
-- Authorization: Bearer <alice_jwt_token>
SELECT * FROM bob.messages;
-- ❌ Error (403): Cannot access another user's table
```

**Solution**:
- Only system role can access other users' tables
- Use shared tables for cross-user data sharing:
  ```sql
  -- Bob creates shared table instead
  CREATE SHARED TABLE shared.bob_public_messages (
    id BIGINT DEFAULT SNOWFLAKE_ID(),
    content TEXT
  ) ACCESS_LEVEL 'public';
  
  -- Now Alice can read
  SELECT * FROM shared.bob_public_messages;
  ```

#### 7. **400 Bad Request: "Invalid request format"**

**Possible Causes**:
- Missing required fields in request body
- Invalid JSON syntax
- Invalid data types

**Example**:
```bash
# ❌ WRONG: Missing password field
curl -X POST http://localhost:2900/v1/auth/login \
  -H "Content-Type: application/json" \
  -d '{"username": "alice"}'
# Error (400): Invalid request format

# ✅ CORRECT: Include all required fields
curl -X POST http://localhost:2900/v1/auth/login \
  -H "Content-Type: application/json" \
  -d '{"username": "alice", "password": "SecurePass123!"}'
```

#### 8. **409 Conflict: "Username already exists"**

**Cause**: Attempting to create user with duplicate username

**Solution**:
```sql
-- Use IF NOT EXISTS to avoid error
CREATE USER IF NOT EXISTS 'alice' 
  WITH PASSWORD 'SecurePass123!' 
  ROLE 'user';

-- Or use different username
CREATE USER 'alice2' 
  WITH PASSWORD 'SecurePass123!' 
  ROLE 'user';
```

#### 9. **Password Too Short/Long**

**Cause**: Password violates length constraints

**Constraints**:
- Minimum: 8 characters
- Maximum: 72 characters (bcrypt limitation)

**Solution**:
```sql
-- ❌ WRONG: Password too short (< 8 chars)
CREATE USER 'alice' WITH PASSWORD 'pass' ROLE 'user';
-- Error (400): Password must be at least 8 characters

-- ❌ WRONG: Password too long (> 72 chars)
CREATE USER 'alice' WITH PASSWORD 'VeryLongPasswordThatExceeds72CharactersAndWillBeRejectedByBcrypt...' ROLE 'user';
-- Error (400): Password must be at most 72 characters

-- ✅ CORRECT: Password within valid range
CREATE USER 'alice' WITH PASSWORD 'SecurePass123!' ROLE 'user';
```

#### 10. **System User Remote Access Error**

**Cause**: Attempting to enable remote access for system user without password

**Example**:
```sql
-- ❌ WRONG: System user with remote access but no password
CREATE USER 'remote_system' 
  WITH INTERNAL 
  ROLE 'system' 
  ALLOW_REMOTE true;
-- Error (400): System users with remote access must have a password

-- ✅ CORRECT: Set password when enabling remote access
CREATE USER 'remote_system' 
  WITH PASSWORD 'SystemKey123!' 
  ROLE 'system' 
  ALLOW_REMOTE true;
```

### Debugging Tips

#### Check Current User Context

```sql
-- Get current authenticated user
SELECT CURRENT_USER();

-- Get full user details
SELECT u.user_id, u.username, u.role, u.auth_type 
FROM system.users u
WHERE u.user_id = CURRENT_USER();
```

#### List All Users

```sql
-- View all users (requires dba or system role)
SELECT user_id, username, role, auth_type, allow_remote, deleted_at, created_at
FROM system.users
ORDER BY created_at DESC;
```

#### Check Shared Table Access Levels

```sql
-- View all shared tables and their access levels
SELECT table_name, access_level, created_by, created_at
FROM system.shared_tables
ORDER BY created_at DESC;
```

#### Enable Debug Logging (Server)

```toml
# config.toml (server configuration)
[logging]
level = "debug"  # Enable detailed auth logs

# Restart server to apply
```

**Log Output**:
```
[DEBUG] kalamdb_auth: Authenticating user: alice
[DEBUG] kalamdb_auth: Password verification: success
[DEBUG] kalamdb_auth: Generating JWT token for user_id: usr_alice, role: user
[INFO]  kalamdb_auth: User 'alice' authenticated successfully
```

---

## Next Steps

1. **Create Your First User**:
   ```sql
   CREATE USER 'your_username' WITH PASSWORD 'YourSecurePass123!' ROLE 'user';
   ```

2. **Test Authentication**:
   ```bash
   curl -X POST http://localhost:2900/v1/auth/login \
     -H "Content-Type: application/json" \
     -d '{"username": "your_username", "password": "YourSecurePass123!"}'
   ```

3. **Explore RBAC**:
   - Try creating a service account
   - Experiment with shared table access levels
   - Test role-based permissions

4. **Integrate with Your Application**:
   - Use JWT tokens for stateless authentication
   - Implement token refresh logic
   - Add error handling for 401/403 responses

5. **Read Full Documentation**:
   - [SQL Syntax Reference](../../docs/architecture/SQL_SYNTAX.md)
   - [API Contracts](./contracts/auth.yaml)
   - [Error Response Schemas](./contracts/errors.yaml)

---

**Questions or Issues?**

- Check the [Troubleshooting](#troubleshooting) section
- Review [error response documentation](./contracts/errors.yaml)
- Examine server logs for detailed error context
- Verify user roles and permissions in `system.users` table
