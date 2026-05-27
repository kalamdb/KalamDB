# ADR-015: Enum Usage Policy for Type Safety

**Status**: Accepted  
**Date**: 2025-10-25  
**Author**: KalamDB Team  
**Context**: User Story 6 (Code Quality) - T416

## Context and Problem Statement

KalamDB's codebase had several string-based state values that could benefit from stronger typing:
- Job statuses ("running", "completed", "failed", "cancelled")
- Job types ("flush", "compact", "cleanup", "backup", "restore")
- Table types (already enum - "user", "shared", "stream", "system")
- Storage types (already enum - "filesystem", "s3")

Using strings for these values leads to:
1. **Runtime Errors**: Typos like "runing" instead of "running" not caught until execution
2. **Incomplete Pattern Matching**: Missing cases for state transitions
3. **Unclear APIs**: Function parameters like `status: String` don't convey allowed values
4. **Maintenance Burden**: Adding new states requires searching all string literals

## Decision Drivers

- **Exhaustiveness**: Compiler enforces handling of all enum variants
- **Type Safety**: Invalid state values rejected at compile time
- **Discoverability**: IDE autocomplete shows all valid values
- **Refactoring Safety**: Renaming a variant updates all uses
- **Domain Modeling**: Business states explicitly modeled in code

## Considered Options

### Option 1: Continue Using Plain Strings
**Pros**: Flexible, easy serialization, no code changes  
**Cons**: No compile-time safety, typo-prone, unclear constraints

### Option 2: String Constants (`const STATUS_RUNNING: &str = "running"`)
**Pros**: Centralized definitions, easier to find usages  
**Cons**: Still runtime errors if wrong constant used, no exhaustiveness checking

### Option 3: Enums with String Conversion (SELECTED)
**Pros**: Full type safety, exhaustive matching, clear constraints  
**Cons**: Requires serialization helpers, more upfront code

### Option 4: Type-Safe String Wrappers (like newtype pattern)
**Pros**: Type safety, ergonomic  
**Cons**: No exhaustiveness checking, unclear valid values

## Decision Outcome

**Chosen Option**: **Enums with String Conversion Helpers**

We use Rust enums for all fixed-set state values, with comprehensive trait implementations for string conversion and serialization.

### Implementation Pattern

```rust
/// Job execution status
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum JobStatus {
    /// Job is currently executing
    Running,
    /// Job completed successfully
    Completed,
    /// Job failed with error
    Failed,
    /// Job was cancelled by user
    Cancelled,
}

impl JobStatus {
    /// Convert enum to string representation
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }
    
    /// Parse string into enum
    pub fn from_str(s: &str) -> Result<Self, String> {
        match s.to_lowercase().as_str() {
            "running" => Ok(Self::Running),
            "completed" => Ok(Self::Completed),
            "failed" => Ok(Self::Failed),
            "cancelled" => Ok(Self::Cancelled),
            _ => Err(format!("Invalid job status: {}", s)),
        }
    }
}

impl Display for JobStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

impl From<&str> for JobStatus {
    fn from(s: &str) -> Self {
        Self::from_str(s).unwrap_or(Self::Failed)  // Or panic for strict mode
    }
}

impl From<String> for JobStatus {
    fn from(s: String) -> Self {
        Self::from(&s)
    }
}
```

## When to Use Enums

### ✅ Use Enums When:

1. **Fixed Set of Values**: The set of valid values is known and unlikely to change frequently
   - ✅ Job statuses: Running, Completed, Failed, Cancelled
   - ✅ Job types: Flush, Compact, Cleanup, Backup, Restore
   - ✅ Table types: User, Shared, Stream, System
   - ✅ User roles: User, Service, DBA, System

2. **State Machines**: Values represent distinct states with transitions
   - ✅ Job lifecycle: Pending → Running → (Completed | Failed | Cancelled)
   - ✅ Connection states: Connecting → Connected → Disconnected

3. **Domain Modeling**: Business concepts with specific behaviors
   - ✅ Storage types: Filesystem, S3, Azure, GCS (each has specific configuration)

4. **Pattern Matching**: Code needs to handle different cases differently
   ```rust
   match job.status {
       JobStatus::Running => continue_execution(),
       JobStatus::Completed => return_result(),
       JobStatus::Failed => retry_or_report(),
       JobStatus::Cancelled => cleanup(),
   }
   ```

