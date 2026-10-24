//! DML Executor - Handles data manipulation (INSERT/UPDATE/DELETE)
//!
//! This module contains the logic for executing data operations against
//! specific table providers. It is designed to be stateless and thread-safe,
//! allowing concurrent execution from multiple Data Raft groups.
//!
//! ## Design
//!
//! - Stateless: No internal locks, safe for parallel execution across shards
//! - Unified: Same code path for standalone and cluster modes
//! - Provider-agnostic: Handles User, Stream, and Shared table types

use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

use kalamdb_commons::{
    ids::{SharedTableRowId, StreamTableRowId, UserTableRowId, VersionId},
    models::{rows::Row, OperationKind, TopicOp, TransactionId, UserId},
    schemas::TableType,
    websocket::{ChangeNotification, ChangeType},
    TableId,
};
use kalamdb_raft::{assign_entry_versions, TransactionApplyResult};
use kalamdb_sharding::{GroupId, ShardRouter};
use kalamdb_system::{FileRef, NotificationService as NotificationServiceTrait, TopicPublisher};
use kalamdb_tables::{utils::base as table_base, SharedTableRow, StreamTableRow, UserTableRow};
use kalamdb_transactions::StagedMutation;

use crate::{
    app_context::AppContext,
    applier::{
        error::ApplierError,
        executor::utils::fileref_util::{
            collect_file_refs_from_row, collect_replaced_file_refs_for_update,
            delete_file_refs_best_effort,
        },
    },
    providers::{
        base::{find_row_by_pk, BaseTableProvider},
        SharedTableProvider, StreamTableProvider, UserTableProvider,
    },
    transactions::CommitSideEffectPlan,
};

/// Executor for DML operations (Data Plane)
///
/// This executor is stateless and designed for concurrent access from
/// multiple Data Raft groups. Each method accesses the schema registry
/// to get the appropriate provider and performs the operation.
fn shared_update_targets(
    updates: &[Row],
    pk_values: Option<&[String]>,
    filter: Option<&str>,
    versions: &[VersionId],
) -> Result<Vec<(String, Row, VersionId)>, ApplierError> {
    if let Some(pk_values) = pk_values {
        if pk_values.len() != updates.len() || pk_values.len() != versions.len() {
            return Err(ApplierError::Validation(format!(
                "shared update version count {} does not match {} primary keys and {} update rows",
                versions.len(),
                pk_values.len(),
                updates.len()
            )));
        }
        return Ok(pk_values
            .iter()
            .zip(updates.iter().cloned())
            .zip(versions.iter().copied())
            .map(|((pk, row), version)| (pk.clone(), row, version))
            .collect());
    }

    if updates.len() != 1 || versions.len() != 1 {
        return Err(ApplierError::Validation(
            "shared update without pk_values writes one final row and requires one version slot"
                .into(),
        ));
    }
    let pk_value = filter.ok_or_else(|| {
        ApplierError::Validation("Update requires filter with PK value".to_string())
    })?;
    Ok(vec![(pk_value.to_string(), updates[0].clone(), versions[0])])
}

pub struct DmlExecutor {
    app_context: Arc<AppContext>,
}

impl DmlExecutor {
    /// Create a new DmlExecutor
    pub fn new(app_context: Arc<AppContext>) -> Self {
        Self { app_context }
    }

    fn dml_group_id(
        &self,
        table_id: &TableId,
        table_type: TableType,
        user_id: Option<&UserId>,
    ) -> Option<GroupId> {
        let router =
            ShardRouter::from_optional_cluster_config(self.app_context.config().cluster.as_ref());
        match table_type {
            TableType::User | TableType::Stream => user_id.map(|uid| router.user_group_id(uid)),
            TableType::Shared => self.app_context.shared_group_id(table_id).ok(),
            TableType::System => None,
        }
    }

    fn local_versions(&self, slot_count: usize) -> Result<(u64, Vec<VersionId>), ApplierError> {
        let log_index = self.app_context.commit_sequence_tracker().allocate_next();
        let versions = assign_entry_versions(log_index, slot_count)
            .map_err(|error| ApplierError::Validation(error.to_string()))?;
        Ok((log_index, versions))
    }

    fn log_index_from_versions(versions: &[VersionId]) -> Result<u64, ApplierError> {
        let version = versions
            .first()
            .ok_or_else(|| ApplierError::Validation("DML apply missing VersionId".to_string()))?;
        Ok(version.as_u64() >> 16)
    }

    fn defer_observe(
        &self,
        table_id: &TableId,
        table_type: TableType,
        user_id: Option<&UserId>,
        log_index: u64,
    ) {
        let tracker = self.app_context.commit_sequence_tracker();
        let group_id = self.dml_group_id(table_id, table_type, user_id);
        kalamdb_store::defer_after_persist(move || {
            if let Some(group_id) = group_id {
                tracker.observe_committed(group_id, log_index);
            }
        });
    }

    fn defer_observe_and_dispatch(
        &self,
        table_id: &TableId,
        table_type: TableType,
        user_id: Option<&UserId>,
        log_index: u64,
        side_effect_plan: crate::transactions::CommitSideEffectPlan,
    ) -> usize {
        let notifications_sent = side_effect_plan
            .notifications
            .iter()
            .map(|dispatch| dispatch.notifications.len())
            .sum();
        let tracker = self.app_context.commit_sequence_tracker();
        let group_id = self.dml_group_id(table_id, table_type, user_id);
        let notification_service = Arc::clone(self.app_context.notification_service());
        let topic_publisher = self.app_context.topic_publisher();
        kalamdb_store::defer_after_persist(move || {
            if let Some(group_id) = group_id {
                tracker.observe_committed(group_id, log_index);
            }
            for dispatch in &side_effect_plan.notifications {
                for notification in &dispatch.notifications {
                    let publish_user =
                        notification.actor_user_id.clone().or_else(|| {
                            match &dispatch.owner_scope {
                                crate::transactions::FanoutOwnerScope::Shared => None,
                                crate::transactions::FanoutOwnerScope::User(uid) => {
                                    Some(uid.clone())
                                },
                            }
                        });
                    if topic_publisher.has_topics_for_table(&notification.table_id) {
                        let op = Self::topic_op_for_change(&notification.change_type);
                        if let Err(error) = topic_publisher.publish_for_table(
                            &notification.table_id,
                            op,
                            &notification.row_data,
                            publish_user.as_ref(),
                        ) {
                            log::warn!(
                                "Topic publish failed for transaction change on table {}: {}",
                                notification.table_id,
                                error
                            );
                        }
                    }
                }
            }
            let _ = notification_service.dispatch_commit_plan(&side_effect_plan);
        });
        notifications_sent
    }

    /// Queue one committed change for post-persist topic publish and live fanout.
    ///
    /// Topic delivery does not require a live subscriber. The deferred hook
    /// publishes only after the Raft batch is durable.
    fn stage_commit_notification(
        &self,
        plan: &mut crate::transactions::CommitSideEffectPlan,
        owner_scope: crate::transactions::FanoutOwnerScope,
        notification: ChangeNotification,
    ) {
        let publish =
            self.app_context.topic_publisher().has_topics_for_table(&notification.table_id);
        if publish {
            plan.record_publisher_event();
        }
        let live = NotificationServiceTrait::has_subscribers(
            self.app_context.notification_service().as_ref(),
            owner_scope.user_id(),
            &notification.table_id,
        );
        if publish || live {
            plan.push_notification(owner_scope, notification);
        }
    }

