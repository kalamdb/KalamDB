//! UserDataStateMachine - Handles per-user data operations
//!
//! This state machine manages:
//! - User table INSERT/UPDATE/DELETE
//!
//! Runs in DataUserShard(N) Raft groups where N = user_id % 32.
//!
//! ## Watermark Synchronization
//!
//! Commands with `required_meta_index > current_meta_index` wait for Meta
//! to catch up before applying. This ensures data operations don't run before
//! their dependent metadata (tables, users) is applied locally.

use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc,
};

use async_trait::async_trait;
use kalamdb_commons::{models::TransactionId, TableType};
use kalamdb_transactions::StagedMutation;
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};

use super::{
    data_apply::{apply_data_entry, DataWatermark},
    decode, encode, ApplyResult, KalamStateMachine, PendingBuffer, PendingCommand,
    StateMachineSnapshot,
};
use crate::{
    applier::UserDataApplier, DataResponse, GroupId, RaftCommand, RaftError, UserDataCommand,
};

/// Snapshot data for UserDataStateMachine
#[derive(Debug, Clone, Serialize, Deserialize)]
struct UserDataSnapshot {
    /// Shard number
    shard: u32,
    /// Total operations count
    total_operations: u64,
    /// Pending commands waiting for Meta to catch up (for crash recovery)
    #[serde(default)]
    pending_commands: Vec<PendingCommand>,
}

enum UserApplyCommand {
    User(UserDataCommand),
    TransactionCommit {
        required_meta_index: u64,
        transaction_id: TransactionId,
        mutations: Vec<StagedMutation>,
    },
}

impl UserApplyCommand {
    fn required_meta_index(&self) -> u64 {
        match self {
            Self::User(command) => command.required_meta_index(),
            Self::TransactionCommit {
                required_meta_index,
                ..
            } => *required_meta_index,
        }
    }

    fn row_slot_count(&self) -> usize {
        match self {
            Self::User(command) => command.row_slot_count(),
            Self::TransactionCommit { mutations, .. } => mutations.len(),
        }
    }
}

/// State machine for user data operations
///
/// Handles commands in DataUserShard(N) Raft groups:
/// - Insert, Update, Delete (user table data)
///
/// Note: All data is persisted via the UserDataApplier after Raft consensus.
/// All nodes (leader and followers) call the applier, ensuring consistent data.
///
/// ## Watermark Synchronization
///
/// Commands with `required_meta_index > current_meta_index` wait for Meta
/// to catch up before applying. Pending commands restored from snapshots
/// are drained once Meta is satisfied.
pub struct UserDataStateMachine {
    /// Which shard this state machine handles (0-31)
    shard: u32,
    /// Last applied log index (for idempotency)
    last_applied_index: AtomicU64,
    /// Last applied log term
    last_applied_term: AtomicU64,
    /// Notifies waiters when the applied index advances.
    last_applied_tx: tokio::sync::watch::Sender<u64>,
    /// Approximate data size in bytes
    approximate_size: AtomicU64,
    /// Total operations processed
    total_operations: AtomicU64,
    /// Optional applier for persisting data to providers
    applier: RwLock<Option<Arc<dyn UserDataApplier>>>,
    /// Buffer for commands waiting for Meta to catch up
    pending_buffer: PendingBuffer,
}

impl std::fmt::Debug for UserDataStateMachine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UserDataStateMachine")
            .field("shard", &self.shard)
            .field("last_applied_index", &self.last_applied_index.load(Ordering::Relaxed))
            .field("has_applier", &self.applier.read().is_some())
            .field("pending_count", &self.pending_buffer.len())
            .finish()
    }
}

impl UserDataStateMachine {
    /// Create a new UserDataStateMachine for the specified shard
    pub fn new(shard: u32) -> Self {
        let (last_applied_tx, _) = tokio::sync::watch::channel(0);
        Self {
            shard,
            last_applied_index: AtomicU64::new(0),
            last_applied_term: AtomicU64::new(0),
            last_applied_tx,
            approximate_size: AtomicU64::new(0),
            total_operations: AtomicU64::new(0),
            applier: RwLock::new(None),
            pending_buffer: PendingBuffer::new(),
        }
    }

    /// Create a new UserDataStateMachine with an applier
    pub fn with_applier(shard: u32, applier: Arc<dyn UserDataApplier>) -> Self {
        let (last_applied_tx, _) = tokio::sync::watch::channel(0);
        Self {
            shard,
            last_applied_index: AtomicU64::new(0),
            last_applied_term: AtomicU64::new(0),
            last_applied_tx,
            approximate_size: AtomicU64::new(0),
            total_operations: AtomicU64::new(0),
            applier: RwLock::new(Some(applier)),
            pending_buffer: PendingBuffer::new(),
        }
    }

