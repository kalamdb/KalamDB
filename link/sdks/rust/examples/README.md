# Rust SDK Examples

Runnable examples for `kalam-client`. Each crate depends on the workspace copy at `link/kalam-client/`.

## Prerequisites

- Rust 1.92+
- A running KalamDB server

```bash
cd backend && cargo run --bin kalamdb-server
```

Common environment variables:

- `KALAMDB_SERVER_URL` (default `http://localhost:2900`)
- `KALAMDB_ROOT_PASSWORD` (default `kalamdb123`)

## Examples

| Example | Source | Command | Features |
|---------|--------|---------|----------|
| `quickstart` | [src](https://github.com/kalamdb/KalamDB/tree/main/link/sdks/rust/examples/quickstart/src) | `cargo run -p quickstart` | HTTP query only |
| `live-inbox` | [src](https://github.com/kalamdb/KalamDB/tree/main/link/sdks/rust/examples/live-inbox/src) | `cargo run -p live-inbox` | `live()` materialized rows |
| `topic-consumer` | [src](https://github.com/kalamdb/KalamDB/tree/main/link/sdks/rust/examples/topic-consumer/src) | `cargo run -p topic-consumer` | `consumer` topic polling |

From `link/sdks/rust/`:

```bash
cargo run -p quickstart
cargo run -p live-inbox
cargo run -p topic-consumer
```