    async fn load_provider(
        &self,
        table_id: &TableId,
        provider_label: &'static str,
    ) -> Result<Arc<dyn datafusion::datasource::TableProvider + Send + Sync>, ApplierError> {
        let schema_registry = self.app_context.schema_registry();

        if let Some(provider) = schema_registry.get_provider(table_id) {
            return Ok(provider);
        }

        schema_registry.get_table_if_exists_async(table_id).await.map_err(|e| {
            ApplierError::Execution(format!(
                "Failed to reload {} metadata for {}: {}",
                provider_label, table_id, e
            ))
        })?;

        schema_registry
            .get_provider(table_id)
            .ok_or_else(|| ApplierError::not_found(provider_label, table_id))
    }

    fn table_has_file_columns(&self, table_id: &TableId) -> bool {
        match self.app_context.schema_registry().get_table_if_exists(table_id) {
            Ok(Some(table_def)) => table_def.columns.iter().any(|column| {
                column.data_type() == kalamdb_commons::models::datatypes::KalamDataType::File
            }),
            Ok(None) => false,
            Err(error) => {
                log::warn!(
                    "Failed to load table definition for {} while checking file columns: {}",
                    table_id,
                    error
                );
                false
            },
        }
    }

    fn validate_file_columns_for_rows(
        &self,
        table_id: &TableId,
        table_type: TableType,
        user_id: Option<&UserId>,
        rows: &[Row],
    ) -> Result<(), ApplierError> {
        let Some(cached) = self.app_context.schema_registry().get(table_id) else {
            return Ok(());
        };
        validate_and_commit_file_columns_for_rows(
            &self.app_context,
            &cached,
            table_id,
            table_type,
            user_id,
            rows,
        )
        .map_err(ApplierError::Validation)
    }

    fn collect_file_refs_for_prior_row(
        &self,
        table_id: &TableId,
        prior_row: &Row,
        operation_kind: OperationKind,
        payload: Option<&Row>,
    ) -> Vec<FileRef> {
        match operation_kind {
            OperationKind::Update => {
                let Some(payload) = payload else {
                    log::warn!(
                        "Missing update payload while collecting file refs for {}",
                        table_id
                    );
                    return Vec::new();
                };
                collect_replaced_file_refs_for_update(
                    self.app_context.as_ref(),
                    table_id,
                    prior_row,
                    payload,
                )
            },
            OperationKind::Delete => {
                collect_file_refs_from_row(self.app_context.as_ref(), table_id, prior_row)
            },
            OperationKind::Insert => Vec::new(),
        }
    }

    async fn collect_user_file_refs_for_mutation(
        &self,
        provider: &UserTableProvider,
        table_id: &TableId,
        user_id: &UserId,
        pk_value: &str,
        operation_kind: OperationKind,
        payload: Option<&Row>,
    ) -> Vec<FileRef> {
        if !self.table_has_file_columns(table_id) {
            return Vec::new();
        }
        let Some(prior_row) =
            self.load_user_row_for_cleanup(provider, table_id, user_id, pk_value).await
        else {
            return Vec::new();
        };
        self.collect_file_refs_for_prior_row(table_id, &prior_row, operation_kind, payload)
    }

    async fn collect_shared_file_refs_for_mutation(
        &self,
        provider: &SharedTableProvider,
        table_id: &TableId,
        pk_value: &str,
        operation_kind: OperationKind,
        payload: Option<&Row>,
    ) -> Vec<FileRef> {
        if !self.table_has_file_columns(table_id) {
            return Vec::new();
        }
        let Some(prior_row) = self.load_shared_row_for_cleanup(provider, table_id, pk_value).await
        else {
            return Vec::new();
        };
        self.collect_file_refs_for_prior_row(table_id, &prior_row, operation_kind, payload)
    }

    fn emit_autocommit_notification(
        &self,
        user_id: Option<&UserId>,
        notification: ChangeNotification,
    ) {
        self.emit_autocommit_notification_with_scope(user_id, user_id, notification);
    }

    fn emit_shared_autocommit_notification(
        &self,
        actor_user_id: Option<&UserId>,
        notification: ChangeNotification,
    ) {
        self.emit_autocommit_notification_with_scope(None, actor_user_id, notification);
    }

    fn emit_autocommit_notification_with_scope(
        &self,
        subscriber_user_id: Option<&UserId>,
        actor_user_id: Option<&UserId>,
        notification: ChangeNotification,
    ) {
        let has_live_subscribers = NotificationServiceTrait::has_subscribers(
            self.app_context.notification_service().as_ref(),
            subscriber_user_id,
            &notification.table_id,
        );
        let scoped_user = subscriber_user_id.cloned();
        let table_id = notification.table_id.clone();
        let notification_service = Arc::clone(self.app_context.notification_service());
        let topic_publisher = self.app_context.topic_publisher();
        let publish_user = actor_user_id.cloned();
        kalamdb_store::defer_after_persist(move || {
            if topic_publisher.has_topics_for_table(&notification.table_id) {
                let op = Self::topic_op_for_change(&notification.change_type);
                if let Err(error) = topic_publisher.publish_for_table(
                    &notification.table_id,
                    op,
                    &notification.row_data,
                    publish_user.as_ref(),
                ) {
                    log::warn!(
                        "Topic publish failed for autocommit change on table {}: {}",
                        notification.table_id,
                        error
                    );
                }
            }
            if has_live_subscribers {
                notification_service.notify_table_change(scoped_user, table_id, notification);
            }
        });
    }

    // =========================================================================
    // User Table Operations (per-user sharded data)
    // =========================================================================

    /// Insert rows into a user table
    pub async fn insert_user_data(
        &self,
        table_id: &TableId,
        user_id: &UserId,
        rows: &[Row],
    ) -> Result<usize, ApplierError> {
        let (_log_index, versions) = self.local_versions(rows.len())?;
        self.insert_user_data_with_versions(table_id, user_id, rows.to_vec(), &versions)
            .await
    }

