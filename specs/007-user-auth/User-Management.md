# User Management & Role-Based Access Control (RBAC)

**Feature**: User Management SQL Commands with Role-Based Access Control  
**Priority**: P2 (Important Features)  
**Status**: Not Started (0% complete)  
**Estimated Effort**: 5 weeks (updated from 3-4 weeks due to new kalamdb-auth crate)  
**Created**: October 27, 2025  
**Last Updated**: October 27, 2025

---

## 📋 Table of Contents

1. [Overview](#overview)
2. [System Architecture](#system-architecture)
3. [Database Schema](#database-schema)
4. [Role Definitions](#role-definitions)
5. [Access Control Rules](#access-control-rules)
6. [SQL Syntax](#sql-syntax)
7. [Soft Delete Mechanism](#soft-delete-mechanism)
8. [Implementation Requirements](#implementation-requirements)
9. [Security Considerations](#security-considerations)
10. [Testing Requirements](#testing-requirements)
11. [Migration Guide](#migration-guide)

---

## Overview

KalamDB requires a comprehensive user management system that supports:

1. **Standard SQL DML operations** on `system.users` (INSERT, UPDATE, DELETE, SELECT)
2. **Role-Based Access Control (RBAC)** with four distinct roles
3. **Soft delete with grace period** for user recovery
4. **Password and OAuth authentication** support
5. **Fine-grained access control** for shared tables
6. **MySQL/PostgreSQL-style user management** commands

This document specifies the complete user management and RBAC implementation for KalamDB.

---

## System Architecture

### Authentication Flow

```
┌─────────────┐
│   Client    │
└──────┬──────┘
       │ 1. Authenticate using ONE of:
       │    a) HTTP Basic Auth (username:password)
       │    b) Bearer JWT token (externally issued)
       ▼
┌──────────────────────┐
│  kalamdb-auth crate  │
│  (Authentication)    │
└──────────┬───────────┘
           │
           ├─► HTTP Basic Auth Path:
           │   - Extract username:password from Authorization header
           │   - Lookup user in system.users
           │   - Verify password (bcrypt) against auth_data
           │   - Check localhost-only restriction for system role
           │   - Return authenticated user context
           │
           └─► Bearer Token Path:
               - Extract JWT from Authorization header
               - Validate JWT signature (external or internal issuer)
               - Extract claims (user_id, username, role)
               - Optionally verify user exists in system.users
               - Return authenticated user context
               ▼
┌─────────────────┐
│ Authorization   │ ──► Enforces access control based on:
│   Middleware    │     - user role (user/service/dba/system)
└────────┬────────┘     - table type (user/shared/system)
         │              - table access (public/private/restricted)
         │              - connection source (localhost vs remote)
         ▼
┌─────────────────┐
│  Query Engine   │
└─────────────────┘
```

**Key Changes**:
- ❌ **Removed**: X-API-KEY header authentication
- ❌ **Removed**: X-USER-ID header
- ✅ **Added**: HTTP Basic Auth support (username:password)
- ✅ **Added**: Bearer JWT token support (external validation)
- ✅ **Added**: New `kalamdb-auth` crate for authentication logic
- ✅ **Added**: Localhost-only enforcement for system users

---

## Database Schema

### 1. `system.users` Table (Enhanced)

**Purpose**: Central user registry with authentication and authorization data

| Column        | Type      | Nullable | Constraints      | Description                                                |
|---------------|-----------|----------|------------------|------------------------------------------------------------|
| `user_id`     | TEXT      | No       | PRIMARY KEY      | Unique user identifier (max 64 chars)                      |
| `username`    | TEXT      | No       | UNIQUE, NOT NULL | Login name (max 128 chars)                                 |
| `email`       | TEXT      | Yes      | -                | User email address                                         |
| `auth_type`   | TEXT      | No       | NOT NULL         | Authentication type: `'password'` or `'oauth'`             |
| `auth_data`   | TEXT      | Yes      | -                | Password hash (bcrypt) or OAuth JSON `{provider, subject}` |
| `role`        | TEXT      | No       | NOT NULL         | User role: `'user'`, `'service'`, `'dba'`, `'system'`      |
| `storage_mode`| TEXT      | Yes      | -                | Storage mode: `'table'` or `'region'`                      |
| `storage_id`  | TEXT      | Yes      | FK               | Foreign key to `system.storages.storage_id`                |
| `metadata`    | JSONB     | Yes      | -                | Custom JSON metadata (application-specific)                |
| `created_at`  | TIMESTAMP | No       | NOT NULL         | User creation timestamp (auto-generated)                   |
| `updated_at`  | TIMESTAMP | No       | NOT NULL         | Last modification timestamp (auto-updated)                 |
| `deleted_at`  | TIMESTAMP | Yes      | -                | Soft delete timestamp (NULL = active user)                 |

**Storage**: RocksDB column family `system_users`  
**Key Format**: `{user_id}`

**Example JSON Entry (Password Auth)**:
```json
{
  "user_id": "alice123",
  "username": "alice",
  "email": "alice@example.com",
  "auth_type": "password",
  "auth_data": "$2b$12$KIXxNv8f.Yq7b5Z8X9vZ0eF1g2h3i4j5k6l7m8n9o0p1q2r3s4t5u6",
  "role": "user",
  "storage_mode": "table",
  "storage_id": "local",
  "metadata": {"department": "engineering", "team": "backend"},
  "created_at": "2025-10-27T10:00:00Z",
  "updated_at": "2025-10-27T10:00:00Z",
  "deleted_at": null
}
```

**Example JSON Entry (OAuth)**:
```json
{
  "user_id": "backup_bot",
  "username": "backup_service",
  "email": null,
  "auth_type": "oauth",
  "auth_data": "{\"provider\": \"google\", \"subject\": \"backup@service.account\"}",
  "role": "service",
  "storage_mode": null,
  "storage_id": null,
  "metadata": {"purpose": "automated_backup"},
  "created_at": "2025-10-27T11:00:00Z",
  "updated_at": "2025-10-27T11:00:00Z",
  "deleted_at": null
}
```

**Example JSON Entry (System User - Localhost Only)**:
```json
{
  "user_id": "system_replicator",
  "username": "replicator",
  "email": null,
  "auth_type": "internal",
  "auth_data": null,
  "role": "system",
  "storage_mode": null,
  "storage_id": null,
  "metadata": {"node": "primary", "cluster": "prod", "allow_remote": false},
  "created_at": "2025-10-27T09:00:00Z",
  "updated_at": "2025-10-27T09:00:00Z",
  "deleted_at": null
}
```

**Example JSON Entry (System User - Remote Access Enabled)**:
```json
{
  "user_id": "system_admin",
  "username": "emergency_admin",
  "email": "admin@company.com",
  "auth_type": "password",
  "auth_data": "$2b$12$KIXxNv8f.Yq7b5Z8X9vZ0eF1g2h3i4j5k6l7m8n9o0p1q2r3s4t5u6",
  "role": "system",
  "storage_mode": null,
  "storage_id": null,
  "metadata": {"allow_remote": true, "purpose": "emergency_access"},
  "created_at": "2025-10-27T09:00:00Z",
  "updated_at": "2025-10-27T09:00:00Z",
  "deleted_at": null
}
```

---

### 2. `system.tables` Table (Enhanced)

**New Column**: Add `access` column for shared tables only

| Column            | Type      | Nullable | Description                                                    |
|-------------------|-----------|----------|----------------------------------------------------------------|
| ... (existing)    | ...       | ...      | All existing columns remain unchanged                          |
| `access`          | TEXT      | Yes      | Access level enum (stored as lowercase text): public/private/restricted |

**Access Column Type**: `TableAccess` enum from `kalamdb-commons`

**Access Column Semantics**:
- **NULL**: Not applicable (for user tables and system tables)
- **`TableAccess::Public`** (`'public'`): Any authenticated user can read (SELECT)
- **`TableAccess::Private`** (`'private'`): Only service/dba/system roles can access
- **`TableAccess::Restricted`** (`'restricted'`): Only service/dba/system roles can access (same as private, reserved for future fine-grained permissions)

**Default**: If omitted during `CREATE SHARED TABLE`, defaults to `TableAccess::Private`

**Rust Type Definition** (in `kalamdb-commons`):
```rust
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum TableAccess {
    Public,
    Private,
    Restricted,
}

impl Default for TableAccess {
    fn default() -> Self {
        TableAccess::Private
    }
}
```

---

## Role Definitions

KalamDB implements four distinct user roles with hierarchical permissions:

### 🔵 Role: `user` (Default End-User)

**Purpose**: Standard application user with limited access

**Permissions**:
- ✅ **SELECT** from shared tables where `access = 'public'`
- ✅ **Full DML** (SELECT/INSERT/UPDATE/DELETE) on **own user tables only**
- ✅ Can view own user info in `system.users` (WHERE user_id = CURRENT_USER())
- ❌ **Cannot** access `system.*` tables (except limited self-query)
- ❌ **Cannot** create or drop tables/namespaces
- ❌ **Cannot** access other users' tables
- ❌ **Cannot** access shared tables with `access = 'private'` or `'restricted'`

**Use Cases**: 
- Mobile app users
- Web application end-users
- SaaS tenant users

---

### 🟢 Role: `service` (Backend Integration Account)

**Purpose**: Internal service accounts for automation and integrations

**Permissions**:
- ✅ **Full DML** (SELECT/INSERT/UPDATE/DELETE) on **all shared tables** (any access level)
- ✅ **Full DML** on **any user table** (cross-user access for integrations)
- ✅ Can execute **FLUSH**, **BACKUP**, **CLEANUP** operations
- ✅ Can read `system.jobs`, `system.live_queries`, `system.tables` (monitoring)
- ❌ **Cannot** CREATE/DROP tables, namespaces, or storages
- ❌ **Cannot** manage users (INSERT/UPDATE/DELETE on `system.users`)
- ❌ **Cannot** modify system tables (except via allowed operations)

**Use Cases**:
- Backup/restore services
- Data synchronization jobs
- ETL pipelines
- Monitoring agents
- API integrations

---

### 🟡 Role: `dba` (Database Administrator)

**Purpose**: Human database administrator with full control

**Permissions**:
- ✅ **Full access** to all tables (system, shared, user)
- ✅ **CREATE/ALTER/DROP** namespaces, tables, storages
- ✅ **Full user management** (CREATE USER, UPDATE, DELETE, restore deleted users)
- ✅ View and manage **logs, jobs, metrics**
- ✅ Execute administrative commands (CLEAR CACHE, KILL LIVE QUERY, etc.)
- ✅ Can query deleted users (`WHERE deleted_at IS NOT NULL`)
- ✅ Can restore soft-deleted users within grace period

**Security**:
- 🚫 Only manually created by another `dba` or `system` role
- 🔐 Requires strong password (min 12 chars, complexity requirements)
- 📝 All DBA actions logged to audit trail

**Use Cases**:
- Database administrators
- DevOps engineers
- System architects

---

### 🔴 Role: `system` (Internal/Administrative)

**Purpose**: Internal processes, background jobs, and emergency administrative access

**Permissions**:
- ✅ **Full access** to all operations (same as `dba`)
- ✅ Background jobs (flush, compaction, cleanup)
- ✅ Raft replication tasks
- ✅ Internal metrics collection
- ✅ Emergency administrative access (when configured for remote access)

**Security & Access Control**:

**Default (New Installation)**:
- 🔒 **Localhost-only access** (127.0.0.1 or Unix socket)
- 🔒 **No password required** for localhost connections
- 🔒 Automatically accessible for internal processes on same node
- 🔒 HTTP API blocks all external connections to system users

**Optional Remote Access** (configured in config.toml):
- � Remote access **disabled by default** for security
- 🔐 When enabled, **password is REQUIRED** (no password = no access)
- � System user with remote access enabled but no password set = **access denied**
- 🔐 Remote connections must use HTTP Basic Auth or Bearer JWT token
- � Remote access should only be enabled for emergency administration

**Configuration Example**:
```toml
[authentication.system_users]
# Allow remote access to system users (default: false)
allow_remote_access = false

# Specific system users allowed remote access (requires passwords)
# remote_allowed = ["system_admin", "emergency_dba"]
```

**Use Cases**:
- Automatic flush scheduler (localhost)
- Compaction background job (localhost)
- Raft consensus protocol (localhost)
- Internal health checks (localhost)
- Emergency administrative access (remote, when configured)

---

## Access Control Rules

### Table Access Matrix

| User Role | System Tables        | User Tables (Own)    | User Tables (Other) | Shared (`public`)    | Shared (`private`/`restricted`) |
|-----------|----------------------|----------------------|---------------------|----------------------|---------------------------------|
| `user`    | ❌ (read-only self)  | ✅ Full DML          | ❌                  | ✅ SELECT only       | ❌                              |
| `service` | ✅ Read-only         | ✅ Full DML          | ✅ Full DML         | ✅ Full DML          | ✅ Full DML                     |
| `dba`     | ✅ Full DML          | ✅ Full DML          | ✅ Full DML         | ✅ Full DML          | ✅ Full DML                     |
| `system`  | ✅ Full DML          | ✅ Full DML          | ✅ Full DML         | ✅ Full DML          | ✅ Full DML                     |

### Operation Matrix

| Operation                          | user | service | dba | system |
|------------------------------------|------|---------|-----|--------|
| CREATE/DROP NAMESPACE              | ❌   | ❌      | ✅  | ✅     |
| CREATE/DROP USER TABLE             | ❌   | ❌      | ✅  | ✅     |
| CREATE/DROP SHARED TABLE           | ❌   | ❌      | ✅  | ✅     |
| CREATE/DROP STREAM TABLE           | ❌   | ❌      | ✅  | ✅     |
| CREATE USER                        | ❌   | ❌      | ✅  | ✅     |
| UPDATE/DELETE users                | ❌   | ❌      | ✅  | ✅     |
| INSERT into own user tables        | ✅   | ✅      | ✅  | ✅     |
| INSERT into other user tables      | ❌   | ✅      | ✅  | ✅     |
| SELECT from public shared tables   | ✅   | ✅      | ✅  | ✅     |
| SELECT from private shared tables  | ❌   | ✅      | ✅  | ✅     |
| STORAGE FLUSH TABLE/STORAGE FLUSH ALL | ❌   | ✅      | ✅  | ✅     |
| KILL LIVE QUERY                    | ❌   | ❌      | ✅  | ✅     |
| CLEAR CACHE                        | ❌   | ❌      | ✅  | ✅     |
| View system.jobs                   | ❌   | ✅      | ✅  | ✅     |
| View deleted users                 | ❌   | ❌      | ✅  | ✅     |

---

## Type Definitions (Rust Enums)

**Location**: `backend/crates/kalamdb-commons/src/models.rs`

### UserRole Enum
```rust
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[serde(rename_all = "lowercase")]
pub enum UserRole {
    User,
    Service,
    Dba,
    System,
}

impl std::fmt::Display for UserRole {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            UserRole::User => write!(f, "user"),
            UserRole::Service => write!(f, "service"),
            UserRole::Dba => write!(f, "dba"),
            UserRole::System => write!(f, "system"),
        }
    }
}

impl std::str::FromStr for UserRole {
    type Err = String;
    
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "user" => Ok(UserRole::User),
            "service" => Ok(UserRole::Service),
            "dba" => Ok(UserRole::Dba),
            "system" => Ok(UserRole::System),
            _ => Err(format!("Invalid role: {}. Valid roles: user, service, dba, system", s)),
        }
    }
}
```

### TableAccess Enum
```rust
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[serde(rename_all = "lowercase")]
pub enum TableAccess {
    Public,
    Private,
    Restricted,
}

impl Default for TableAccess {
    fn default() -> Self {
        TableAccess::Private
    }
}

impl std::fmt::Display for TableAccess {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TableAccess::Public => write!(f, "public"),
            TableAccess::Private => write!(f, "private"),
            TableAccess::Restricted => write!(f, "restricted"),
        }
    }
}

impl std::str::FromStr for TableAccess {
    type Err = String;
    
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "public" => Ok(TableAccess::Public),
            "private" => Ok(TableAccess::Private),
            "restricted" => Ok(TableAccess::Restricted),
            _ => Err(format!("Invalid access level: {}. Valid levels: public, private, restricted", s)),
        }
    }
}
```

**Usage Example**:
```rust
use kalamdb_commons::{UserRole, TableAccess};

// Parse from string
let role = "dba".parse::<UserRole>()?;  // UserRole::Dba
let access = "public".parse::<TableAccess>()?;  // TableAccess::Public

// Convert to string
assert_eq!(UserRole::Service.to_string(), "service");
assert_eq!(TableAccess::Private.to_string(), "private");

// Default value
assert_eq!(TableAccess::default(), TableAccess::Private);

// Pattern matching
match role {
    UserRole::User => println!("Limited access user"),
    UserRole::Service => println!("Service account with cross-user access"),
    UserRole::Dba => println!("Full admin access"),
    UserRole::System => println!("Internal system user"),
}

// Access control logic
fn can_access_table(user_role: UserRole, table_access: TableAccess) -> bool {
    match (user_role, table_access) {
        (_, TableAccess::Public) => true,  // Anyone can access public
        (UserRole::User, _) => false,      // Users can only access public
        _ => true,                          // Service/Dba/System can access all
    }
}
```

---

## SQL Syntax

### 1. User Management Commands

#### A. CREATE USER (MySQL/PostgreSQL Style)

**Basic Password Authentication**:
```sql
CREATE USER 'alice' 
  WITH PASSWORD 'secret123' 
  ROLE 'user';
```

**With Email and Metadata**:
```sql
CREATE USER 'bob' 
  WITH PASSWORD 'p@ssw0rd!' 
  ROLE 'service'
  EMAIL 'bob@company.com'
  METADATA '{"team": "backend", "environment": "production"}';
```

**OAuth Authentication**:
```sql
CREATE USER 'backup_service'
  WITH OAUTH PROVIDER 'google'
  SUBJECT 'backup@service.account'
  ROLE 'service';
```

**Internal System User (Localhost-Only)**:
```sql
CREATE USER 'replicator'
  WITH INTERNAL
  ROLE 'system';
-- No password required, localhost-only access
```

**System User with Remote Access (Requires Password)**:
```sql
CREATE USER 'emergency_admin'
  WITH PASSWORD 'strong_password_123!'
  ROLE 'system'
  ALLOW_REMOTE true;
-- Password is REQUIRED when allow_remote is enabled
```

**Syntax Variants** (MySQL-compatible):
```sql
-- Equivalent to CREATE USER
CREATE USER IF NOT EXISTS 'alice' WITH PASSWORD 'secret' ROLE 'user';

-- Set role separately
CREATE USER 'charlie' WITH PASSWORD 'pass';
ALTER USER 'charlie' SET ROLE 'dba';
```

**Parameters**:
- `user_id`: Auto-generated from username (or explicitly provided)
- `PASSWORD`: Hashed with bcrypt (min 8 chars, recommend 12+)
  - **REQUIRED** for `role='system'` when `ALLOW_REMOTE true`
  - **REQUIRED** for `role='user'`, `'service'`, `'dba'`
  - **OPTIONAL** for `role='system'` when localhost-only
- `ROLE`: One of `'user'`, `'service'`, `'dba'`, `'system'` (default: `'user'`)
  - Implemented as enum in `kalamdb-commons` crate
- `EMAIL`: Optional email address
- `METADATA`: Optional JSON object
- `OAUTH PROVIDER`: OAuth provider name (e.g., 'google', 'github', 'azure')
- `SUBJECT`: OAuth subject identifier
- `INTERNAL`: Mark as internal system user (localhost-only, no password needed)
- `ALLOW_REMOTE`: Allow remote connections (only for `role='system'`, requires password)

**Authorization**: Only `dba` and `system` roles can execute CREATE USER

**Validation**:
- If `ROLE='system'` AND `ALLOW_REMOTE=true` → `PASSWORD` is **REQUIRED**
- If `ROLE='system'` AND `ALLOW_REMOTE=true` AND no password → **ERROR**: "System users with remote access must have a password"
- If `ROLE='system'` AND `ALLOW_REMOTE=false` OR `INTERNAL` → password optional

---

#### B. ALTER USER

**Change Password**:
```sql
ALTER USER 'alice' SET PASSWORD 'new_password_456';
```

**Change Role** (DBA only):
```sql
ALTER USER 'bob' SET ROLE 'dba';
```

**Update Email and Metadata**:
```sql
ALTER USER 'alice' SET 
  EMAIL 'alice.new@company.com',
  METADATA '{"team": "frontend", "level": "senior"}';
```

**Authorization**: 
- Users can change their own password
- Only `dba` and `system` can change roles and other user attributes

---

#### C. DROP USER (Soft Delete)

**Soft Delete User**:
```sql
DROP USER 'alice';
-- Equivalent to: DELETE FROM system.users WHERE user_id = 'alice';
```

**Soft Delete with IF EXISTS**:
```sql
DROP USER IF EXISTS 'old_user';
```

**Behavior**:
- Sets `deleted_at` to current timestamp
- User becomes hidden from default queries
- User's tables remain accessible during grace period
- Scheduled cleanup job deletes user after grace period (default: 30 days)

**Authorization**: Only `dba` and `system` roles

---

#### D. Restore Deleted User (DBA Only)

```sql
-- Standard SQL UPDATE syntax
UPDATE system.users 
SET deleted_at = NULL 
WHERE user_id = 'alice';
```

**Requirements**:
- Must be within grace period (default: 30 days from deletion)
- Only `dba` and `system` roles can restore users

---

### 2. Standard SQL DML on system.users

#### A. INSERT INTO system.users

```sql
INSERT INTO system.users (user_id, username, auth_type, auth_data, role, email, metadata)
VALUES (
  'alice123', 
  'alice', 
  'password', 
  '$2b$12$KIXxNv8f...', 
  'user',
  'alice@example.com',
  '{"department": "engineering"}'
);
```

**Auto-Generated Fields**:
- `created_at`: Set to `NOW()`
- `updated_at`: Set to `NOW()`
- `deleted_at`: Set to `NULL`

**Validation**:
- `user_id` must be unique
- `username` must be unique
- `auth_type` must be 'password', 'oauth', or 'internal'
- `role` must be 'user', 'service', 'dba', or 'system'
- `metadata` must be valid JSON (if provided)

**Error**: `User with user_id 'X' already exists`

---

#### B. UPDATE system.users

```sql
-- Partial update (only specified fields)
UPDATE system.users 
SET username = 'alice_new', 
    email = 'alice.new@example.com'
WHERE user_id = 'alice123';

-- Update metadata
UPDATE system.users
SET metadata = '{"department": "frontend", "level": "senior"}'
WHERE user_id = 'alice123';

-- Change role (DBA only)
UPDATE system.users
SET role = 'service'
WHERE username = 'integration_bot';
```

**Auto-Updated Field**:
- `updated_at`: Automatically set to `NOW()`

**Validation**:
- User must exist
- `metadata` must be valid JSON (if provided)
- Role changes require `dba` or `system` role

**Error**: `User with user_id 'X' not found`

---

#### C. DELETE FROM system.users (Soft Delete)

```sql
DELETE FROM system.users 
WHERE user_id = 'alice123';
```

**Behavior**:
- Sets `deleted_at = NOW()`
- User hidden from default SELECT queries
- User's tables remain accessible during grace period

**Error**: `User with user_id 'X' not found`

---

#### D. SELECT FROM system.users

**Default (excludes deleted users)**:
```sql
SELECT * FROM system.users;
-- Implicit: WHERE deleted_at IS NULL
```

**View only active users (explicit)**:
```sql
SELECT user_id, username, role, created_at 
FROM system.users 
WHERE deleted_at IS NULL;
```

**View only deleted users (DBA only)**:
```sql
SELECT user_id, username, deleted_at, 
       DATEDIFF(day, deleted_at, NOW()) AS days_until_cleanup
FROM system.users 
WHERE deleted_at IS NOT NULL;
```

**Filter by role**:
```sql
SELECT * FROM system.users 
WHERE role = 'service'
ORDER BY created_at DESC;
```

**Search by username pattern**:
```sql
SELECT * FROM system.users 
WHERE username LIKE '%admin%';
```

---

### 3. Shared Table Access Control

#### A. CREATE SHARED TABLE with Access Level

**Public shared table** (any user can read):
```sql
CREATE SHARED TABLE app.analytics (
  id BIGINT NOT NULL DEFAULT SNOWFLAKE_ID(),
  event_name TEXT NOT NULL,
  event_data JSON,
  created_at TIMESTAMP NOT NULL DEFAULT NOW()
) 
ACCESS public
STORAGE local
FLUSH ROW_THRESHOLD 10000;
```

**Private shared table** (only service/dba/system can access):
```sql
CREATE SHARED TABLE app.internal_metrics (
  metric_id BIGINT NOT NULL DEFAULT SNOWFLAKE_ID(),
  metric_name TEXT NOT NULL,
  value DOUBLE NOT NULL,
  recorded_at TIMESTAMP NOT NULL DEFAULT NOW()
)
ACCESS private
STORAGE local
FLUSH INTERVAL 300;
```

**Default access** (if omitted, defaults to `private`):
```sql
CREATE SHARED TABLE app.secrets (
  secret_key TEXT NOT NULL,
  secret_value TEXT NOT NULL
)
STORAGE local;
-- Implicitly: ACCESS private
```

---

#### B. ALTER TABLE SET ACCESS

**Change access level after creation**:
```sql
-- Make public table private
ALTER TABLE app.analytics SET ACCESS private;

-- Make private table public
ALTER TABLE app.internal_metrics SET ACCESS public;

-- Use restricted (reserved for future fine-grained permissions)
ALTER TABLE app.sensitive_data SET ACCESS restricted;
```

**Authorization**: Only `dba` and `system` roles

---

## Soft Delete Mechanism

### Grace Period Configuration

**File**: `backend/config.toml`

```toml
[user_management]
# Number of days before soft-deleted users are permanently removed
deletion_grace_period_days = 30  # Default: 30 days

# Cleanup job schedule (cron expression)
cleanup_job_schedule = "0 2 * * *"  # Daily at 2:00 AM
```

---

### Scheduled Cleanup Job

**Job Name**: `user_cleanup_job`  
**Schedule**: Daily at 2:00 AM (configurable)  
**Logic**:

```rust
// Pseudo-code for cleanup job
fn cleanup_deleted_users(grace_period_days: i64) {
    let cutoff_time = now() - Duration::days(grace_period_days);
    
    // Find users to delete permanently
    let users_to_delete = SELECT user_id, username 
                          FROM system.users 
                          WHERE deleted_at IS NOT NULL 
                          AND deleted_at < cutoff_time;
    
    for user in users_to_delete {
        // Delete user's tables
        DELETE FROM system.tables 
        WHERE namespace_id LIKE format!("{}:%", user.user_id);
        
        // Delete user record
        DELETE FROM system.users 
        WHERE user_id = user.user_id;
        
        log_info!("Permanently deleted user: {}", user.username);
    }
}
```

**System Table Entry** (in `system.jobs`):
```json
{
  "job_id": "cleanup_001",
  "job_type": "user_cleanup",
  "status": "completed",
  "started_at": "2025-11-26T02:00:00Z",
  "completed_at": "2025-11-26T02:00:15Z",
  "result": "Deleted 3 users: [user123, old_service, test_account]"
}
```

---

### Restore Within Grace Period

**DBA can restore users before permanent deletion**:

```sql
-- Check deleted users and time remaining
SELECT user_id, username, deleted_at,
       30 - DATEDIFF(day, deleted_at, NOW()) AS days_remaining
FROM system.users
WHERE deleted_at IS NOT NULL;

-- Restore user
UPDATE system.users 
SET deleted_at = NULL 
WHERE user_id = 'alice123';
```

**Effect**:
- User becomes active again
- Scheduled cleanup job skips this user
- User can log in and access their tables

---

## Architecture: kalamdb-auth Crate

### New Crate: `backend/crates/kalamdb-auth/`

**Purpose**: Centralized authentication and authorization logic

**Responsibilities**:
1. HTTP Basic Auth parsing and validation
2. JWT token parsing and validation (internal + external)
3. Password verification (bcrypt)
4. User lookup in `system.users`
5. Role extraction and validation
6. Connection source detection (localhost vs remote)
7. System user access control enforcement
8. Authenticated session context creation

**Public API**:
```rust
// File: backend/crates/kalamdb-auth/src/lib.rs

pub struct AuthService {
    db: Arc<DB>,
    config: AuthConfig,
}

pub struct AuthenticatedUser {
    pub user_id: String,
    pub username: String,
    pub role: UserRole,  // enum from kalamdb-commons
    pub connection_info: ConnectionInfo,
}

pub struct ConnectionInfo {
    pub remote_addr: IpAddr,
    pub is_localhost: bool,
}

impl AuthService {
    /// Authenticate request using Authorization header
    pub async fn authenticate(
        &self,
        auth_header: &str,
        connection_info: ConnectionInfo,
    ) -> Result<AuthenticatedUser, AuthError>;
    
    /// Parse and validate HTTP Basic Auth
    fn authenticate_basic(&self, credentials: &str, conn: &ConnectionInfo) 
        -> Result<AuthenticatedUser, AuthError>;
    
    /// Parse and validate Bearer JWT token
    fn authenticate_bearer(&self, token: &str, conn: &ConnectionInfo) 
        -> Result<AuthenticatedUser, AuthError>;
    
    /// Verify password using bcrypt
    fn verify_password(&self, plaintext: &str, hash: &str) -> Result<bool>;
    
    /// Enforce system user access control
    fn check_system_user_access(&self, user: &User, conn: &ConnectionInfo) 
        -> Result<()>;
}
```

**Dependencies**:
- `bcrypt` - Password hashing and verification
- `jsonwebtoken` - JWT parsing and validation
- `base64` - Basic Auth decoding
- `kalamdb-commons` - UserRole enum, error types
- `kalamdb-store` - Database access for user lookup

**Error Types**:
```rust
// File: backend/crates/kalamdb-auth/src/error.rs

#[derive(Debug, thiserror::Error)]
pub enum AuthError {
    #[error("Missing Authorization header")]
    MissingAuthHeader,
    
    #[error("Invalid Authorization header format")]
    InvalidAuthHeader,
    
    #[error("Invalid username or password")]
    InvalidCredentials,
    
    #[error("User not found: {0}")]
    UserNotFound(String),
    
    #[error("JWT validation failed: {0}")]
    JwtValidationFailed(String),
    
    #[error("JWT expired")]
    JwtExpired,
    
    #[error("Invalid JWT issuer")]
    InvalidIssuer,
    
    #[error("System users can only connect from localhost")]
    SystemUserLocalhostOnly,
    
    #[error("System users with remote access must have a password")]
    SystemUserRemoteAccessRequiresPassword,
    
    #[error("User account has been deleted")]
    UserDeleted,
}
```

---

## Implementation Requirements

### Functional Requirements

#### FR-USER-001: User Management Commands
- System MUST support `CREATE USER` syntax with password and OAuth authentication
- System MUST support `ALTER USER` for password and role changes
- System MUST support `DROP USER` for soft deletion
- System MUST support role specification: `user`, `service`, `dba`, `system`

#### FR-USER-002: Standard SQL DML on system.users
- System MUST support `INSERT INTO system.users`
- System MUST support `UPDATE system.users` with partial updates
- System MUST support `DELETE FROM system.users` (soft delete)
- System MUST support `SELECT FROM system.users` with filtering

#### FR-USER-003: Soft Delete Behavior
- DELETE operation MUST set `deleted_at` to current timestamp
- Deleted users MUST be hidden from default SELECT queries
- Deleted users MUST remain in database during grace period
- System MUST add `deleted_at` column (TIMESTAMP, nullable) to schema

#### FR-USER-004: Grace Period and Cleanup
- System MUST support configurable grace period (default: 30 days)
- Scheduled cleanup job MUST run daily (configurable schedule)
- Cleanup MUST permanently delete users where `deleted_at + grace_period < NOW()`
- Cleanup MUST also delete all tables owned by deleted users

#### FR-USER-005: User Restoration
- DBAs MUST be able to restore deleted users within grace period
- Restoration MUST set `deleted_at = NULL`
- Restoration MUST cancel scheduled cleanup for that user

#### FR-USER-006: Validation
- System MUST enforce unique `user_id` constraint
- System MUST enforce unique `username` constraint
- System MUST validate `metadata` as valid JSON
- System MUST validate `role` enum values
- System MUST validate `auth_type` enum values

#### FR-USER-007: Auto-Generated Fields
- `created_at` MUST be auto-set to NOW() on INSERT
- `updated_at` MUST be auto-updated to NOW() on UPDATE
- `deleted_at` MUST be auto-set to NOW() on DELETE

#### FR-USER-008: Role-Based Access Control
- System MUST enforce role permissions on all operations
- System MUST validate user role from JWT token
- System MUST block unauthorized operations with clear error messages

#### FR-USER-009: Shared Table Access Control
- System MUST support `ACCESS` clause in CREATE SHARED TABLE
- System MUST enforce access levels: `public`, `private`, `restricted`
- System MUST allow ALTER TABLE SET ACCESS
- Default access MUST be `private` if omitted

#### FR-USER-010: Password Security
- Passwords MUST be hashed with bcrypt (cost factor 12)
- System MUST enforce minimum password length (8 characters)
- System MUST reject weak passwords (optional: complexity rules)

#### FR-USER-011: OAuth Support
- System MUST support OAuth authentication via `auth_type = 'oauth'`
- OAuth data MUST be stored as JSON: `{"provider": "...", "subject": "..."}`
- System MUST validate OAuth tokens with configured providers

#### FR-USER-012: System User Access Control
- System MUST support `auth_type = 'internal'` for localhost-only system users
- System users MUST be accessible from localhost without password by default
- System users with remote access enabled MUST have a password configured
- System MUST reject remote connections to system users without passwords
- System MUST validate connection source (localhost vs remote) before authentication
- Config.toml MUST support `allow_remote_access` flag for system users globally
- Individual system users MUST support `allow_remote` metadata flag

#### FR-USER-013: Authentication Methods
- System MUST support HTTP Basic Authentication (username:password)
- System MUST support Bearer JWT token authentication
- System MUST validate JWT signatures using configured keys
- System MUST support both internal JWT issuance and external JWT validation
- System MUST extract user_id from JWT `sub` claim
- System MUST optionally verify user existence in system.users for JWT auth

#### FR-USER-014: Removed Authentication Methods
- System MUST NOT use X-API-KEY header for authentication
- System MUST NOT use X-USER-ID header for user identification
- All API endpoints MUST use Authorization header exclusively

#### FR-USER-015: kalamdb-auth Crate
- New `kalamdb-auth` crate MUST be created in `backend/crates/`
- kalamdb-auth MUST handle all authentication logic
- kalamdb-auth MUST provide AuthService for HTTP Basic and JWT validation
- kalamdb-auth MUST enforce system user access control
- kalamdb-auth MUST return AuthenticatedUser context with role

#### FR-USER-016: UserRole Enum
- UserRole enum MUST be defined in `kalamdb-commons` crate
- UserRole MUST be accessible across all crates
- UserRole MUST implement Serialize, Deserialize, Debug, Clone, PartialEq, Eq
- UserRole variants: User, Service, Dba, System

#### FR-USER-017: TableAccess Enum
- TableAccess enum MUST be defined in `kalamdb-commons` crate
- TableAccess MUST be accessible across all crates
- TableAccess MUST implement Serialize, Deserialize, Debug, Clone, PartialEq, Eq
- TableAccess variants: Public, Private, Restricted
- TableAccess MUST be used for `access` column in system.tables
- TableAccess MUST have Default trait implementation (default: Private)

---

### Non-Functional Requirements

#### NFR-USER-001: Performance
- User lookups by `user_id` MUST complete in < 5ms (indexed)
- User lookups by `username` MUST complete in < 10ms (indexed)
- Cleanup job MUST process 10,000 users in < 60 seconds

#### NFR-USER-002: Security
- All passwords MUST be hashed (never stored in plaintext)
- DBA actions MUST be logged to audit trail
- Failed login attempts MUST be rate-limited (future: account lockout)

#### NFR-USER-003: Scalability
- System MUST support 1,000,000+ users
- Soft delete queries MUST not degrade performance
- Grace period cleanup MUST not block active operations

---

## Security Considerations

### 1. Password Hashing

**Algorithm**: bcrypt  
**Cost Factor**: 12 (adjustable in config.toml)  
**Salt**: Auto-generated per password

```rust
use bcrypt::{hash, verify, DEFAULT_COST};

// On user creation
let hashed = hash(plaintext_password, DEFAULT_COST)?;

// On login
let is_valid = verify(plaintext_password, &stored_hash)?;
```

---

### 2. Authentication Methods

KalamDB supports **two authentication methods**:

#### A. HTTP Basic Authentication

**Format**: `Authorization: Basic <base64(username:password)>`

**Flow**:
1. Client sends username and password in Authorization header
2. `kalamdb-auth` extracts credentials
3. Lookup user in `system.users` by username
4. Verify password using bcrypt against `auth_data` column
5. Check role and connection source (localhost vs remote)
6. Create authenticated session context

**Example**:
```http
POST /v1/api/sql HTTP/1.1
Authorization: Basic YWxpY2U6c2VjcmV0MTIz
Content-Type: application/json

{"sql": "SELECT * FROM app.todos"}
```

**Use Cases**:
- Simple clients without JWT support
- Testing and development
- Emergency administrative access
- Internal services on same network

---

#### B. Bearer JWT Token

**Format**: `Authorization: Bearer <jwt_token>`

**JWT Claims** (externally issued or KalamDB-issued):
```json
{
  "sub": "alice123",        // user_id (REQUIRED)
  "username": "alice",      // username (optional, looked up if missing)
  "role": "user",           // user role (optional, looked up if missing)
  "iat": 1698422400,        // issued at (REQUIRED)
  "exp": 1698508800,        // expires (REQUIRED)
  "iss": "external-auth"    // issuer (validates against config.toml)
}
```

**Validation Flow**:
1. Client sends JWT in Authorization header
2. `kalamdb-auth` extracts token
3. Validate signature using configured public key or secret
4. Verify issuer matches allowed issuers in config.toml
5. Check expiration timestamp
6. Extract user_id from `sub` claim
7. Optionally verify user exists in `system.users`
8. Extract or lookup role
9. Create authenticated session context

**Configuration** (in config.toml):
```toml
[authentication.jwt]
# JWT validation mode: "internal" or "external"
validation_mode = "external"

# For internal mode (KalamDB issues JWTs)
secret_key = "your-256-bit-secret-key"
algorithm = "HS256"  # HS256, HS384, HS512, RS256, RS384, RS512

# For external mode (validate externally issued JWTs)
issuer = "https://auth.company.com"  # Required issuer claim
public_key_url = "https://auth.company.com/.well-known/jwks.json"
# OR
public_key_file = "/etc/kalamdb/public_key.pem"

# Token expiration for internal mode (in seconds)
expiration_seconds = 86400  # 24 hours

# Verify user exists in system.users (recommended)
verify_user_exists = true
```

**Example**:
```http
POST /v1/api/sql HTTP/1.1
Authorization: Bearer eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9...
Content-Type: application/json

{"sql": "SELECT * FROM app.todos"}
```

**Use Cases**:
- Single Sign-On (SSO) integration
- OAuth2/OIDC flows
- Microservices with shared auth
- Mobile and web applications

---

### 3. System User Security

**System users** (`role = 'system'`):

**Localhost-Only (Default)**:
- Accessible from 127.0.0.1, ::1, or Unix socket only
- No password required for localhost connections
- Used by internal processes (flush jobs, compaction, replication)
- Middleware blocks all non-localhost connections
- Marked with `auth_type = 'internal'` and `auth_data = null`

**Remote Access (Optional, Must Be Explicitly Configured)**:
- Disabled by default for security
- Requires `allow_remote = true` in user metadata or config.toml
- **Password is MANDATORY** when remote access is enabled
- Connections without password are **rejected** with error
- Must authenticate via HTTP Basic Auth or Bearer JWT
- Used for emergency administrative access only
- All remote system user actions logged to audit trail

**Enforcement Logic** (in `kalamdb-auth` crate):
```rust
fn authenticate_system_user(user: &User, connection_info: &ConnectionInfo) -> Result<()> {
    // Check if connection is from localhost
    let is_localhost = connection_info.is_localhost();
    
    if is_localhost {
        // Localhost: no password required
        return Ok(());
    }
    
    // Non-localhost: check if remote access allowed
    let allow_remote = user.metadata.get("allow_remote")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    
    if !allow_remote {
        return Err(AuthError::SystemUserLocalhostOnly);
    }
    
    // Remote access allowed, but password is REQUIRED
    if user.auth_data.is_none() || user.auth_data == "" {
        return Err(AuthError::SystemUserRemoteAccessRequiresPassword);
    }
    
    // Validate password via HTTP Basic Auth or JWT
    Ok(())
}
```

---

### 4. Audit Logging

**DBA actions that MUST be logged**:
- CREATE USER
- ALTER USER (role changes)
- DROP USER
- Restore deleted user
- ALTER TABLE SET ACCESS

**Log Format** (in `system.audit_log` - future enhancement):
```json
{
  "audit_id": "audit_001",
  "timestamp": "2025-10-27T14:30:00Z",
  "user_id": "admin_dba",
  "action": "CREATE_USER",
  "target": "alice123",
  "details": {"role": "user", "email": "alice@example.com"},
  "ip_address": "192.168.1.100"
}
```

---

## Testing Requirements

### Integration Tests (15 tests)

**File**: `backend/tests/integration/test_user_management_sql.rs`

#### 1. User Creation Tests (3 tests)
- `test_create_user_with_password`: CREATE USER with password auth
- `test_create_user_with_oauth`: CREATE USER with OAuth auth
- `test_create_user_duplicate_error`: Verify uniqueness constraint

#### 2. User Update Tests (3 tests)
- `test_update_user_partial`: UPDATE only username, verify other fields unchanged
- `test_update_user_metadata`: UPDATE metadata with valid JSON
- `test_update_user_not_found`: UPDATE non-existent user, verify error

#### 3. User Deletion Tests (3 tests)
- `test_delete_user_soft_delete`: DELETE sets deleted_at, user hidden from SELECT
- `test_delete_user_not_found`: DELETE non-existent user, verify error
- `test_restore_deleted_user`: UPDATE deleted_at = NULL within grace period

#### 4. User Query Tests (2 tests)
- `test_select_users_excludes_deleted`: Default SELECT hides deleted users
- `test_select_deleted_users_explicit`: WHERE deleted_at IS NOT NULL shows deleted

#### 5. Grace Period Tests (2 tests)
- `test_cleanup_job_deletes_expired_users`: Simulate cleanup after 30 days
- `test_user_tables_deleted_with_user`: Verify cascade delete of user's tables

#### 6. Role-Based Access Tests (2 tests)
- `test_user_role_cannot_create_user`: Verify role='user' cannot CREATE USER
- `test_service_role_access_private_table`: Verify role='service' can access private shared tables

---

### Unit Tests (10 tests)

**File**: `backend/crates/kalamdb-sql/src/ddl/user_management.rs`

- `test_parse_create_user_password`
- `test_parse_create_user_oauth`
- `test_parse_create_user_internal`
- `test_parse_alter_user_password`
- `test_parse_alter_user_role`
- `test_parse_drop_user`
- `test_validate_role_enum`
- `test_validate_auth_type_enum`
- `test_bcrypt_password_hashing`
- `test_jwt_role_extraction`

---

## Migration Guide

### From Current Schema to Enhanced Schema

**Step 1: Add new columns to system.users**

```sql
-- Add role column
ALTER TABLE system.users 
ADD COLUMN role TEXT NOT NULL DEFAULT 'user';

-- Add auth columns
ALTER TABLE system.users 
ADD COLUMN auth_type TEXT NOT NULL DEFAULT 'password';

ALTER TABLE system.users 
ADD COLUMN auth_data TEXT;

-- Add metadata column
ALTER TABLE system.users 
ADD COLUMN metadata JSONB;

-- Add soft delete column
ALTER TABLE system.users 
ADD COLUMN deleted_at TIMESTAMP;
```

**Step 2: Add access column to system.tables**

```sql
ALTER TABLE system.tables 
ADD COLUMN access TEXT;
```

**Step 3: Migrate existing users**

```sql
-- Set auth_type for existing users
UPDATE system.users 
SET auth_type = 'password'
WHERE auth_type IS NULL;

-- Generate placeholder hashes (users must reset passwords)
UPDATE system.users 
SET auth_data = '<PLACEHOLDER_HASH>'
WHERE auth_data IS NULL AND auth_type = 'password';
```

**Step 4: Create indexes for performance**

```sql
CREATE INDEX idx_users_username ON system.users(username);
CREATE INDEX idx_users_role ON system.users(role);
CREATE INDEX idx_users_deleted_at ON system.users(deleted_at);
CREATE INDEX idx_tables_access ON system.tables(access);
```

---

## Configuration Reference

**File**: `backend/config.toml`

```toml
[authentication]
# Authentication mode: "basic", "jwt", or "both" (default: "both")
mode = "both"

# Bcrypt cost factor (10-12 recommended, higher = more secure but slower)
bcrypt_cost = 12

# Minimum password length
min_password_length = 8

# Enforce password complexity (uppercase, lowercase, digit, special char)
enforce_password_complexity = true

# System user remote access control
[authentication.system_users]
# Allow remote access to system users globally (default: false)
allow_remote_access = false

# Specific system users allowed remote access (requires passwords)
# Example: remote_allowed = ["system_admin", "emergency_dba"]
remote_allowed = []

# JWT Configuration
[authentication.jwt]
# JWT validation mode: "internal" (KalamDB issues tokens) or "external" (validate external tokens)
validation_mode = "internal"  # Options: "internal", "external"

# For internal mode (KalamDB issues JWTs after successful auth)
[authentication.jwt.internal]
# Secret key for signing JWTs (MUST be changed in production)
secret_key = "your-256-bit-secret-key-change-me-in-production"

# Algorithm: HS256, HS384, HS512, RS256, RS384, RS512
algorithm = "HS256"

# Token expiration in seconds (default: 24 hours)
expiration_seconds = 86400

# For external mode (validate JWTs issued by external auth provider)
[authentication.jwt.external]
# Required issuer claim (e.g., "https://auth.company.com")
issuer = "https://auth.example.com"

# Public key for RS256/RS384/RS512 validation
# Option 1: JWKS URL (JSON Web Key Set)
public_key_url = "https://auth.example.com/.well-known/jwks.json"

# Option 2: Public key file path (PEM format)
# public_key_file = "/etc/kalamdb/auth_public_key.pem"

# Verify user exists in system.users (recommended for security)
verify_user_exists = true

# Allow JWT without role claim (will lookup from system.users)
allow_role_lookup = true

# HTTP Basic Authentication
[authentication.basic]
# Enable HTTP Basic Auth (default: true)
enabled = true

# Session timeout for Basic Auth (in seconds, default: 1 hour)
session_timeout = 3600

[user_management]
# Grace period before permanent deletion (in days)
deletion_grace_period_days = 30

# Cleanup job schedule (cron expression: "sec min hour day month weekday")
cleanup_job_schedule = "0 2 * * *"  # Daily at 2:00 AM

# Allow users to change their own passwords
allow_self_password_change = true

[oauth]
# Enable OAuth authentication (future enhancement)
enabled = false

# Supported providers (comma-separated)
providers = "google,github,azure"

# OAuth configuration per provider (future enhancement)
# [oauth.google]
# client_id = "..."
# client_secret = "..."
# redirect_uri = "http://localhost:2900/auth/callback/google"
```

---

### Example Configurations

#### Development (Internal JWT, Basic Auth Enabled)
```toml
[authentication]
mode = "both"
bcrypt_cost = 10  # Lower cost for faster dev iteration

[authentication.system_users]
allow_remote_access = false  # Localhost only

[authentication.jwt]
validation_mode = "internal"

[authentication.jwt.internal]
secret_key = "dev-secret-key-not-for-production"
algorithm = "HS256"
expiration_seconds = 86400

[authentication.basic]
enabled = true
```

#### Production (External JWT, SSO Integration)
```toml
[authentication]
mode = "jwt"  # Only JWT, disable Basic Auth
bcrypt_cost = 12

[authentication.system_users]
allow_remote_access = true
remote_allowed = ["emergency_admin"]

[authentication.jwt]
validation_mode = "external"

[authentication.jwt.external]
issuer = "https://auth.company.com"
public_key_url = "https://auth.company.com/.well-known/jwks.json"
verify_user_exists = true
allow_role_lookup = true

[authentication.basic]
enabled = false  # Disable for production SSO
```

#### Hybrid (Both Internal and Basic Auth)
```toml
[authentication]
mode = "both"
bcrypt_cost = 12

[authentication.system_users]
allow_remote_access = false

[authentication.jwt]
validation_mode = "internal"

[authentication.jwt.internal]
secret_key = "production-secret-key-256-bit-minimum"
algorithm = "HS256"
expiration_seconds = 28800  # 8 hours

[authentication.basic]
enabled = true
session_timeout = 3600  # 1 hour
```

---

## Implementation Phases

### Phase 1: Authentication Infrastructure (Week 1-2)
- [ ] Create `kalamdb-auth` crate in `backend/crates/kalamdb-auth/`
- [ ] Add UserRole enum to `kalamdb-commons` crate
  - [ ] Define enum: User, Service, Dba, System
  - [ ] Implement Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Hash
  - [ ] Add Display and FromStr traits
  - [ ] Add unit tests for parsing and conversion
- [ ] Add TableAccess enum to `kalamdb-commons` crate
  - [ ] Define enum: Public, Private, Restricted
  - [ ] Implement Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Hash
  - [ ] Add Default trait (default: Private)
  - [ ] Add Display and FromStr traits
  - [ ] Add unit tests for parsing and conversion
- [ ] Implement HTTP Basic Auth in kalamdb-auth
  - [ ] Parse Authorization header
  - [ ] Extract and decode base64 credentials
  - [ ] Lookup user in system.users
  - [ ] Verify password with bcrypt
- [ ] Implement JWT Bearer token validation in kalamdb-auth
  - [ ] Parse Authorization header
  - [ ] Validate JWT signature (internal + external modes)
  - [ ] Extract claims (sub, username, role, exp, iss)
  - [ ] Verify issuer and expiration
- [ ] Implement system user access control
  - [ ] Detect localhost connections (127.0.0.1, ::1)
  - [ ] Enforce localhost-only for system users by default
  - [ ] Enforce password requirement for remote system users
- [ ] Remove X-API-KEY and X-USER-ID authentication
  - [ ] Update all API handlers to use Authorization header
  - [ ] Remove API key generation code
  - [ ] Update integration tests
- [ ] 15 unit tests for kalamdb-auth

### Phase 2: User Management Schema (Week 2-3)
- [ ] Add `role`, `auth_type`, `auth_data`, `metadata`, `deleted_at` columns to schema
- [ ] Update system.users table provider
- [ ] Implement CREATE USER command (password auth)
- [ ] Implement CREATE USER with ALLOW_REMOTE flag
- [ ] Implement ALTER USER command
- [ ] Implement DROP USER command (soft delete)
- [ ] Implement standard SQL INSERT/UPDATE/DELETE on system.users
- [ ] Add password validation (role=system + remote access requires password)
- [ ] 10 unit tests

### Phase 3: Role-Based Access Control (Week 3-4)
- [ ] Integrate kalamdb-auth with API middleware
  - [ ] Replace API key middleware with AuthService
  - [ ] Extract AuthenticatedUser from all requests
  - [ ] Pass user context to query engine
- [ ] Implement role enforcement middleware
  - [ ] Check role for CREATE/DROP operations
  - [ ] Check role for system table access
  - [ ] Check role for cross-user table access
- [ ] Add `access` column to system.tables (type: TableAccess enum)
  - [ ] Update schema to use TableAccess enum
  - [ ] Convert string to enum on read, enum to string on write
  - [ ] Validate enum values in SQL parser
- [ ] Implement CREATE SHARED TABLE with ACCESS clause
  - [ ] Parse ACCESS keyword (public/private/restricted)
  - [ ] Convert to TableAccess enum
  - [ ] Default to TableAccess::Private if omitted
- [ ] Implement ALTER TABLE SET ACCESS
  - [ ] Parse new access level
  - [ ] Validate enum value
  - [ ] Update system.tables with TableAccess enum
- [ ] Enforce role permissions on all operations
  - [ ] Use TableAccess enum for access control logic
  - [ ] Check user role against table access level
- [ ] 12 integration tests (authentication + RBAC)

### Phase 4: Soft Delete & Cleanup (Week 4)
- [ ] Implement soft delete logic (deleted_at timestamp)
- [ ] Implement default SELECT filter (WHERE deleted_at IS NULL)
- [ ] Implement grace period configuration
- [ ] Implement scheduled cleanup job
- [ ] Implement user restoration (UPDATE deleted_at = NULL)
- [ ] 4 integration tests (soft delete, cleanup, restore)

### Phase 5: OAuth & Advanced Features (Week 5)
- [ ] Implement OAuth authentication (CREATE USER WITH OAUTH)
- [ ] Configure external JWT validation with JWKS
- [ ] Add audit logging for DBA actions
- [ ] Add password complexity validation
- [ ] Update CLI to use HTTP Basic Auth or JWT
- [ ] Update example applications
- [ ] 5 integration tests (OAuth, JWT external, system user remote access)

---

## Success Criteria

✅ **Definition of Done**:
1. kalamdb-auth crate implemented with full test coverage
2. UserRole enum available in kalamdb-commons (with Display, FromStr, tests)
3. TableAccess enum available in kalamdb-commons (with Display, FromStr, Default, tests)
4. HTTP Basic Auth working (username:password)
5. JWT Bearer token validation working (internal + external modes)
6. X-API-KEY and X-USER-ID removed completely
7. System user localhost-only enforcement working
8. System user remote access with password validation working
9. All 20+ integration tests passing
10. All 30+ unit tests passing (kalamdb-auth + user management + enums)
11. CREATE/ALTER/DROP USER commands working
12. Standard SQL INSERT/UPDATE/DELETE on system.users working
13. Role-based access control enforced on all operations using UserRole enum
14. Table access control enforced using TableAccess enum
15. CREATE SHARED TABLE with ACCESS clause working (validates TableAccess enum)
16. ALTER TABLE SET ACCESS working (validates TableAccess enum)
17. Soft delete with grace period functional
18. Scheduled cleanup job running daily
19. User restoration working within grace period
20. Password hashing with bcrypt implemented
21. Configuration complete in config.toml (JWT, Basic Auth, system users)
22. Documentation complete (this document + code comments + API docs)

---

## References

- **Main Spec**: `specs/004-system-improvements-and/spec.md` (US10)
- **Tasks**: `specs/004-system-improvements-and/tasks.md` (T555-T569)
- **Data Model**: `specs/004-system-improvements-and/data-model.md` (UserRecord entity)
- **Current Schema**: `backend/crates/kalamdb-core/src/tables/system/users.rs`
- **Next Steps**: `Next.md` (Section 7: User Management SQL)
- **Authentication Crate**: `backend/crates/kalamdb-auth/` (new)
- **Commons Enums**: `backend/crates/kalamdb-commons/src/models.rs` (UserRole and TableAccess enums)

---

**Document Version**: 1.0  
**Last Updated**: October 27, 2025  
**Author**: KalamDB Development Team  
**Status**: Requirements Approved ✅
