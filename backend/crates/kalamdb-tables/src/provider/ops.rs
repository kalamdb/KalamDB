//! Typed reads and writes on [`KalamTableProvider`].
//!
//! Callers outside this crate use these methods. The user and shared bodies
//! stay in the versioned engine, and stream writes stay on [`StreamTableEngine`].

use std::sync::Arc;

use datafusion::{logical_expr::Expr, scalar::ScalarValue};
use kalamdb_commons::{
    ids::{SharedTableRowId, UserTableRowId, VersionId},
    models::{rows::Row, UserId},
    websocket::ChangeNotification,
    PolicyCommand, Role, TableId,
};
use kalamdb_rls::{AuthorizationCacheMetrics, BoundLiveAuthorization};

use crate::{
    error::KalamDbError,
    shared_tables::{SharedTableIndexedStore, SharedTableProvider, SharedTableRow},
    user_tables::{UserTableIndexedStore, UserTableProvider, UserTableRow},
    utils::{
        base::{BaseTableProvider, TableProviderCore},
        row_utils::system_user_id,
    },
    versioned::DeferredMutation,
};

use super::KalamTableProvider;

fn not_user() -> KalamDbError {
    KalamDbError::InvalidOperation("table provider is not a user table".into())
}

fn not_shared() -> KalamDbError {
    KalamDbError::InvalidOperation("table provider is not a shared table".into())
}

impl KalamTableProvider {
    pub fn user(core: Arc<TableProviderCore>, store: Arc<UserTableIndexedStore>) -> Self {
        Self::from_user(UserTableProvider::new(core, store))
    }

    pub fn shared(core: Arc<TableProviderCore>, store: Arc<SharedTableIndexedStore>) -> Self {
        Self::from_shared(SharedTableProvider::new(core, store))
    }

    pub fn is_user(&self) -> bool {
        self.as_user().is_some()
    }

    pub fn is_shared(&self) -> bool {
        self.as_shared().is_some()
    }

    pub fn is_stream(&self) -> bool {
        self.as_stream().is_some()
    }

    pub fn user_store(&self) -> Option<Arc<UserTableIndexedStore>> {
        self.as_user().map(UserTableProvider::store)
    }

    pub fn shared_store(&self) -> Option<Arc<SharedTableIndexedStore>> {
        self.as_shared().map(SharedTableProvider::store)
    }

    pub fn primary_key_field_name(&self) -> &str {
        if let Some(provider) = self.as_user() {
            return provider.primary_key_field_name();
        }
        if let Some(provider) = self.as_shared() {
            return provider.primary_key_field_name();
        }
        self.as_stream()
            .map(crate::stream_tables::StreamTableEngine::primary_key_name)
            .unwrap_or("")
    }

    pub async fn insert_user_batch_with_versions(
        &self,
        user_id: &UserId,
        rows: Vec<Row>,
        versions: &[VersionId],
    ) -> Result<Vec<UserTableRowId>, KalamDbError> {
        self.as_user()
            .ok_or_else(not_user)?
            .insert_batch_with_versions(user_id, rows, versions)
            .await
    }

    pub async fn insert_shared_batch_with_versions(
        &self,
        actor_user_id: Option<&UserId>,
        rows: Vec<Row>,
        versions: &[VersionId],
    ) -> Result<Vec<SharedTableRowId>, KalamDbError> {
        self.as_shared()
            .ok_or_else(not_shared)?
            .insert_batch_with_versions(actor_user_id, rows, versions)
            .await
    }

    pub async fn insert_user_encoded_fields_with_versions(
        &self,
        user_id: &UserId,
        payloads: &[Vec<u8>],
        versions: &[VersionId],
    ) -> Result<usize, KalamDbError> {
        self.as_user()
            .ok_or_else(not_user)?
            .insert_encoded_fields_with_versions(user_id, payloads, versions)
            .await
    }

