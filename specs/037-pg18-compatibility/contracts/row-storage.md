# Contract: Row Storage v2

Reviewed 2026-10-11 against workspace `be5efdd2`. This is a proposed format, not a measured performance result. G3 proves semantic representation first; the format switch additionally requires the benchmarks and upgrade gates below. It must not delay independent parse/cache/copy improvements. No unconditional promise of one lifetime format transition or zero-copy execution is made.

## Goals and ownership

- Decode only projected columns into Arrow batches, without name-keyed maps or intermediate `ScalarValue` vectors on SELECT and flush.
- Encode validated Arrow values into reusable, bounded buffers. Preserve defaults, constraints, RLS, generated values, triggers, RETURNING, and atomic failure behavior.
- `kalamdb-serialization` owns byte validation and codecs; `kalamdb-store` owns RocksDB buffers; `kalamdb-filestore` owns Parquet and file lifecycle. Core supplies immutable resolved descriptors, never per-row registry calls.
- Table data uses the shared codec where applicable. Existing catalog structs retain FlexBuffers inside KOBJ; do not migrate every system entity simply because it is exposed as a SQL table.

## Inspected baseline, not measured baseline

`row/batch.rs` builds `Vec<Vec<ScalarValue>>`, clones decoded slots, and calls `ScalarValue::iter_to_array`. `StorageSchema` currently has a u16 version and fields, with no historical-layout resolver. KOBJ has a 15-byte header; its current decoder accepts flag bits without checking a supported mask. These are improvement opportunities, not timing measurements. T087 measures the actual paths and workloads before comparison.

## Proposed layout

Keep the KOBJ envelope and existing row keys only after T097 proves format compatibility. A new row-body flag is insufficient by itself to protect old readers. Reserve and validate the version/flag combination and define a tombstone-only body independently of a full row.

| Region | Encoding and invariant |
| --- | --- |
| Row flags | Deletion, null-bitmap presence, offset width; reserved combinations rejected |
| Slot span | u16 physical ordinal span for this historical schema; not the number of live columns. It must agree with the resolved layout. Dropped slots are never reused. |
| Null bitmap | Optional, `ceil(span / 8)`; absent means no stored nulls. Missing historical columns are distinct from explicit NULL. |
| Fixed region | Per-version offsets and widths; NULL slots zeroed. Benchmark packed versus aligned layouts before freezing; widest-first packing alone is not a proven speedup. |
| Variable offsets | Monotone end offsets relative to variable-data start, one per stored variable slot. Smallest width fitting the final end: u8/u16/u32; checked arithmetic, no wrap. |
| Variable data | Validated slices interpreted using the historical logical descriptor, with explicit nested revision and array-shape information |

The bitmap's optional length changes the region base, not offsets within the fixed region. O(1) field access uses both. Row decoding cannot derive layout from the current schema or from column count alone. No per-value scalar type tag is needed where the historical descriptor fixes that type; nested format/revision discriminators remain where needed for evolution.

### Validation and limits

Validate kind, supported protocol/body version and flags, total length, slot span, bitmap and region sizes, monotone offsets, nested sizes, and UTF-8 before exposing a trusted `RowView` or Arrow string. Persisted, replicated, and restored bytes are not inherently trusted. Never construct invalid Rust strings or use unchecked Arrow constructors based solely on write-time validation. Projection may skip value conversion, not structural safety checks.

Enforce checked aggregate lengths and bounds before allocation, including recursion, array cardinality, maximum cell/row size, and decoded expansion. Preserve or tighten the existing batch limits (16,384 rows / 32 MiB input, nesting 32, list length 1,048,576) until T096 establishes explicit alternatives; also budget output, scratch space, and retained owners. Input bytes alone do not bound decoded memory. Split oversized batches; a single oversized value fails with a deterministic resource error before a partial write.

No automatic out-of-row pointers in the initial v2 implementation. Keep bounded inline values. Larger values require a separate design for replication, atomic publication, snapshot lifetime, access control, garbage collection, backup, restore, and crash recovery. `file` is a user-visible extension, not an interchangeable TOAST implementation. Its public type is the built-in `pg_catalog.file` type in [type-system.md](type-system.md); the row still stores the reference, and the bytes stay in the filestore. Limits remain explicit compatibility gaps where narrower than PostgreSQL.

## Schema history and DDL

Persist immutable layouts keyed by stable table identity and storage revision, including referenced custom-type revisions. Publish a schema revision durably before any row, log record, or segment referencing it becomes visible. Resolve layouts once per distinct revision in a bounded scan batch and cache them with `Arc`; do not retain every historical schema in RAM.