    pub async fn insert_user_data_with_versions(
        &self,
        table_id: &TableId,
        user_id: &UserId,
        rows: Vec<Row>,
        versions: &[VersionId],
    ) -> Result<usize, ApplierError> {
        if rows.is_empty() {
            return Ok(0);
        }
        let log_index = Self::log_index_from_versions(versions)?;
        self.validate_file_columns_for_rows(table_id, TableType::User, Some(user_id), &rows)?;

        let provider_arc = self.load_provider(table_id, "Table provider").await?;

        // Try UserTableProvider first, then StreamTableProvider
        if let Some(provider) =
            (provider_arc.as_ref() as &dyn std::any::Any).downcast_ref::<UserTableProvider>()
        {
            let row_ids = provider
                .insert_batch_with_versions(user_id, rows, versions)
                .await
                .map_err(|e| ApplierError::from_table(e, "Failed to insert batch"))?;
            self.defer_observe(table_id, TableType::User, Some(user_id), log_index);
            log::debug!("DmlExecutor: Inserted {} rows into {}", row_ids.len(), table_id);
            Ok(row_ids.len())
        } else if let Some(provider) =
            (provider_arc.as_ref() as &dyn std::any::Any).downcast_ref::<StreamTableProvider>()
        {
            let row_ids = provider.insert_batch(user_id, rows, versions).await.map_err(|e| {
                ApplierError::Execution(format!("Failed to insert stream batch: {}", e))
            })?;
            self.defer_observe(table_id, TableType::Stream, Some(user_id), log_index);
            log::debug!("DmlExecutor: Inserted {} stream rows into {}", row_ids.len(), table_id);
            Ok(row_ids.len())
        } else {
            Err(ApplierError::Execution(format!(
                "Provider type mismatch for user table {}",
                table_id
            )))
        }
    }

    /// Update rows in a user table
    pub async fn update_user_data(
        &self,
        table_id: &TableId,
        user_id: &UserId,
        updates: &[Row],
        filter: Option<&str>,
    ) -> Result<usize, ApplierError> {
        let (_log_index, versions) = self.local_versions(1)?;
        self.update_user_data_with_versions(table_id, user_id, updates, filter, &versions)
            .await
    }

    pub async fn update_user_data_with_versions(
        &self,
        table_id: &TableId,
        user_id: &UserId,
        updates: &[Row],
        filter: Option<&str>,
        versions: &[VersionId],
    ) -> Result<usize, ApplierError> {
        let pk_value = filter.ok_or_else(|| {
            ApplierError::Validation("Update requires filter with PK value".to_string())
        })?;

        let update_row = updates.first().ok_or_else(|| {
            ApplierError::Validation("Update requires at least one update row".to_string())
        })?;
        let log_index = Self::log_index_from_versions(versions)?;
        let version = *versions.first().expect("version");
        self.validate_file_columns_for_rows(table_id, TableType::User, Some(user_id), updates)?;

        let provider_arc = self.load_provider(table_id, "Table provider").await?;

        if let Some(provider) =
            (provider_arc.as_ref() as &dyn std::any::Any).downcast_ref::<UserTableProvider>()
        {
            let applied = provider
                .update_batch_by_pk_values_deferred(
                    user_id,
                    &[(pk_value.to_string(), update_row.clone(), version)],
                )
                .await
                .map_err(|e| ApplierError::Execution(format!("Failed to update row: {}", e)))?;

            if let Some(mutation) = applied.into_iter().next() {
                if let Some(notification) = mutation.notification {
                    self.emit_autocommit_notification(Some(user_id), notification);
                }
                let replaced_refs = self.collect_file_refs_for_prior_row(
                    table_id,
                    &mutation.prior,
                    OperationKind::Update,
                    mutation.payload.as_ref(),
                );
                delete_file_refs_best_effort(
                    self.app_context.as_ref(),
                    table_id,
                    TableType::User,
                    Some(user_id),
                    &replaced_refs,
                )
                .await;
                self.defer_observe(table_id, TableType::User, Some(user_id), log_index);
                Ok(1)
            } else {
                Ok(0)
            }
        } else if let Some(provider) =
            (provider_arc.as_ref() as &dyn std::any::Any).downcast_ref::<StreamTableProvider>()
        {
            self.update_stream_provider(provider, user_id, pk_value, update_row.clone(), version)
                .await
        } else {
            Err(ApplierError::Execution(format!(
                "Provider type mismatch for user table {}",
                table_id
            )))
        }
    }

    /// Delete rows from a user table
    pub async fn delete_user_data(
        &self,
        table_id: &TableId,
        user_id: &UserId,
        pk_values: Option<&[String]>,
    ) -> Result<usize, ApplierError> {
        let slot_count = pk_values.map(|values| values.len()).unwrap_or(0);
        let (_log_index, versions) = self.local_versions(slot_count.max(1))?;
        self.delete_user_data_with_versions(table_id, user_id, pk_values, &versions)
            .await
    }

    pub async fn delete_user_data_with_versions(
        &self,
        table_id: &TableId,
        user_id: &UserId,
        pk_values: Option<&[String]>,
        versions: &[VersionId],
    ) -> Result<usize, ApplierError> {
        let pk_values = pk_values.ok_or_else(|| {
            ApplierError::Validation("Delete requires pk_values list".to_string())
        })?;

        if pk_values.is_empty() {
            return Ok(0);
        }
        let log_index = Self::log_index_from_versions(versions)?;

        let provider_arc = self.load_provider(table_id, "Table provider").await?;

        if let Some(provider) =
            (provider_arc.as_ref() as &dyn std::any::Any).downcast_ref::<UserTableProvider>()
        {
            let applied = provider
                .delete_batch_by_pk_values_deferred(user_id, pk_values, versions)
                .await
                .map_err(|e| ApplierError::Execution(format!("Failed to delete row: {}", e)))?;
            let deleted_count = applied.len();
            for mutation in applied {
                if let Some(notification) = mutation.notification {
                    self.emit_autocommit_notification(Some(user_id), notification);
                }
                let file_refs = self.collect_file_refs_for_prior_row(
                    table_id,
                    &mutation.prior,
                    OperationKind::Delete,
                    None,
                );
                delete_file_refs_best_effort(
                    self.app_context.as_ref(),
                    table_id,
                    TableType::User,
                    Some(user_id),
                    &file_refs,
                )
                .await;
            }
            log::debug!("DmlExecutor: Deleted {} rows from {}", deleted_count, table_id);
            if deleted_count > 0 {
                self.defer_observe(table_id, TableType::User, Some(user_id), log_index);
            }
            Ok(deleted_count)
        } else if let Some(provider) =
            (provider_arc.as_ref() as &dyn std::any::Any).downcast_ref::<StreamTableProvider>()
        {
            let mut deleted_count = 0;
            for (pk_value, version) in pk_values.iter().zip(versions.iter().copied()) {
                if provider
                    .delete_by_id_field(user_id, pk_value, version)
                    .await
                    .map_err(|e| ApplierError::Execution(format!("Failed to delete row: {}", e)))?
                {
                    deleted_count += 1;
                }
            }
            log::debug!("DmlExecutor: Deleted {} stream rows from {}", deleted_count, table_id);
            Ok(deleted_count)
        } else {
            Err(ApplierError::Execution(format!(
                "Provider type mismatch for user table {}",
                table_id
            )))
        }
    }

    // =========================================================================
    // Shared Table Operations (global data, not per-user)
    // =========================================================================

    /// Insert rows into a shared table
    pub async fn insert_shared_data(
        &self,
        table_id: &TableId,
        actor_user_id: Option<&UserId>,
        rows: &[Row],
    ) -> Result<usize, ApplierError> {
        let (_log_index, versions) = self.local_versions(rows.len())?;
        self.insert_shared_data_with_versions(table_id, actor_user_id, rows.to_vec(), &versions)
            .await
    }

