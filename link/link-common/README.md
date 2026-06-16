# link-common

Shared Rust implementation for KalamDB client crates.

Application code should depend on [`kalam-client`](https://crates.io/crates/kalam-client) for native Rust apps, or the language SDK packages for TypeScript and Dart. Browser WASM builds are implemented in the separate `kalam-link-wasm` crate, which depends on this crate for shared protocol logic.