- ADD COLUMN: preserve a separate missing-value record captured at DDL time where PostgreSQL permits metadata-only addition. Never read an old row using today's default expression. Explicit NULL stays NULL.
- SET/DROP DEFAULT affects subsequent writes, not historical values. Volatile defaults and other rewrite-requiring additions need a transactional backfill before success; they cannot be evaluated lazily on SELECT.
- DROP/RENAME uses stable slots; drop and re-add with the same name gets a new slot. Prepared outputs, indexes, and old snapshots retain their correct descriptors.
- ALTER TYPE is classified before execution. Only a proven representation-compatible transition may avoid rewrite. Other changes evaluate USING/conversion and validate dependent constraints/indexes before atomic publication. Conversion failure leaves the previous schema and data intact. No blanket cast-on-read policy and no repeated volatile USING evaluation.

The reference distinguishes metadata defaults from changes that require rewriting and says changed defaults do not alter existing rows. [PostgreSQL 18 ALTER TABLE](https://www.postgresql.org/docs/18/sql-altertable.html).

A v1-to-v2 physical re-encoding may be lazy only when logical values remain unchanged. Flush/compaction must preserve snapshot-visible semantics and must not opportunistically apply a new SQL cast. Parquet manifests carry the necessary stable field/type identities and schema revision, or an equivalent complete descriptor; physical Arrow types alone are insufficient.

Durable history is retained while reachable from hot rows, segments, logs, replicas, active snapshots, and supported recovery artifacts. Cache eviction must not delete history. History GC requires an auditable reachability watermark and backup retention policy; self-contained backups include referenced descriptors. Rewrites are resumable and crash-safe. Define u16 exhaustion: reject schema-changing DDL before revision/slot overflow without changing data, until a versioned wider format is shipped. Never wrap or reuse an observable revision.

## Type representations

- Decimal128 is a fast path only for proven compatible numeric values/operations. Special values, intermediate overflow, rounding, and wide values follow [type-system.md](type-system.md).
- Store enum member identity, not its current position. Label order is separate revisioned metadata; inserting a label BEFORE/AFTER must not reinterpret old rows. [PostgreSQL 18 pg_enum](https://www.postgresql.org/docs/18/catalog-pg-enum.html).
- Arrays carry dimensions, lower bounds, element identity/revision and element validity; variable elements need offsets. A composite array cannot be represented by a single bitmap and packed elements alone. [PostgreSQL 18 arrays](https://www.postgresql.org/docs/18/arrays.html).
- Specify timestamptz units, epoch, range, infinity representation, and wire conversion explicitly. JSONB's storage representation and codecs are versioned and distinct from JSON text and PostgreSQL wire bytes.

## Read, write, and flush paths

`RowView` borrows a validated payload. Fixed primitives are copied into contiguous Arrow arrays. Text/binary may use view arrays retaining bounded owned blocks; short inline views, validation, collation, conversion, Parquet encoding, and wire encoding can still copy. Use precise copy/allocation measurements, not an end-to-end zero-copy claim.

RocksDB iterators feed bounded chunks directly into owned buffers or retain per-value owners, whichever T087/T098 demonstrates is better. Do not first collect all `Vec<u8>` values and then copy them into a giant block. Cancellation and backpressure release ownership promptly. A reusable encoder buffer may be overwritten only after its consumer has copied it or ownership has transferred; cap retained capacity after an outlier row.

Resolve a projection plan once per schema revision. Skip unused conversion while DataFusion remains responsible for visibility/version resolution and deletion filtering. Do not push predicates/limits below version resolution unless proven safe: filtering the latest row must never resurrect an older version.

Use `Utf8View`/`BinaryView` only where the pinned execution kernels and Parquet writers support them. JSONB needs typed operators rather than ordinary bytewise comparison. Measure owner bytes retained after selective filters, LIMIT, joins, and slow portals; compact survivors into smaller buffers when this reduces retained memory enough to justify copying. A small result must not indefinitely pin a large scan chunk.

SELECT and flush share codec/building primitives, but have different visibility and projection callers. Flush preserves required versions and tombstones and must not accidentally use a user SELECT's current-snapshot filter. Codec batching cannot alter retention or live-delivery frontiers.

## Upgrade and rollback

T097 supplies executable old-writer/new-reader fixtures before T090. New readers deploy first; v2 writers activate only after every reader, replica, snapshot restore tool, and Raft/log consumer in the supported topology advertises support through a durable format capability barrier. Pre-barrier writes remain in the old supported format or fail explicitly for unrepresentable values. Old binaries must refuse an incompatible store rather than attempt decoding unknown flags.

Test leader failover during activation, interrupted schema-history publication, mixed v1/v2 rows, backup/restore, and restart. After first v2 write, downgrade requires a verified export/rewrite to the older format or is explicitly unsupported. Removing the v1 decoder requires evidence across the supported upgrade/backup window, not telemetry from live rows alone.

## Acceptance and deletion

Use the dev-profile benchmark command in [quickstart.md](../quickstart.md), with workload and statistical gates from [execution-performance.md](execution-performance.md). Include null-heavy fixed rows, wide/sparse rows, short/long text, binary, nested arrays, tombstones, mixed schema versions, projected reads, selective results, DML and flush. Report time in seconds, allocations, encoded bytes, peak and retained memory. Do not trade data validation or semantics for a benchmark win.

Delete superseded map/scalar materialization only after all consumers move and semantic tests pass. Retain narrowly versioned compatibility decoders with owner/removal criteria. No claim that every possible row gets smaller: freeze the format only after workload-specific tradeoffs pass preregistered budgets.

## Function consumption

Function consumers retain the same bounded Arrow owners produced by this path; do not decode into maps or serialize KOBJ/JSON again merely to call a routine. A storage RowView borrowed from a temporary scan buffer cannot cross an async/V8 lifetime. Native reference ownership, projected authorization and V8 mutability rules are in [functions-v8.md](functions-v8.md). Returned compatible views can reuse native data before the final write/wire encoder; this does not remove validation, transaction semantics or output-encoding costs.

## Implemented Evidence (T086–T098, T101–T102)

- **Envelope & capability barrier (`T097`, `T090`)**:
  - `SUPPORTED_ROW_FLAGS = 0x0037` (`FLAG_DELETED | FLAG_SHARED_ROW | FLAG_SYSTEM_ROW | FLAG_ROW_BODY_V2 | FLAG_TOMBSTONE_ONLY`) in `backend/crates/kalamdb-serialization/src/object/envelope.rs`. Unknown flags and unsupported protocol versions fail closed with `SerializationError::InvalidEnvelope`.
  - Read-old/write-old deployment stage controlled by `is_row_v2_writer_enabled()` / `set_row_v2_writer_enabled(bool)` (defaults to `false` until cluster capability barrier completes).
  - Tombstone-only body (`FLAG_ROW_BODY_V2 | FLAG_TOMBSTONE_ONLY`) carries `u16(0)` with no column slots (`encode_tombstone_row_v2`).
- **Per-version layout & schema history (`T088`, `T089`, `T101`)**:
  - `StorageVersionLayout` (`backend/crates/kalamdb-serialization/src/row/schema.rs`) precomputes `slot_span`, `fixed_slots`, `fixed_offsets`, `fixed_size`, `var_slots`, and `slot_to_col` once per schema version.
  - `StorageSchema::register_historical_version` and `StorageSchema::layout_for_version` resolve historical row layouts by envelope `schema_version`.
  - `ColumnDefinition.missing_default` (`KALAM_MISSING_DEFAULT_METADATA_KEY`) captures non-volatile `ADD COLUMN ... DEFAULT` values at DDL time; subsequent `SET/DROP DEFAULT` does not alter existing rows (`backend/crates/kalamdb-core/tests/pg18_schema_evolution.rs`).
- **v2 encoder, `RowView<'a>`, and batch decoding (`T086`, `T091`–`T095`, `T098`, `T102`)**:
  - `RowView::parse` validates slot span, null-bitmap length, fixed region bounds, offset width (`0|1|2|4`), monotone variable end offsets, and zero trailing bytes before exposing `fixed_bytes`, `variable_slice`, or `decode_field`.
  - `decode_payloads_to_arrow_batch` resolves per-row `envelope.schema_version` against historical `StorageSchema` versions and writes directly into columnar accumulators (`ColumnAccum::Scalar`), verified across schema evolution (`v1_and_v2_row_decodes_across_add_drop_rename_and_alter_type` in `row/mod.rs`) and adversarial truncation/overflow cases (`row/decode.rs`).
  - Extended `backend/crates/kalamdb-serialization/benches/codec.rs` with `row_codec_text_heavy` and `payloads_to_record_batch` (`v1_128_rows` and `v2_128_rows`).