    pub async fn insert_shared_data_with_versions(
        &self,
        table_id: &TableId,
        actor_user_id: Option<&UserId>,
        rows: Vec<Row>,
        versions: &[VersionId],
    ) -> Result<usize, ApplierError> {
        if rows.is_empty() {
            return Ok(0);
        }
        let log_index = Self::log_index_from_versions(versions)?;
        self.validate_file_columns_for_rows(table_id, TableType::Shared, None, &rows)?;

        let provider_arc = self.load_provider(table_id, "Shared table provider").await?;

        if let Some(provider) =
            (provider_arc.as_ref() as &dyn std::any::Any).downcast_ref::<SharedTableProvider>()
        {
            let row_ids = provider
                .insert_batch_with_versions(actor_user_id, rows, versions)
                .await
                .map_err(|e| ApplierError::from_table(e, "Failed to insert batch"))?;
            self.defer_observe(table_id, TableType::Shared, actor_user_id, log_index);
            log::debug!("DmlExecutor: Inserted {} shared rows into {}", row_ids.len(), table_id);
            Ok(row_ids.len())
        } else {
            Err(ApplierError::Execution(format!(
                "Provider type mismatch for shared table {}",
                table_id
            )))
        }
    }

    /// Update rows in a shared table
    pub async fn update_shared_data(
        &self,
        table_id: &TableId,
        actor_user_id: Option<&UserId>,
        updates: &[Row],
        filter: Option<&str>,
    ) -> Result<usize, ApplierError> {
        let (_log_index, versions) = self.local_versions(1)?;
        self.update_shared_data_with_versions(
            table_id,
            actor_user_id,
            updates,
            None,
            filter,
            &versions,
        )
        .await
    }

    pub async fn update_shared_data_with_versions(
        &self,
        table_id: &TableId,
        actor_user_id: Option<&UserId>,
        updates: &[Row],
        pk_values: Option<&[String]>,
        filter: Option<&str>,
        versions: &[VersionId],
    ) -> Result<usize, ApplierError> {
        if updates.is_empty() {
            return Ok(0);
        }
        let targets = shared_update_targets(updates, pk_values, filter, versions)?;
        let log_index = Self::log_index_from_versions(versions)?;
        self.validate_file_columns_for_rows(table_id, TableType::Shared, None, updates)?;

        let provider_arc = self.load_provider(table_id, "Shared table provider").await?;

        if let Some(provider) =
            (provider_arc.as_ref() as &dyn std::any::Any).downcast_ref::<SharedTableProvider>()
        {
            let applied = provider
                .update_batch_by_pk_values_deferred(&targets)
                .await
                .map_err(|e| ApplierError::Execution(format!("Failed to update row: {}", e)))?;
            let affected_rows = applied.len();
            for mutation in applied {
                if let Some(notification) = mutation.notification {
                    self.emit_shared_autocommit_notification(actor_user_id, notification);
                }
                let replaced_refs = self.collect_file_refs_for_prior_row(
                    table_id,
                    &mutation.prior,
                    OperationKind::Update,
                    mutation.payload.as_ref(),
                );
                delete_file_refs_best_effort(
                    self.app_context.as_ref(),
                    table_id,
                    TableType::Shared,
                    None,
                    &replaced_refs,
                )
                .await;
            }

            log::debug!("DmlExecutor: Updated {} shared row(s) in {}", affected_rows, table_id);
            if affected_rows > 0 {
                self.defer_observe(table_id, TableType::Shared, actor_user_id, log_index);
            }
            Ok(affected_rows)
        } else {
            Err(ApplierError::Execution(format!(
                "Provider type mismatch for shared table {}",
                table_id
            )))
        }
    }

    /// Delete rows from a shared table
    pub async fn delete_shared_data(
        &self,
        table_id: &TableId,
        actor_user_id: Option<&UserId>,
        pk_values: Option<&[String]>,
    ) -> Result<usize, ApplierError> {
        let slot_count = pk_values.map(|values| values.len()).unwrap_or(0);
        let (_log_index, versions) = self.local_versions(slot_count.max(1))?;
        self.delete_shared_data_with_versions(table_id, actor_user_id, pk_values, &versions)
            .await
    }

    pub async fn delete_shared_data_with_versions(
        &self,
        table_id: &TableId,
        actor_user_id: Option<&UserId>,
        pk_values: Option<&[String]>,
        versions: &[VersionId],
    ) -> Result<usize, ApplierError> {
        let pk_values = pk_values.ok_or_else(|| {
            ApplierError::Validation("Delete requires pk_values list".to_string())
        })?;

        if pk_values.is_empty() {
            return Ok(0);
        }
        if versions.len() != pk_values.len() {
            return Err(ApplierError::Validation(format!(
                "delete version count {} does not match primary key count {}",
                versions.len(),
                pk_values.len()
            )));
        }
        let log_index = Self::log_index_from_versions(versions)?;

        let provider_arc = self.load_provider(table_id, "Shared table provider").await?;

        if let Some(provider) =
            (provider_arc.as_ref() as &dyn std::any::Any).downcast_ref::<SharedTableProvider>()
        {
            let applied = provider
                .delete_batch_by_pk_values_deferred(pk_values, versions)
                .await
                .map_err(|e| ApplierError::Execution(format!("Failed to delete row: {}", e)))?;
            let deleted_count = applied.len();
            for mutation in applied {
                if let Some(notification) = mutation.notification {
                    self.emit_shared_autocommit_notification(actor_user_id, notification);
                }
                let file_refs = self.collect_file_refs_for_prior_row(
                    table_id,
                    &mutation.prior,
                    OperationKind::Delete,
                    None,
                );
                delete_file_refs_best_effort(
                    self.app_context.as_ref(),
                    table_id,
                    TableType::Shared,
                    None,
                    &file_refs,
                )
                .await;
            }

            log::debug!("DmlExecutor: Deleted {} shared rows from {}", deleted_count, table_id);
            if deleted_count > 0 {
                self.defer_observe(table_id, TableType::Shared, actor_user_id, log_index);
            }
            Ok(deleted_count)
        } else {
            Err(ApplierError::Execution(format!(
                "Provider type mismatch for shared table {}",
                table_id
            )))
        }
    }

