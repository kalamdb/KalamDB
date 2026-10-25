# Workspace dependency map

Snapshot: `2101950e09daabb80aad95c222211c05a056d4c2` with the existing dirty lockfile, 2026-10-25.

All 62 workspace packages are listed. Edges below are resolved normal dependencies between workspace members in the workspace-default metadata snapshot; optional enabled features and platform conditions are retained in `dependency-map.json`. Dev/build edges and external dependencies are also in that JSON. Test counts include feature-gated targets. Reverse reach counts are structural rebuild exposure, not measured rebuilds.

| Package | Default server | CLI | Integration targets | Direct internal normal dependencies | Internal reverse reach |
|---|---|---|---:|---|---:|
| `kalam-cli` | — | yes | 2 | `kalam-client`, `kalam-schema-diff`, `kalamdb-commons`, `kalamdb-dialect`, `kalamdb-functions-host` | 0 |
| `kalam-cli-e2e` | — | — | 1 | — | 0 |
| `kalam-client` | — | yes | 1 | `link-common` | 4 |
| `kalam-client-e2e` | — | — | 1 | — | 0 |
| `kalam-consumer-wasm` | — | — | 0 | `link-common` | 0 |
| `kalam-link-dart` | — | — | 0 | `kalam-client` | 0 |
| `kalam-link-wasm` | — | — | 1 | `link-common` | 0 |
| `kalam-pg-api` | — | — | 1 | `kalam-pg-common`, `kalamdb-commons` | 3 |
| `kalam-pg-client` | — | — | 2 | `kalam-pg-api`, `kalam-pg-common`, `kalamdb-pg` | 1 |
| `kalam-pg-common` | — | — | 1 | — | 5 |
| `kalam-pg-extension` | — | — | 6 | `kalam-pg-api`, `kalam-pg-client`, `kalam-pg-common`, `kalam-pg-fdw`, `kalamdb-commons` | 0 |
| `kalam-pg-fdw` | — | — | 3 | `kalam-pg-api`, `kalam-pg-common`, `kalamdb-commons` | 1 |
| `kalam-pg-types` | — | — | 1 | `kalam-pg-common`, `kalamdb-commons` | 0 |
| `kalam-schema-diff` | — | yes | 4 | `kalamdb-commons`, `kalamdb-dialect` | 1 |
| `kalamdb-api` | yes | — | 0 | `kalamdb-auth`, `kalamdb-commons`, `kalamdb-configs`, `kalamdb-core`, `kalamdb-dialect`, `kalamdb-filestore`, `kalamdb-functions`, `kalamdb-jobs`, `kalamdb-live`, `kalamdb-observability`, `kalamdb-raft`, `kalamdb-serialization`, `kalamdb-session`, `kalamdb-system`, `kalamdb-transfer` | 1 |
| `kalamdb-auth` | yes | — | 3 | `kalamdb-commons`, `kalamdb-configs`, `kalamdb-observability`, `kalamdb-session`, `kalamdb-system` | 17 |
| `kalamdb-backend` | yes | — | 4 | `kalamdb-commons`, `kalamdb-transactions` | 17 |
| `kalamdb-commons` | yes | yes | 2 | `kalamdb-observability` | 53 |
| `kalamdb-configs` | yes | yes | 0 | — | 38 |
| `kalamdb-core` | yes | — | 55 | `kalamdb-auth`, `kalamdb-backend`, `kalamdb-commons`, `kalamdb-configs`, `kalamdb-datafusion-sources`, `kalamdb-dialect`, `kalamdb-filestore`, `kalamdb-flush`, `kalamdb-functions`, `kalamdb-functions-host`, `kalamdb-live`, `kalamdb-observability`, `kalamdb-plan-cache`, `kalamdb-publisher`, `kalamdb-raft`, `kalamdb-rls`, `kalamdb-serialization`, `kalamdb-session`, `kalamdb-session-datafusion`, `kalamdb-sharding`, `kalamdb-store`, `kalamdb-system`, `kalamdb-tables`, `kalamdb-transactions`, `kalamdb-vector`, `kalamdb-views` | 13 |
| `kalamdb-datafusion-sources` | yes | yes | 5 | `kalamdb-commons` | 35 |
| `kalamdb-dba` | yes | — | 1 | `kalamdb-commons`, `kalamdb-core`, `kalamdb-macros`, `kalamdb-serialization`, `kalamdb-system`, `kalamdb-tables` | 1 |
| `kalamdb-dialect` | yes | yes | 10 | `kalamdb-commons`, `kalamdb-session`, `kalamdb-system` | 22 |
| `kalamdb-filestore` | yes | — | 0 | `kalamdb-commons`, `kalamdb-configs`, `kalamdb-observability`, `kalamdb-store`, `kalamdb-system` | 20 |
| `kalamdb-flush` | yes | — | 0 | `kalamdb-commons`, `kalamdb-configs`, `kalamdb-filestore`, `kalamdb-observability`, `kalamdb-store`, `kalamdb-system`, `kalamdb-tables` | 14 |
| `kalamdb-functions` | yes | — | 2 | `kalamdb-commons`, `kalamdb-configs`, `kalamdb-filestore`, `kalamdb-functions-host`, `kalamdb-observability`, `kalamdb-serialization`, `kalamdb-store`, `kalamdb-system`, `kalamdb-views` | 14 |
| `kalamdb-functions-host` | yes | yes | 0 | — | 16 |
| `kalamdb-handlers` | yes | — | 0 | `kalamdb-commons`, `kalamdb-core`, `kalamdb-dialect`, `kalamdb-handlers-admin`, `kalamdb-handlers-ddl`, `kalamdb-handlers-stream`, `kalamdb-handlers-support`, `kalamdb-handlers-user` | 1 |
| `kalamdb-handlers-admin` | yes | — | 0 | `kalamdb-commons`, `kalamdb-core`, `kalamdb-dialect`, `kalamdb-handlers-support`, `kalamdb-jobs`, `kalamdb-raft`, `kalamdb-session`, `kalamdb-system`, `kalamdb-transfer` | 2 |
| `kalamdb-handlers-ddl` | yes | — | 0 | `kalamdb-commons`, `kalamdb-core`, `kalamdb-dialect`, `kalamdb-filestore`, `kalamdb-functions`, `kalamdb-handlers-support`, `kalamdb-jobs`, `kalamdb-raft`, `kalamdb-session`, `kalamdb-store`, `kalamdb-system`, `kalamdb-tables`, `kalamdb-vector` | 2 |
| `kalamdb-handlers-stream` | yes | — | 0 | `kalamdb-commons`, `kalamdb-core`, `kalamdb-dialect`, `kalamdb-handlers-support`, `kalamdb-observability`, `kalamdb-system`, `kalamdb-tables` | 2 |
| `kalamdb-handlers-support` | yes | — | 0 | `kalamdb-commons`, `kalamdb-core`, `kalamdb-dialect`, `kalamdb-session`, `kalamdb-system` | 6 |
| `kalamdb-handlers-user` | yes | — | 0 | `kalamdb-auth`, `kalamdb-commons`, `kalamdb-core`, `kalamdb-dialect`, `kalamdb-handlers-support`, `kalamdb-system` | 2 |
| `kalamdb-jobs` | yes | — | 0 | `kalamdb-commons`, `kalamdb-core`, `kalamdb-flush`, `kalamdb-observability`, `kalamdb-raft`, `kalamdb-sharding`, `kalamdb-store`, `kalamdb-system`, `kalamdb-tables`, `kalamdb-transfer` | 5 |
| `kalamdb-live` | yes | — | 0 | `kalamdb-commons`, `kalamdb-dialect`, `kalamdb-observability`, `kalamdb-publisher`, `kalamdb-row-filter`, `kalamdb-session`, `kalamdb-system` | 14 |
| `kalamdb-macros` | yes | yes | 0 | — | 33 |
| `kalamdb-observability` | yes | yes | 1 | — | 54 |
| `kalamdb-pg` | yes | — | 11 | `kalamdb-auth`, `kalamdb-backend`, `kalamdb-commons`, `kalamdb-server-auth` | 4 |
| `kalamdb-pg-bridge` | yes | — | 0 | `kalamdb-auth`, `kalamdb-backend`, `kalamdb-commons`, `kalamdb-core`, `kalamdb-pg`, `kalamdb-raft`, `kalamdb-session-datafusion`, `kalamdb-tables`, `kalamdb-transactions` | 1 |
| `kalamdb-plan-cache` | yes | — | 0 | `kalamdb-commons` | 14 |
| `kalamdb-postgres-wire` | yes | — | 4 | `kalamdb-auth`, `kalamdb-backend`, `kalamdb-commons`, `kalamdb-configs`, `kalamdb-core`, `kalamdb-dialect`, `kalamdb-observability`, `kalamdb-session` | 1 |
| `kalamdb-publisher` | yes | — | 0 | `kalamdb-commons`, `kalamdb-observability`, `kalamdb-row-filter`, `kalamdb-store`, `kalamdb-system`, `kalamdb-tables` | 15 |
| `kalamdb-python` | — | — | 0 | `kalam-client` | 0 |
| `kalamdb-raft` | yes | — | 5 | `kalamdb-commons`, `kalamdb-configs`, `kalamdb-observability`, `kalamdb-serialization`, `kalamdb-sharding`, `kalamdb-store`, `kalamdb-system`, `kalamdb-transactions` | 16 |
| `kalamdb-rls` | yes | — | 0 | `kalamdb-commons`, `kalamdb-dialect` | 18 |
| `kalamdb-row-filter` | yes | — | 0 | `kalamdb-commons`, `kalamdb-dialect` | 16 |
| `kalamdb-serialization` | yes | yes | 0 | `kalamdb-commons` | 42 |
| `kalamdb-server` | yes | — | 3 | `kalamdb-api`, `kalamdb-auth`, `kalamdb-commons`, `kalamdb-configs`, `kalamdb-core`, `kalamdb-dba`, `kalamdb-functions`, `kalamdb-handlers`, `kalamdb-jobs`, `kalamdb-live`, `kalamdb-observability`, `kalamdb-pg-bridge`, `kalamdb-postgres-wire`, `kalamdb-store`, `kalamdb-system` | 0 |
| `kalamdb-server-auth` | yes | — | 0 | — | 5 |
| `kalamdb-session` | yes | yes | 0 | `kalamdb-commons` | 34 |
| `kalamdb-session-datafusion` | yes | yes | 0 | `kalamdb-commons`, `kalamdb-session` | 33 |
| `kalamdb-sharding` | yes | yes | 0 | `kalamdb-commons`, `kalamdb-configs` | 37 |
| `kalamdb-store` | yes | yes | 0 | `kalamdb-commons`, `kalamdb-configs`, `kalamdb-observability`, `kalamdb-serialization`, `kalamdb-sharding` | 33 |
| `kalamdb-streams` | yes | — | 0 | `kalamdb-commons`, `kalamdb-serialization`, `kalamdb-sharding` | 18 |
| `kalamdb-system` | yes | yes | 3 | `kalamdb-commons`, `kalamdb-datafusion-sources`, `kalamdb-macros`, `kalamdb-serialization`, `kalamdb-session-datafusion`, `kalamdb-store` | 32 |
| `kalamdb-tables` | yes | — | 5 | `kalamdb-commons`, `kalamdb-datafusion-sources`, `kalamdb-filestore`, `kalamdb-macros`, `kalamdb-observability`, `kalamdb-rls`, `kalamdb-serialization`, `kalamdb-session`, `kalamdb-session-datafusion`, `kalamdb-sharding`, `kalamdb-store`, `kalamdb-streams`, `kalamdb-system`, `kalamdb-transactions`, `kalamdb-vector` | 17 |
| `kalamdb-transactions` | yes | — | 1 | `kalamdb-commons`, `kalamdb-datafusion-sources`, `kalamdb-sharding` | 25 |
| `kalamdb-transfer` | yes | — | 0 | `kalamdb-commons`, `kalamdb-core`, `kalamdb-system` | 6 |
| `kalamdb-vector` | yes | — | 2 | `kalamdb-commons`, `kalamdb-datafusion-sources`, `kalamdb-filestore`, `kalamdb-serialization`, `kalamdb-session-datafusion`, `kalamdb-store`, `kalamdb-system` | 18 |
| `kalamdb-views` | yes | — | 3 | `kalamdb-commons`, `kalamdb-configs`, `kalamdb-datafusion-sources`, `kalamdb-raft`, `kalamdb-session`, `kalamdb-session-datafusion`, `kalamdb-system` | 15 |
| `link-common` | — | yes | 0 | `kalamdb-commons`, `kalamdb-serialization` | 7 |
| `quickstart` | — | — | 0 | `kalam-client` | 0 |

