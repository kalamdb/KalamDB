# KalamDB Logging

This file used to describe an older logging implementation and is no longer the canonical source.

Use the published docs instead:

- https://kalamdb.org/docs/server/configurations/logging

Current state in the backend:

- KalamDB uses `tracing-subscriber`, not `fern`
- `logging.format = "json"` writes JSON Lines to `server.jsonl`
- `logging.format = "compact"` writes human-readable text logs to `server.log`
- `KALAMDB_LOG_FORMAT=json` enables JSON logging from environment variables
- `[logging.otlp]` configures OTLP trace export for Jaeger or other collectors
- `system.server_logs` expects JSON logging to be enabled