    async fn prevalidate_user_transaction_batch(
        &self,
        transaction_id: &TransactionId,
        mutations: &[StagedMutation],
    ) -> Result<(), ApplierError> {
        if let Some(first_mutation) = mutations.first() {
            if let Some(user_id) = first_mutation.user_id.as_ref() {
                let all_inserts_same_table_and_user = mutations.iter().all(|mutation| {
                    &mutation.transaction_id == transaction_id
                        && mutation.operation_kind == OperationKind::Insert
                        && mutation.table_id == first_mutation.table_id
                        && mutation.user_id.as_ref() == Some(user_id)
                });

                if all_inserts_same_table_and_user {
                    let provider_arc =
                        self.load_provider(&first_mutation.table_id, "Table provider").await?;
                    let provider = (provider_arc.as_ref() as &dyn std::any::Any)
                        .downcast_ref::<UserTableProvider>()
                        .ok_or_else(|| {
                            ApplierError::Execution(format!(
                                "Provider type mismatch for user table {}",
                                first_mutation.table_id
                            ))
                        })?;

                    let batch_rows: Vec<&Row> =
                        mutations.iter().map(|mutation| &mutation.payload).collect();

                    provider.validate_insert_batch_rows(user_id, batch_rows).await.map_err(
                        |error| ApplierError::from_table(error, "Failed to insert batch row"),
                    )?;

                    return Ok(());
                }
            }
        }

        let mut seen_insert_keys = HashSet::with_capacity(mutations.len());
        let mut cached_provider: Option<(
            TableId,
            Arc<dyn datafusion::datasource::TableProvider + Send + Sync>,
        )> = None;

        for mutation in mutations {
            if mutation.operation_kind != OperationKind::Insert {
                continue;
            }

            if &mutation.transaction_id != transaction_id {
                return Err(ApplierError::Validation(format!(
                    "staged mutation transaction mismatch: expected '{}', got '{}'",
                    transaction_id, mutation.transaction_id
                )));
            }

            let user_id = mutation.user_id.clone().ok_or_else(|| {
                ApplierError::Validation(
                    "user transaction batch mutation missing user_id".to_string(),
                )
            })?;

            let provider_arc = if cached_provider
                .as_ref()
                .map(|(table_id, _)| table_id == &mutation.table_id)
                .unwrap_or(false)
            {
                Arc::clone(&cached_provider.as_ref().expect("cached provider").1)
            } else {
                let provider_arc = self.load_provider(&mutation.table_id, "Table provider").await?;
                cached_provider = Some((mutation.table_id.clone(), Arc::clone(&provider_arc)));
                provider_arc
            };
            let provider = (provider_arc.as_ref() as &dyn std::any::Any)
                .downcast_ref::<UserTableProvider>()
                .ok_or_else(|| {
                    ApplierError::Execution(format!(
                        "Provider type mismatch for user table {}",
                        mutation.table_id
                    ))
                })?;

            let batch_key =
                format!("{}|{}|{}", mutation.table_id, user_id.as_str(), mutation.primary_key);
            if !seen_insert_keys.insert(batch_key) {
                return Err(ApplierError::Execution(format!(
                    "Failed to insert batch row: Already exists: Primary key violation: value \
                     '{}' appears multiple times in the transaction batch for column '{}'",
                    mutation.primary_key,
                    provider.primary_key_field_name()
                )));
            }

            table_base::ensure_unique_pk_value(provider, Some(&user_id), &mutation.payload)
                .await
                .map_err(|error| ApplierError::from_table(error, "Failed to insert batch row"))?;
        }

        Ok(())
    }

    async fn prevalidate_shared_transaction_batch(
        &self,
        transaction_id: &TransactionId,
        mutations: &[StagedMutation],
    ) -> Result<(), ApplierError> {
        let mut seen_insert_keys = HashSet::with_capacity(mutations.len());
        let mut cached_provider: Option<(
            TableId,
            Arc<dyn datafusion::datasource::TableProvider + Send + Sync>,
        )> = None;

        for mutation in mutations {
            if mutation.operation_kind != OperationKind::Insert {
                continue;
            }

            if &mutation.transaction_id != transaction_id {
                return Err(ApplierError::Validation(format!(
                    "staged mutation transaction mismatch: expected '{}', got '{}'",
                    transaction_id, mutation.transaction_id
                )));
            }

            let provider_arc = if cached_provider
                .as_ref()
                .map(|(table_id, _)| table_id == &mutation.table_id)
                .unwrap_or(false)
            {
                Arc::clone(&cached_provider.as_ref().expect("cached provider").1)
            } else {
                let provider_arc =
                    self.load_provider(&mutation.table_id, "Shared table provider").await?;
                cached_provider = Some((mutation.table_id.clone(), Arc::clone(&provider_arc)));
                provider_arc
            };
            let provider = (provider_arc.as_ref() as &dyn std::any::Any)
                .downcast_ref::<SharedTableProvider>()
                .ok_or_else(|| {
                    ApplierError::Execution(format!(
                        "Provider type mismatch for shared table {}",
                        mutation.table_id
                    ))
                })?;

            let batch_key = format!("{}|{}", mutation.table_id, mutation.primary_key);
            if !seen_insert_keys.insert(batch_key) {
                return Err(ApplierError::Execution(format!(
                    "Failed to insert batch row: Already exists: Primary key violation: value \
                     '{}' appears multiple times in the transaction batch for column '{}'",
                    mutation.primary_key,
                    provider.primary_key_field_name()
                )));
            }

            table_base::ensure_unique_pk_value(provider, None, &mutation.payload)
                .await
                .map_err(|error| ApplierError::from_table(error, "Failed to insert batch row"))?;
        }

        Ok(())
    }

