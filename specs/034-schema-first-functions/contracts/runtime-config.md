# Config: `[functions]` and `[functions.runtime]`

## Project (`kalam.toml` / project config)

```toml
[functions]
path = "functions"
runtime = "typescript"
module = "backend"
```

This is the **only** runtime declaration for project-backed procedures.

Defaults when the section is omitted: `path = "functions"`, `runtime = "typescript"`, `module = "backend"`.

## Server (`server.toml`)

```toml
[functions.runtime]
max_active = 16
max_queued = 128
workers = 4                 # default min(cpus, 4); 0 = auto
max_memory_mb = 256
heap_soft_mb = 16
heap_hard_mb = 64
max_idle_per_lane = 1
idle_ttl_secs = 30
max_instance_age_secs = 300
max_invocations_per_instance = 10000
timeout_ms = 5000
max_depth = 16
max_artifact_bytes = 16777216
max_value_bytes = 8388608
# additional host-op limits (SQL text, result rows/bytes, topic bytes, log bytes, header bytes)
```

Do not hard-code these as the only legal numbers; they are starting defaults after Stage 0. Admission must enforce **both** `max_active` and `max_memory_mb`.

`nested_reserve` is removed from the operator model.
