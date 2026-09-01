# FlatBuffers/FlexBuffers Migration Plan with Vortex Serde Learnings

## Scope
This document defines the migration plan for replacing bincode in key KalamDB persistence and replication paths while preserving current runtime semantics and enabling forward-compatible schema evolution.

It covers:
1. System table model persistence using FlatBuffers with explicit per-table schemas.
2. User/shared table row persistence using FlatBuffers with generic row encoding.
3. Raft command and response serialization using FlexBuffers for this phase.
4. Vortex-inspired scalar serialization design choices for performance and maintainability.

It does not cover full Vortex file-format adoption for flushed storage in this phase.

## Final Locked Decisions
1. `KSerializable` remains the storage boundary API. Targeted types override default bincode with envelope-based codecs.
2. New FlatBuffer envelope contract is mandatory:
   - `codec_kind`
   - `schema_id`
   - `schema_version`
   - `payload`
3. User/shared row payload uses generic row schema:
   - typed scalar tagged union (Vortex-inspired)
   - ordinal-vector row representation (not name-keyed map)
4. System tables use explicit `.fbs` per table with committed generated code and schema drift checks.
5. Raft migration in this phase is limited to commands/responses with FlexBuffers.
6. OpenRaft RPC and snapshot internals remain on current serialization path in this phase.
7. FlatBuffers evolution policy is append-only.
8. FlexBuffers evolution policy is additive optional fields with strict kind/version validation.
9. Legacy bincode on-disk values are not auto-migrated in this phase. Fail-fast with reset guidance.
10. Vortex is reference-only for serde patterns now. Parquet replacement is deferred to a future segment-format abstraction.

## Serialization Architecture by Subsystem

### 1) Common Storage Boundary (`kalamdb-commons`, `kalamdb-store`)
1. Keep `KSerializable` trait as the API used by `EntityStore` and `IndexedEntityStore`.
2. Introduce envelope helpers in commons serialization modules.
3. Decode flow:
   - read envelope
   - validate `codec_kind`, `schema_id`, `schema_version`
   - dispatch payload decoder
4. Encode flow:
   - encode type payload using selected codec
   - wrap with envelope metadata

Relevant current files:
- `backend/crates/kalamdb-commons/src/serialization.rs`
- `backend/crates/kalamdb-store/src/entity_store.rs`
- `backend/crates/kalamdb-store/src/indexed_store.rs`

### 2) User/Shared Tables (`kalamdb-commons`, `kalamdb-tables`)
1. Persist `Row`, `UserTableRow`, and `SharedTableRow` via FlatBuffers.
2. Use ordinal vectors to represent row values aligned to table definition ordering.
3. Keep `ScalarValue` in-memory for DataFusion and query logic.
4. Keep storage keys and partitioning unchanged.

Relevant current files:
- `backend/crates/kalamdb-commons/src/models/rows/row.rs`
- `backend/crates/kalamdb-commons/src/models/rows/user_table_row.rs`
- `backend/crates/kalamdb-tables/src/shared_tables/shared_table_store.rs`
- `backend/crates/kalamdb-tables/src/user_tables/user_table_store.rs`

### 3) System Tables (`kalamdb-system`)
1. Each persisted system model gets a dedicated `.fbs` schema.
2. Rust model continues to be source of behavior and provider logic.
3. `.fbs` is source of binary wire contract with explicit field IDs.
4. Build/CI checks enforce Rust schema and `.fbs` schema consistency.

Representative model locations:
- `backend/crates/kalamdb-system/src/providers/users/models/user.rs`
- `backend/crates/kalamdb-system/src/providers/jobs/models/job.rs`
- `backend/crates/kalamdb-system/src/providers/namespaces/models/namespace.rs`
- `backend/crates/kalamdb-system/src/providers/live_queries/models/live_query.rs`
- `backend/crates/kalamdb-system/src/providers/storages/models/storage.rs`

### 4) Raft Commands/Responses (`kalamdb-raft`)
1. Replace bincode serialization for command/response envelopes with FlexBuffers:
   - `RaftCommand`
   - `MetaCommand`
   - `UserDataCommand`
   - `SharedDataCommand`
   - `MetaResponse`
   - `DataResponse`
   - `RaftResponse`
2. Preserve routing/sharding semantics and apply flow.
3. Keep OpenRaft network RPC and snapshot serialization unchanged in this phase.

Relevant current files:
- `backend/crates/kalamdb-raft/src/commands/mod.rs`
- `backend/crates/kalamdb-raft/src/commands/meta.rs`
- `backend/crates/kalamdb-raft/src/commands/user_data.rs`
- `backend/crates/kalamdb-raft/src/commands/shared_data.rs`
- `backend/crates/kalamdb-raft/src/commands/data_response.rs`
- `backend/crates/kalamdb-raft/src/manager/raft_group.rs`
- `backend/crates/kalamdb-raft/src/manager/raft_manager.rs`
- `backend/crates/kalamdb-raft/src/state_machine/serde_helpers.rs`