    pub async fn insert_shared_encoded_fields_with_versions(
        &self,
        actor_user_id: Option<&UserId>,
        payloads: &[Vec<u8>],
        versions: &[VersionId],
    ) -> Result<usize, KalamDbError> {
        self.as_shared()
            .ok_or_else(not_shared)?
            .insert_encoded_fields_with_versions(actor_user_id, payloads, versions)
            .await
    }

    pub async fn update_user_batch_deferred(
        &self,
        user_id: &UserId,
        updates: &[(String, Row, VersionId)],
    ) -> Result<Vec<DeferredMutation<UserTableRowId>>, KalamDbError> {
        self.as_user()
            .ok_or_else(not_user)?
            .update_batch_by_pk_values_deferred(user_id, updates)
            .await
    }

    pub async fn delete_user_batch_deferred(
        &self,
        user_id: &UserId,
        pk_values: &[String],
        versions: &[VersionId],
    ) -> Result<Vec<DeferredMutation<UserTableRowId>>, KalamDbError> {
        self.as_user()
            .ok_or_else(not_user)?
            .delete_batch_by_pk_values_deferred(user_id, pk_values, versions)
            .await
    }

    pub async fn update_shared_batch_deferred(
        &self,
        updates: &[(String, Row, VersionId)],
    ) -> Result<Vec<DeferredMutation<SharedTableRowId>>, KalamDbError> {
        self.as_shared()
            .ok_or_else(not_shared)?
            .update_batch_by_pk_values_deferred(updates)
            .await
    }

    pub async fn delete_shared_batch_deferred(
        &self,
        pk_values: &[String],
        versions: &[VersionId],
    ) -> Result<Vec<DeferredMutation<SharedTableRowId>>, KalamDbError> {
        self.as_shared()
            .ok_or_else(not_shared)?
            .delete_batch_by_pk_values_deferred(pk_values, versions)
            .await
    }

