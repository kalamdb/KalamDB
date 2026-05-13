# kalam-link Public API Contract

**Version**: 1.0.0  
**Package**: kalam-link  
**Language**: Rust  
**Target**: wasm32-unknown-unknown (WebAssembly compatible) + native

## Purpose

This document defines the public API contract for the `kalam-link` library, which provides connectivity to KalamDB for query execution, WebSocket subscriptions, and authentication. The API must be stable, well-documented, and work in both native Rust and WebAssembly environments.

---

## Core Types

### KalamLinkClient

**Purpose**: Main client struct managing HTTP connections, authentication, and WebSocket subscriptions.

```rust
/// KalamDB connection client
///
/// Provides methods for executing SQL queries and establishing live subscriptions
/// to KalamDB tables. Thread-safe and can be cloned cheaply (uses Arc internally).
///
/// # Examples
///
/// ```rust
/// use kalam_link::{KalamLinkClient, Auth};
///
/// #[tokio::main]
/// async fn main() -> Result<(), Box<dyn std::error::Error>> {
///     let client = KalamLinkClient::builder()
///         .host("http://localhost:2900")?
///         .user("jamal")
///         .auth(Auth::jwt("eyJhbGc...")?)
///         .build()?;
///
///     let result = client.query("SELECT * FROM messages LIMIT 10").await?;
///     println!("Rows: {}", result.row_count());
///
///     Ok(())
/// }
/// ```
#[derive(Clone)]
pub struct KalamLinkClient {
    // Internal fields (not public)
}

impl KalamLinkClient {
    /// Create a new client builder
    pub fn builder() -> KalamLinkClientBuilder {
        KalamLinkClientBuilder::default()
    }
    
    /// Execute a SQL query
    ///
    /// # Arguments
    /// * `sql` - SQL query string (SELECT, INSERT, UPDATE, DELETE, CREATE, etc.)
    ///
    /// # Returns
    /// `QueryResponse` containing result rows and metadata
    ///
    /// # Errors
    /// * Connection error if server is unreachable
    /// * Authentication error if credentials are invalid
    /// * SQL error if query syntax is invalid
    ///
    /// # Examples
    ///
    /// ```rust
    /// let result = client.query("SELECT * FROM messages WHERE user_id = 'jamal'").await?;
    /// for batch in result.batches() {
    ///     println!("Batch: {:?}", batch);
    /// }
    /// ```
    pub async fn query(&self, sql: impl Into<String>) -> Result<QueryResponse>;
    
    /// Execute a parametrized SQL query
    ///
    /// # Arguments
    /// * `sql` - SQL query with parameter placeholders ($1, $2, ...)
    /// * `params` - Array of parameter values
    ///
    /// # Examples
    ///
    /// ```rust
    /// let result = client.query_with_params(
    ///     "SELECT * FROM messages WHERE user_id = $1 AND created_at > $2",
    ///     &[
    ///         QueryParam::String("jamal".to_string()),
    ///         QueryParam::Timestamp(Utc::now() - Duration::days(7)),
    ///     ],
    /// ).await?;
    /// ```
    pub async fn query_with_params(
        &self,
        sql: impl Into<String>,
        params: &[QueryParam],
    ) -> Result<QueryResponse>;
    
    /// Execute a batch of SQL statements
    ///
    /// Statements are executed sequentially. If any statement fails, execution
    /// stops and an error is returned with the index of the failed statement.
    ///
    /// # Arguments
    /// * `statements` - Array of SQL statements or semicolon-separated SQL string
    ///
    /// # Examples
    ///
    /// ```rust
    /// let results = client.batch(&[
    ///     "CREATE TABLE test (id INT, name TEXT)",
    ///     "INSERT INTO test VALUES (1, 'Alice')",
    ///     "INSERT INTO test VALUES (2, 'Bob')",
    /// ]).await?;
    ///
    /// for (i, result) in results.iter().enumerate() {
    ///     println!("Statement {}: {} rows affected", i, result.rows_affected());
    /// }
    /// ```
    pub async fn batch(&self, statements: &[impl AsRef<str>]) -> Result<Vec<QueryResponse>>;
    
