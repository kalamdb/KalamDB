//! One versioned table provider. User and shared scopes supply the row types.

use std::{collections::HashMap, sync::Arc};

use datafusion::{error::DataFusionError, scalar::ScalarValue};
use kalamdb_commons::{
    ids::{SeqId, VersionId},
    models::{rows::Row, UserId},
};
use kalamdb_store::{EntityStore, IndexedEntityStore};
use kalamdb_vector::VectorHotOp;

use crate::{
    error::KalamDbError,
    manifest::manifest_helpers::ensure_manifest_ready,
    utils::{
        base::{self, BaseTableProvider, TableProviderCore},
        row_utils::ScanRow,
    },
    versioned::pk_index::VersionedPkIndex,
};

use super::scope::{MutationGuard, SharedScope, TableScope, UserScope};

#[derive(Clone)]
pub(crate) struct VersionedTable<S: TableScope> {
    pub(crate) core: Arc<TableProviderCore>,
    pub(crate) store: Arc<IndexedEntityStore<S::RowId, S::Row>>,
    pub(super) pk_index: VersionedPkIndex,
    pub(super) vector_columns: Vec<(String, u32)>,
    vector_stores: HashMap<String, Arc<IndexedEntityStore<S::VectorOpId, VectorHotOp>>>,
    pub(crate) authorization: S::Auth,
}

pub(crate) type UserTableProvider = VersionedTable<UserScope>;
pub(crate) type SharedTableProvider = VersionedTable<SharedScope>;

impl MutationGuard for VersionedTable<UserScope> {
    type Guard = ();

    fn begin_guard(&self) {}
}

impl MutationGuard for VersionedTable<SharedScope> {
    type Guard = kalamdb_rls::AuthorizationMutationGuard;

    fn begin_guard(&self) -> Self::Guard {
        self.authorization.begin_mutation()
    }
}