## Duplicate package versions in the default server

These are candidates for inspection, not proof that versions are safely interchangeable.
- `alloc-no-stdlib`: 2.0.4, 3.0.0
- `alloc-stdlib`: 0.2.2, 0.3.0
- `base64`: 0.21.7, 0.22.1, 0.23.1
- `bitflags`: 1.3.2, 2.13.0
- `block-buffer`: 0.10.4, 0.12.0
- `brotli`: 8.0.2, 9.0.0
- `brotli-decompressor`: 5.0.0, 6.0.1
- `const-oid`: 0.10.2, 0.9.6
- `cpufeatures`: 0.2.17, 0.3.0
- `crypto-common`: 0.1.7, 0.2.1
- `darling`: 0.20.11, 0.24.1
- `darling_core`: 0.20.11, 0.24.1
- `darling_macro`: 0.20.11, 0.24.1
- `derive_more`: 1.0.0, 2.1.1
- `derive_more-impl`: 1.0.0, 2.1.1
- `digest`: 0.10.7, 0.11.2
- `foldhash`: 0.1.5, 0.2.0
- `getrandom`: 0.2.17, 0.3.4, 0.4.2
- `h2`: 0.3.27, 0.4.15
- `hashbrown`: 0.14.5, 0.15.5, 0.17.1
- `hmac`: 0.12.1, 0.13.0
- `http`: 0.2.12, 1.4.0
- `itertools`: 0.10.5, 0.13.0, 0.14.0, 0.15.0
- `miniz_oxide`: 0.8.9, 0.9.1
- `num-bigint`: 0.4.6, 0.5.1
- `object_store`: 0.13.2, 0.14.2
- `rand`: 0.10.3, 0.8.8, 0.9.5
- `rand_chacha`: 0.3.1, 0.9.0
- `rand_core`: 0.10.0, 0.6.4, 0.9.5
- `rustix`: 0.38.44, 1.1.4
- `sha2`: 0.10.9, 0.11.1
- `shlex`: 1.3.0, 2.0.1
- `syn`: 1.0.109, 2.0.119, 3.0.9
- `thiserror`: 1.0.69, 2.0.21
- `thiserror-impl`: 1.0.69, 2.0.21
- `toml_datetime`: 0.6.11, 1.1.1+spec-1.1.0
- `untrusted`: 0.7.1, 0.9.0
- `winnow`: 0.5.40, 1.0.0
- `zstd`: 0.13.3, 0.14.3
- `zstd-safe`: 7.2.4, 8.2.0

## Package totals

| Scenario | Packages including roots | Workspace packages | Edges |
|---|---:|---:|---:|
| `server-linux` | 620 | 45 | 2620 |
| `server-production-linux` | 645 | 45 | 2725 |
| `cli-linux` | 503 | 17 | 1756 |
| `cli-e2e-linux` | 524 | 18 | 1833 |
| `ci-linux` | 736 | 56 | 3157 |
