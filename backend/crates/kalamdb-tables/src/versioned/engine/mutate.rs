//! Insert, update, and delete for both versioned scopes.

use std::collections::{HashMap, HashSet};

use datafusion::arrow::datatypes::SchemaRef;
use kalamdb_commons::{
    ids::VersionId,
    models::{rows::Row, UserId},
    websocket::ChangeNotification,
};
use tracing::Instrument;

use crate::{
    error::KalamDbError, manifest::manifest_helpers::ensure_manifest_ready,
    utils::base::BaseTableProvider,
};

use super::{
    provider::VersionedTable,
    scope::{MutationGuard, TableScope},
};

impl<S: TableScope> VersionedTable<S>
where
    Self: MutationGuard
        + super::scan::ScopeScan<RowId = S::RowId, Row = S::Row, Context = S::ScanContext>
        + BaseTableProvider<S::RowId, S::Row>,
{
    pub async fn insert_batch_with_versions(
        &self,
        actor: Option<&UserId>,
        rows: Vec<Row>,
        versions: &[VersionId],
    ) -> Result<Vec<S::RowId>, KalamDbError> {
        let row_count = rows.len();
        let span = tracing::debug_span!(
            "table.insert_batch",
            table_id = %self.core.table_id(),
            scope = S::LABEL,
            row_count
        );
        async move {
            let scope = self.storage_user(actor)?;
            let (row_keys, retained) =
                self.persist_insert_batch_rows(actor, rows, versions, true, false).await?;
            crate::versioned::fanout_insert_notifications(&self.core, scope, actor, || {
                retained.iter().map(|(_, entity)| Self::notification_row(entity)).collect()
            })
            .await;
            Ok(row_keys)
        }
        .instrument(span)
        .await
    }

    pub async fn insert_batch_deferred_prevalidated_with_versions(
        &self,
        actor: Option<&UserId>,
        rows: Vec<Row>,
        versions: &[VersionId],
    ) -> Result<Vec<(S::RowId, Option<ChangeNotification>)>, KalamDbError> {
        let row_count = rows.len();
        let span = tracing::debug_span!(
            "table.insert_batch",
            table_id = %self.core.table_id(),
            scope = S::LABEL,
            row_count,
            deferred_side_effects = true
        );
        async move {
            let (row_ids, retained) =
                self.persist_insert_batch_rows(actor, rows, versions, false, false).await?;
            let table_id = self.core.table_id().clone();
            let scope = self.storage_user(actor)?;
            let watched = crate::versioned::table_watch(&self.core, scope).any();
            if retained.is_empty() {
                return Ok(row_ids.into_iter().map(|row_key| (row_key, None)).collect());
            }
            Ok(retained
                .into_iter()
                .map(|(row_key, entity)| {
                    let notification = crate::versioned::insert_notification_when_watched(
                        watched,
                        table_id.clone(),
                        || Self::notification_row(&entity),
                    );
                    (row_key, notification)
                })
                .collect())
        }
        .instrument(span)
        .await
    }

    pub(crate) async fn plan_insert_batch_prevalidated(
        &self,
        actor: Option<&UserId>,
        rows: Vec<Row>,
        versions: &[VersionId],
    ) -> Result<
        (
            Vec<kalamdb_store::Operation>,
            Vec<(S::RowId, Option<ChangeNotification>)>,
            Vec<(S::RowId, S::Row)>,
        ),
        KalamDbError,
    > {
        let (_row_ids, entries, operations) =
            self.collect_insert_batch_rows(actor, rows, versions, false, true).await?;
        let table_id = self.core.table_id().clone();
        let scope = self.storage_user(actor)?;
        let watched = crate::versioned::table_watch(&self.core, scope).any();
        let applied = entries
            .iter()
            .map(|(row_key, entity)| {
                let notification = crate::versioned::insert_notification_when_watched(
                    watched,
                    table_id.clone(),
                    || Self::notification_row(entity),
                );
                (row_key.clone(), notification)
            })
            .collect();
        Ok((operations, applied, entries))
    }

    pub(crate) async fn plan_insert_deferred(
        &self,
        actor: Option<&UserId>,
        row_data: Row,
        version: VersionId,
        hot_predecessor: Option<(S::RowId, S::Row)>,
        operations: &mut Vec<kalamdb_store::Operation>,
    ) -> Result<(S::RowId, Option<ChangeNotification>, Row), KalamDbError> {
        self.insert_deferred_internal(actor, row_data, version, hot_predecessor, Some(operations))
            .await
    }

    async fn insert_deferred_internal(
        &self,
        actor: Option<&UserId>,
        row_data: Row,
        version: VersionId,
        hot_predecessor: Option<(S::RowId, S::Row)>,
        planned_ops: Option<&mut Vec<kalamdb_store::Operation>>,
    ) -> Result<(S::RowId, Option<ChangeNotification>, Row), KalamDbError> {
        let span = tracing::debug_span!(
            "table.insert",
            table_id = %self.core.table_id(),
            scope = S::LABEL,
            column_count = row_data.values.len(),
            deferred_side_effects = true
        );
        async move {
            let _guard = self.begin_guard();
            let scope = self.storage_user(actor)?;
            ensure_manifest_ready(&self.core, scope, S::PROVIDER_NAME)?;
            crate::utils::datafusion_dml::validate_not_null_with_set(
                self.core.non_null_columns(),
                std::slice::from_ref(&row_data),
            )?;
            let entity = S::make_row(scope, version, false, row_data);
            let row_key = S::make_row_id(scope, version);
            let stored_fields = S::row_fields(&entity).clone();
            let failed = format!("Failed to insert {} table row", S::LABEL);
            self.persist_replaced(
                actor,
                scope,
                planned_ops,
                hot_predecessor,
                row_key.clone(),
                entity.clone(),
                &failed,
                PersistEffect::Upsert,
            )
            .await?;
            let notification = crate::versioned::insert_notification_when_watched(
                crate::versioned::table_watch(&self.core, scope).any(),
                self.core.table_id().clone(),
                || Self::notification_row(&entity),
            );
            Ok((row_key, notification, stored_fields))
        }
        .instrument(span)
        .await
    }

    pub(crate) async fn plan_update_by_pk_value_deferred_with_hot(
        &self,
        actor: Option<&UserId>,
        pk_value: &str,
        updates: Row,
        version: VersionId,
        known: Option<crate::versioned::AuthoritativeBaseline<S::RowId, S::Row>>,
        operations: &mut Vec<kalamdb_store::Operation>,
    ) -> Result<Option<(S::RowId, S::Row, Option<ChangeNotification>)>, KalamDbError> {
        self.update_by_pk_value_deferred_with_hot_ops(
            actor,
            pk_value,
            updates,
            version,
            known,
            Some(operations),
        )
        .await
    }

    async fn update_by_pk_value_deferred_with_hot_ops(
        &self,
        actor: Option<&UserId>,
        pk_value: &str,
        updates: Row,
        version: VersionId,
        known: Option<crate::versioned::AuthoritativeBaseline<S::RowId, S::Row>>,
        planned_ops: Option<&mut Vec<kalamdb_store::Operation>>,
    ) -> Result<Option<(S::RowId, S::Row, Option<ChangeNotification>)>, KalamDbError> {
        let span = tracing::debug_span!(
            "table.update",
            table_id = %self.core.table_id(),
            scope = S::LABEL,
            pk = pk_value,
            deferred_side_effects = true
        );
        async move {
            let _guard = self.begin_guard();
            let scope = self.storage_user(actor)?;
            let schema = self.core.schema_ref();
            let updates = crate::versioned::coerce_update_row(updates, &schema)?;
            let pk_name = self.primary_key_field_name().to_string();
            let loaded = match known {
                Some(baseline) => crate::versioned::take_update_baseline(
                    baseline,
                    S::row_deleted,
                    &pk_name,
                    pk_value,
                )?,
                None => crate::versioned::take_update_baseline(
                    crate::versioned::authoritative_baseline(
                        self.load_mutation_target(actor, pk_value).await?,
                    ),
                    S::row_deleted,
                    &pk_name,
                    pk_value,
                )?,
            };
            let (hot_key, row) = match loaded {
                crate::versioned::LoadedBaseline::Hot(key, row) => (Some(key), row),
                crate::versioned::LoadedBaseline::Cold(row) => (None, row),
            };
            let Some(new_fields) = crate::versioned::changed_update_fields(
                S::row_fields(&row),
                updates,
                self.core.non_null_columns(),
            )?
            else {
                tracing::debug!(
                    table_id = %self.core.table_id(),
                    pk = pk_value,
                    "table.update_noop: row unchanged, skipping write"
                );
                return Ok(None);
            };
            let watched = crate::versioned::table_watch(&self.core, scope).any();
            let old_notification = watched.then(|| Self::notification_row(&row));
            let previous = hot_key.map(|key| (key, row));
            let entity = S::make_row(scope, version, false, new_fields);
            let row_key = S::make_row_id(scope, version);
            let failed = format!("Failed to update {} table row", S::LABEL);
            self.persist_replaced(
                actor,
                scope,
                planned_ops,
                previous,
                row_key.clone(),
                entity.clone(),
                &failed,
                PersistEffect::Upsert,
            )
            .await?;
            let notification = old_notification.map(|old_row| {
                crate::versioned::update_notification(
                    self.core.table_id().clone(),
                    old_row,
                    Self::notification_row(&entity),
                    self.primary_key_field_name().to_string(),
                )
            });
            Ok(Some((row_key, entity, notification)))
        }
        .instrument(span)
        .await
    }

    pub async fn delete_by_pk_value_deferred(
        &self,
        actor: Option<&UserId>,
        pk_value: &str,
        version: VersionId,
    ) -> Result<Option<(S::RowId, Option<ChangeNotification>)>, KalamDbError> {
        Ok(self
            .delete_by_pk_value_deferred_with_hot(actor, pk_value, version, None)
            .await?
            .map(|(row_key, _row, notification)| (row_key, notification)))
    }

    pub async fn delete_by_pk_value_deferred_with_hot(
        &self,
        actor: Option<&UserId>,
        pk_value: &str,
        version: VersionId,
        known_hot: Option<(S::RowId, S::Row)>,
    ) -> Result<Option<(S::RowId, S::Row, Option<ChangeNotification>)>, KalamDbError> {
        self.delete_by_pk_value_deferred_with_hot_ops(
            actor,
            pk_value,
            version,
            known_hot.map(|(key, row)| crate::versioned::AuthoritativeBaseline::Hot { key, row }),
            None,
        )
        .await
    }

    pub(crate) async fn plan_delete_by_pk_value_deferred_with_hot(
        &self,
        actor: Option<&UserId>,
        pk_value: &str,
        version: VersionId,
        known: Option<crate::versioned::AuthoritativeBaseline<S::RowId, S::Row>>,
        operations: &mut Vec<kalamdb_store::Operation>,
    ) -> Result<Option<(S::RowId, S::Row, Option<ChangeNotification>)>, KalamDbError> {
        self.delete_by_pk_value_deferred_with_hot_ops(
            actor,
            pk_value,
            version,
            known,
            Some(operations),
        )
        .await
    }

    async fn delete_by_pk_value_deferred_with_hot_ops(
        &self,
        actor: Option<&UserId>,
        pk_value: &str,
        version: VersionId,
        known: Option<crate::versioned::AuthoritativeBaseline<S::RowId, S::Row>>,
        planned_ops: Option<&mut Vec<kalamdb_store::Operation>>,
    ) -> Result<Option<(S::RowId, S::Row, Option<ChangeNotification>)>, KalamDbError> {
        let span = tracing::debug_span!(
            "table.delete",
            table_id = %self.core.table_id(),
            scope = S::LABEL,
            pk = pk_value,
            deferred_side_effects = true
        );
        async move {
            let _guard = self.begin_guard();
            let scope = self.storage_user(actor)?;
            let loaded = match known {
                Some(baseline) => crate::versioned::take_delete_baseline(baseline, S::row_deleted),
                None => crate::versioned::take_delete_baseline(
                    crate::versioned::authoritative_baseline(
                        self.load_mutation_target(actor, pk_value).await?,
                    ),
                    S::row_deleted,
                ),
            };
            let Some(loaded) = loaded else {
                return Ok(None);
            };
            let (previous, latest_row) = match loaded {
                crate::versioned::LoadedBaseline::Hot(key, row) => (Some((key, row.clone())), row),
                crate::versioned::LoadedBaseline::Cold(row) => (None, row),
            };
            let entity = S::make_row(scope, version, true, S::row_fields(&latest_row).clone());
            let row_key = S::make_row_id(scope, version);
            let failed = format!("Failed to delete {} table row", S::LABEL);
            self.persist_replaced(
                actor,
                scope,
                planned_ops,
                previous,
                row_key.clone(),
                entity.clone(),
                &failed,
                PersistEffect::Delete { pk: pk_value },
            )
            .await?;
            let notification = if crate::versioned::table_watch(&self.core, scope).any() {
                Some(self.delete_notification(
                    self.core.table_id().clone(),
                    Self::notification_row(&entity),
                ))
            } else {
                None
            };
            Ok(Some((row_key, entity, notification)))
        }
        .instrument(span)
        .await
    }

    pub async fn delete_batch_by_pk_values_deferred(
        &self,
        actor: Option<&UserId>,
        pk_values: &[String],
        versions: &[VersionId],
    ) -> Result<Vec<crate::versioned::DeferredMutation<S::RowId>>, KalamDbError> {
        if pk_values.is_empty() {
            return Ok(Vec::new());
        }
        let scope = self.storage_user(actor)?;
        let table_id = self.core.table_id().clone();
        let watched = crate::versioned::table_watch(&self.core, scope).any();
        let targets = self.load_mutation_targets(actor, pk_values).await?;
        let mut items = Vec::with_capacity(pk_values.len());
        let mut applied = Vec::with_capacity(pk_values.len());
        for ((pk_value, version), target) in
            pk_values.iter().zip(versions.iter().copied()).zip(targets)
        {
            let Some((hot_key, row)) = crate::versioned::optional_live_row(target) else {
                continue;
            };
            let prior = S::row_fields(&row).clone();
            let previous = hot_key.map(|key| (key, row));
            let entity = S::make_row(scope, version, true, prior.clone());
            let row_key = S::make_row_id(scope, version);
            let notification = watched.then(|| {
                self.delete_notification(table_id.clone(), Self::notification_row(&entity))
            });
            if let Err(error) = self.stage_vector_delete(scope, version, pk_value).await {
                crate::versioned::warn_vector_delete(
                    self.core.table_id(),
                    scope,
                    version,
                    pk_value,
                    &error,
                );
            }
            items.push((previous, row_key.clone(), entity));
            applied.push(crate::versioned::DeferredMutation {
                key: row_key,
                notification,
                prior,
                payload: None,
            });
        }
        if items.is_empty() {
            return Ok(applied);
        }
        self.store.replace_batch_async(items).await.map_err(|error| {
            KalamDbError::InvalidOperation(format!(
                "Failed to delete {} table rows: {}",
                S::LABEL,
                error
            ))
        })?;
        crate::versioned::mark_table_pending_write(&self.core, scope);
        Ok(applied)
    }

    pub async fn update_batch_by_pk_values_deferred(
        &self,
        actor: Option<&UserId>,
        updates: &[(String, Row, VersionId)],
    ) -> Result<Vec<crate::versioned::DeferredMutation<S::RowId>>, KalamDbError> {
        if updates.is_empty() {
            return Ok(Vec::new());
        }
        let scope = self.storage_user(actor)?;
        let schema: SchemaRef = self.core.schema_ref();
        let table_id = self.core.table_id().clone();
        let watched = crate::versioned::table_watch(&self.core, scope).any();
        let pk_column = self.primary_key_field_name().to_string();
        let pk_values: Vec<String> = updates.iter().map(|(pk, _, _)| pk.clone()).collect();
        let targets = self.load_mutation_targets(actor, &pk_values).await?;
        let mut items = Vec::with_capacity(updates.len());
        let mut applied = Vec::with_capacity(updates.len());
        for ((pk_value, update_row, version), target) in updates.iter().zip(targets) {
            let coerced = crate::versioned::coerce_update_row(update_row.clone(), &schema)?;
            let (hot_key, row) = crate::versioned::require_live_row(target, &pk_column, pk_value)?;
            let prior = S::row_fields(&row).clone();
            let old_notification = watched.then(|| Self::notification_row(&row));
            let previous = hot_key.map(|key| (key, row));
            let Some(new_fields) = crate::versioned::changed_update_fields(
                &prior,
                coerced,
                self.core.non_null_columns(),
            )?
            else {
                continue;
            };
            let entity = S::make_row(scope, *version, false, new_fields);
            let row_key = S::make_row_id(scope, *version);
            let notification = crate::versioned::update_notification_when_watched(
                watched,
                table_id.clone(),
                || old_notification.expect("notification row is built when a watcher is present"),
                || Self::notification_row(&entity),
                pk_column.clone(),
            );
            if let Err(error) =
                self.stage_vector_upsert(scope, *version, S::row_fields(&entity)).await
            {
                crate::versioned::warn_vector_upsert(self.core.table_id(), scope, *version, &error);
            }
            items.push((previous, row_key.clone(), entity));
            applied.push(crate::versioned::DeferredMutation {
                key: row_key,
                notification,
                prior,
                payload: Some(update_row.clone()),
            });
        }
        if items.is_empty() {
            return Ok(applied);
        }
        self.store.replace_batch_async(items).await.map_err(|error| {
            KalamDbError::InvalidOperation(format!(
                "Failed to update {} table rows: {}",
                S::LABEL,
                error
            ))
        })?;
        crate::versioned::mark_table_pending_write(&self.core, scope);
        Ok(applied)
    }

    async fn persist_replaced(
        &self,
        actor: Option<&UserId>,
        scope: Option<&UserId>,
        planned_ops: Option<&mut Vec<kalamdb_store::Operation>>,
        previous: Option<(S::RowId, S::Row)>,
        row_key: S::RowId,
        entity: S::Row,
        error_context: &str,
        effect: PersistEffect<'_>,
    ) -> Result<(), KalamDbError> {
        if let Some(operations) = planned_ops {
            crate::versioned::append_replaced_row_ops(
                &self.store,
                operations,
                previous,
                &row_key,
                &entity,
                error_context,
            )?;
            return Ok(());
        }
        let version = S::row_version(&entity);
        self.write_replaced_row(previous, row_key, entity.clone(), error_context)
            .await?;
        self.stage_persist_effect(actor, scope, version, &entity, effect).await;
        crate::versioned::mark_table_pending_write(&self.core, scope);
        Ok(())
    }
}

