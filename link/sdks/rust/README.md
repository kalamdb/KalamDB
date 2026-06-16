# kalam-client

Official Rust SDK for [KalamDB](https://kalamdb.org) — SQL, materialized live rows, and strong tenant isolation in one crate.

KalamDB is built for apps where every user or tenant owns a private data space. The same SQL can run for every signed-in customer, while USER tables ensure each query only touches that caller's data. On the server and in native SDKs, the default realtime API is `live()`: you receive the current materialized row set, not a stream of low-level diff frames that your UI has to reconcile.

→ **[kalamdb.org](https://kalamdb.org)** · [Docs](https://kalamdb.org/docs/sdk/rust) · [GitHub](https://github.com/kalamdb/KalamDB)

`kalam-client` provides:

- SQL execution over HTTP
- materialized live query rows over WebSocket with `live()` and `live_with_config()`
- low-level realtime events with `live_events()` when you need raw frames
- per-user and per-tenant isolation with USER tables
- optional topic consumer workers behind the `consumer` feature
- optional multipart file upload helpers behind the `file-uploads` feature

Runtime targets:

- Tokio-based async Rust (`native-sdk`, enabled by default)

Browser WebAssembly builds for `@kalamdb/client` live in the separate
`kalam-link-wasm` crate (not part of this published SDK).

## Installation

Add the crate from crates.io (or a path dependency while developing inside this repository):

```toml
[dependencies]
kalam-client = "0.5"
tokio = { version = "1", features = ["macros", "rt-multi-thread"] }
```

Enable optional capabilities with Cargo features:

```toml
# App-facing SQL + live rows (default)
kalam-client = { version = "0.5", features = ["native-sdk"] }

# Topic workers and batch consumption
kalam-client = { version = "0.5", features = ["native-sdk", "consumer"] }

# Multipart SQL file uploads
kalam-client = { version = "0.5", features = ["native-sdk", "file-uploads"] }
```

| Feature | Description |
|---------|-------------|
| `native-sdk` (default) | Tokio runtime, HTTP queries, auth flows, live subscriptions |
| `consumer` | `TopicConsumer`, `ConsumerBuilder`, consume/ack topic APIs |
| `file-uploads` | Multipart SQL upload helpers |
| `healthcheck` | Cached `/v1/api/healthcheck` helper |
| `setup` | First-run server setup helpers |
| `cluster` | Cluster health inspection |

Topic workers ship in the same crate behind `consumer` so app-only installs stay lean, matching how `@kalamdb/consumer` extends `@kalamdb/client` in TypeScript.

## Why `live()` First

Most UIs do not want `subscription_ack`, `initial_data_batch`, `change`, and `error` frames. They want the latest rows.

`live()` gives you exactly that:

- the current row set already reconciled for insert, update, and delete
- one event shape for initial load and future changes
- shared behavior with the TypeScript and Dart clients
- simpler services, CLIs, and background workers

Use `SubscriptionOptions::with_last_rows()` when you want an initial rewind from the server. Use `LiveRowsConfig { limit: Some(n), .. }` when you want the client to keep the materialized live row set bounded over time.

The knobs apply at different layers:

- `batch_size` chunks the initial snapshot from the server
- `last_rows` chooses how much history to rewind first
- `limit` caps the materialized live row set the client keeps afterward

Use `live_events()` only when you need the raw event protocol.

## Quick Start

Start with a USER table. The SQL stays simple, and KalamDB scopes the data per authenticated user.

```sql
CREATE NAMESPACE IF NOT EXISTS support;

CREATE TABLE support.inbox (
  id         BIGINT PRIMARY KEY DEFAULT SNOWFLAKE_ID(),
  room       TEXT NOT NULL DEFAULT 'main',
  role       TEXT NOT NULL,
  body       TEXT NOT NULL,
  created_at TIMESTAMP NOT NULL DEFAULT NOW()
) WITH (TYPE = 'USER');
```

```rust,no_run
use std::time::Duration;

use kalam_client::{
    AuthProvider, KalamLinkClient, LiveRowsConfig, LiveRowsEvent, SubscriptionConfig,
    SubscriptionOptions,
};
use serde_json::json;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let client = KalamLinkClient::builder()
        .base_url("http://localhost:2900")
        .auth(AuthProvider::basic_auth("alice".into(), "Secret123!".into()))
        .timeout(Duration::from_secs(30))
        .build()?;

    client.connect().await?;

    let inbox_sql = "
        SELECT id, room, role, body, created_at
        FROM support.inbox
        WHERE room = 'main'
    ";

    let mut config = SubscriptionConfig::new("inbox", inbox_sql);
    config.options = Some(
        SubscriptionOptions::new()
            .with_last_rows(200)
            .with_batch_size(200),
    );

    let mut live = client
        .live_with_config(
            config,
            LiveRowsConfig {
                limit: Some(200),
                ..LiveRowsConfig::default()
            },
        )
        .await?;

    // `support.inbox` is a USER table. Every signed-in user can run the same SQL
    // text, but KalamDB only returns that caller's rows.
    while let Some(event) = live.next().await {
        match event? {
            LiveRowsEvent::Rows { rows, .. } => {
                for row in rows {
                    println!(
                        "{} {}: {}",
                        row.get("role").and_then(|v| v.as_text()).unwrap_or(""),
                        row.get("id").and_then(|v| v.as_text()).unwrap_or(""),
                        row.get("body").and_then(|v| v.as_text()).unwrap_or(""),
                    );
                }
            }
            LiveRowsEvent::Error { code, message, .. } => {
                eprintln!("live error {code}: {message}");
                break;
            }
        }
    }

    client
        .execute_query(
            "INSERT INTO support.inbox (room, role, body) VALUES ($1, $2, $3)",
            None,
            Some(vec![json!("main"), json!("user"), json!("Need help with billing")]),
            None,
        )
        .await?;

    live.close().await?;
    client.disconnect().await;
    Ok(())
}
```

See [QUICKSTART.md](https://github.com/kalamdb/KalamDB/blob/main/link/sdks/rust/QUICKSTART.md) for a shorter copy-paste flow and [examples](https://github.com/kalamdb/KalamDB/tree/main/link/sdks/rust/examples) for runnable projects.

## Resume From a Specific `SeqId`

When you want offline resume or a durable checkpoint, persist the last `SeqId` you applied and feed it back through `SubscriptionOptions::with_from()`.

```rust,no_run
use kalam_client::{SeqId, SubscriptionOptions};

let start_from = SeqId::from(42_i64);
let options = SubscriptionOptions::new()
    .with_last_rows(200)
    .with_from(start_from);
```

Each `LiveRowsEvent::Rows` includes `last_seq_id` so you can persist checkpoints between sessions.

## Lower-Level Realtime API

If you need raw protocol frames, use `live_events()`.

```rust,no_run
use kalam_client::{ChangeEvent, SubscriptionConfig};

let config =
    SubscriptionConfig::new("raw-inbox", "SELECT * FROM support.inbox WHERE room = 'main'");
let mut events = client.live_events_with_config(config).await?;

while let Some(change) = events.next().await {
    match change? {
        ChangeEvent::Insert { rows, .. } => println!("inserted {}", rows.len()),
        ChangeEvent::Update { rows, .. } => println!("updated {}", rows.len()),
        ChangeEvent::Delete { old_rows, .. } => println!("deleted {}", old_rows.len()),
        _ => {}
    }
}
```

Use this API for protocol tooling, debugging, or custom reconciliation. For app UI state, prefer `live()`.

## Topics and Workers

Topic workers live behind the optional `consumer` feature so app-only installs keep the main SDK lean.

```toml
[dependencies]
kalam-client = { version = "0.5", features = ["native-sdk", "consumer"] }
```

Use the default client surface for app-facing SQL, live rows, subscriptions, auth, and files. Enable `consumer` for `TopicConsumer`, `ConsumerBuilder`, `consume_batch()`, and `ack()`.

```rust,no_run
use kalam_client::AutoOffsetReset;

let consumer = client
    .consumer()
    .group_id("billing-workers")
    .topic("support.events")
    .auto_offset_reset(AutoOffsetReset::Earliest)
    .build()?;
```

See [examples/topic-consumer](https://github.com/kalamdb/KalamDB/tree/main/link/sdks/rust/examples/topic-consumer/src) for a fuller worker loop.

## Files

KalamDB tables can include `FILE` columns. Upload with multipart SQL, read metadata from query results, and download bytes through the same client.

Enable uploads with the `file-uploads` feature (included in `native-cli` / `e2e-tests`):

```toml
kalam-client = { version = "0.5", features = ["native-sdk", "file-uploads"] }
```

```rust,no_run
use kalam_client::{FileUpload, QueryParam};

let files = vec![FileUpload::new("upload", "photo.jpg", std::fs::read("photo.jpg")?).with_mime("image/jpeg")];

client.execute_with_files(
    "INSERT INTO docs.files (id, attachment) VALUES ($1, FILE(\"upload\"))",
    files,
    Some(vec![QueryParam::from("doc1")]),
    None,
).await?;

let rows = client
    .execute_query(
        "SELECT attachment FROM docs.files WHERE id = $1",
        None,
        Some(vec![QueryParam::from("doc1")]),
        None,
    )
    .await?;

let table_id = TableId::from_strings("docs", "files");
let file_ref = rows.results[0].rows[0][0]
    .as_bound_file(&table_id)
    .expect("FILE column");
let download = client.download_bound_file(&file_ref, None).await?;
std::fs::write("downloaded.jpg", &download.bytes)?;
```

Key types:

- `FileRef` — table-agnostic file metadata plus `stored_name()`
- `TableId` — type-safe namespace/table identity from `kalamdb-commons`
- `BoundFileRef` — `FileRef` plus table context for downloads
- `FileUpload` — multipart upload payload for `FILE("placeholder")` SQL
- `QueryParam` — type-safe SQL parameters (`Text`, `Int`, `File`, …)
- `KalamCellValue::as_file()` — parse FILE cells from query/live rows
- `KalamCellValue::as_bound_file(&TableId)` — parse and attach table context
- `FileDownload` — bytes plus response headers from `download_bound_file()` / `download_file()`

## Authentication

`AuthProvider` is the canonical way to configure the client.

```rust,no_run
use kalam_client::AuthProvider;

// Static JWT
let auth = AuthProvider::jwt_token(token);

// Basic auth (exchanged for JWT on first use)
let auth = AuthProvider::basic_auth("alice".into(), "secret".into());

// Local root / system user
let auth = AuthProvider::system_user_auth(std::env::var("KALAMDB_ROOT_PASSWORD")?);
```

The SDK handles:

- Basic-auth-to-JWT exchange
- default namespace forwarding for `/v1/api/sql` plus unqualified live/file contexts
- shared WebSocket connection management
- reconnect controls and `SeqId` tracking

## Examples and Tests

| Example | What it shows |
|---------|----------------|
| [quickstart](https://github.com/kalamdb/KalamDB/tree/main/link/sdks/rust/examples/quickstart/src) | Connect, run `SELECT CURRENT_USER()`, disconnect |
| [file-attachments](https://github.com/kalamdb/KalamDB/tree/main/link/sdks/rust/examples/file-attachments/src) | FILE upload, `as_bound_file`, `download_bound_file` |
| [live-inbox](https://github.com/kalamdb/KalamDB/tree/main/link/sdks/rust/examples/live-inbox/src) | USER table + materialized `live()` loop |
| [topic-consumer](https://github.com/kalamdb/KalamDB/tree/main/link/sdks/rust/examples/topic-consumer/src) | `consumer` feature + `TopicConsumer` |
| `tests/*.rs` | Offline API guards plus server-backed integration tests |

Run the SDK test suite from this directory:

```bash
NO_SERVER=true ./test.sh   # offline API tests only
./test.sh                    # full suite (requires a running server)
```

From the repo root:

```bash
cd link/sdks/rust
cargo run -p quickstart
cargo run -p file-attachments
cargo run -p live-inbox
cargo run -p topic-consumer
```

Set `KALAMDB_SERVER_URL` (default `http://localhost:2900`) and credentials as needed.

## API Pointers

- `execute_query()` and related SQL helpers for reads and writes
- `QueryParam`, `FileUpload`, `FileRef`, and `FileDownload` for typed SQL and FILE columns
- `execute_with_files()` for FILE column uploads (`file-uploads` feature)
- `download_bound_file()` for context-bound FILE downloads; `download_file()` remains available for explicit namespace/table calls
- `live()` and `live_with_config()` for materialized realtime rows
- `live_events()` and `live_events_with_config()` for low-level subscription frames
- `TopicConsumer` and `client.consumer()` (feature `consumer`) for topic consumption and commits

Full docs: [kalamdb.org/docs/sdk/rust](https://kalamdb.org/docs/sdk/rust)  
Issues: [github.com/kalamdb/KalamDB/issues](https://github.com/kalamdb/KalamDB/issues)

---

For crate development, publishing, and contribution details, see [DEV.md](https://github.com/kalamdb/KalamDB/blob/main/link/sdks/rust/DEV.md).

## License

Licensed under the Apache License, Version 2.0 (`Apache-2.0`). See [LICENSE](https://github.com/kalamdb/KalamDB/blob/main/link/sdks/rust/LICENSE).
