# Gate W Evidence Record (T071, T077, T078)

**Recorded:** 2026-10-24

WebSocket production paths encode with `KDOM` (`ws_domain`) or explicit JSON. The MessagePack cache, `WireNotification::to_msgpack`, the `kalamdb-commons` `msgpack` feature, and the unused `kalamdb-api` `rmp-serde` dependency are removed.

## Commands

- `python3 link/tests/sdk-contracts/check_codec_ownership.py` — passed. Lean `kalamdb-serialization` has no forbidden server dependencies. Production WebSocket, API, and SDK sources contain no `rmp_serde`, `to_msgpack`, or `SerializationType::MessagePack`. The built-in negative control rejects a synthetic snippet that contains those tokens. The guarantee matrix lists all nine required surfaces.
- `cargo nextest run --lib -p kalamdb-commons -p kalamdb-serialization -p kalamdb-dialect -p kalamdb-core -p kalamdb-api -p link-common -p kalam-cli` — 2209 passed, 1 skipped.
- `python3 scripts/check-serialization-boundary.py --fail` — 0 forbidden codec findings.