### ❌ Avoid Enums When:

1. **Open-Ended Values**: Set of values is unbounded or frequently changing
   - ❌ Table names (user-defined, infinite possibilities)
   - ❌ User IDs (arbitrary identifiers)
   - ❌ SQL query text (unpredictable content)

2. **User Input**: Values come directly from users without validation
   - ❌ Search queries
   - ❌ Freeform text fields
   - ❌ Custom metadata

3. **External Integration**: Values defined by third-party systems
   - ⚠️ HTTP status codes (use established crates instead)
   - ⚠️ Error codes from external APIs (wrap if needed)
    - ❌ OIDC provider families or identity providers. Store issuer and subject as data, because the set of providers is open-ended and controlled outside KalamDB.

4. **Performance Critical Paths**: Enum-to-string conversion is bottleneck
   - ⚠️ Usually not an issue, but profile first

## Enums Implemented (Tasks T383-T385)

### 1. JobStatus (`kalamdb-commons/src/models/mod.rs`)
```rust
pub enum JobStatus {
    Running,    // Job is executing
    Completed,  // Finished successfully
    Failed,     // Error occurred
    Cancelled,  // User/system terminated
}
```
**Usage**: Job tracking, status queries, UI display

### 2. JobType (`kalamdb-commons/src/models/mod.rs`)
```rust
pub enum JobType {
    Flush,    // Flush table to Parquet
    Compact,  // Compact Parquet files
    Cleanup,  // Remove old data
    Backup,   // Backup to external storage
    Restore,  // Restore from backup
}
```
**Usage**: Job creation, filtering, scheduling

### 3. TableType (Already Implemented)
```rust
pub enum TableType {
    User,    // User-isolated tables
    Shared,  // Globally shared tables
    Stream,  // Event stream tables
    System,  // Internal system tables
}
```
**Usage**: Table creation, routing, permissions

### 4. StorageType (Already Implemented)
```rust
pub enum StorageType {
    Filesystem { path: String },
    S3 { bucket: String, region: String },
}
```
**Usage**: Storage backend configuration

## Enums Deferred (Waiting for Feature Implementation)

### UserRole (T386-T387 - DEFERRED)
```rust
pub enum UserRole {
    User,     // Regular user with standard permissions
    Service,  // Service account for API access
    DBA,      // Database administrator
    System,   // Internal system user (not assignable)
}
```
**Status**: Deferred until US10 (User Management) implements role-based access control

### TableAccessLevel (T386 - DEFERRED)
```rust
pub enum TableAccessLevel {
    Public,     // Accessible by all users
    Private,    // Accessible by owner + DBAs only
    Restricted, // Requires explicit grants
}
```
**Status**: Deferred until shared table access control is implemented

## Usage Patterns

### Creating Enum Values

```rust
// Direct construction
let status = JobStatus::Running;

// From string (infallible with default)
let status = JobStatus::from("completed");

// From string (fallible with error handling)
let status = JobStatus::from_str("pending")
    .unwrap_or(JobStatus::Failed);

// In struct initialization
let job = Job {
    id: "job123",
    status: JobStatus::Running,
    job_type: JobType::Flush,
};
```

### Converting to String

```rust
// To &str
let status_str: &str = status.as_str();

// Via Display trait
println!("Status: {}", status);
format!("Job is {}", status);

// For database storage
db.store("status", status.as_str());
```

### Pattern Matching (Exhaustiveness Checking)

```rust
// Compiler ensures all variants handled
match job.status {
    JobStatus::Running => { /* ... */ },
    JobStatus::Completed => { /* ... */ },
    JobStatus::Failed => { /* ... */ },
    JobStatus::Cancelled => { /* ... */ },
    // Forget a variant? Compiler error!
}

// Partial matching with wildcard
match job.status {
    JobStatus::Completed => "done",
    _ => "in progress",
}
```

### Serialization (with serde feature)

```rust
#[derive(Serialize, Deserialize)]
struct JobResponse {
    id: String,
    status: JobStatus,  // Serializes as "running", "completed", etc.
    job_type: JobType,
}

// JSON output: {"id": "job123", "status": "completed", "job_type": "flush"}
```

## Benefits Realized