impl<S: TableScope> VersionedTable<S>
where
    Self: MutationGuard,
{
    pub fn new(
        core: Arc<TableProviderCore>,
        store: Arc<IndexedEntityStore<S::RowId, S::Row>>,
    ) -> Self {
        let pk_index = S::new_pk_index(core.table_id(), core.primary_key_field_name());
        let vector_columns = base::embedding_columns(core.table_def());
        let vector_stores = crate::utils::vector_staging::build_vector_store_map(
            store.backend().clone(),
            core.table_id(),
            &vector_columns,
            S::new_vector_store,
        );
        if log::log_enabled!(log::Level::Debug) {
            let field_names: Vec<_> =
                core.schema().fields().iter().map(|field| field.name()).collect();
            log::debug!(
                "{}: Created for {} with schema fields: {:?}",
                S::PROVIDER_NAME,
                core.table_id(),
                field_names
            );
        }
        Self {
            core,
            store,
            pk_index,
            vector_columns,
            vector_stores,
            authorization: S::new_auth(),
        }
    }

    pub fn primary_key_field_name(&self) -> &str {
        self.core.primary_key_field_name()
    }

    pub fn store(&self) -> Arc<IndexedEntityStore<S::RowId, S::Row>> {
        Arc::clone(&self.store)
    }

    pub(crate) fn is_auto_increment_pk(&self) -> bool {
        self.core.is_auto_increment_pk()
    }

    pub(crate) fn has_vector_columns(&self) -> bool {
        !self.vector_columns.is_empty()
    }

    pub(super) fn notification_row(entity: &S::Row) -> Row {
        base::build_notification_row(
            S::row_fields(entity),
            S::row_version(entity),
            S::row_deleted(entity),
        )
    }

    pub(super) fn storage_user<'a>(
        &self,
        actor: Option<&'a UserId>,
    ) -> Result<Option<&'a UserId>, KalamDbError> {
        S::storage_user(actor)
    }

    pub(super) async fn stage_vector_upsert(
        &self,
        actor: Option<&UserId>,
        version: VersionId,
        row: &Row,
    ) -> Result<(), KalamDbError> {
        if self.vector_columns.is_empty() {
            return Ok(());
        }
        let seq = SeqId::from_i64(version.as_i64());
        let ops_by_column = crate::utils::vector_staging::build_vector_upsert_batch_ops(
            self.core.table_id(),
            self.primary_key_field_name(),
            &self.vector_columns,
            std::iter::once((seq, row)),
            |(_, row)| row,
            |(seq, _), pk| S::vector_op_id(actor, *seq, pk.to_string()),
        )?;
        crate::utils::vector_staging::stage_vector_ops_by_column(
            &self.vector_stores,
            ops_by_column,
            "stage vector upsert",
        )
        .await
    }

    pub(super) async fn stage_vector_upsert_batch(
        &self,
        actor: Option<&UserId>,
        entries: &[(S::RowId, S::Row)],
    ) -> Result<(), KalamDbError> {
        if self.vector_columns.is_empty() || entries.is_empty() {
            return Ok(());
        }
        let ops_by_column = crate::utils::vector_staging::build_vector_upsert_batch_ops(
            self.core.table_id(),
            self.primary_key_field_name(),
            &self.vector_columns,
            entries.iter(),
            |(_, entity)| S::row_fields(entity),
            |(row_key, _), pk| {
                S::vector_op_id(
                    actor,
                    SeqId::from_i64(S::row_id_version(row_key).as_i64()),
                    pk.to_string(),
                )
            },
        )?;
        crate::utils::vector_staging::stage_vector_ops_by_column(
            &self.vector_stores,
            ops_by_column,
            "batch stage vector upsert",
        )
        .await
    }

    pub(super) async fn stage_vector_delete(
        &self,
        actor: Option<&UserId>,
        version: VersionId,
        pk: &str,
    ) -> Result<(), KalamDbError> {
        if self.vector_columns.is_empty() {
            return Ok(());
        }
        let seq = SeqId::from_i64(version.as_i64());
        let ops_by_column = crate::utils::vector_staging::build_vector_delete_ops(
            self.core.table_id(),
            &self.vector_columns,
            pk,
            |primary_key| S::vector_op_id(actor, seq, primary_key.to_string()),
        );
        crate::utils::vector_staging::stage_vector_ops_by_column(
            &self.vector_stores,
            ops_by_column,
            "stage vector delete",
        )
        .await
    }

    pub(crate) async fn finish_planned_insert(
        &self,
        actor: Option<&UserId>,
        entries: &[(S::RowId, S::Row)],
    ) {
        if entries.is_empty() {
            return;
        }
        let Ok(scope) = self.storage_user(actor) else {
            return;
        };
        if let Err(error) = self.stage_vector_upsert_batch(scope, entries).await {
            crate::versioned::warn_vector_upsert_batch(self.core.table_id(), scope, &error);
        }
        crate::versioned::mark_table_pending_write(&self.core, scope);
    }

    pub(crate) async fn finish_planned_scope(
        &self,
        actor: Option<&UserId>,
        ops: &[crate::versioned::PlannedScopeFinish],
    ) {
        let Ok(scope) = self.storage_user(actor) else {
            return;
        };
        if !self.vector_columns.is_empty() {
            for op in ops {
                match op {
                    crate::versioned::PlannedScopeFinish::Upsert { version, row } => {
                        if let Err(error) = self.stage_vector_upsert(scope, *version, row).await {
                            crate::versioned::warn_vector_upsert(
                                self.core.table_id(),
                                scope,
                                *version,
                                &error,
                            );
                        }
                    },
                    crate::versioned::PlannedScopeFinish::Delete { version, pk } => {
                        if let Err(error) = self.stage_vector_delete(scope, *version, pk).await {
                            crate::versioned::warn_vector_delete(
                                self.core.table_id(),
                                scope,
                                *version,
                                pk,
                                &error,
                            );
                        }
                    },
                }
            }
        }
        crate::versioned::mark_table_pending_write(&self.core, scope);
    }

    pub(super) async fn latest_hot_pk_entry(
        &self,
        actor: Option<&UserId>,
        pk_value: &ScalarValue,
    ) -> Result<Option<(S::RowId, S::Row)>, KalamDbError> {
        self.latest_hot_pk_entry_maybe_selected(actor, pk_value, None).await
    }

    pub(super) async fn latest_hot_pk_entry_maybe_selected(
        &self,
        actor: Option<&UserId>,
        pk_value: &ScalarValue,
        storage_ordinals: Option<&[usize]>,
    ) -> Result<Option<(S::RowId, S::Row)>, KalamDbError> {
        let scope = self.storage_user(actor)?;
        let prefix = S::pk_prefix(&self.pk_index, scope, pk_value);
        crate::versioned::latest_hot_by_prefix(&self.store, prefix, storage_ordinals).await
    }

    pub(super) async fn hot_pk_lookup(
        &self,
        actor: Option<&UserId>,
        pk_value: &ScalarValue,
    ) -> Result<crate::versioned::HotPkLookup<S::RowId, S::Row>, KalamDbError> {
        Ok(crate::versioned::classify_hot_pk(
            self.latest_hot_pk_entry(actor, pk_value).await?,
            S::row_deleted,
        ))
    }

    pub(crate) async fn load_mutation_target(
        &self,
        actor: Option<&UserId>,
        pk_value: &str,
    ) -> Result<crate::versioned::MutationTarget<S::RowId, S::Row>, KalamDbError>
    where
        Self: super::scan::ScopeScan<RowId = S::RowId, Row = S::Row, Context = S::ScanContext>
            + BaseTableProvider<S::RowId, S::Row>,
    {
        let scope = self.storage_user(actor)?;
        let pk_name = self.primary_key_field_name();
        let pk_scalar =
            crate::versioned::pk_text_to_scalar(&self.core.schema_ref(), pk_name, pk_value)?;
        let lookup = self.hot_pk_lookup(actor, &pk_scalar).await?;
        let cold = if matches!(lookup, crate::versioned::HotPkLookup::Missing) {
            base::find_row_by_pk(self, scope, pk_value).await?.map(|(_, row)| row)
        } else {
            None
        };
        kalamdb_observability::dml_stage::record_baseline_decode();
        Ok(crate::versioned::mutation_target_from_hot(lookup, cold))
    }

    pub async fn find_by_pk(
        &self,
        actor: Option<&UserId>,
        pk_value: &ScalarValue,
    ) -> Result<Option<(S::RowId, S::Row)>, KalamDbError> {
        Ok(crate::versioned::live_indexed_row(
            self.latest_hot_pk_entry(actor, pk_value).await?,
            S::row_deleted,
        ))
    }

    pub(super) async fn insert_predecessor(
        &self,
        actor: Option<&UserId>,
        row_data: &Row,
    ) -> Result<Option<(S::RowId, S::Row)>, KalamDbError>
    where
        Self: super::scan::ScopeScan<RowId = S::RowId, Row = S::Row, Context = S::ScanContext>
            + BaseTableProvider<S::RowId, S::Row>,
    {
        let scope = self.storage_user(actor)?;
        let pk_name = self.primary_key_field_name();
        let Some((pk_value, pk_str)) =
            crate::versioned::insert_pk_parts(self.core.is_auto_increment_pk(), row_data, pk_name)?
        else {
            return Ok(None);
        };
        let prefix = S::pk_prefix(&self.pk_index, scope, pk_value);
        match crate::versioned::hot_insert_predecessor(&self.store, &prefix, &pk_str, pk_name)? {
            crate::versioned::InsertPredecessor::Tombstone(key, row) => Ok(Some((key, row))),
            crate::versioned::InsertPredecessor::Missing => {
                base::ensure_pk_absent_in_cold(self, scope, &pk_str).await?;
                Ok(None)
            },
        }
    }

    pub(super) async fn write_replaced_row(
        &self,
        previous: Option<(S::RowId, S::Row)>,
        row_key: S::RowId,
        entity: S::Row,
        error_context: &str,
    ) -> Result<(), KalamDbError> {
        crate::versioned::write_replaced_row(&self.store, previous, row_key, entity, error_context)
            .await
    }

    pub async fn validate_insert_batch_rows<'a, I>(
        &self,
        actor: Option<&UserId>,
        rows: I,
    ) -> Result<HashMap<String, (S::RowId, S::Row)>, KalamDbError>
    where
        I: IntoIterator<Item = &'a Row>,
    {
        let scope = self.storage_user(actor)?;
        let pk_name = self.primary_key_field_name();
        let pk_values = crate::versioned::batch_primary_keys(pk_name, rows)?;
        if pk_values.is_empty() {
            return Ok(HashMap::new());
        }
        let prefixes = pk_values
            .iter()
            .map(|(pk_str, pk_value)| {
                (pk_str.clone(), S::pk_prefix(&self.pk_index, scope, pk_value))
            })
            .collect();
        crate::versioned::tombstones_replacing_insert_keys(
            Arc::clone(&self.store),
            &self.core,
            scope,
            pk_name,
            prefixes,
            Some("hot"),
            Some("cold"),
        )
        .await
    }

    pub(crate) async fn collect_insert_batch_rows(
        &self,
        actor: Option<&UserId>,
        rows: Vec<Row>,
        versions: &[VersionId],
        validate_unique_pk: bool,
    ) -> Result<(Vec<(S::RowId, S::Row)>, Vec<kalamdb_store::Operation>), KalamDbError> {
        if rows.is_empty() {
            return Ok((Vec::new(), Vec::new()));
        }
        let _guard = self.begin_guard();
        let scope = self.storage_user(actor)?;
        ensure_manifest_ready(&self.core, scope, S::PROVIDER_NAME)?;
        let coerced_rows = crate::versioned::coerce_row_batch(rows, &self.core.schema_ref())?;
        crate::utils::datafusion_dml::validate_not_null_with_set(
            self.core.non_null_columns(),
            &coerced_rows,
        )?;
        let row_count = coerced_rows.len();
        let replaced_tombstones = if validate_unique_pk {
            self.validate_insert_batch_rows(actor, coerced_rows.iter()).await?
        } else {
            HashMap::new()
        };
        crate::versioned::require_version_count(versions.len(), row_count)?;
        let mut entries = Vec::with_capacity(row_count);
        for (row_data, version) in coerced_rows.into_iter().zip(versions.iter().copied()) {
            entries.push((
                S::make_row_id(scope, version),
                S::make_row(scope, version, false, row_data),
            ));
        }
        let mut operations = Vec::new();
        crate::versioned::append_insert_batch_ops(
            &self.store,
            &entries,
            replaced_tombstones,
            self.primary_key_field_name(),
            S::row_fields,
            &mut operations,
        )?;
        Ok((entries, operations))
    }

    pub(super) async fn persist_insert_batch_rows(
        &self,
        actor: Option<&UserId>,
        rows: Vec<Row>,
        versions: &[VersionId],
        validate_unique_pk: bool,
    ) -> Result<Vec<(S::RowId, S::Row)>, KalamDbError> {
        let (entries, operations) = self
            .collect_insert_batch_rows(actor, rows, versions, validate_unique_pk)
            .await?;
        if entries.is_empty() {
            return Ok(entries);
        }
        self.store.commit_operations(operations).map_err(|error| {
            KalamDbError::InvalidOperation(format!("Failed to batch insert table rows: {error}"))
        })?;
        self.finish_planned_insert(actor, &entries).await;
        if S::ONE_STATEMENT_COMMIT {
            let scope = self.storage_user(actor)?;
            log::debug!(
                "Batch inserted {} {} table rows for user {} with _seq range [{}, {}]",
                entries.len(),
                S::LABEL,
                scope.map(|user| user.as_str()).unwrap_or(""),
                entries.first().map(|(key, _)| S::row_id_version(key).as_i64()).unwrap_or(0),
                entries.last().map(|(key, _)| S::row_id_version(key).as_i64()).unwrap_or(0)
            );
        }
        Ok(entries)
    }

    pub async fn insert_encoded_fields_with_versions(
        &self,
        actor: Option<&UserId>,
        payloads: &[Vec<u8>],
        versions: &[VersionId],
    ) -> Result<usize, KalamDbError>
    where
        Self: super::scan::ScopeScan<RowId = S::RowId, Row = S::Row, Context = S::ScanContext>
            + BaseTableProvider<S::RowId, S::Row>,
    {
        if payloads.is_empty() {
            return Ok(0);
        }
        if crate::versioned::table_has_file_columns(self.core.table_def()) {
            let rows = crate::versioned::rows_from_field_payloads(
                payloads,
                self.core.storage_schema().as_ref(),
            )?;
            return Ok(self.insert_batch_with_versions(actor, rows, versions).await?.len());
        }
        let entries = self.persist_encoded_field_rows(actor, payloads, versions).await?;
        let row_count = entries.len();
        let scope = self.storage_user(actor)?;
        crate::versioned::fanout_insert_notifications(&self.core, scope, actor, || {
            entries.iter().map(|(_, entity)| Self::notification_row(entity)).collect()
        })
        .await;
        Ok(row_count)
    }

    async fn persist_encoded_field_rows(
        &self,
        actor: Option<&UserId>,
        payloads: &[Vec<u8>],
        versions: &[VersionId],
    ) -> Result<Vec<(S::RowId, S::Row)>, KalamDbError> {
        let scope = self.storage_user(actor)?;
        ensure_manifest_ready(&self.core, scope, S::PROVIDER_NAME)?;
        let storage = Arc::clone(self.core.storage_schema());
        let arrow = self.core.schema_ref();
        let decoded = crate::versioned::decode_ordinal_insert(
            payloads,
            storage.as_ref(),
            &arrow,
            self.core.non_null_columns(),
            versions,
        )?;
        if !S::PREENCODE_ORDINAL {
            let rows = decoded
                .columns
                .into_iter()
                .map(|columns| Row::from_aligned(&decoded.names, columns))
                .collect();
            return self.persist_insert_batch_rows(actor, rows, versions, true).await;
        }
        let names = decoded.names;
        let decoded_columns = decoded.columns;
        let pk_name = self.primary_key_field_name();
        let pk_ordinal = storage.fields.iter().position(|field| field.name == pk_name);
        let pk_rows: Vec<Row> = decoded_columns
            .iter()
            .map(|columns| {
                let mut values = std::collections::BTreeMap::new();
                if let Some(ordinal) = pk_ordinal {
                    if let Some(value) = columns.get(ordinal) {
                        values.insert(pk_name.to_string(), value.clone());
                    }
                }
                Row::new(values)
            })
            .collect();
        let replaced_tombstones = self.validate_insert_batch_rows(actor, pk_rows.iter()).await?;
        if !replaced_tombstones.is_empty() {
            let rows = decoded_columns
                .into_iter()
                .map(|columns| Row::from_aligned(&names, columns))
                .collect();
            return self.persist_insert_batch_rows(actor, rows, versions, false).await;
        }
        let mut entries = Vec::with_capacity(decoded_columns.len());
        let mut encoded_values = Vec::with_capacity(decoded_columns.len());
        for (columns, version) in decoded_columns.into_iter().zip(versions.iter().copied()) {
            let bytes = kalamdb_serialization::encode_user_row_from_columns(
                false,
                &columns,
                storage.as_ref(),
            )
            .map_err(|error| {
                KalamDbError::InvalidOperation(format!("ordinal insert encode failed: {error}"))
            })?
            .into_bytes();
            encoded_values.push(bytes);
            entries.push((
                S::make_row_id(scope, version),
                S::make_row(scope, version, false, Row::from_aligned(&names, columns)),
            ));
        }
        self.store.insert_batch_preencoded(&entries, encoded_values).map_err(|error| {
            KalamDbError::InvalidOperation(format!("Failed to batch insert table rows: {error}"))
        })?;
        if let Err(error) = self.stage_vector_upsert_batch(scope, &entries).await {
            crate::versioned::warn_vector_upsert_batch(self.core.table_id(), scope, &error);
        }
        crate::versioned::mark_table_pending_write(&self.core, scope);
        Ok(entries)
    }

    pub(super) fn delete_notification(
        &self,
        table_id: kalamdb_commons::TableId,
        row: Row,
    ) -> kalamdb_commons::websocket::ChangeNotification {
        let mut notification =
            kalamdb_commons::websocket::ChangeNotification::delete_soft(table_id, row);
        if !S::ONE_STATEMENT_COMMIT {
            notification.pk_columns = vec![self.primary_key_field_name().to_string()];
        }
        notification
    }

    pub(super) fn row_from_parquet(
        &self,
        user_id: &UserId,
        row_data: crate::utils::version_resolution::ParquetRowData,
    ) -> Result<(S::RowId, S::Row), DataFusionError> {
        let _ = self;
        Ok(S::row_from_parquet(user_id, &row_data))
    }

    fn _scan_row(row: &S::Row) -> &Row
    where
        S::Row: ScanRow,
    {
        row.row()
    }
}