## Vortex Serde Learnings Adopted
The plan adopts design patterns validated by Vortex scalar serialization:

1. Typed scalar union:
   - Use a single tagged scalar union in FlatBuffers for scalar payload.
   - Avoid ad-hoc per-type structs in row payloads.

2. Type/value separation:
   - Keep value payload compact.
   - Use table schema ordinal/type metadata as the primary type contract.

3. Strict decode validation:
   - reject missing kind tags
   - reject invalid widths for decimal/binary variants
   - reject invalid union/state combinations

4. Primitive canonicalization:
   - centralize numeric conversion logic in one module
   - avoid per-callsite inconsistent casts
   - preserve total ordering and stable hashing semantics where applicable

5. Additive evolution discipline:
   - allow unknown optional fields
   - reject unknown required discriminants/kinds

Reference sources:
- `vortex-scalar` `scalar_value.rs`
- `vortex-scalar` `proto.rs`
- `vortex-scalar` `pvalue.rs`
- `vortex-proto` `scalar.proto`

## Schema and Versioning Policy

### FlatBuffers
1. Append-only field evolution.
2. Never reuse, remove, or repurpose field IDs.
3. New fields must be optional or have safe defaults.
4. `schema_id` is stable per logical payload type.
5. `schema_version` increments on any wire-level change.

### FlexBuffers
1. Command/response root object includes:
   - `v` (version)
   - `kind` (required discriminant)
2. Additive optional fields are allowed.
3. Unknown fields are ignored.
4. Unknown required `kind` values are rejected with typed errors.

## Implementation Phases

### Phase 1: Foundation and Build Plumbing
1. Add schema layout in commons:
   - `backend/crates/kalamdb-commons/src/serialization/schema/`
   - `backend/crates/kalamdb-commons/src/serialization/generated/`
2. Add FlatBuffer codegen command and CI drift check.
3. Add envelope schema and helper encode/decode APIs.
4. Keep existing behavior unchanged until type-by-type cutover.

### Phase 2: User/Shared Row Cutover
1. Add row scalar schema (`ScalarValue` tagged union).
2. Add row payload schema (ordinal vectors).
3. Implement `KSerializable` overrides:
   - `Row`
   - `UserTableRow`
   - `SharedTableRow`
4. Add roundtrip and compatibility tests for row payloads.

### Phase 3: System Table Cutover
1. Add `.fbs` for each persisted system model.
2. Implement per-model FlatBuffer encode/decode overrides.
3. Add schema drift checks between Rust table definitions and `.fbs`.
4. Add negative decode tests for version/schema mismatch.

### Phase 4: Raft Command/Response Cutover
1. Add new command codec module for FlexBuffers.
2. Replace command/response serialize/deserialize call sites in:
   - raft group propose paths
   - manager forwarding paths
   - state machine apply/response paths
3. Preserve OpenRaft RPC/snapshot codec for this phase.
4. Add exhaustive command/response roundtrip tests.

### Phase 5: Rollout Controls
1. Add clear fail-fast error messages for legacy bincode data.
2. Document reset workflow for incompatible local/dev data.
3. Monitor payload size and latency against acceptance gates.

## Test Plan and Performance Gates
1. Scalar roundtrip tests for all supported scalar variants:
   - null
   - bool
   - numeric widths
   - decimal
   - utf8/binary
   - list-like embedding values
2. Row roundtrip tests for:
   - sparse and dense rows
   - mixed type columns
   - null-heavy payloads
3. System model roundtrip tests for each provider model codec.
4. Raft command/response roundtrip tests for all variants.
5. Negative tests:
   - wrong `schema_id`
   - unsupported `schema_version`
   - malformed union tag
   - unknown required command kind
6. Compatibility tests:
   - append-only schema additions decode with defaults
7. Performance gates:
   - p95 encode/decode latency regression <= 5%
   - payload size growth bounded and reported

## Rollout and Fallback
1. Rollout order:
   - commons foundation
   - user/shared rows
   - system tables
   - raft commands/responses
2. Fallback during development:
   - if decode fails for legacy data, return explicit remediation message
   - do not attempt silent mixed-format decode in this phase
3. Production safety:
   - include metrics for encode/decode failures by `schema_id` and version
   - include startup checks for unsupported persisted schema versions

## Explicit Non-Goals (This Phase)
1. Full OpenRaft RPC/snapshot migration to FlexBuffers/FlatBuffers.
2. Automatic historical bincode-to-FlatBuffer migration tooling.
3. Replacing Parquet with Vortex for flushed file segments.
4. Introducing per-user-table `.fbs` schema generation for dynamic tables.

## Assumptions and Defaults
1. Plan location is `docs/plans` with date-prefixed filename.
2. No historical data migration tool is required now.
3. `ScalarValue` remains the in-memory/query boundary type.
4. Generated FlatBuffer code is committed and CI-enforced for drift.
5. Vortex file-format adoption is future work after introducing segment format abstraction in `kalamdb-filestore` and manifest metadata (`Parquet | Vortex`).