### Compile-Time Safety
```rust
// Before (strings)
fn update_job_status(status: &str) {
    if status == "runing" {  // ❌ Typo - runtime bug!
        // ...
    }
}

// After (enums)
fn update_job_status(status: JobStatus) {
    if status == JobStatus::Runnung {  // ❌ Compile error - caught immediately!
        // ...
    }
}
```

### Exhaustive State Handling
```rust
// Compiler error if any variant is missing
match job.status {
    JobStatus::Running => continue_execution(),
    JobStatus::Completed => return_success(),
    JobStatus::Failed => report_error(),
    // JobStatus::Cancelled => ???  <- Forget this? Compiler warns!
}
```

### Clear Constraints
```rust
// Before
fn create_job(job_type: String) -> Result<Job>  // What are valid types?

// After  
fn create_job(job_type: JobType) -> Result<Job>  // Only 5 valid types!
```

### Refactoring Safety
```rust
// Rename JobStatus::Running to JobStatus::InProgress
// Compiler finds ALL usages automatically!
```

## Consequences

### Positive
- **Type Safety**: Invalid values rejected at compile time
- **Exhaustiveness**: Compiler ensures all cases handled
- **Discoverability**: IDE shows all valid enum variants
- **Maintenance**: Adding/renaming variants updates all usages
- **Documentation**: Enum definition is self-documenting

### Negative
- **String Conversion**: Must explicitly convert to/from strings
- **Database Schema**: Requires string column for storage (or integer with mapping)
- **API Changes**: External APIs may expect strings (solved by serialization)

### Neutral
- **Memory**: Enums typically 1-4 bytes vs strings (often beneficial)
- **Learning Curve**: Developers must understand enum pattern

## Migration Strategy

### Phase 1: Create Enums ✅ COMPLETE
- Define enum types in `kalamdb-commons`
- Implement conversion traits (Display, From, as_str, from_str)
- Add serde support with feature flag

### Phase 2: Update Internal Usage 🔄 IN PROGRESS
- Replace `String` with enum in struct fields
- Update function signatures to accept enums
- Convert at system boundaries (DB, API)

### Phase 3: Database Integration ⏸️ DEFERRED
- Store enums as strings in RocksDB/Parquet
- Convert on read/write boundaries

### Phase 4: API Integration ⏸️ DEFERRED
- Serde serializes enums as strings automatically
- Add validation in API handlers

## Related Decisions

- **ADR-014**: Type-Safe Wrappers (when to use wrappers vs enums)
- **ADR-013**: Model Organization (where to define enums)

## Guidelines for New Enums

1. **Document Variants**: Add doc comments explaining each variant's meaning
2. **Implement Traits**: Always include Display, From<&str>, as_str, from_str
3. **Add Serde**: Use `#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]`
4. **Test Conversions**: Unit tests for string ↔ enum conversion
5. **Consider Default**: Implement Default if there's a sensible default value

## Example: Adding a New Enum

```rust
/// Connection state for WebSocket clients
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum ConnectionState {
    /// Initial connection attempt
    Connecting,
    /// Successfully connected
    Connected,
    /// Gracefully disconnected
    Disconnected,
    /// Connection error occurred
    Error,
}

impl ConnectionState {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Connecting => "connecting",
            Self::Connected => "connected",
            Self::Disconnected => "disconnected",
            Self::Error => "error",
        }
    }
    
    pub fn from_str(s: &str) -> Result<Self, String> {
        match s.to_lowercase().as_str() {
            "connecting" => Ok(Self::Connecting),
            "connected" => Ok(Self::Connected),
            "disconnected" => Ok(Self::Disconnected),
            "error" => Ok(Self::Error),
            _ => Err(format!("Invalid connection state: {}", s)),
        }
    }
}

// Implement Display, From, etc. following the pattern above
```

## References

- Rust Enum Documentation: https://doc.rust-lang.org/book/ch06-01-defining-an-enum.html
- Type-Driven Design: https://lexi-lambda.github.io/blog/2019/11/05/parse-don-t-validate/
- Implementation: `backend/crates/kalamdb-commons/src/models/mod.rs`

## Notes

Enums in Rust are zero-cost abstractions when used correctly. The compiler can optimize enum variants into simple integers or even eliminate the enum entirely in release builds.

Always pair enums with comprehensive documentation explaining what each variant represents and when it should be used.