    /// Set the applier for persisting data to providers
    ///
    /// This is called after RaftManager creation once providers are available.
    pub fn set_applier(&self, applier: Arc<dyn UserDataApplier>) {
        let mut guard = self.applier.write();
        *guard = Some(applier);
        log::debug!(
            "UserDataStateMachine[{}]: Applier registered for data persistence",
            self.shard
        );
    }

    /// Get the number of pending commands
    pub fn pending_count(&self) -> usize {
        self.pending_buffer.len()
    }

    fn publish_last_applied(&self, index: u64) {
        self.last_applied_tx.send_replace(index);
    }

    fn watermark(&self) -> DataWatermark<'_> {
        DataWatermark {
            last_applied_index: &self.last_applied_index,
            last_applied_term: &self.last_applied_term,
            notify: &self.last_applied_tx,
            pending: &self.pending_buffer,
        }
    }

    /// Apply a user data command
    /// Note: user_id is extracted from inside each command variant
    async fn apply_command(
        &self,
        cmd: UserDataCommand,
        versions: &[kalamdb_commons::ids::VersionId],
    ) -> Result<DataResponse, RaftError> {
        // Get applier reference
        let applier = {
            let guard = self.applier.read();
            guard.clone()
        };

        match cmd {
            UserDataCommand::Insert {
                table_id,
                user_id,
                rows,
                encoded_fields,
                ..
            } => {
                let insert_count = if encoded_fields.is_empty() {
                    rows.len()
                } else {
                    encoded_fields.len()
                };
                log::debug!(
                    "UserDataStateMachine[{}]: Insert into {:?} ({} rows)",
                    self.shard,
                    table_id,
                    insert_count
                );

                // Persist data via applier if available
                let rows_affected = if let Some(ref a) = applier {
                    match a.insert(&table_id, &user_id, &rows, &encoded_fields, versions).await {
                        Ok(count) => count,
                        Err(e) => {
                            // Convert applier errors to DataResponse::Error
                            // This is required because RaftError cannot be serialized
                            log::warn!(
                                "UserDataStateMachine[{}]: Insert failed: {}",
                                self.shard,
                                e
                            );
                            return Ok(DataResponse::from_raft_error(e));
                        },
                    }
                } else {
                    log::warn!(
                        "UserDataStateMachine[{}]: No applier set, data not persisted!",
                        self.shard
                    );
                    0
                };

                self.total_operations.fetch_add(1, Ordering::Relaxed);
                self.approximate_size.fetch_add(insert_count as u64, Ordering::Relaxed);

                Ok(DataResponse::RowsAffected(rows_affected))
            },

            UserDataCommand::Update {
                table_id,
                user_id,
                updates,
                filter,
                ..
            } => {
                log::debug!("UserDataStateMachine[{}]: Update {:?}", self.shard, table_id);

                let rows_affected = if let Some(ref a) = applier {
                    match a.update(&table_id, &user_id, &updates, filter.as_deref(), versions).await
                    {
                        Ok(count) => count,
                        Err(e) => {
                            log::warn!(
                                "UserDataStateMachine[{}]: Update failed: {}",
                                self.shard,
                                e
                            );
                            return Ok(DataResponse::from_raft_error(e));
                        },
                    }
                } else {
                    log::warn!(
                        "UserDataStateMachine[{}]: No applier set, update not persisted!",
                        self.shard
                    );
                    0
                };

                self.total_operations.fetch_add(1, Ordering::Relaxed);
                Ok(DataResponse::RowsAffected(rows_affected))
            },

            UserDataCommand::Delete {
                table_id,
                user_id,
                pk_values,
                ..
            } => {
                log::debug!("UserDataStateMachine[{}]: Delete from {:?}", self.shard, table_id);

                let rows_affected = if let Some(ref a) = applier {
                    match a.delete(&table_id, &user_id, pk_values.as_deref(), versions).await {
                        Ok(count) => count,
                        Err(e) => {
                            log::warn!(
                                "UserDataStateMachine[{}]: Delete failed: {}",
                                self.shard,
                                e
                            );
                            return Ok(DataResponse::from_raft_error(e));
                        },
                    }
                } else {
                    log::warn!(
                        "UserDataStateMachine[{}]: No applier set, delete not persisted!",
                        self.shard
                    );
                    0
                };

                self.total_operations.fetch_add(1, Ordering::Relaxed);
                Ok(DataResponse::RowsAffected(rows_affected))
            },
        }
    }

    fn decode_apply_command(command: &[u8]) -> Result<UserApplyCommand, RaftError> {
        if let Ok(cmd) = crate::codec::command_codec::decode_user_data_command(command) {
            return Ok(UserApplyCommand::User(cmd));
        }

        match crate::codec::command_codec::decode_raft_command(command)? {
            RaftCommand::TransactionCommit {
                required_meta_index,
                transaction_id,
                mutations,
            } => Ok(UserApplyCommand::TransactionCommit {
                required_meta_index,
                transaction_id,
                mutations,
            }),
            other => Err(RaftError::Serialization(format!(
                "Unexpected raft command for user data state machine: {:?}",
                other
            ))),
        }
    }

    async fn apply_decoded_command(
        &self,
        cmd: UserApplyCommand,
        versions: &[kalamdb_commons::ids::VersionId],
    ) -> Result<DataResponse, RaftError> {
        match cmd {
            UserApplyCommand::User(command) => self.apply_command(command, versions).await,
            UserApplyCommand::TransactionCommit {
                required_meta_index: _,
                transaction_id,
                mutations,
            } => self.apply_transaction_commit(transaction_id, mutations, versions).await,
        }
    }

    async fn apply_transaction_commit(
        &self,
        transaction_id: TransactionId,
        mutations: Vec<StagedMutation>,
        versions: &[kalamdb_commons::ids::VersionId],
    ) -> Result<DataResponse, RaftError> {
        if mutations
            .iter()
            .any(|mutation| mutation.table_type != TableType::User || mutation.user_id.is_none())
        {
            return Ok(DataResponse::error(
                "TransactionCommit contained non-user mutation for user data shard",
            ));
        }

        let applier = {
            let guard = self.applier.read();
            guard.clone()
        };

        let Some(applier) = applier else {
            return Ok(DataResponse::error("No applier set, transaction commit not persisted"));
        };

        match applier.apply_transaction_batch(&transaction_id, &mutations, versions).await {
            Ok(result) => {
                self.total_operations.fetch_add(1, Ordering::Relaxed);
                Ok(DataResponse::TransactionCommitted(result))
            },
            Err(error) => Ok(DataResponse::from_raft_error(error)),
        }
    }
}

