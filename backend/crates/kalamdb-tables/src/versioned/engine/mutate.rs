//! Insert, update, and delete for both versioned scopes.

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
            let entries = self.persist_insert_batch_rows(actor, rows, versions, true).await?;
            let row_keys = entries.iter().map(|(row_key, _)| row_key.clone()).collect();
            crate::versioned::fanout_insert_notifications(&self.core, scope, actor, || {
                entries.iter().map(|(_, entity)| Self::notification_row(entity)).collect()
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
            let entries = self.persist_insert_batch_rows(actor, rows, versions, false).await?;
            let table_id = self.core.table_id().clone();
            let scope = self.storage_user(actor)?;
            let watched = crate::versioned::table_watch(&self.core, scope).any();
            Ok(entries
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
        let (entries, operations) =
            self.collect_insert_batch_rows(actor, rows, versions, false).await?;
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
        let mut items = Vec::with_capacity(pk_values.len());
        let mut applied = Vec::with_capacity(pk_values.len());
        for (pk_value, version) in pk_values.iter().zip(versions.iter().copied()) {
            let Some((previous, latest_row)) = crate::versioned::optional_live_row(
                self.load_mutation_target(actor, pk_value).await?,
            ) else {
                continue;
            };
            let prior = S::row_fields(&latest_row).clone();
            let entity = S::make_row(scope, version, true, S::row_fields(&latest_row).clone());
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
        let mut items = Vec::with_capacity(updates.len());
        let mut applied = Vec::with_capacity(updates.len());
        for (pk_value, update_row, version) in updates {
            let coerced = crate::versioned::coerce_update_row(update_row.clone(), &schema)?;
            let (previous, latest_row) = crate::versioned::require_live_row(
                self.load_mutation_target(actor, pk_value).await?,
                &pk_column,
                pk_value,
            )?;
            let Some(new_fields) = crate::versioned::changed_update_fields(
                S::row_fields(&latest_row),
                coerced,
                self.core.non_null_columns(),
            )?
            else {
                continue;
            };
            let prior = S::row_fields(&latest_row).clone();
            let entity = S::make_row(scope, *version, false, new_fields);
            let row_key = S::make_row_id(scope, *version);
            let notification = crate::versioned::update_notification_when_watched(
                watched,
                table_id.clone(),
                || Self::notification_row(&latest_row),
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
        if S::DEFERRED_EFFECTS_BEFORE_ROW {
            self.stage_persist_effect(actor, scope, version, &entity, effect).await;
            self.write_replaced_row(previous, row_key, entity, error_context).await?;
        } else {
            self.write_replaced_row(previous, row_key, entity.clone(), error_context)
                .await?;
            self.stage_persist_effect(actor, scope, version, &entity, effect).await;
        }
        crate::versioned::mark_table_pending_write(&self.core, scope);
        Ok(())
    }
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
            let (previous, latest_row) = crate::versioned::require_live_row(
                self.load_mutation_target(Some(actor), pk_value).await?,
                &pk_name,
                pk_value,
            )?;
            let coerced = crate::versioned::coerce_update_row(updates, &self.core.schema_ref())?;
            let new_fields =
                crate::versioned::merge_row_updates(S::row_fields(&latest_row), coerced);
            if new_fields == *S::row_fields(&latest_row) {
                if !S::ONE_STATEMENT_COMMIT {
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
                    crate::versioned::update_notification(
                        table_id,
                        Self::notification_row(&latest_row),
                        new_row,
                        pk_col,
                    )
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
            let Some((previous, latest_row)) = crate::versioned::optional_live_row(
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
            let values = S::row_fields(&latest_row).values.clone();
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