    pub async fn apply_user_transaction_batch_with_versions(
        &self,
        transaction_id: &TransactionId,
        mutations: &[StagedMutation],
        versions: &[VersionId],
    ) -> Result<TransactionApplyResult, ApplierError> {
        let log_index = Self::log_index_from_versions(versions)?;
        for mutation in mutations {
            if matches!(mutation.operation_kind, OperationKind::Insert | OperationKind::Update) {
                self.validate_file_columns_for_rows(
                    &mutation.table_id,
                    TableType::User,
                    mutation.user_id.as_ref(),
                    std::slice::from_ref(&mutation.payload),
                )?;
            }
        }
        self.prevalidate_user_transaction_batch(transaction_id, mutations).await?;

        let mut affected_rows = 0;
        let mut side_effect_plan = CommitSideEffectPlan::new(transaction_id.clone());

        if let Some(first_mutation) = mutations.first() {
            if let Some(user_id) = first_mutation.user_id.clone() {
                let all_inserts_same_table_and_user = mutations.iter().all(|mutation| {
                    &mutation.transaction_id == transaction_id
                        && mutation.operation_kind == OperationKind::Insert
                        && mutation.table_id == first_mutation.table_id
                        && mutation.user_id.as_ref() == Some(&user_id)
                });

                if all_inserts_same_table_and_user {
                    let provider_arc =
                        self.load_provider(&first_mutation.table_id, "Table provider").await?;
                    let provider = (provider_arc.as_ref() as &dyn std::any::Any)
                        .downcast_ref::<UserTableProvider>()
                        .ok_or_else(|| {
                            ApplierError::Execution(format!(
                                "Provider type mismatch for user table {}",
                                first_mutation.table_id
                            ))
                        })?;

                    let applied = provider
                        .insert_batch_deferred_prevalidated_with_versions(
                            &user_id,
                            mutations.iter().map(|mutation| mutation.payload.clone()).collect(),
                            versions,
                        )
                        .await
                        .map_err(|e| ApplierError::from_table(e, "Failed to insert batch rows"))?;

                    if applied.len() != mutations.len() {
                        return Err(ApplierError::Execution(format!(
                            "User transaction batch insert length mismatch: expected {}, got {}",
                            mutations.len(),
                            applied.len()
                        )));
                    }

                    for (_row_key, notification) in applied {
                        affected_rows += 1;
                        side_effect_plan.record_manifest_update();

                        if let Some(notification) = notification {
                            self.stage_commit_notification(
                                &mut side_effect_plan,
                                crate::transactions::FanoutOwnerScope::User(user_id.clone()),
                                notification,
                            );
                        }
                    }

                    let notifications_sent = self.defer_observe_and_dispatch(
                        &mutations[0].table_id,
                        TableType::User,
                        Some(&user_id),
                        log_index,
                        side_effect_plan.clone(),
                    );

                    return Ok(TransactionApplyResult {
                        rows_affected: affected_rows,
                        log_index,
                        notifications_sent,
                        manifest_updates: side_effect_plan.manifest_updates,
                        publisher_events: side_effect_plan.publisher_events,
                    });
                }
            }
        }

        let mut cached_provider: Option<(
            TableId,
            Arc<dyn datafusion::datasource::TableProvider + Send + Sync>,
        )> = None;
        // Writes in this commit are buffered until the Raft entry finishes, so a
        // later statement cannot read them back from RocksDB.
        let mut written: HashMap<(TableId, UserId, String), (UserTableRowId, UserTableRow)> =
            HashMap::with_capacity(mutations.len());

        for (mutation, version) in mutations.iter().zip(versions.iter().copied()) {
            if &mutation.transaction_id != transaction_id {
                return Err(ApplierError::Validation(format!(
                    "staged mutation transaction mismatch: expected '{}', got '{}'",
                    transaction_id, mutation.transaction_id
                )));
            }

            let user_id = mutation.user_id.clone().ok_or_else(|| {
                ApplierError::Validation(
                    "user transaction batch mutation missing user_id".to_string(),
                )
            })?;

            let provider_arc = if cached_provider
                .as_ref()
                .map(|(table_id, _)| table_id == &mutation.table_id)
                .unwrap_or(false)
            {
                Arc::clone(&cached_provider.as_ref().expect("cached provider").1)
            } else {
                let provider_arc = self.load_provider(&mutation.table_id, "Table provider").await?;
                cached_provider = Some((mutation.table_id.clone(), Arc::clone(&provider_arc)));
                provider_arc
            };
            let provider = (provider_arc.as_ref() as &dyn std::any::Any)
                .downcast_ref::<UserTableProvider>()
                .ok_or_else(|| {
                    ApplierError::Execution(format!(
                        "Provider type mismatch for user table {}",
                        mutation.table_id
                    ))
                })?;

            let written_key =
                (mutation.table_id.clone(), user_id.clone(), mutation.primary_key.clone());
            let known = written.get(&written_key).cloned();
            let file_refs_to_delete = if let Some((_, row)) = known.as_ref().filter(|_| {
                matches!(mutation.operation_kind, OperationKind::Update | OperationKind::Delete)
            }) {
                if self.table_has_file_columns(&mutation.table_id) {
                    self.collect_file_refs_for_prior_row(
                        &mutation.table_id,
                        &row.fields,
                        mutation.operation_kind,
                        Some(&mutation.payload),
                    )
                } else {
                    Vec::new()
                }
            } else {
                self.collect_user_file_refs_for_mutation(
                    provider,
                    &mutation.table_id,
                    &user_id,
                    &mutation.primary_key,
                    mutation.operation_kind,
                    Some(&mutation.payload),
                )
                .await
            };

            let applied = match mutation.operation_kind {
                OperationKind::Insert => {
                    if known.as_ref().is_some_and(|(_, row)| !row._deleted) {
                        return Err(ApplierError::Execution(format!(
                            "Failed to insert batch row: Already exists: Primary key violation: \
                             value '{}' appears multiple times in the transaction batch for \
                             column '{}'",
                            mutation.primary_key,
                            provider.primary_key_field_name()
                        )));
                    }
                    let (row_key, notification) = if let Some(previous) = known {
                        provider
                            .insert_over_hot_deferred(
                                &user_id,
                                mutation.payload.clone(),
                                version,
                                previous,
                            )
                            .await
                    } else {
                        provider
                            .insert_deferred_prevalidated(
                                &user_id,
                                mutation.payload.clone(),
                                version,
                            )
                            .await
                    }
                    .map_err(|e| ApplierError::from_table(e, "Failed to insert batch row"))?;
                    written.insert(
                        written_key,
                        (
                            row_key.clone(),
                            UserTableRow {
                                user_id:  user_id.clone(),
                                _version: version,
                                _deleted: false,
                                fields:   mutation.payload.clone(),
                            },
                        ),
                    );
                    Some((row_key, notification))
                },
                OperationKind::Update => {
                    let updated = provider
                        .update_by_pk_value_deferred_with_hot(
                            &user_id,
                            mutation.primary_key.as_str(),
                            mutation.payload.clone(),
                            version,
                            known,
                        )
                        .await
                        .map_err(|e| {
                            ApplierError::Execution(format!("Failed to update row: {}", e))
                        })?;
                    updated.map(|(row_key, row, notification)| {
                        written.insert(written_key, (row_key.clone(), row));
                        (row_key, notification)
                    })
                },
                OperationKind::Delete => {
                    let deleted = provider
                        .delete_by_pk_value_deferred_with_hot(
                            &user_id,
                            mutation.primary_key.as_str(),
                            version,
                            known,
                        )
                        .await
                        .map_err(|e| {
                            ApplierError::Execution(format!("Failed to delete row: {}", e))
                        })?;
                    deleted.map(|(row_key, row, notification)| {
                        written.insert(written_key, (row_key.clone(), row));
                        (row_key, notification)
                    })
                },
            };

            let Some((_row_key, notification)) = applied else {
                continue;
            };

            delete_file_refs_best_effort(
                self.app_context.as_ref(),
                &mutation.table_id,
                TableType::User,
                Some(&user_id),
                &file_refs_to_delete,
            )
            .await;

            affected_rows += 1;
            side_effect_plan.record_manifest_update();

            if let Some(notification) = notification {
                self.stage_commit_notification(
                    &mut side_effect_plan,
                    crate::transactions::FanoutOwnerScope::User(user_id.clone()),
                    notification,
                );
            }
        }

        let notifications_sent = self.defer_observe_and_dispatch(
            &mutations[0].table_id,
            TableType::User,
            mutations.first().and_then(|m| m.user_id.as_ref()),
            log_index,
            side_effect_plan.clone(),
        );

        Ok(TransactionApplyResult {
            rows_affected: affected_rows,
            log_index,
            notifications_sent,
            manifest_updates: side_effect_plan.manifest_updates,
            publisher_events: side_effect_plan.publisher_events,
        })
    }