    pub async fn validate_user_insert_rows<'a, I>(
        &self,
        user_id: &UserId,
        rows: I,
    ) -> Result<(), KalamDbError>
    where
        I: IntoIterator<Item = &'a Row>,
    {
        self.as_user()
            .ok_or_else(not_user)?
            .validate_insert_batch_rows(user_id, rows)
            .await?;
        Ok(())
    }

    pub async fn ensure_insert_pk_unique(
        &self,
        scope: Option<&UserId>,
        row: &Row,
    ) -> Result<(), KalamDbError> {
        if let Some(provider) = self.as_user() {
            return crate::utils::base::ensure_unique_pk_value(provider, scope, row).await;
        }
        if let Some(provider) = self.as_shared() {
            return crate::utils::base::ensure_unique_pk_value(provider, scope, row).await;
        }
        Err(not_user())
    }

    pub async fn insert_user_batch_deferred_prevalidated(
        &self,
        user_id: &UserId,
        rows: Vec<Row>,
        versions: &[VersionId],
    ) -> Result<Vec<(UserTableRowId, Option<ChangeNotification>)>, KalamDbError> {
        self.as_user()
            .ok_or_else(not_user)?
            .insert_batch_deferred_prevalidated_with_versions(user_id, rows, versions)
            .await
    }

    pub async fn insert_shared_batch_deferred_prevalidated(
        &self,
        rows: Vec<Row>,
        versions: &[VersionId],
    ) -> Result<Vec<(SharedTableRowId, Option<ChangeNotification>)>, KalamDbError> {
        self.as_shared()
            .ok_or_else(not_shared)?
            .insert_batch_deferred_prevalidated_with_versions(rows, versions)
            .await
    }

    pub async fn insert_user_deferred_prevalidated(
        &self,
        user_id: &UserId,
        row_data: Row,
        version: VersionId,
    ) -> Result<(UserTableRowId, Option<ChangeNotification>), KalamDbError> {
        self.as_user()
            .ok_or_else(not_user)?
            .insert_deferred_prevalidated(user_id, row_data, version)
            .await
    }

    pub async fn insert_user_over_hot_deferred(
        &self,
        user_id: &UserId,
        row_data: Row,
        version: VersionId,
        previous: (UserTableRowId, UserTableRow),
    ) -> Result<(UserTableRowId, Option<ChangeNotification>), KalamDbError> {
        self.as_user()
            .ok_or_else(not_user)?
            .insert_over_hot_deferred(user_id, row_data, version, previous)
            .await
    }

    pub async fn insert_shared_deferred_prevalidated(
        &self,
        row_data: Row,
        version: VersionId,
    ) -> Result<(SharedTableRowId, Option<ChangeNotification>), KalamDbError> {
        self.as_shared()
            .ok_or_else(not_shared)?
            .insert_deferred_prevalidated(row_data, version)
            .await
    }

    pub async fn insert_shared_over_hot_deferred(
        &self,
        row_data: Row,
        version: VersionId,
        previous: (SharedTableRowId, SharedTableRow),
    ) -> Result<(SharedTableRowId, Option<ChangeNotification>), KalamDbError> {
        self.as_shared()
            .ok_or_else(not_shared)?
            .insert_over_hot_deferred(row_data, version, previous)
            .await
    }

    pub async fn update_user_deferred_with_hot(
        &self,
        user_id: &UserId,
        pk_value: &str,
        updates: Row,
        version: VersionId,
        known_hot: Option<(UserTableRowId, UserTableRow)>,
    ) -> Result<Option<(UserTableRowId, UserTableRow, Option<ChangeNotification>)>, KalamDbError>
    {
        self.as_user()
            .ok_or_else(not_user)?
            .update_by_pk_value_deferred_with_hot(user_id, pk_value, updates, version, known_hot)
            .await
    }

    pub async fn delete_user_by_pk_value_deferred(
        &self,
        user_id: &UserId,
        pk_value: &str,
        version: VersionId,
    ) -> Result<Option<(UserTableRowId, Option<ChangeNotification>)>, KalamDbError> {
        self.as_user()
            .ok_or_else(not_user)?
            .delete_by_pk_value_deferred(user_id, pk_value, version)
            .await
    }

    pub async fn delete_user_deferred_with_hot(
        &self,
        user_id: &UserId,
        pk_value: &str,
        version: VersionId,
        known_hot: Option<(UserTableRowId, UserTableRow)>,
    ) -> Result<Option<(UserTableRowId, UserTableRow, Option<ChangeNotification>)>, KalamDbError>
    {
        self.as_user()
            .ok_or_else(not_user)?
            .delete_by_pk_value_deferred_with_hot(user_id, pk_value, version, known_hot)
            .await
    }

    pub async fn update_shared_deferred_with_hot(
        &self,
        pk_value: &str,
        updates: Row,
        version: VersionId,
        known_hot: Option<(SharedTableRowId, SharedTableRow)>,
    ) -> Result<Option<(SharedTableRowId, SharedTableRow, Option<ChangeNotification>)>, KalamDbError>
    {
        self.as_shared()
            .ok_or_else(not_shared)?
            .update_by_pk_value_deferred_with_hot(pk_value, updates, version, known_hot)
            .await
    }

    pub async fn delete_shared_deferred_with_hot(
        &self,
        pk_value: &str,
        version: VersionId,
        known_hot: Option<(SharedTableRowId, SharedTableRow)>,
    ) -> Result<Option<(SharedTableRowId, SharedTableRow, Option<ChangeNotification>)>, KalamDbError>
    {
        self.as_shared()
            .ok_or_else(not_shared)?
            .delete_by_pk_value_deferred_with_hot(pk_value, version, known_hot)
            .await
    }

    pub async fn find_user_by_pk(
        &self,
        user_id: &UserId,
        pk_value: &ScalarValue,
    ) -> Result<Option<(UserTableRowId, UserTableRow)>, KalamDbError> {
        self.as_user().ok_or_else(not_user)?.find_by_pk(user_id, pk_value).await
    }

    pub async fn find_by_pk(
        &self,
        pk_value: &ScalarValue,
    ) -> Result<Option<(SharedTableRowId, SharedTableRow)>, KalamDbError> {
        self.as_shared().ok_or_else(not_shared)?.find_by_pk(pk_value).await
    }

    pub async fn shared_row_by_pk_value(
        &self,
        pk_value: &str,
    ) -> Result<Option<Row>, KalamDbError> {
        self.as_shared().ok_or_else(not_shared)?.row_by_pk_value(pk_value).await
    }

    pub async fn check_shared_rows_authorized(
        &self,
        user_id: &UserId,
        role: Role,
        command: PolicyCommand,
        check: bool,
        rows: &[Row],
        snapshot_commit_seq: Option<VersionId>,
    ) -> Result<(), KalamDbError> {
        self.as_shared()
            .ok_or_else(not_shared)?
            .check_rows_authorized(user_id, role, command, check, rows, snapshot_commit_seq)
            .await
    }

    pub async fn bind_shared_live_authorization(
        &self,
        user_id: &UserId,
        role: Role,
    ) -> Result<BoundLiveAuthorization, KalamDbError> {
        self.as_shared()
            .ok_or_else(not_shared)?
            .bind_live_authorization(user_id, role)
            .await
    }

    pub async fn shared_select_authorizes_file_path(
        &self,
        user_id: &UserId,
        role: Role,
        subfolder: &str,
        stored_name: &str,
    ) -> Result<bool, KalamDbError> {
        self.as_shared()
            .ok_or_else(not_shared)?
            .select_authorizes_file_path(user_id, role, subfolder, stored_name)
            .await
    }

    pub fn authorization_cache_metrics(&self) -> AuthorizationCacheMetrics {
        self.as_shared()
            .expect("authorization cache metrics require a shared table")
            .authorization_cache_metrics()
    }

    pub async fn patch_latest_commit_seq_by_pk(
        &self,
        pk_value: &str,
        version: VersionId,
    ) -> Result<bool, KalamDbError> {
        self.as_shared()
            .ok_or_else(not_shared)?
            .patch_latest_commit_seq_by_pk(pk_value, version)
            .await
    }

    pub async fn scan_with_version_resolution_to_kvs_async(
        &self,
        user_id: &UserId,
        filter: Option<&Expr>,
        since_seq: Option<VersionId>,
        limit: Option<usize>,
        keep_deleted: bool,
        cold_columns: Option<&[String]>,
        snapshot_commit_seq: Option<VersionId>,
    ) -> Result<Vec<(SharedTableRowId, SharedTableRow)>, KalamDbError> {
        let shared = self.as_shared().ok_or_else(not_shared)?;
        BaseTableProvider::scan_with_version_resolution_to_kvs_async(
            shared,
            user_id,
            filter,
            since_seq,
            limit,
            keep_deleted,
            cold_columns,
            snapshot_commit_seq,
        )
        .await
    }

    pub async fn shared_has_live_row(&self) -> Result<bool, KalamDbError> {
        let rows = self
            .scan_with_version_resolution_to_kvs_async(
                system_user_id(),
                None,
                None,
                Some(1),
                false,
                None,
                None,
            )
            .await?;
        Ok(!rows.is_empty())
    }

    pub async fn user_cleanup_row(
        &self,
        table_id: &TableId,
        user_id: &UserId,
        pk_value: &str,
    ) -> Option<Row> {
        let provider = self.as_user()?;
        let pk_name = provider.primary_key_field_name();
        let schema = datafusion::datasource::TableProvider::schema(provider);
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
            Ok(None) => {
                match crate::utils::base::find_row_by_pk(provider, Some(user_id), pk_value).await {
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
                }
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

    pub async fn shared_cleanup_row(&self, table_id: &TableId, pk_value: &str) -> Option<Row> {
        let provider = self.as_shared()?;
        let pk_name = provider.primary_key_field_name();
        let schema = datafusion::datasource::TableProvider::schema(provider);
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
            Ok(None) => match crate::utils::base::find_row_by_pk(provider, None, pk_value).await {
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
}