enum DirectNotice<R> {
    Insert(R),
    Update { old: R, new: R },
    Delete(R),
}

enum PersistEffect<'a> {
    Upsert,
    Delete { pk: &'a str },
}

impl<S: TableScope> VersionedTable<S>
where
    Self: MutationGuard,
{
    async fn stage_persist_effect(
        &self,
        actor: Option<&UserId>,
        scope: Option<&UserId>,
        version: VersionId,
        entity: &S::Row,
        effect: PersistEffect<'_>,
    ) {
        match effect {
            PersistEffect::Upsert => {
                if let Err(error) =
                    self.stage_vector_upsert(actor, version, S::row_fields(entity)).await
                {
                    crate::versioned::warn_vector_upsert(
                        self.core.table_id(),
                        scope,
                        version,
                        &error,
                    );
                }
            },
            PersistEffect::Delete { pk } => {
                if let Err(error) = self.stage_vector_delete(scope, version, pk).await {
                    crate::versioned::warn_vector_delete(
                        self.core.table_id(),
                        scope,
                        version,
                        pk,
                        &error,
                    );
                }
            },
        }
    }
}

impl<S: TableScope> VersionedTable<S>
where
    Self: MutationGuard
        + super::scan::ScopeScan<RowId = S::RowId, Row = S::Row, Context = S::ScanContext>
        + BaseTableProvider<S::RowId, S::Row>,
{
    pub(super) async fn insert_row(
        &self,
        actor: &UserId,
        row_data: Row,
        version: VersionId,
    ) -> Result<S::RowId, KalamDbError> {
        let span = tracing::debug_span!(
            "table.insert",
            table_id = %self.core.table_id(),
            scope = S::LABEL,
            column_count = row_data.values.len()
        );
        async move {
            let _guard = self.begin_guard();
            let scope = self.storage_user(Some(actor))?;
            ensure_manifest_ready(&self.core, scope, S::PROVIDER_NAME)?;
            crate::utils::datafusion_dml::validate_not_null_with_set(
                self.core.non_null_columns(),
                std::slice::from_ref(&row_data),
            )?;
            let predecessor = self.insert_predecessor(Some(actor), &row_data).await?;
            let entity = S::make_row(scope, version, false, row_data);
            let row_key = S::make_row_id(scope, version);
            let failed = format!("Failed to insert {} table row", S::LABEL);
            self.write_replaced_row(predecessor, row_key.clone(), entity.clone(), &failed)
                .await?;
            log::debug!("Inserted {} table row with _seq {}", S::LABEL, version.as_i64());
            if let Err(error) =
                self.stage_vector_upsert(scope, version, S::row_fields(&entity)).await
            {
                crate::versioned::warn_vector_upsert(self.core.table_id(), scope, version, &error);
            }
            crate::versioned::mark_table_pending_write(&self.core, scope);
            crate::versioned::fanout_row_change(
                &self.core,
                scope,
                Some(actor),
                kalamdb_commons::models::TopicOp::Insert,
                || Self::notification_row(&entity),
                |table_id, row| ChangeNotification::insert(table_id, row),
            )
            .await;
            Ok(row_key)
        }
        .instrument(span)
        .await
    }

    pub(super) async fn update_by_pk_value_with_version(
        &self,
        actor: &UserId,
        pk_value: &str,
        updates: Row,
        version: VersionId,
    ) -> Result<Option<S::RowId>, KalamDbError> {
        let span = tracing::debug_span!(
            "table.update",
            table_id = %self.core.table_id(),
            scope = S::LABEL,
            pk = pk_value,
            update_columns = updates.values.len()
        );
        async move {
            let _guard = self.begin_guard();
            let scope = self.storage_user(Some(actor))?;
            let pk_name = self.primary_key_field_name().to_string();
            let (hot_key, row) = crate::versioned::require_live_row(
                self.load_mutation_target(Some(actor), pk_value).await?,
                &pk_name,
                pk_value,
            )?;
            let old_notice = Self::notification_row(&row);
            let latest_fields = S::row_fields(&row).clone();
            let previous = hot_key.map(|key| (key, row));
            let coerced = crate::versioned::coerce_update_row(updates, &self.core.schema_ref())?;
            let new_fields = crate::versioned::merge_row_updates(&latest_fields, coerced);
            if new_fields == latest_fields {
                if !S::USER_SCOPED_ROWS {
                    tracing::debug!(
                        table_id = %self.core.table_id(),
                        pk = pk_value,
                        "table.update_noop: row unchanged, skipping write"
                    );
                }
                return Ok(None);
            }
            if S::VALIDATE_DIRECT_NOT_NULL {
                crate::utils::datafusion_dml::validate_not_null_with_set(
                    self.core.non_null_columns(),
                    &[new_fields.clone()],
                )?;
            }
            let entity = S::make_row(scope, version, false, new_fields);
            let row_key = S::make_row_id(scope, version);
            let failed = format!("Failed to update {} table row", S::LABEL);
            self.write_replaced_row(previous, row_key.clone(), entity.clone(), &failed)
                .await?;
            if let Err(error) =
                self.stage_vector_upsert(scope, version, S::row_fields(&entity)).await
            {
                crate::versioned::warn_vector_upsert(self.core.table_id(), scope, version, &error);
            }
            crate::versioned::mark_table_pending_write(&self.core, scope);
            let pk_col = self.primary_key_field_name().to_string();
            crate::versioned::fanout_row_change(
                &self.core,
                scope,
                Some(actor),
                kalamdb_commons::models::TopicOp::Update,
                || Self::notification_row(&entity),
                |table_id, new_row| {
                    crate::versioned::update_notification(table_id, old_notice, new_row, pk_col)
                },
            )
            .await;
            Ok(Some(row_key))
        }
        .instrument(span)
        .await
    }

    pub(super) async fn delete_by_pk_value_with_version(
        &self,
        actor: &UserId,
        pk_value: &str,
        version: VersionId,
    ) -> Result<bool, KalamDbError> {
        let span = tracing::debug_span!(
            "table.delete",
            table_id = %self.core.table_id(),
            scope = S::LABEL,
            pk = pk_value
        );
        async move {
            let _guard = self.begin_guard();
            let scope = self.storage_user(Some(actor))?;
            let pk_name = self.primary_key_field_name().to_string();
            let Some((hot_key, row)) = crate::versioned::optional_live_row(
                self.load_mutation_target(Some(actor), pk_value).await?,
            ) else {
                log::trace!(
                    "[{} DELETE_BY_PK] Row with {}={} not found",
                    S::PROVIDER_NAME,
                    pk_name,
                    pk_value
                );
                return Ok(false);
            };
            let values = S::row_fields(&row).values.clone();
            let previous = hot_key.map(|key| (key, row));
            let entity = S::make_row(scope, version, true, Row::new(values));
            let row_key = S::make_row_id(scope, version);
            log::debug!(
                "[{} DELETE_BY_PK] Writing tombstone: pk={}, _seq={}",
                S::PROVIDER_NAME,
                pk_value,
                version.as_i64()
            );
            let failed = format!("Failed to delete {} table row", S::LABEL);
            self.write_replaced_row(previous, row_key, entity.clone(), &failed).await?;
            if let Err(error) = self.stage_vector_delete(scope, version, pk_value).await {
                crate::versioned::warn_vector_delete(
                    self.core.table_id(),
                    scope,
                    version,
                    pk_value,
                    &error,
                );
            }
            crate::versioned::mark_table_pending_write(&self.core, scope);
            crate::versioned::fanout_row_change(
                &self.core,
                scope,
                Some(actor),
                kalamdb_commons::models::TopicOp::Delete,
                || Self::notification_row(&entity),
                |table_id, row| self.delete_notification(table_id, row),
            )
            .await;
            Ok(true)
        }
        .instrument(span)
        .await
    }

    /// Delete every live key in one storage batch.
    pub(super) async fn commit_direct_deletes(
        &self,
        actor: &UserId,
        pk_values: &[String],
        commit_seq: u64,
        ordinal: &mut u32,
    ) -> Result<u64, KalamDbError> {
        if pk_values.is_empty() {
            return Ok(0);
        }
        let _guard = self.begin_guard();
        let scope = self.storage_user(Some(actor))?;
        let targets = self.load_mutation_targets(Some(actor), pk_values).await?;
        let mut items = Vec::new();
        let mut effects = Vec::new();
        let mut deleted = Vec::new();
        for (pk_value, target) in pk_values.iter().zip(targets) {
            let Some((hot_key, row)) = crate::versioned::optional_live_row(target) else {
                continue;
            };
            let version = crate::utils::base::next_statement_version(commit_seq, ordinal)?;
            let fields = S::row_fields(&row).clone();
            let previous = hot_key.map(|key| (key, row));
            let entity = S::make_row(scope, version, true, fields);
            let row_key = S::make_row_id(scope, version);
            items.push((previous, row_key, entity.clone()));
            effects.push(crate::versioned::PlannedScopeFinish::Delete {
                version,
                pk: pk_value.clone(),
            });
            deleted.push((pk_value.clone(), entity));
        }
        let deleted_count = deleted.len() as u64;
        if items.is_empty() {
            return Ok(0);
        }
        self.store.replace_batch_async(items).await.map_err(|error| {
            KalamDbError::InvalidOperation(format!(
                "Failed to delete {} table rows: {error}",
                S::LABEL
            ))
        })?;
        self.finish_planned_scope(Some(actor), &effects).await;
        for (_pk_value, entity) in deleted {
            crate::versioned::fanout_row_change(
                &self.core,
                scope,
                Some(actor),
                kalamdb_commons::models::TopicOp::Delete,
                || Self::notification_row(&entity),
                |table_id, row| self.delete_notification(table_id, row),
            )
            .await;
        }
        Ok(deleted_count)
    }

    /// Apply primary-key moves and field updates in one storage batch.
    ///
    /// The returned count is the number of field updates that changed a row.
    /// Primary-key moves are counted by the caller.
    pub(super) async fn commit_direct_updates(
        &self,
        actor: &UserId,
        commit_seq: u64,
        ordinal: &mut u32,
        direct_moves: Vec<(String, Row)>,
        direct_updates: Vec<(String, Row)>,
    ) -> Result<u64, KalamDbError> {
        if direct_moves.is_empty() && direct_updates.is_empty() {
            return Ok(0);
        }
        let _guard = self.begin_guard();
        let scope = self.storage_user(Some(actor))?;
        let pk_name = self.primary_key_field_name().to_string();
        let schema = self.core.schema_ref();
        let mut load_keys = Vec::with_capacity(direct_moves.len() + direct_updates.len());
        for (pk_value, _) in &direct_moves {
            load_keys.push(pk_value.clone());
        }
        for (pk_value, _) in &direct_updates {
            load_keys.push(pk_value.clone());
        }
        let mut loaded = self.load_mutation_targets(Some(actor), &load_keys).await?;
        let update_targets = loaded.split_off(direct_moves.len());
        let move_targets = loaded;
        let mut items = Vec::new();
        let mut effects = Vec::new();
        let mut notices = Vec::new();
        let mut claimed = HashSet::new();
        let mut reused: HashMap<String, Option<(S::RowId, S::Row)>> = HashMap::new();
        let mut prepared = Vec::with_capacity(direct_moves.len());
        for ((old_pk, inserted_row), target) in direct_moves.into_iter().zip(move_targets) {
            let new_pk = match crate::versioned::insert_pk_parts(
                self.core.is_auto_increment_pk(),
                &inserted_row,
                &pk_name,
            )? {
                Some((_, pk_text)) => {
                    if !claimed.insert(pk_text.clone()) {
                        return Err(KalamDbError::AlreadyExists(format!(
                            "Primary key violation: value '{pk_text}' appears multiple times in \
                             the statement for column '{pk_name}'"
                        )));
                    }
                    Some(pk_text)
                },
                None => None,
            };
            crate::utils::datafusion_dml::validate_not_null_with_set(
                self.core.non_null_columns(),
                std::slice::from_ref(&inserted_row),
            )?;
            let live = match crate::versioned::optional_live_row(target) {
                Some((hot_key, row)) => {
                    let fields = S::row_fields(&row).clone();
                    let previous = hot_key.map(|key| (key, row));
                    reused.insert(old_pk.clone(), previous.clone());
                    Some((previous, fields))
                },
                None => None,
            };
            prepared.push((old_pk, inserted_row, new_pk, live));
        }
        let mut early_deletes = Vec::new();
        let mut inserts = Vec::new();
        let mut late_deletes = Vec::new();
        for (old_pk, inserted_row, new_pk, live) in prepared {
            let predecessor = if let Some(pk_text) = new_pk.as_ref() {
                if let Some(previous) = reused.get(pk_text) {
                    previous.clone()
                } else {
                    self.insert_predecessor(Some(actor), &inserted_row).await?
                }
            } else {
                self.insert_predecessor(Some(actor), &inserted_row).await?
            };
            inserts.push((predecessor, inserted_row));
            if let Some((previous, fields)) = live {
                if reused.contains_key(&old_pk) {
                    early_deletes.push((previous, fields, old_pk));
                } else {
                    late_deletes.push((previous, fields, old_pk));
                }
            }
        }
        let push_delete = |items: &mut Vec<_>,
                           effects: &mut Vec<_>,
                           notices: &mut Vec<_>,
                           ordinal: &mut u32,
                           previous,
                           fields,
                           pk: String| {
            let delete_version = crate::utils::base::next_statement_version(commit_seq, ordinal)?;
            let tombstone = S::make_row(scope, delete_version, true, fields);
            let delete_key = S::make_row_id(scope, delete_version);
            items.push((previous, delete_key, tombstone.clone()));
            effects.push(crate::versioned::PlannedScopeFinish::Delete {
                version: delete_version,
                pk,
            });
            notices.push(DirectNotice::Delete(tombstone));
            Ok::<(), KalamDbError>(())
        };
        for (previous, fields, pk) in early_deletes {
            push_delete(&mut items, &mut effects, &mut notices, ordinal, previous, fields, pk)?;
        }
        for (predecessor, inserted_row) in inserts {
            let insert_version = crate::utils::base::next_statement_version(commit_seq, ordinal)?;
            let inserted = S::make_row(scope, insert_version, false, inserted_row);
            let insert_key = S::make_row_id(scope, insert_version);
            items.push((predecessor, insert_key, inserted.clone()));
            effects.push(crate::versioned::PlannedScopeFinish::Upsert {
                version: insert_version,
                row: S::row_fields(&inserted).clone(),
            });
            notices.push(DirectNotice::Insert(inserted));
        }
        for (previous, fields, pk) in late_deletes {
            push_delete(&mut items, &mut effects, &mut notices, ordinal, previous, fields, pk)?;
        }
        let mut updated = 0u64;
        for ((pk_value, updates), target) in direct_updates.into_iter().zip(update_targets) {
            let (hot_key, row) = crate::versioned::require_live_row(target, &pk_name, &pk_value)?;
            let latest_row = row.clone();
            let previous = hot_key.map(|key| (key, row));
            let coerced = crate::versioned::coerce_update_row(updates, &schema)?;
            let new_fields =
                crate::versioned::merge_row_updates(S::row_fields(&latest_row), coerced);
            if new_fields == *S::row_fields(&latest_row) {
                continue;
            }
            if S::VALIDATE_DIRECT_NOT_NULL {
                crate::utils::datafusion_dml::validate_not_null_with_set(
                    self.core.non_null_columns(),
                    std::slice::from_ref(&new_fields),
                )?;
            }
            let version = crate::utils::base::next_statement_version(commit_seq, ordinal)?;
            let entity = S::make_row(scope, version, false, new_fields);
            let row_key = S::make_row_id(scope, version);
            items.push((previous, row_key, entity.clone()));
            effects.push(crate::versioned::PlannedScopeFinish::Upsert {
                version,
                row: S::row_fields(&entity).clone(),
            });
            notices.push(DirectNotice::Update {
                old: latest_row,
                new: entity,
            });
            updated += 1;
        }
        if items.is_empty() {
            return Ok(0);
        }
        self.store.replace_batch_async(items).await.map_err(|error| {
            KalamDbError::InvalidOperation(format!(
                "Failed to update {} table rows: {error}",
                S::LABEL
            ))
        })?;
        self.finish_planned_scope(Some(actor), &effects).await;
        let pk_column = pk_name;
        for notice in notices {
            match notice {
                DirectNotice::Insert(entity) => {
                    crate::versioned::fanout_row_change(
                        &self.core,
                        scope,
                        Some(actor),
                        kalamdb_commons::models::TopicOp::Insert,
                        || Self::notification_row(&entity),
                        |table_id, row| {
                            kalamdb_commons::websocket::ChangeNotification::insert(table_id, row)
                        },
                    )
                    .await;
                },
                DirectNotice::Update { old, new } => {
                    crate::versioned::fanout_row_change(
                        &self.core,
                        scope,
                        Some(actor),
                        kalamdb_commons::models::TopicOp::Update,
                        || Self::notification_row(&new),
                        |table_id, new_row| {
                            crate::versioned::update_notification(
                                table_id,
                                Self::notification_row(&old),
                                new_row,
                                pk_column.clone(),
                            )
                        },
                    )
                    .await;
                },
                DirectNotice::Delete(entity) => {
                    crate::versioned::fanout_row_change(
                        &self.core,
                        scope,
                        Some(actor),
                        kalamdb_commons::models::TopicOp::Delete,
                        || Self::notification_row(&entity),
                        |table_id, row| self.delete_notification(table_id, row),
                    )
                    .await;
                },
            }
        }
        Ok(updated)
    }

    pub(super) async fn committed_primary_key_is_live(
        &self,
        actor: Option<&UserId>,
        primary_key: &str,
    ) -> Result<bool, datafusion::error::DataFusionError> {
        let target = self
            .load_mutation_target(actor, primary_key)
            .await
            .map_err(|error| datafusion::error::DataFusionError::Execution(error.to_string()))?;
        Ok(matches!(
            target,
            crate::versioned::MutationTarget::Hot(_, _) | crate::versioned::MutationTarget::Cold(_)
        ))
    }
}