#[async_trait]
impl KalamStateMachine for UserDataStateMachine {
    fn group_id(&self) -> GroupId {
        GroupId::DataUserShard(self.shard)
    }

    async fn apply(&self, index: u64, term: u64, command: &[u8]) -> Result<ApplyResult, RaftError> {
        apply_data_entry(
            "UserDataStateMachine",
            self.shard,
            self.watermark(),
            index,
            term,
            command,
            Self::decode_apply_command,
            |cmd| cmd.required_meta_index(),
            |cmd| cmd.row_slot_count(),
            |cmd, versions| async move { self.apply_decoded_command(cmd, &versions).await },
        )
        .await
    }

    fn last_applied_index(&self) -> u64 {
        self.last_applied_index.load(Ordering::Acquire)
    }

    fn subscribe_last_applied(&self) -> Option<tokio::sync::watch::Receiver<u64>> {
        Some(self.last_applied_tx.subscribe())
    }

    fn mark_applied_index(&self, index: u64, term: u64) {
        self.watermark().mark(index, term);
    }

    fn rewind_applied_index(&self, index: u64, term: u64) {
        self.watermark().rewind(index, term);
    }

    fn last_applied_term(&self) -> u64 {
        self.last_applied_term.load(Ordering::Acquire)
    }

    async fn snapshot(&self) -> Result<StateMachineSnapshot, RaftError> {
        let pending_commands = self.pending_buffer.get_all();

        let snapshot = UserDataSnapshot {
            shard: self.shard,
            total_operations: self.total_operations.load(Ordering::Relaxed),
            pending_commands,
        };

        let data = encode(&snapshot)?;

        Ok(StateMachineSnapshot::new(
            self.group_id(),
            self.last_applied_index(),
            self.last_applied_term(),
            data,
        ))
    }