    /// Establish a WebSocket subscription to a table
    ///
    /// Creates a long-lived WebSocket connection that delivers real-time change
    /// notifications (INSERT, UPDATE, DELETE) as they occur.
    ///
    /// # Arguments
    /// * `query` - SQL SELECT query defining the subscription filter
    ///
    /// # Returns
    /// `Subscription` handle with an event stream
    ///
    /// # Examples
    ///
    /// ```rust
    /// let mut subscription = client.subscribe(
    ///     "SELECT * FROM messages WHERE user_id = 'jamal'"
    /// ).await?;
    ///
    /// let mut events = subscription.events();
    /// while let Some(event) = events.next().await {
    ///     match event? {
    ///         ChangeEvent::Insert { data } => println!("New: {:?}", data),
    ///         ChangeEvent::Update { old, new } => println!("Updated: {:?} -> {:?}", old, new),
    ///         ChangeEvent::Delete { data } => println!("Deleted: {:?}", data),
    ///         ChangeEvent::InitialData { data } => println!("Initial: {:?}", data),
    ///     }
    /// }
    /// ```
    pub async fn subscribe(&self, query: impl Into<String>) -> Result<Subscription>;
    
    /// Establish a WebSocket subscription with options
    ///
    /// # Arguments
    /// * `query` - SQL SELECT query
    /// * `options` - Subscription configuration (last_rows, buffer_size)
    ///
    /// # Examples
    ///
    /// ```rust
    /// let subscription = client.subscribe_with_options(
    ///     "SELECT * FROM messages",
    ///     SubscriptionOptions::builder()
    ///         .last_rows(100)  // Fetch last 100 rows immediately
    ///         .buffer_size(1000)
    ///         .build(),
    /// ).await?;
    /// ```
    pub async fn subscribe_with_options(
        &self,
        query: impl Into<String>,
        options: SubscriptionOptions,
    ) -> Result<Subscription>;
    
    /// Check server health
    ///
    /// # Returns
    /// Server health status including version, uptime, and metrics
    pub async fn health(&self) -> Result<HealthStatus>;
}
```

---

### KalamLinkClientBuilder

```rust
/// Builder for KalamLinkClient
///
/// # Examples
///
/// ```rust
/// let client = KalamLinkClient::builder()
///     .host("http://localhost:2900")?
///     .user("jamal")
///     .auth(Auth::jwt("token")?)
///     .timeout(Duration::from_secs(30))
///     .build()?;
/// ```
pub struct KalamLinkClientBuilder {
    // Internal fields
}

impl KalamLinkClientBuilder {
    /// Set server host URL (required)
    ///
    /// # Examples
    /// ```rust
    /// .host("http://localhost:2900")?
    /// .host("https://kalamdb.example.com")?
    /// ```
    pub fn host(self, url: impl TryInto<Url>) -> Result<Self>;
    
    /// Set user ID (required)
    pub fn user(self, user_id: impl Into<String>) -> Self;
    
    /// Set authentication (optional for localhost bypass)
    pub fn auth(self, auth: Auth) -> Self;
    
    /// Set request timeout (default: 30 seconds)
    pub fn timeout(self, timeout: Duration) -> Self;
    
    /// Set maximum retry attempts (default: 3)
    pub fn max_retries(self, retries: usize) -> Self;
    
    /// Build the client
    ///
    /// # Errors
    /// * Missing required fields (host, user)
    /// * Invalid URL format
    pub fn build(self) -> Result<KalamLinkClient>;
}

impl Default for KalamLinkClientBuilder {
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(30),
            max_retries: 3,
            // ...
        }
    }
}
```

---

### Auth

```rust
/// Authentication credentials for KalamDB
#[derive(Clone)]
pub enum Auth {
    /// JWT token authentication
    Jwt(String),
    
    /// API key authentication
    ApiKey(String),
}

