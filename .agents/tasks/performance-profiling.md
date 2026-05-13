# Performance Profiling Agent Task

## Purpose
Iteratively identify and fix INSERT hot-path bottlenecks using Jaeger distributed tracing.

## Prerequisites
- **Jaeger** running with OTLP gRPC endpoint on `http://127.0.0.1:4317` and UI on `http://127.0.0.1:16686`
  ```bash
  # Start via docker compose (already configured in docker/utils)
  cd docker/utils && docker-compose up -d jaeger
  # Or standalone:
  docker run -d --name jaeger -p 16686:16686 -p 4317:4317 -p 4318:4318 jaegertracing/jaeger:latest
  ```
- **KalamDB server** running with OTLP enabled in `server.toml`:
  ```toml
  [logging.otlp]
  enabled = true
  endpoint = "http://127.0.0.1:4317"
  protocol = "grpc"
  service_name = "kalamdb-server"
  timeout_ms = 3000
  ```

## Workflow (Iterative Loop)

### Step 1: Run Benchmark
```bash
cd cli && KALAMDB_SERVER_URL="http://127.0.0.1:2900" KALAMDB_ROOT_PASSWORD="kalamdb123" \
  cargo test --features e2e-tests --test smoke smoke_test_insert_throughput_summary -- --nocapture
```

Record baseline numbers:
- Single-row inserts: ___ /s
- Batched (100/batch): ___ /s
- Parallel (10 threads): ___ /s

### Step 2: Analyze Traces in Jaeger
```bash
python3 /tmp/analyze_traces.py
# Or open Jaeger UI: http://localhost:16686
# Service: kalamdb-server, Operation: sql.execute
```

Key things to look for:
1. **Large time gaps** between child spans = unaccounted overhead
2. **Span durations** exceeding expectations (>1ms for in-memory ops)
3. **Aggregate statistics** across many traces to find consistent bottlenecks

### Step 3: Identify Bottleneck
Look at the trace waterfall and identify:
- Which span takes the most time?
- Is there a gap between spans where time is unaccounted?
- Is the bottleneck in parsing, planning, coercion, encoding, or storage?

### Step 4: Add Targeted Spans (if needed)
If you find an unaccounted gap, add tracing spans around the suspected code:

**For sync functions** (safe pattern):
```rust
let _span = tracing::info_span!("my_operation", field = value).entered();
// ... sync code ...
// span auto-drops at end of scope
```

**For async functions** (use one of these patterns):
```rust
// Pattern 1: #[tracing::instrument] on the fn (cleanest)
#[tracing::instrument(name = "my_op", skip_all, fields(count))]
async fn my_op(&self) -> Result<T, E> { ... }

// Pattern 2: .instrument() on async block
async move { ... }.instrument(tracing::info_span!("my_op")).await

// Pattern 3: Event-based (no span, just timing)
let start = std::time::Instant::now();
let result = some_async_op().await;
tracing::info!(elapsed_ms = %format!("{:.3}", start.elapsed().as_micros() as f64 / 1000.0), "my_op");
```

**NEVER** use `.entered()` in async functions across `.await` points — it creates `!Send` futures.

### Step 5: Fix the Bottleneck
Common fixes:
- **SQL parsing overhead**: Use plan caching, parameterized queries
- **Coercion overhead**: Batch coercion, skip for already-typed data
- **Encoding overhead**: FlatBuffer reuse, batch encoding (already optimized)
- **RocksDB overhead**: WriteBatch (already used), column family tuning
- **Auth overhead**: Cache JWT validation results
- **DataFusion planning**: Session reuse, plan caching

### Step 6: Rebuild & Re-run
```bash
# Kill server
lsof -i :2900 -t | xargs kill

# Rebuild (incremental)
cd backend && cargo build --bin kalamdb-server

# Start server
cargo run --bin kalamdb-server &

# Wait for startup
sleep 10 && curl -s http://127.0.0.1:2900/health

# Re-run benchmark (Step 1)
```

### Step 7: Compare Results
Compare before/after throughput numbers. If improved, commit. If not, investigate further.

## Current Tracing Spans (INSERT Hot Path)

```
POST /v1/api/sql                          (tracing-actix-web, HTTP layer)
├── auth.check                            (kalamdb-auth, JWT/Basic validation)
│   └── auth.bearer / auth.username_password
└── sql.execute                           (kalamdb-core, SQL executor)
    └── sql.dml_datafusion                (kalamdb-core, DML via DataFusion)
        ├── [sql.dml_plan - logged as event with plan_ms]
        ├── record_batches_to_rows        (kalamdb-tables, Arrow→Row conversion)
        ├── coerce_rows                   (kalamdb-commons, type coercion)
        ├── insert_batch.coerce_rows      (kalamdb-tables, coerce inside batch insert)
        ├── rocksdb.scan                  (kalamdb-store, PK uniqueness check)
        ├── insert_batch.encode           (kalamdb-tables, FlatBuffer encoding)
        │   └── batch_encode_user_table_rows / batch_encode_shared_table_rows
        ├── insert_batch.store_write      (kalamdb-tables, RocksDB write)
        │   └── store.insert_batch_preencoded
        │       └── rocksdb.batch         (kalamdb-store, RocksDB WriteBatch)
        ├── [sql.dml_collect - logged as event with collect_ms]
        └── rocksdb.batch                 (manifest mark_pending_write)
```

## Jaeger Analysis Script

Save this as `/tmp/analyze_traces.py` for quick CLI analysis:
```bash
python3 /tmp/analyze_traces.py
```

The script fetches recent `sql.execute` traces from Jaeger's API, displays them as a waterfall with timing, and prints aggregate statistics.

## Known Bottlenecks (as of last analysis)

**Single INSERT trace breakdown (~3.4ms total in `sql.dml_datafusion`):**

| Component | Average | % of Total | Notes |
|-----------|---------|-----------|-------|
| DataFusion `df.collect()` | 2.869ms | 85% | Physical plan execution + `insert_into` call |
| DataFusion `session.sql()` plan | 0.327ms | 10% | SQL parse + logical plan |
| Auth (JWT) | ~0.15ms | 4% | Bearer token validation |
| Storage+encode sub-spans | ~0.2ms | 6% | Everything below: |
| - Coercion (`coerce_rows`) | 0.01ms | <1% | Type coercion |
| - PK check (`rocksdb.scan`) | 0.03ms | <1% | Primary key uniqueness |
| - FlatBuffer encode | 0.03ms | <1% | Batch encoding (optimized) |
| - RocksDB write | 0.05ms | <1% | WriteBatch (optimized) |

**Inside `df.collect()` (2.87ms avg):**
- Our `insert_into` → `insert_batch` code only takes ~0.2ms
- The remaining ~2.67ms is **DataFusion's internal physical plan execution overhead** 
  (physical plan creation, values→Arrow conversion, CastExec, DML execution framework)

**Primary optimization target**: DataFusion's `df.collect()` overhead (~2.7ms per INSERT).

Potential fixes:
1. **Bypass DataFusion for simple DML** — detect single-row INSERTs and route directly to table provider
2. **Plan caching for non-parameterized INSERTs** — reuse physical plans
3. **Session reuse** — avoid creating new SessionState per request
4. **Batch API** — accept multiple SQL statements per HTTP request to amortize overhead
