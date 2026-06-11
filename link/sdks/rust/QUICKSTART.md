# KalamDB Rust SDK Quick Start

Get a working query + realtime flow running with the current `kalam-client` API.

## Prerequisites

- Rust 1.92+ (`rustup update stable`)
- A running KalamDB server
- Valid credentials for that server

If you are running KalamDB locally from this repo, a common local default is:

- URL: `http://localhost:2900`
- user: `admin`
- password: `kalamdb123`

For server setup and auth flows, see:

- [README.md](https://github.com/kalamdb/KalamDB/blob/main/link/sdks/rust/README.md)
- https://kalamdb.org/docs/getting-started/authentication

## Installation

```toml
[dependencies]
kalam-client = "0.5"
tokio = { version = "1", features = ["macros", "rt-multi-thread"] }
```

Inside this repository, depend on the workspace crate directly:

```toml
kalam-client = { path = "../../kalam-client", features = ["native-sdk"] }
```

## 1. Create a Client

```rust,no_run
use std::time::Duration;

use kalam_client::{AuthProvider, KalamLinkClient};

let client = KalamLinkClient::builder()
    .base_url("http://localhost:2900")
    .auth(AuthProvider::basic_auth("admin".into(), "kalamdb123".into()))
    .timeout(Duration::from_secs(30))
    .build()?;
```

## 2. Execute a Query

HTTP queries work without manually calling `connect()`.

```rust,no_run
let response = client
    .execute_query("SELECT CURRENT_USER()", None, None, None)
    .await?;

println!("status = {:?}", response.status);
```

Most apps do not need to call `connect()` for one-off SQL. Call `connect()` before `live()` or `live_events()` so subscriptions share one WebSocket.

## 3. Start Realtime Updates

For UI or service state, prefer `live()` so the SDK gives you the current materialized row set directly.

```rust,no_run
use kalam_client::{
    LiveRowsConfig, LiveRowsEvent, SubscriptionConfig, SubscriptionOptions,
};

client.connect().await?;

let mut config = SubscriptionConfig::new(
    "messages",
    "SELECT * FROM app.messages WHERE room = 'main'",
);
config.options = Some(SubscriptionOptions::new().with_last_rows(20));

let mut live = client
    .live_with_config(config, LiveRowsConfig::default())
    .await?;

while let Some(event) = live.next().await {
    if let LiveRowsEvent::Rows { rows, .. } = event? {
        println!("rows {}", rows.len());
    }
}
```

Live SQL should stay in the strict supported form:

- `SELECT ... FROM ... WHERE ...`
- do not put `ORDER BY` or `LIMIT` inside `live()` / `live_events()` SQL
- do ordering or capping in application code after rows arrive, or via `LiveRowsConfig::limit`

## 4. Low-level Subscription API

Use `live_events()` only when you need raw subscription protocol events.

```rust,no_run
use kalam_client::SubscriptionConfig;

let config = SubscriptionConfig::new(
    "raw-messages",
    "SELECT * FROM app.messages WHERE room = 'main'",
);

let mut events = client.live_events_with_config(config).await?;

while let Some(change) = events.next().await {
    println!("{:?}", change?);
}

events.close().await?;
```

## 5. Cleanup

```rust,no_run
live.close().await?;
client.disconnect().await?;
```

## Complete Example

Run the included quickstart example from this directory:

```bash
cd link/sdks/rust
export KALAMDB_SERVER_URL=http://localhost:2900
cargo run -p quickstart
```

Or copy the full flow from [examples/quickstart/src/main.rs](https://github.com/kalamdb/KalamDB/blob/main/link/sdks/rust/examples/quickstart/src/main.rs).

## Optional: Topic Consumers

Enable the `consumer` feature when you need topic workers:

```toml
kalam-client = { version = "0.5", features = ["native-sdk", "consumer"] }
```

Then see [examples/topic-consumer](https://github.com/kalamdb/KalamDB/tree/main/link/sdks/rust/examples/topic-consumer/src).

## Next Steps

- Full SDK docs: [README.md](https://github.com/kalamdb/KalamDB/blob/main/link/sdks/rust/README.md)
- SQL reference: https://kalamdb.org/docs/reference/sql
- Workspace examples: https://github.com/kalamdb/KalamDB/tree/main/link/sdks/rust/examples
