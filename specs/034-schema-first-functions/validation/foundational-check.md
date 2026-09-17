# T014 Foundational check

Command:

```bash
cargo check -p kalamdb-functions -p kalamdb-core -p kalamdb-system -p kalamdb-configs
```

Also verified: `kalamdb-handlers`, `kalamdb-commons`, `kalam-cli`.

Result: **PASS** (`Finished dev profile [unoptimized] target(s) in 42.39s`).

Notes:

- `wasm-runtime` is no longer a default feature of `kalamdb-functions`; Wasmtime is not pulled unless that feature is enabled.
- Pre-existing warning in `kalamdb-store` (`find_index_for_filter` unused) is unrelated.