    pub async fn apply_shared_transaction_batch_with_versions(
        &self,
        transaction_id: &TransactionId,
        mutations: &[StagedMutation],
        versions: &[VersionId],
    ) -> Result<TransactionApplyResult, ApplierError> {
        let log_index = Self::log_index_from_versions(versions)?;
        for mutation in mutations {
            if matches!(mutation.operation_kind, OperationKind::Insert | OperationKind::Update) {
                self.validate_file_columns_for_rows(
                    &mutation.table_id,
                    TableType::Shared,
                    None,
                    std::slice::from_ref(&mutation.payload),
                )?;
            }
        }
        self.prevalidate_shared_transaction_batch(transaction_id, mutations).await?;

        let mut affected_rows = 0;
        let mut side_effect_plan = CommitSideEffectPlan::new(transaction_id.clone());

        if let Some(first_mutation) = mutations.first() {
            let all_inserts_same_table = mutations.iter().all(|mutation| {
                &mutation.transaction_id == transaction_id
                    && mutation.operation_kind == OperationKind::Insert
                    && mutation.table_id == first_mutation.table_id
            });

            if all_inserts_same_table {
                let provider_arc =
                    self.load_provider(&first_mutation.table_id, "Shared table provider").await?;
                let provider = (provider_arc.as_ref() as &dyn std::any::Any)
                    .downcast_ref::<SharedTableProvider>()
                    .ok_or_else(|| {
                        ApplierError::Execution(format!(
                            "Provider type mismatch for shared table {}",
                            first_mutation.table_id
                        ))
                    })?;

                let applied = provider
                    .insert_batch_deferred_prevalidated_with_versions(
                        mutations.iter().map(|mutation| mutation.payload.clone()).collect(),
                        versions,
                    )
                    .await
                    .map_err(|e| ApplierError::from_table(e, "Failed to insert batch rows"))?;

                if applied.len() != mutations.len() {
                    return Err(ApplierError::Execution(format!(
                        "Shared transaction batch insert length mismatch: expected {}, got {}",
                        mutations.len(),
                        applied.len()
                    )));
                }

                for (mutation, (_row_key, notification)) in mutations.iter().zip(applied) {
                    affected_rows += 1;
                    side_effect_plan.record_manifest_update();

                    if let Some(notification) = notification {
                        self.stage_commit_notification(
                            &mut side_effect_plan,
                            crate::transactions::FanoutOwnerScope::Shared,
                            notification.with_actor(mutation.user_id.clone()),
                        );
                    }
                }

                let notifications_sent = self.defer_observe_and_dispatch(
                    &mutations[0].table_id,
                    TableType::Shared,
                    first_mutation.user_id.as_ref(),
                    log_index,
                    side_effect_plan.clone(),
                );

                return Ok(TransactionApplyResult {
                    rows_affected: affected_rows,
                    log_index,
                    notifications_sent,
                    manifest_updates: side_effect_plan.manifest_updates,
                    publisher_events: side_effect_plan.publisher_events,
                });
            }
        }

        let mut cached_provider: Option<(
            TableId,
            Arc<dyn datafusion::datasource::TableProvider + Send + Sync>,
        )> = None;
        // Writes in this commit are buffered until the Raft entry finishes, so a
        // later statement cannot read them back from RocksDB.
        let mut written: HashMap<(TableId, String), (SharedTableRowId, SharedTableRow)> =
            HashMap::with_capacity(mutations.len());

        for (mutation, version) in mutations.iter().zip(versions.iter().copied()) {
            if &mutation.transaction_id != transaction_id {
                return Err(ApplierError::Validation(format!(
                    "staged mutation transaction mismatch: expected '{}', got '{}'",
                    transaction_id, mutation.transaction_id
                )));
            }

            let provider_arc = if cached_provider
                .as_ref()
                .map(|(table_id, _)| table_id == &mutation.table_id)
                .unwrap_or(false)
            {
                Arc::clone(&cached_provider.as_ref().expect("cached provider").1)
            } else {
                let provider_arc =
                    self.load_provider(&mutation.table_id, "Shared table provider").await?;
                cached_provider = Some((mutation.table_id.clone(), Arc::clone(&provider_arc)));
                provider_arc
            };
            let provider = (provider_arc.as_ref() as &dyn std::any::Any)
                .downcast_ref::<SharedTableProvider>()
                .ok_or_else(|| {
                    ApplierError::Execution(format!(
                        "Provider type mismatch for shared table {}",
                        mutation.table_id
                    ))
                })?;

            let written_key = (mutation.table_id.clone(), mutation.primary_key.clone());
            let known = written.get(&written_key).cloned();
            let file_refs_to_delete = if let Some((_, row)) = known.as_ref().filter(|_| {
                matches!(mutation.operation_kind, OperationKind::Update | OperationKind::Delete)
            }) {
                if self.table_has_file_columns(&mutation.table_id) {
                    self.collect_file_refs_for_prior_row(
                        &mutation.table_id,
                        &row.fields,
                        mutation.operation_kind,
                        Some(&mutation.payload),
                    )
                } else {
                    Vec::new()
                }
            } else {
                self.collect_shared_file_refs_for_mutation(
                    provider,
                    &mutation.table_id,
                    &mutation.primary_key,
                    mutation.operation_kind,
                    Some(&mutation.payload),
                )
                .await
            };

            let applied = match mutation.operation_kind {
                OperationKind::Insert => {
                    if known.as_ref().is_some_and(|(_, row)| !row._deleted) {
                        return Err(ApplierError::Execution(format!(
                            "Failed to insert batch row: Already exists: Primary key violation: \
                             value '{}' appears multiple times in the transaction batch for \
                             column '{}'",
                            mutation.primary_key,
                            provider.primary_key_field_name()
                        )));
                    }
                    let (row_key, notification) = if let Some(previous) = known {
                        provider
                            .insert_over_hot_deferred(mutation.payload.clone(), version, previous)
                            .await
                    } else {
                        provider
                            .insert_deferred_prevalidated(mutation.payload.clone(), version)
                            .await
                    }
                    .map_err(|e| ApplierError::from_table(e, "Failed to insert batch row"))?;
                    written.insert(
                        written_key,
                        (
                            row_key,
                            SharedTableRow {
                                _version: version,
                                _deleted: false,
                                fields:   mutation.payload.clone(),
                            },
                        ),
                    );
                    Some((row_key, notification))
                },
                OperationKind::Update => {
                    let updated = provider
                        .update_by_pk_value_deferred_with_hot(
                            mutation.primary_key.as_str(),
                            mutation.payload.clone(),
                            version,
                            known,
                        )
                        .await
                        .map_err(|e| {
                            ApplierError::Execution(format!("Failed to update row: {}", e))
                        })?;
                    updated.map(|(row_key, row, notification)| {
                        written.insert(written_key, (row_key, row));
                        (row_key, notification)
                    })
                },
                OperationKind::Delete => {
                    let deleted = provider
                        .delete_by_pk_value_deferred_with_hot(
                            mutation.primary_key.as_str(),
                            version,
                            known,
                        )
                        .await
                        .map_err(|e| {
                            ApplierError::Execution(format!("Failed to delete row: {}", e))
                        })?;
                    deleted.map(|(row_key, row, notification)| {
                        written.insert(written_key, (row_key, row));
                        (row_key, notification)
                    })
                },
            };

            let Some((_row_key, notification)) = applied else {
                continue;
            };

            delete_file_refs_best_effort(
                self.app_context.as_ref(),
                &mutation.table_id,
                TableType::Shared,
                None,
                &file_refs_to_delete,
            )
            .await;

            affected_rows += 1;
            side_effect_plan.record_manifest_update();

            if let Some(notification) = notification {
                self.stage_commit_notification(
                    &mut side_effect_plan,
                    crate::transactions::FanoutOwnerScope::Shared,
                    notification.with_actor(mutation.user_id.clone()),
                );
            }
        }

        let notifications_sent = self.defer_observe_and_dispatch(
            &mutations[0].table_id,
            TableType::Shared,
            mutations.first().and_then(|m| m.user_id.as_ref()),
            log_index,
            side_effect_plan.clone(),
        );

        Ok(TransactionApplyResult {
            rows_affected: affected_rows,
            log_index,
            notifications_sent,
            manifest_updates: side_effect_plan.manifest_updates,
            publisher_events: side_effect_plan.publisher_events,
        })
    }

