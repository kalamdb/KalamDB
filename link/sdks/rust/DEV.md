# kalam-client — Developer Notes

This document is for contributors and developers working on the Rust SDK packaging in `link/sdks/rust/` and the underlying crate in `link/kalam-client/`.

For usage documentation, see [README.md](https://github.com/kalamdb/KalamDB/blob/main/link/sdks/rust/README.md).

## Architecture

The Rust SDK is a native Tokio client built on the shared link implementation:

```
Application (Rust service, CLI, worker)
  └─ kalam-client (crates.io)              ← app-facing API, feature flags
      └─ link/link-common/src/             ← shared transport, auth, live rows, consumer
          └─ backend/crates/kalamdb-commons ← protocol types and shared IDs
```

The same `link-common` core also powers:

- `@kalamdb/client` (WASM via `kalam-client` with feature `wasm`)
- `@kalamdb/consumer` (separate WASM entry crate)
- `kalam_link` (Dart via `kalam-link-dart`)
- `kalamdb` (Python via PyO3 in `link/sdks/python`)

TypeScript splits consumer into `@kalamdb/consumer`. Rust keeps topic workers in the same crate behind the `consumer` Cargo feature.

## Repository Layout

| Path | Purpose |
|------|---------|
| `link/sdks/rust/README.md` | Canonical user-facing SDK readme (referenced by the crate manifest) |
| `link/sdks/rust/QUICKSTART.md` | Short onboarding guide |
| `link/sdks/rust/examples/` | Runnable example binaries |
| `link/sdks/rust/tests/` | SDK integration tests (`tests/*.rs`) |
| `link/sdks/rust/src/lib.rs` | Shared test helpers (`rust-sdk-tests` lib) |
| `link/sdks/rust/publish.sh` | crates.io publish helper |
| `link/kalam-client/` | The `kalam-client` library crate published to crates.io |
| `link/link-common/` | Shared implementation (path dependency today) |

## Prerequisites

| Tool | Version |
|------|---------|
| Rust | 1.92+ (workspace MSRV) |
| Tokio | via `native-sdk` feature (default) |

## Build From Source

Compile the library crate:

```bash
cd link/kalam-client
cargo build --features native-sdk
```

Build the example workspace:

```bash
cd link/sdks/rust
cargo build --workspace
```

Enable consumer or file uploads in dependent crates:

```bash
cargo build -p topic-consumer
```

## Running Examples

Examples live under `link/sdks/rust/examples/` and depend on `kalam-client` via path.

```bash
cd link/sdks/rust

# Terminal 1: start a KalamDB server
cd ../../../backend && cargo run --bin kalamdb-server

# Terminal 2: run examples
export KALAMDB_SERVER_URL=http://localhost:2900
export KALAMDB_ROOT_PASSWORD=kalamdb123

cargo run -p quickstart
cargo run -p live-inbox
cargo run -p topic-consumer
```

`topic-consumer` requires the `consumer` feature and topic routing support on the server.

## Running Tests

SDK-focused tests live in the `rust-sdk-tests` package at `link/sdks/rust/`:

```bash
cd link/sdks/rust
NO_SERVER=true ./test.sh          # offline API tests only
./test.sh                         # offline + integration + quickstart example
```

CI uses the release-server harness:

```bash
KALAMDB_SERVER_BIN=./kalamdb-server ./scripts/test-rust-sdk-release.sh
```

Additional integration coverage exists in `link/kalam-client` with `cargo test --features e2e-tests`.

## Feature Flags

| Feature | Enables |
|---------|---------|
| `native-sdk` (default) | `tokio-runtime`, `auth-flows` |
| `consumer` | `TopicConsumer`, `client.consumer()`, consume/ack APIs |
| `file-uploads` | Multipart SQL upload helpers |
| `healthcheck` | Cached health endpoint helper |
| `setup` | Bootstrap/setup helpers |
| `cluster` | Cluster health inspection |
| `wasm` | WASM bindings for JavaScript SDK builds |

Recommended app dependency:

```toml
kalam-client = { version = "0.5", features = ["native-sdk"] }
```

Workers that consume topics:

```toml
kalam-client = { version = "0.5", features = ["native-sdk", "consumer"] }
```

## Publishing to crates.io

The published crate name is **`kalam-client`**. User docs and examples live in `link/sdks/rust/`, but `cargo publish` runs from `link/kalam-client/`.

Today the crate depends on path-only workspace crates (`link-common`, `kalamdb-commons`, …). Before the first public release, publish or vendor those dependencies so `cargo publish --dry-run` succeeds without path references.

Use the helper script:

```bash
cd link/sdks/rust
./publish.sh --dry-run
```

Environment:

- `CARGO_REGISTRY_TOKEN` — crates.io API token (required except for `--dry-run`)

Options mirror the TypeScript SDK publish script:

- `--dry-run` — validate packaging without uploading
- `--skip-check` — skip `cargo test` gate
- `--version VERSION` — override version written into `link/kalam-client/Cargo.toml`

## Versioning

Release versions are tracked in the repo-root `Cargo.toml` workspace and mirrored in `versions.json` under `packages.rust.kalam-client`.

When bumping SDK versions, keep `kalam-client`, TypeScript packages, Dart, and Python cohorts aligned via `python3 scripts/versions.py verify`.

## License

Apache-2.0. See [LICENSE](https://github.com/kalamdb/KalamDB/blob/main/link/sdks/rust/LICENSE).
