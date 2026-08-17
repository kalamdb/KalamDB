# kalam-link-wasm

Browser WebAssembly bindings for `@kalamdb/client`.

This crate owns the browser transport layer (`wasm-bindgen`, `fetch`, `WebSocket`).
Shared protocol types, subscription materialization, compression, and auth models
live in [`link-common`](../link-common/README.md).

Native Rust applications should use [`kalam-client`](../sdks/rust/README.md).