    // =========================================================================
    // Helper Methods
    // =========================================================================

    #[inline]
    fn topic_op_for_change(change_type: &ChangeType) -> TopicOp {
        match change_type {
            ChangeType::Insert => TopicOp::Insert,
            ChangeType::Update => TopicOp::Update,
            ChangeType::Delete => TopicOp::Delete,
        }
    }

    async fn load_user_row_for_cleanup(
        &self,
        provider: &UserTableProvider,
        table_id: &TableId,
        user_id: &UserId,
        pk_value: &str,
    ) -> Option<Row> {
        let pk_name = provider.primary_key_field_name();
        let schema = provider.schema_ref();
        let pk_field = match schema.field_with_name(pk_name) {
            Ok(field) => field,
            Err(err) => {
                log::warn!(
                    "Failed to resolve PK field for file cleanup in {} (pk={}): {}",
                    table_id,
                    pk_value,
                    err
                );
                return None;
            },
        };

        let pk_scalar = match kalamdb_commons::conversions::parse_string_as_scalar(
            pk_value,
            pk_field.data_type(),
        ) {
            Ok(value) => value,
            Err(err) => {
                log::warn!(
                    "Failed to parse PK value for file cleanup in {} (pk={}): {}",
                    table_id,
                    pk_value,
                    err
                );
                return None;
            },
        };

        match provider.find_by_pk(user_id, &pk_scalar).await {
            Ok(Some((_key, row))) => Some(row.fields),
            Ok(None) => match find_row_by_pk(provider, Some(user_id), pk_value).await {
                Ok(Some((_key, row))) => Some(row.fields),
                Ok(None) => None,
                Err(err) => {
                    log::warn!(
                        "Failed to load cold row for file cleanup in {} (pk={}): {}",
                        table_id,
                        pk_value,
                        err
                    );
                    None
                },
            },
            Err(err) => {
                log::warn!(
                    "Failed to load hot row for file cleanup in {} (pk={}): {}",
                    table_id,
                    pk_value,
                    err
                );
                None
            },
        }
    }

    async fn load_shared_row_for_cleanup(
        &self,
        provider: &SharedTableProvider,
        table_id: &TableId,
        pk_value: &str,
    ) -> Option<Row> {
        let pk_name = provider.primary_key_field_name();
        let schema = provider.schema_ref();
        let pk_field = match schema.field_with_name(pk_name) {
            Ok(field) => field,
            Err(err) => {
                log::warn!(
                    "Failed to resolve PK field for file cleanup in {} (pk={}): {}",
                    table_id,
                    pk_value,
                    err
                );
                return None;
            },
        };

        let pk_scalar = match kalamdb_commons::conversions::parse_string_as_scalar(
            pk_value,
            pk_field.data_type(),
        ) {
            Ok(value) => value,
            Err(err) => {
                log::warn!(
                    "Failed to parse PK value for file cleanup in {} (pk={}): {}",
                    table_id,
                    pk_value,
                    err
                );
                return None;
            },
        };

        match provider.find_by_pk(&pk_scalar).await {
            Ok(Some((_key, row))) => Some(row.fields),
            Ok(None) => match find_row_by_pk(provider, None, pk_value).await {
                Ok(Some((_key, row))) => Some(row.fields),
                Ok(None) => None,
                Err(err) => {
                    log::warn!(
                        "Failed to load cold row for file cleanup in {} (pk={}): {}",
                        table_id,
                        pk_value,
                        err
                    );
                    None
                },
            },
            Err(err) => {
                log::warn!(
                    "Failed to load hot row for file cleanup in {} (pk={}): {}",
                    table_id,
                    pk_value,
                    err
                );
                None
            },
        }
    }

    /// Update with fallback for StreamTableProvider
    async fn update_stream_provider(
        &self,
        provider: &StreamTableProvider,
        user_id: &UserId,
        pk_value: &str,
        updates: Row,
        version: VersionId,
    ) -> Result<usize, ApplierError> {
        match provider.update_by_id_field(user_id, pk_value, updates.clone(), version).await {
            Ok(result) => Ok(usize::from(result.is_some())),
            Err(kalamdb_tables::TableError::NotFound(_)) => {
                if let Some(key) =
                    provider.find_row_key_by_id_field(user_id, pk_value).await.map_err(|e| {
                        ApplierError::Execution(format!("Failed to find row key: {}", e))
                    })?
                {
                    let updated = <StreamTableProvider as BaseTableProvider<
                        StreamTableRowId,
                        StreamTableRow,
                    >>::update(
                        provider, user_id, &key, updates, version
                    )
                    .await
                    .map_err(|e| ApplierError::Execution(format!("Failed to update row: {}", e)))?;
                    Ok(usize::from(updated.is_some()))
                } else {
                    Ok(0)
                }
            },
            Err(e) => Err(ApplierError::Execution(format!("Failed to update row: {}", e))),
        }
    }
}

pub(crate) fn validate_and_commit_file_columns_for_rows(
    app_context: &Arc<AppContext>,
    cached_table: &crate::schema_registry::CachedTableData,
    table_id: &TableId,
    table_type: TableType,
    user_id: Option<&UserId>,
    rows: &[Row],
) -> Result<(), String> {
    let file_columns: Vec<&str> = cached_table
        .table
        .columns
        .iter()
        .filter(|col| col.data_type() == kalamdb_commons::models::datatypes::KalamDataType::File)
        .map(|col| col.column_name.as_str())
        .collect();
    if file_columns.is_empty() {
        return Ok(());
    }
    let storage_id = cached_table.storage_id.clone();
    let file_service = app_context.file_storage_service();
    for row in rows {
        for &col_name in &file_columns {
            let Some(scalar) = row.values.get(col_name) else {
                continue;
            };
            if scalar.is_null() {
                continue;
            }
            let raw_text = match scalar {
                datafusion::scalar::ScalarValue::Utf8(Some(text))
                | datafusion::scalar::ScalarValue::LargeUtf8(Some(text))
                | datafusion::scalar::ScalarValue::Utf8View(Some(text)) => text.as_str(),
                other => {
                    return Err(format!(
                        "column '{col_name}' is of type pg_catalog.file and cannot be constructed \
                         from {:?}; use file_put()",
                        other.data_type()
                    ));
                },
            };
            let file_ref = FileRef::try_from_json(raw_text).map_err(|error| {
                format!(
                    "invalid or forged pg_catalog.file reference in column '{col_name}': {error}"
                )
            })?;
            file_service
                .validate_and_commit_file_ref(&file_ref, &storage_id, table_type, table_id, user_id)
                .map_err(|error| {
                    format!(
                        "forged or invalid pg_catalog.file reference in column '{col_name}': \
                         {error}"
                    )
                })?;
        }
    }
    Ok(())
}