    async fn restore(&self, snapshot: StateMachineSnapshot) -> Result<(), RaftError> {
        let data: UserDataSnapshot = decode(&snapshot.data)?;

        if data.shard != self.shard {
            return Err(RaftError::InvalidState(format!(
                "Snapshot shard {} doesn't match state machine shard {}",
                data.shard, self.shard
            )));
        }

        // Restore pending buffer from snapshot
        self.pending_buffer.load_from(data.pending_commands);

        self.total_operations.store(data.total_operations, Ordering::Release);
        self.last_applied_index.store(snapshot.last_applied_index, Ordering::Release);
        self.last_applied_term.store(snapshot.last_applied_term, Ordering::Release);
        self.publish_last_applied(snapshot.last_applied_index);

        log::debug!(
            "UserDataStateMachine[{}]: Loaded in-memory user-shard tracker from snapshot \
             (applied={}, term={}, pending_meta_waiters={})",
            self.shard,
            snapshot.last_applied_index,
            snapshot.last_applied_term,
            self.pending_buffer.len()
        );

        Ok(())
    }

    fn approximate_size(&self) -> usize {
        self.approximate_size.load(Ordering::Relaxed) as usize
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use async_trait::async_trait;
    use kalamdb_commons::{
        models::{rows::Row, OperationKind, SchemaId},
        TableId, UserId,
    };

    use super::*;

    struct TransactionBatchUserApplier;

    #[async_trait]
    impl UserDataApplier for TransactionBatchUserApplier {
        async fn insert(
            &self,
            _table_id: &TableId,
            _user_id: &UserId,
            rows: &[Row],
            encoded_fields: &[Vec<u8>],
            _versions: &[kalamdb_commons::ids::VersionId],
        ) -> Result<usize, RaftError> {
            Ok(rows.len().max(encoded_fields.len()))
        }

        async fn update(
            &self,
            _table_id: &TableId,
            _user_id: &UserId,
            _updates: &[Row],
            _filter: Option<&str>,
            _versions: &[kalamdb_commons::ids::VersionId],
        ) -> Result<usize, RaftError> {
            Ok(1)
        }

        async fn delete(
            &self,
            _table_id: &TableId,
            _user_id: &UserId,
            _pk_values: Option<&[String]>,
            _versions: &[kalamdb_commons::ids::VersionId],
        ) -> Result<usize, RaftError> {
            Ok(1)
        }

        async fn apply_transaction_batch(
            &self,
            _transaction_id: &TransactionId,
            mutations: &[StagedMutation],
            versions: &[kalamdb_commons::ids::VersionId],
        ) -> Result<crate::TransactionApplyResult, RaftError> {
            Ok(crate::TransactionApplyResult {
                rows_affected: mutations.len(),
                log_index: versions.first().map(|v| v.as_u64() >> 16).unwrap_or(1),
                notifications_sent: 0,
                manifest_updates: 0,
                publisher_events: 0,
            })
        }
    }

    #[tokio::test]
    async fn test_user_data_state_machine_insert() {
        let sm = UserDataStateMachine::new(0);

        let cmd = UserDataCommand::Insert {
            table_id: TableId::new(SchemaId::default(), "users".into()),
            user_id: UserId::new("user123"),
            rows: vec![],
            encoded_fields: Vec::new(),
            required_meta_index: 0,
            transaction_id: None,
        };

        let payload = crate::codec::command_codec::encode_user_data_command(&cmd).unwrap();

        let result = sm.apply(1, 1, &payload).await.unwrap();
        assert!(result.is_ok());
        assert_eq!(sm.last_applied_index(), 1);
    }

    #[tokio::test]
    async fn test_user_data_state_machine_transaction_commit() {
        let sm = UserDataStateMachine::with_applier(0, Arc::new(TransactionBatchUserApplier));
        let transaction_id = TransactionId::new("01960f7b-3d15-7d6d-b26c-7e4db6f25f8d");
        let table_id = TableId::new(SchemaId::default(), "users".into());

        let cmd = RaftCommand::TransactionCommit {
            required_meta_index: 0,
            transaction_id: transaction_id.clone(),
            mutations: vec![StagedMutation::new(
                transaction_id,
                table_id,
                TableType::User,
                Some(UserId::new("user123")),
                OperationKind::Insert,
                "1",
                Row::new(BTreeMap::new()),
                false,
            )],
        };

        let payload = crate::codec::command_codec::encode_raft_command(&cmd).unwrap();
        let result = sm.apply(1, 1, &payload).await.unwrap();

        match result {
            ApplyResult::Ok(data) => {
                let response = crate::codec::command_codec::decode_data_response(&data).unwrap();
                match response {
                    DataResponse::TransactionCommitted(result) => {
                        assert_eq!(result.rows_affected, 1);
                        assert_eq!(result.log_index, 1);
                    },
                    other => panic!("unexpected response: {:?}", other),
                }
            },
            other => panic!("unexpected apply result: {:?}", other),
        }
    }
}
