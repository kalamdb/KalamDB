# Contract: OpenRaft 0.9.25 → 0.10.0

**Feature**: `039-unified-table-transactions`  
**Stories**: User Story 8 (FR-048–FR-055, SC-016, SC-017)  
**Tasks**: T090–T119  
**Origin**: [GitHub issue #451](https://github.com/kalamdb/KalamDB/issues/451) and the OpenRaft section of [PR #412](https://github.com/kalamdb/KalamDB/pull/412), transcribed 2026-11-15. This file is the implementation contract. A link back to the issue is not completion. When this file and `spec.md` disagree, this file wins and `spec.md` is corrected.

**Branch**: `feat/0.9-unified-table-transactions` (PR #412). Land the migration in separately reviewable commits on that branch. Do not open a second branch.

**Upstream references**:

- [v0.10.0 changelog](https://github.com/databendlabs/openraft/blob/v0.10.0/change-log/v0.10.0.md)
- [0.9 → 0.10 upgrade guide](https://docs.rs/openraft/0.10.0/openraft/docs/upgrade_guide/upgrade_09_10/index.html)
- [openraft-multi](https://github.com/databendlabs/openraft/blob/v0.10.0/multiraft/README.md) and its [GroupRouter adapter](https://github.com/databendlabs/openraft/blob/v0.10.0/multiraft/src/network.rs)
- [multi-raft-kv example](https://github.com/databendlabs/openraft/blob/v0.10.0/examples/multi-raft-kv/README.md) is architecture only

API names in this contract were checked against the upgrade guide on 2026-11-15. `Raft::as_leader()`, `api_batch_capacity`, and `api_batch_linger_ms` are not in the guide. Confirm them in the 0.10.0 API docs before a task depends on them.

## What this upgrade is

KalamDB moves from OpenRaft **0.9.25** (`serde`, `storage-v2`) to **0.10.0** and adopts an upstream API only when it is safer or smaller than the current code on a measured path. The workspace already meets the 0.10 toolchain floor (Rust 1.88, Tokio 1.39; the workspace has Rust 1.99.1 and Tokio 1.54). Adopting a 2024-edition crate does not change the workspace edition.

KalamDB is already multi-Raft. `RaftManager` owns independent Meta, USER, and SHARED groups (defaults 1 + 32 + 4). Do not build a second group manager to match an upstream example.

## Boundaries

1. Each Raft group replicates alone. `openraft-multi` and `client_write_many()` do not add cross-group atomicity. Distributed XA and shard placement stay in issues #432, #433, and #440–#450.
2. A group-local `LogId` or `VersionId` is never compared with one from another USER or SHARED group.
3. Standalone single-entry commit stays one RocksDB `WriteBatch` covering the Raft log, `last_applied`, and the table or index mutations. Multi-node append stays on the log I/O path. Meta→Data materialization watermarks stay (issue #436). Do not treat Raft `last_applied` as that watermark.
4. Keep `RaftPartitionStore` and the application state machines. `examples/log-rocks` and `examples/sm-rocks` are illustrations (RocksDB 0.22). KalamDB uses RocksDB 0.25. Do not add a second RocksDB instance or a column family per group.
5. One SQL transaction stays one `RaftCommand::TransactionCommit` on one group. Several Raft log entries are not one atomic SQL transaction. Live events wait until the apply is persisted.
6. If a safety or compatibility gate fails, stop. Do not ship a partial 0.10 dependency switch or a disabled test. Record the blocker in issue #451 and in the decision log below.

## Required before merge

| Area | Outcome |
| --- | --- |
| Dependency | Workspace pin is `openraft = { version = "0.10.0", default-features = false, features = ["serde", "tokio-rt"] }`. `tokio-rt` is the 0.10 default runtime and is required. `clap`, the other default feature, stays off. The removed `storage-v2` feature is gone. `openraft-legacy` is absent from the final tree. |
| Type config | `KalamTypeConfig` uses the 0.10 associated types, with `Node = KalamNode` and `LeaderId = leader_id_std::LeaderId<u64, u64>` (see [Leader id and persisted format](#leader-id-and-persisted-format)). Application payload is a `RaftCommandBytes` newtype (`Debug + Display`) whose `Display` prints at most the command kind and byte length, never payload bytes. Existing log, vote, membership, `KalamNode`, and snapshot bytes still decode. |
| Snapshots | `SnapshotData` lives on the state machine. Snapshot meta has no `snapshot_id`. Existing snapshot files stay readable. See [Snapshot lifecycle](#snapshot-lifecycle). |
| Storage | `RaftLogStorage`, `RaftLogReader`, and `RaftStateMachine::apply` match 0.10, including `io::Error`, `read_vote` on the log reader, `truncate_after` (the named index is kept), `IOFlushed` / `io_completed`, and a streamed apply. `entries_stream` walks RocksDB in order with a bounded buffer and a cancellable reader. A read or decode failure is an `io::Error`. A reader never returns an empty or shorter range in place of an error, and never skips an entry it cannot decode. |
| Apply batch | Each applied entry writes through one explicit batch that the storage owns and passes to the state machine. On the standalone path, the batch also holds the log record and `last_applied`. The entry's responder fires only after that batch persists. `kalamdb_store::with_write_coalesce` is not used on the Raft path. |
| Startup | A store with purged logs missing from the state machine or snapshot, an index mismatch, a broken log tail, or an undecodable membership, vote, or snapshot fails with a diagnostic. Repair does not delete the last healthy copy or recreate an inconsistent group. See [Standalone deferred durability](#standalone-deferred-durability) for the one legal missing tail. |
| Network | `kalamdb-raft/src/network/` implements `RaftNetworkV2` for vote, append, snapshot, pre-vote, and leadership transfer. Group id routing, the shared tonic channel map, and cluster RPC authentication and TLS stay. `full_snapshot` is sent in bounded chunks below the gRPC message limit and reassembled before install. Append requests are bounded by bytes as well as by `max_payload_entries`. The Raft RPC envelope carries a protocol version, and a peer with another version is rejected before payload decode. |
| Leadership | `RaftGroup::transfer_leadership()` uses `trigger().transfer_leader`. Pre-Vote is enabled and fault-tested. Not-leader, lease expired, and leadership transfer are distinct errors. `ForwardToLeader` with reason `LeaseExpired` retries on the same node with bounded backoff and is never forwarded to itself. `ClientWriteError::LogEntryDiscarded` returns an outcome-unknown error and is not retried blindly. Step-down and shutdown handover use the transfer API. |
| Metrics | `system.cluster_groups.committed` is the cluster commit index, not `NULL`. Local commit, flushed log, applied log, and SQL-materialized frontier stay distinct. Lag is computed only inside one group. `system.cluster` keeps its existing column names and order. `millis_since_last_heartbeat` uses quorum-ack or heartbeat data when the local node has it, and stays unknown when it does not. |
| Proof | OpenRaft log-storage suite, 0.9 fixture decode, persisted 0.9 restart on 0.10, standalone WriteBatch commit, 3-node failover, follower restart, and snapshot tests, and the current spec 039 transaction tests pass. CDC `VersionId` order and group-local guarantees do not regress. |

## Leader id and persisted format

OpenRaft 0.9 defaults to the advanced leader id, where a committed leader id is `(term, node_id)`. KalamDB's log record stores only term and index, and `stored_log_id()` in `backend/crates/kalamdb-raft/src/storage/raft_store.rs` rebuilds every stored log id with node `0`. In a cluster, an entry read from RocksDB can therefore compare unequal to the same entry in another node's memory.

Decision: KalamDB uses `leader_id_std::LeaderId<u64, u64>` on 0.10. A committed leader id is the term, which is what the log record already stores. This is the standard single-leader-per-term Raft election, and Pre-Vote is enabled with it.

KalamDB owns every persisted Raft byte layout. It does not bincode-encode an OpenRaft type into its own storage:

- The log record and vote already use KalamDB types. They keep their layout.
- Stored membership (`save_last_membership`) and the snapshot header (`StateMachineData`'s `last_applied_log` and `last_membership`) move to KalamDB-owned records that convert to and from the 0.10 types.
- The reader accepts the 0.9 advanced-leader-id bincode layout for those two records and drops the node id, proven by the T090 fixtures. The writer emits only the new records.

Decision log rows that depend on this: `LeaderId`, persisted format.

## Standalone deferred durability

A standalone group (`peers` empty at `RaftGroup::start`) defers the log record into the apply batch. `append()` reports `io_completed` before the record is on disk. The log record, `last_applied`, and the table mutations then persist together, so applied never passes the durable log tail.

- After a crash, a log tail above `last_applied` may be missing. That is legal in standalone mode and is not a torn tail. No client was acknowledged for those entries.
- The mode is fixed at start. `add_learner` on a standalone group is rejected until the node restarts in flush-on-append mode. A follower never counts a record that is not yet durable.
- In cluster mode the log is durable before `io_completed`.

## Snapshot lifecycle

- A snapshot is named by its covered `last_log_id` (term and index) plus a local build counter, not by `snapshot_id`. KalamDB's own `RaftSnapshotMeta` may keep a local file id. The counter is recovered from that meta, not by parsing a string.
- After a new snapshot is durable, older snapshot files for that group are deleted down to the configured retention. `max_snapshots_to_keep` controls snapshot files. It is not passed to `max_in_snapshot_log_to_keep`, which counts log entries kept after a snapshot.
- Install is crash-safe: write and fsync the file, then write one `WriteBatch` with snapshot meta, data path, `last_applied`, membership, and purge marker. Delete older logs after that batch.
- Build, current-snapshot reads, and transfer share one `Arc` buffer or stream from the file. They do not copy the whole payload.
- Snapshot file I/O stays behind one small helper in `kalamdb-raft`. Moving it to `kalamdb-filestore` is a separate decision.

## Evaluate, then adopt or decline in writing

Record each row in the decision log before calling the upgrade done. Declining is a valid result.

| Topic | Rule |
| --- | --- |
| `openraft-multi` | It is a thin `GroupRouter` over an existing channel pool. It does not own groups, membership, SQL forwarding, or the shard router. Its adapter implements append, vote, pre-vote, full snapshot, and transfer, and it does **not** override `stream_append`, so OpenRaft falls back to sequential append. Adopting the crate is not pipelined replication. Keep it only when a proof of concept has less code or fewer allocations per append, and still preserves auth, TLS, timeouts, cancellation, and the shared channel pool. |
| `stream_append` | Add a real pipelined transport only with an on/off test. Otherwise leave unary append in place. |
| Four `log_io` workers | `flush_log_records` uses a blocking `sync_channel` send from the async append path, so a full queue blocks a Tokio worker thread. Either delete the workers and write through `spawn_blocking`, or turn them into group commit: each worker drains its queue and writes one `WriteBatch` per drain across groups, with per-group order kept. Choose from a profile. No option may block a Tokio worker thread. |
| Read linearizers | `ReadPolicy::ReadIndex`, lease reads, and `get_local_read_linearizer()` apply only to reads that are already specified as linearizable. Wait until the returned `ReadLogId` is applied. Do not quote the upstream in-core lease-read benchmark as SQL `SELECT` throughput. |
| Write batching | `Raft::write()`, `on_commit` versus `on_complete`, `client_write_many()`, and any API batch settings may change how entries are submitted. They must not split or merge SQL transactions, and they must not publish a commit before the apply is durable. |
| `runtime-stats` and `openraft-metrics-otel` | Optional and off unless a benchmark justifies them. They are not a DataFusion table provider. Label cardinality stays bounded. Per-entry histories do not go into SQL tables. |
| Quorum kinds | Majority membership stays. Weighted, grid, and threshold quorums are out of this migration. |

## 0.10 behavior changes

Each row is decided in T119 and recorded in the decision log. The default is the upstream behavior unless a test shows it breaks a KalamDB contract.

| Change | Decision needed |
| --- | --- |
| Leader lease (`LeaseExpired`) | Handled as in the Leadership row above. Not configurable away. |
| `enable_leader_restore` | A restarted leader campaigns for a higher term. Keep for clusters. Standalone needs no change. |
| Backoff (200 ms doubling to 1 s; 0.9 used 500 ms constant) | Keep upstream or set `Config::backoff`. Record which. |
| Bounded API, notification, and state-machine channels | A write burst waits for capacity. Map that wait into the existing SQL timeout and admission errors. |
| `removed_leader_step_down` | Keep the 150 ms default unless a membership test needs otherwise. |
| `trigger().elect(pre_vote)` | Manual election calls pass an explicit value. `ClusterAction::TriggerElection` keeps its current meaning. |
| First tick up to one interval later | Adjust test startup budgets, not production timeouts. |

## System views

`system.cluster_groups` is one row per local group. Keep the current column names and order. Field `committed` (today always `NULL` in `backend/crates/kalamdb-views/src/system/cluster_groups.rs`) becomes the cluster commit index. New progress columns are appended, or published on a separate versioned view. A follower's local frontier is not the quorum frontier.

`system.cluster` is one row per physical node. Keep the existing columns and CLI/UI order. Summaries may be appended: leaders, followers, lagging groups, quorum health, worst lag, heartbeat age, observation time. The Meta leader is not the leader of every group.

`system.cluster_nodes` is not registered (checked 2026-11-15). Do not add it. `backend/tests/docs/CLUSTER_SCENARIOS.md` still names it and is corrected to `system.cluster`.

Remote applied indexes come from the existing authenticated `GetNodeInfo` RPC through `peer_stats_cache` in `backend/crates/kalamdb-raft/src/executor/raft.rs`. Do not add a second peer RPC. A local watch cannot see another node's state machine. Missing data is `NULL` or unknown, including after a leader change.

Progress watches (`watch_log_progress`, `watch_vote_progress`, `watch_commit_progress`, `watch_apply_progress`, `watch_snapshot_progress`) feed one bounded cached snapshot. A SQL query does not spawn a watch task.

## Delivery order

1. **T090** records the 0.9.25 baseline and serialization fixtures before the dependency changes.
2. **T091–T097** compile against 0.10 and keep the old behavior. No performance rewrite in this step.
3. **T098–T101, T115–T118** finish streaming reads, the explicit apply batch, `truncate_after`, startup checks, snapshot lifecycle, and the log-storage suite.
4. **T102–T106** replace the leadership-transfer stub, enable Pre-Vote, and fill real cluster metrics.
5. **T107–T109, T119** record the optional-adoption and behavior-change decisions. Implement one only when its test exists.
6. **T110–T114** run faults, spec 039 regressions, benchmarks, the rolling-upgrade policy, and 0.9 cleanup.

## Verification workloads

- Standalone: one node, Meta + USER + SHARED, one `WriteBatch` per applied entry, 1,000-row and 25,000-row inserts and indexed updates.
- Cluster: 3 nodes; one busy group, the default group count, and a 100-group idle case. Record memory and CPU per Raft instance, connections, heartbeat load, append writes, and snapshot throughput. Include a snapshot larger than 4 MiB and a single `TransactionCommit` entry larger than 4 MiB.
- Faults: partition and rejoin, lagging follower, follower restart followed by more writes, restart during append and during snapshot, forced election, leadership transfer, quorum loss, expired leader lease, corrupt or missing snapshot, flooded bounded channels.
- Compatibility: read of persisted 0.9 log, vote, membership, snapshot, `KalamNode`, and command codec. Group names, group ids, shard routing, and version order stay.

## Rolling upgrade

**Coordinated restart of the whole cluster.** This is the final policy for this upgrade. KalamDB bincode-encodes OpenRaft RPC types. Upstream states that a positional format cannot decode a 0.9 `ForwardToLeader`, `VoteRequest` gains `leadership_transfer_from`, and error bodies do not interoperate. The leader id type also changes the `LogId` wire layout. Mixed 0.9/0.10 binaries are refused by the RPC protocol version check instead of failing inside decode.

## Decision log

Fill this table from T107–T109, T113, and T119. Leave a row `open` until that task records a result.

| Topic | Decision | Evidence |
| --- | --- | --- |
| Store backend | Keep `RaftPartitionStore`. Do not vendor `log-rocks` / `sm-rocks`. | This contract |
| SQL transaction shape | One `RaftCommand::TransactionCommit` per single-group SQL transaction | This contract |
| Cross-group atomicity | Unchanged. Not provided by this upgrade | Issues #432, #433, #440–#450 |
| `LeaderId` | `leader_id_std::LeaderId<u64, u64>` | [Leader id and persisted format](#leader-id-and-persisted-format), T090 fixtures |
| Persisted Raft format | KalamDB-owned records for log, vote, membership, and snapshot header; 0.9 bincode layout read-only | T093, T094 |
| Startup recovery | `Raft::build` restores the state machine and calls `try_create_snapshot_builder` (implemented on `KalamRaftStorage`, not deferred). Standalone `RaftGroup::start` then calls `wait_for_recovery` when `last_applied` is already set. Cluster `start` does not wait, because peers are not up yet. The Meta-to-Data watermark stays `MetaStateMachine::publish_last_applied` → coordinator `advance`, not the Raft `last_applied` of a data group | T101 |
| Mixed-version rollout | Coordinated restart of the whole cluster; RPC protocol version rejects mixed peers | This contract, T113 |
| `system.cluster_nodes` | Not registered. Not added | Checked 2026-11-15, T106 |
| `openraft-multi` | decline | T107. One `RaftChannelPool` is already shared by every group. `GroupNetworkAdapter` (openraft 0.10 `multiraft/src/network.rs`) clones the target and group id on append, vote, pre-vote, snapshot, and transfer, and it does not implement `stream_append`. A `GroupRouter` would sit on the existing auth, TLS, timeout, and cancellation path, adding code and a clone per RPC. The prototype was not kept. |
| Pipelined `stream_append` | decline | T108. `RaftNetworkV2::append_entries` stays the unary `send_raft_rpc` path. `stream_append` is not overridden, so OpenRaft uses `stream_append_sequential`. No off/on test was added. |
| `log_io` workers | keep the four shard workers | T109. A full queue uses `try_send`. The wait uses `block_in_place` on a Tokio worker so the runtime can move other tasks. The job is queued before `flush_log_records` returns, so one group stays on one shard in order. Workers were not deleted and were not collapsed into one cross-group `WriteBatch`; that needs a profile, and each job is already one `append_encoded` batch. |
| Lease / `ReadIndex` reads | decline | T109. SQL reads stay on the local applied state. No `ReadIndex` or `get_local_read_linearizer` is installed, so there is no `ReadLogId` to wait for. |
| API batching | decline extra batching | T109. OpenRaft names are `api_batch_capacity` (default 4096) and `api_batch_linger_ms` (default 0). Linger stays 0. KalamDB does not call `client_write_many`. Concurrent proposals remain separate `RaftCommand` log entries; one storage append does not fuse two SQL transactions. |
| `runtime-stats` | decline | T109. The feature is not enabled. System views remain the metrics surface. |
| `openraft-metrics-otel` | decline | T109. The crate is not a dependency. Export stays opt-in and is not a table provider. |
| `enable_leader_restore` | keep upstream default | T119. `Config::default` leaves it `None`, and OpenRaft treats that as enabled. A restarted cluster leader campaigns for a higher term. Standalone uses the same default. |
| Backoff | keep upstream | T119. Group configs do not set `Config::backoff`. OpenRaft keeps 200 ms doubling to 1 s. |
| Bounded API channels | keep OpenRaft defaults | T119. `api_channel_size` stays 65536. Notification and state-machine channels stay at `Config::default`. A full API channel waits inside `client_write`, which is the statement future. An oversized transaction is rejected by `admit_staged_commit` as `InvalidOperation` before it is queued. No second timeout is wrapped around apply. `RaftError::Timeout` stays the existing command-timeout error. |
| `removed_leader_step_down` | keep `After(150)` | T119. Group configs do not override the OpenRaft default. |
| `trigger().elect(pre_vote)` | real election for `ClusterAction::TriggerElection` | T119. `trigger_election` calls `elect(false)`. The election timer uses Pre-Vote because `enable_pre_vote` is `Some(true)`. |
| First tick | keep production timeouts | T119. Test waits stay at their existing multi-second budgets. Production election and heartbeat intervals are unchanged. |
| Busy-group proposals | accept the measured `RaftManager::new` flush | T112. 0.9.25 `KalamRaftStorage::new` had `persistent_store: None`, so the 3-node test acknowledged an append with no log IO. 0.10 `new()` uses `InMemoryBackend`, and a cluster group flushes before ack. Clean dev-profile p50 for 100 Meta proposals moved from 0.063006s to 0.131350s (+108%). Standalone RocksDB commit p50 moved from 0.033129s to 0.033810s (1,000 rows, +2.1%) and from 0.842913s to 0.845035s (25,000 rows, +0.3%). |
| Persisted vote | `Vote::new_committed` | T114. Recovery no longer calls deprecated `Vote::commit`. A stored committed vote is rebuilt with `Vote::new_committed`. |
| Remote RPC error | return the peer's error string | T114. A non-empty `RaftRpcResponse.error` is a network error carrying that string. Decode does not substitute `Fatal(Panicked)`. |
| `RaftRpcKind` | keep | T114. `RaftNetworkV2` still sends the tonic `RaftRpc` envelope. The kind selects vote, append, snapshot, pre-vote, and transfer. There is no second generic RPC path to delete. |
| `write_coalesce` | keep for flush | T114. `with_write_coalesce` is absent from `kalamdb-raft`. `kalamdb-flush` still calls it. T115 deletes the module only when no production caller remains. |
| Proposed command bytes | keep the per-command encoders | T114. `propose_with_index` calls `encode_proposed_command`. Meta, user, and shared entries keep their encoders, including packed insert and delete. `TransactionCommit` stays the `RaftCommand` envelope because both data state machines decode that wrapper. Encoding every command as that wrapper would replace the packed bytes already stored in logs. |
| In-memory Raft mode | removed | T117. `KalamRaftStorage::new` and `new_persistent` both use `RaftPartitionStore`. `new()` backs it with `InMemoryBackend`. |
| Startup commit index | seed from `last_applied` | T114. Recovery does not read a leftover `meta:commit`. `save_committed` updates the in-memory commit only. |
| Shutdown | wait for `Raft::shutdown` | T114. `RaftGroup::shutdown` joins `RaftCore` and the tick task. The 5 second timeout and `mem::forget` are gone. `single_node_shutdown_returns` finishes that join inside 2 seconds. |