impl Auth {
    /// Create JWT authentication
    ///
    /// # Errors
    /// * Token format validation error
    pub fn jwt(token: impl Into<String>) -> Result<Self> {
        let token = token.into();
        // Validate JWT format (header.payload.signature)
        if !Self::is_valid_jwt(&token) {
            return Err(Error::InvalidJwt);
        }
        Ok(Auth::Jwt(token))
    }
    
    /// Create API key authentication
    pub fn api_key(key: impl Into<String>) -> Self {
        Auth::ApiKey(key.into())
    }
}
```

---

### QueryResponse

```rust
/// Response from a SQL query
pub struct QueryResponse {
    // Internal fields
}

impl QueryResponse {
    /// Get result schema (column names and types)
    pub fn schema(&self) -> &Schema;
    
    /// Get result batches (Arrow RecordBatch)
    pub fn batches(&self) -> &[RecordBatch];
    
    /// Get total row count across all batches
    pub fn row_count(&self) -> usize;
    
    /// Get number of rows affected (for INSERT/UPDATE/DELETE)
    pub fn rows_affected(&self) -> usize;
    
    /// Get query execution time (if available)
    pub fn execution_time(&self) -> Option<Duration>;
    
    /// Convert to JSON string
    ///
    /// # Examples
    /// ```rust
    /// let json = response.to_json()?;
    /// println!("{}", json);
    /// ```
    pub fn to_json(&self) -> Result<String>;
    
    /// Convert to CSV string
    pub fn to_csv(&self) -> Result<String>;
}
```

---

### QueryParam

```rust
/// Query parameter value
#[derive(Debug, Clone, PartialEq)]
pub enum QueryParam {
    String(String),
    Integer(i64),
    Float(f64),
    Boolean(bool),
    Timestamp(DateTime<Utc>),
    Null,
}

impl From<&str> for QueryParam {
    fn from(s: &str) -> Self {
        QueryParam::String(s.to_string())
    }
}

impl From<i64> for QueryParam {
    fn from(i: i64) -> Self {
        QueryParam::Integer(i)
    }
}

// ... similar From implementations for other types
```

---

### Subscription

```rust
/// Active WebSocket subscription handle
///
/// Automatically closes WebSocket connection when dropped.
pub struct Subscription {
    // Internal fields
}

impl Subscription {
    /// Get subscription ID
    pub fn id(&self) -> SubscriptionId;
    
    /// Get event stream
    ///
    /// Returns a stream of change events. The stream ends when the subscription
    /// is closed or an error occurs.
    ///
    /// # Examples
    ///
    /// ```rust
    /// let mut events = subscription.events();
    /// while let Some(event) = events.next().await {
    ///     match event? {
    ///         ChangeEvent::Insert { data } => { /* ... */ },
    ///         // ...
    ///     }
    /// }
    /// ```
    pub fn events(&mut self) -> impl Stream<Item = Result<ChangeEvent>> + '_;
    
    /// Close the subscription gracefully
    ///
    /// Sends a close frame to the server and waits for acknowledgment.
    /// The subscription is also closed automatically when dropped.
    pub async fn close(&mut self) -> Result<()>;
    
    /// Check if subscription is still active
    pub fn is_active(&self) -> bool;
}

pub type SubscriptionId = Uuid;
```

---

### SubscriptionOptions

```rust
/// Configuration for WebSocket subscriptions
#[derive(Debug, Clone, Default)]
pub struct SubscriptionOptions {
    /// Number of recent rows to fetch immediately (0 = none)
    pub last_rows: Option<usize>,
    
    /// WebSocket buffer size (default: 1000)
    pub buffer_size: usize,
}

impl SubscriptionOptions {
    pub fn builder() -> SubscriptionOptionsBuilder {
        SubscriptionOptionsBuilder::default()
    }
}

pub struct SubscriptionOptionsBuilder {
    last_rows: Option<usize>,
    buffer_size: usize,
}

impl SubscriptionOptionsBuilder {
    /// Fetch last N rows immediately before streaming changes
    pub fn last_rows(mut self, n: usize) -> Self {
        self.last_rows = Some(n);
        self
    }
    
