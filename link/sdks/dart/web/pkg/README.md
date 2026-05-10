# kalam-link

Shared transport/core crate and SDK workspace for KalamDB clients.

This directory contains:

- the Rust `kalam-link` core crate used by higher-level SDKs
- the publishable TypeScript package in [sdks/typescript](sdks/typescript/README.md)
- the publishable Dart/Flutter package in [sdks/dart](sdks/dart/README.md)

## Canonical SDK Docs

Use the package-specific READMEs as the source of truth for public APIs:

- TypeScript / JavaScript SDK: [sdks/typescript/README.md](sdks/typescript/README.md)
- Dart / Flutter SDK: [sdks/dart/README.md](sdks/dart/README.md)

Older constructor-based examples, manual `connect()` walkthroughs, and raw WASM `KalamClient(...)` snippets are not accurate for the current SDKs.

## Current SDK Shape

### TypeScript / JavaScript

- Create clients with `createClient({ url, authProvider, ... })`
- `authProvider` is required and can return `Auth.basic(...)`, `Auth.jwt(...)`, or `Auth.none()`
- WebSocket connection management is automatic; with `wsLazyConnect: true` the SDK connects on the first realtime call
- Prefer `live()` / `liveTable()` for UI state and `liveEvents()` when you need raw protocol events
- Topic worker APIs live in the TypeScript SDK: `consumer()`, `consumeBatch()`, `ack()`, `runConsumer()`

### Dart / Flutter

- Call `KalamClient.init()` once before first use
- Create clients with `KalamClient.connect(url: ..., authProvider: ...)`
- Auth flows use `authProvider`, `login(...)`, `refreshToken(...)`, and `refreshAuth(...)`
- The SDK exposes low-level events via `liveEvents(...)` and materialized row helpers via `live(...)` / `liveTable(...)`

## Repository Layout

```text
link/
|-- Cargo.toml
|-- src/                  # shared Rust transport/core crate
`-- sdks/
    |-- typescript/       # npm package: kalam-link
    `-- dart/             # pub package: kalam_link
```

## Build Notes

Package-specific build, test, and publish instructions live with each SDK:

- TypeScript / JavaScript: [sdks/typescript/README.md](sdks/typescript/README.md)
- Dart / Flutter: [sdks/dart/README.md](sdks/dart/README.md)

If you change the shared Rust core in this directory, validate the affected SDK package afterward.
│   ├── client.rs           # Native Rust client (tokio-runtime)
│   ├── auth.rs             # Authentication (tokio-runtime)
│   ├── query.rs            # Query execution (tokio-runtime)
│   ├── subscription.rs     # WebSocket subscriptions (tokio-runtime)
│   ├── error.rs            # Error types (conditional conversions)
│   └── wasm.rs             # WASM bindings (wasm feature)
├── pkg/                    # WASM build output (generated)
└── test-wasm.mjs           # Node.js WASM test script
```

### Building for Different Targets

**Native (CLI usage):**
```bash
cargo build --release
```

**WASM (web target):**
```bash
wasm-pack build --target web --no-opt --profile release-dist --features wasm --no-default-features
wasm-opt -Oz --all-features -o pkg/kalam_link_bg.wasm pkg/kalam_link_bg.wasm
```

**WASM (Node.js target):**
```bash
wasm-pack build --target nodejs --no-opt --profile release-dist --features wasm --no-default-features
wasm-opt -Oz --all-features -o pkg/kalam_link_bg.wasm pkg/kalam_link_bg.wasm
```

**WASM (bundler target for Webpack/Rollup):**
```bash
wasm-pack build --target bundler --no-opt --profile release-dist --features wasm --no-default-features
wasm-opt -Oz --all-features -o pkg/kalam_link_bg.wasm pkg/kalam_link_bg.wasm
```

If `wasm-opt` is not installed, install Binaryen and rerun the optimization step.

## License

Licensed under the Apache License, Version 2.0 (`Apache-2.0`). See [../../../../../LICENSE.txt](../../../../../LICENSE.txt) and [../../../../../NOTICE](../../../../../NOTICE).

## Contributing

See the main KalamDB repository for contribution guidelines.