    /// Set WebSocket buffer size (number of events buffered)
    pub fn buffer_size(mut self, size: usize) -> Self {
        self.buffer_size = size;
        self
    }
    
    pub fn build(self) -> SubscriptionOptions {
        SubscriptionOptions {
            last_rows: self.last_rows,
            buffer_size: self.buffer_size,
        }
    }
}

impl Default for SubscriptionOptionsBuilder {
    fn default() -> Self {
        Self {
            last_rows: None,
            buffer_size: 1000,
        }
    }
}
```

---

### ChangeEvent

```rust
/// WebSocket change notification
#[derive(Debug, Clone)]
pub enum ChangeEvent {
    /// New row inserted
    Insert {
        data: RecordBatch,
    },
    
    /// Existing row updated
    Update {
        old: RecordBatch,
        new: RecordBatch,
    },
    
    /// Row deleted (soft delete with _deleted flag)
    Delete {
        data: RecordBatch,  // Includes _deleted=true
    },
    
    /// Initial data from last_rows fetch
    InitialData {
        data: RecordBatch,
    },
}

impl ChangeEvent {
    /// Get the change type as string
    pub fn change_type(&self) -> &'static str {
        match self {
            ChangeEvent::Insert { .. } => "INSERT",
            ChangeEvent::Update { .. } => "UPDATE",
            ChangeEvent::Delete { .. } => "DELETE",
            ChangeEvent::InitialData { .. } => "INITIAL",
        }
    }
    
    /// Extract RecordBatch(es) from event
    pub fn data(&self) -> Vec<&RecordBatch> {
        match self {
            ChangeEvent::Insert { data } => vec![data],
            ChangeEvent::Update { old, new } => vec![old, new],
            ChangeEvent::Delete { data } => vec![data],
            ChangeEvent::InitialData { data } => vec![data],
        }
    }
}
```

---

### HealthStatus

```rust
/// Server health status
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthStatus {
    pub status: String,          // "ok" or "degraded"
    pub version: String,         // e.g., "0.1.0-alpha"
    pub uptime_seconds: u64,
    pub metrics: Option<ServerMetrics>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerMetrics {
    pub active_connections: usize,
    pub active_subscriptions: usize,
    pub queries_per_second: f64,
}
```

---

### Error

```rust
/// kalam-link error types
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Connection error: {0}")]
    Connection(String),
    
    #[error("Authentication failed: {0}")]
    Authentication(String),
    
    #[error("SQL error: {0}")]
    Sql(String),
    
    #[error("Invalid JWT token format")]
    InvalidJwt,
    
    #[error("Invalid URL: {0}")]
    InvalidUrl(String),
    
    #[error("Subscription error: {0}")]
    Subscription(String),
    
    #[error("Timeout after {0:?}")]
    Timeout(Duration),
    
    #[error("HTTP error {status}: {message}")]
    Http { status: u16, message: String },
    
    #[error("WebSocket error: {0}")]
    WebSocket(String),
    
    #[error("Serialization error: {0}")]
    Serialization(String),
}

pub type Result<T> = std::result::Result<T, Error>;
```

---

## WebAssembly Compatibility

The following considerations ensure WebAssembly compatibility:

### Dependencies
- **tokio**: Use `wasm` feature flag
- **reqwest**: Use `wasm` feature flag
- **tungstenite**: Use wasm-compatible WebSocket implementation (web-sys)

### Not Allowed in kalam-link
- File system access (`std::fs`)
- Threading (`std::thread`)
- Process spawning
- RocksDB or other native libraries

### Example Cargo.toml

```toml
[package]
name = "kalam-link"
version = "1.0.0"
edition = "2021"

[dependencies]
tokio = { version = "1.35", features = ["macros", "rt"], optional = true }
tokio-tungstenite = { version = "0.21", optional = true }
reqwest = { version = "0.11", features = ["json"] }
serde = { version = "1.0", features = ["derive"] }
serde_json = "1.0"
arrow = "50.0"
uuid = { version = "1.6", features = ["v4", "serde"] }
chrono = { version = "0.4", features = ["serde"] }
thiserror = "1.0"
url = "2.5"

# WebAssembly support
[target.'cfg(target_arch = "wasm32")'.dependencies]
tokio = { version = "1.35", features = ["macros", "sync"] }
reqwest = { version = "0.11", features = ["json"], default-features = false }
ws_stream_wasm = "0.7"  # WebSocket for wasm

# Native support
[target.'cfg(not(target_arch = "wasm32"))'.dependencies]
tokio = { version = "1.35", features = ["macros", "rt-multi-thread"] }
tokio-tungstenite = "0.21"

[features]
default = []
```

---

## Usage Examples

### Basic Query

```rust
use kalam_link::{KalamLinkClient, Auth};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let client = KalamLinkClient::builder()
        .host("http://localhost:2900")?
        .user("jamal")
        .auth(Auth::jwt("eyJhbGc...")?)
        .build()?;
    
    let result = client.query("SELECT * FROM messages LIMIT 10").await?;
    
    println!("Columns: {:?}", result.schema());
    println!("Rows: {}", result.row_count());
    
    for batch in result.batches() {
        println!("Batch: {:?}", batch);
    }
    
    Ok(())
}
```

### Parametrized Query

```rust
use kalam_link::QueryParam;

let result = client.query_with_params(
    "INSERT INTO messages (user_id, content, created_at) VALUES ($1, $2, $3)",
    &[
        QueryParam::from("jamal"),
        QueryParam::from("Hello, world!"),
        QueryParam::Timestamp(Utc::now()),
    ],
).await?;

println!("Rows inserted: {}", result.rows_affected());
```

### WebSocket Subscription

```rust
let mut subscription = client.subscribe_with_options(
    "SELECT * FROM messages WHERE user_id = 'jamal'",
    SubscriptionOptions::builder()
        .last_rows(100)
        .build(),
).await?;

let mut events = subscription.events();

while let Some(event) = events.next().await {
    match event? {
        ChangeEvent::InitialData { data } => {
            println!("Initial batch: {} rows", data.num_rows());
        }
        ChangeEvent::Insert { data } => {
            println!("New message: {:?}", data);
        }
        ChangeEvent::Update { old, new } => {
            println!("Updated: {:?} -> {:?}", old, new);
        }
        ChangeEvent::Delete { data } => {
            println!("Deleted: {:?}", data);
        }
    }
}
```

### Batch Execution

```rust
let results = client.batch(&[
    "CREATE NAMESPACE demo",
    "CREATE USER TABLE demo.messages (id INT, content TEXT)",
    "INSERT INTO demo.messages VALUES (1, 'First')",
    "INSERT INTO demo.messages VALUES (2, 'Second')",
]).await?;

for (i, result) in results.iter().enumerate() {
    println!("Statement {}: {} rows affected", i, result.rows_affected());
}
```

---

## Testing Strategy

### Unit Tests
- Builder validation
- Parameter type conversion
- Error handling
- URL parsing

### Integration Tests (against real server)
- Query execution (SELECT, INSERT, UPDATE, DELETE)
- Parametrized queries with various parameter types
- WebSocket subscription lifecycle
- Authentication (JWT, API key, localhost bypass)
- Batch execution
- Health check endpoint

### Mock Tests
- HTTP request/response mocking
- WebSocket message mocking
- Network error simulation
- Timeout simulation

---

## Stability Guarantees

**API Stability**: This API is considered stable for kalam-link 1.x versions. Breaking changes will only occur in major version bumps (2.0.0+).

**Supported Rust Versions**: Minimum Rust 1.75+ (stable toolchain)

**WebAssembly Support**: Guaranteed to compile to wasm32-unknown-unknown target

**Semantic Versioning**: Follows SemVer strictly
- Patch (1.0.x): Bug fixes, no API changes
- Minor (1.x.0): New features, backward compatible
- Major (x.0.0): Breaking changes

---

## Roadmap

**Future Enhancements** (not part of 1.0.0):
- Connection pooling
- Query result caching
- Offline mode with sync
- Compression support (gzip, zstd)
- Streaming large result sets
- Transaction support
- Prepared statement handle reuse
