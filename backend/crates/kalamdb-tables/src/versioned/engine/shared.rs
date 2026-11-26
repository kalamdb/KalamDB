//! Shared-scope behavior: row-level security, write routing, and shared-only reads.

use datafusion::{
    error::{DataFusionError, Result as DataFusionResult},
    execution::context::SessionState,
    scalar::ScalarValue,
};
use kalamdb_commons::{
    ids::VersionId,
    models::{rows::Row, Role, UserId},
    NotLeaderError, PolicyCommand,
};
use kalamdb_rls::{BoundAuthorization, BoundLiveAuthorization, BoundTablePolicies};
use kalamdb_session_datafusion::RlsCommandContext;
use kalamdb_transactions::extract_transaction_query_context;

use crate::{
    error::KalamDbError,
    utils::base::{self, BaseTableProvider},
};

use super::{
    provider::VersionedTable,
    scope::{SharedScanContext, SharedScope},
};

impl VersionedTable<SharedScope> {
    pub(super) fn session_state_with_rls_command(
        state: &dyn datafusion::catalog::Session,
        command: PolicyCommand,
    ) -> DataFusionResult<SessionState> {
        let mut state = state
            .as_any()
            .downcast_ref::<SessionState>()
            .ok_or_else(|| DataFusionError::Execution("Expected SessionState".to_string()))?
            .clone();
        state
            .config_mut()
            .options_mut()
            .extensions
            .insert(RlsCommandContext { command });
        Ok(state)
    }

    pub(super) async fn bind_authorization(
        &self,
        policies: &BoundTablePolicies,
        snapshot_commit_seq: Option<VersionId>,
    ) -> Result<BoundAuthorization, KalamDbError> {
        self.authorization.bind_authorization(self, policies, snapshot_commit_seq).await
    }

    pub(super) fn bind_policies(
        &self,
        user_id: &UserId,
        role: Role,
        command: PolicyCommand,
        check: bool,
    ) -> Result<BoundTablePolicies, KalamDbError> {
        self.authorization
            .bind_policies(self.core.as_ref(), user_id, role, command, check)
    }

    pub(super) async fn ensure_rows_authorized(
        &self,
        policies: &BoundTablePolicies,
        rows: &[Row],
        snapshot_commit_seq: Option<VersionId>,
        operation: &str,
    ) -> DataFusionResult<()> {
        self.authorization
            .ensure_rows_authorized(self, policies, rows, snapshot_commit_seq, operation)
            .await
    }

    pub async fn check_rows_authorized(
        &self,
        user_id: &UserId,
        role: Role,
        command: PolicyCommand,
        check: bool,
        rows: &[Row],
        snapshot_commit_seq: Option<VersionId>,
    ) -> Result<(), KalamDbError> {
        self.authorization
            .check_rows_authorized(self, user_id, role, command, check, rows, snapshot_commit_seq)
            .await
    }

    pub async fn bind_live_authorization(
        &self,
        user_id: &UserId,
        role: Role,
    ) -> Result<BoundLiveAuthorization, KalamDbError> {
        self.authorization.bind_live_authorization(self, user_id, role).await
    }

    pub(super) fn authorization_cold_columns(
        &self,
        scan_context: &SharedScanContext,
        cold_columns: Option<&[String]>,
    ) -> Option<Vec<String>> {
        self.authorization.authorization_cold_columns(self, scan_context, cold_columns)
    }

    pub fn authorization_cache_metrics(&self) -> kalamdb_rls::AuthorizationCacheMetrics {
        self.authorization.cache_metrics()
    }

    pub(super) async fn ensure_shared_write_leader(&self) -> Result<(), KalamDbError> {
        if self.core.services.cluster_coordinator.is_cluster_mode().await {
            let is_leader = self
                .core
                .services
                .cluster_coordinator
                .is_leader_for_shared(self.table_id())
                .await;
            if !is_leader {
                let leader_addr = self
                    .core
                    .services
                    .cluster_coordinator
                    .leader_addr_for_shared(self.table_id())
                    .await;
                return Err(KalamDbError::NotLeader { leader_addr });
            }
        }
        Ok(())
    }

    pub(super) async fn ensure_shared_write_route(
        &self,
        state: &dyn datafusion::catalog::Session,
    ) -> DataFusionResult<()> {
        if extract_transaction_query_context(state).is_some() {
            self.validate_transaction_table_access(state)?;
            return Ok(());
        }
        self.ensure_shared_write_leader().await.map_err(|error| match error {
            KalamDbError::NotLeader { leader_addr } => {
                DataFusionError::External(Box::new(NotLeaderError::new(leader_addr)))
            },
            other => crate::error::into_datafusion(other),
        })
    }

    pub async fn row_by_pk_value(&self, pk_value: &str) -> Result<Option<Row>, KalamDbError> {
        let pk_name = self.primary_key_field_name();
        let pk_scalar = crate::versioned::pk_column_scalar(&self.schema_ref(), pk_name, pk_value)?;
        match self.hot_pk_lookup(None, &pk_scalar).await? {
            crate::versioned::HotPkLookup::Live(_, row) => return Ok(Some(row.fields)),
            crate::versioned::HotPkLookup::Tombstoned(_, _) => return Ok(None),
            crate::versioned::HotPkLookup::Missing => {},
        }
        Ok(base::find_row_by_pk(self, None, pk_value).await?.map(|(_, row)| row.fields))
    }

    pub async fn patch_commit_seq_for_row_key(
        &self,
        row_key: &kalamdb_commons::ids::SharedTableRowId,
        version: VersionId,
    ) -> Result<(), KalamDbError> {
        let _ = version;
        crate::versioned::patch_stored_row(&self.store, row_key, row_key.as_i64()).await
    }

    pub async fn patch_latest_commit_seq_by_pk(
        &self,
        pk_value: &str,
        version: VersionId,
    ) -> Result<bool, KalamDbError> {
        let pk_scalar = crate::versioned::pk_column_scalar(
            &self.schema_ref(),
            self.primary_key_field_name(),
            pk_value,
        )?;
        let Some((row_key, _)) = self.latest_hot_pk_entry(None, &pk_scalar).await? else {
            return Ok(false);
        };
        self.patch_commit_seq_for_row_key(&row_key, version).await?;
        Ok(true)
    }

    pub async fn select_authorizes_file_path(
        &self,
        user_id: &UserId,
        role: Role,
        subfolder: &str,
        stored_name: &str,
    ) -> Result<bool, KalamDbError> {
        let policies = self.bind_policies(user_id, role, PolicyCommand::Select, false)?;
        if policies.bypasses_rls() {
            return Ok(true);
        }
        if policies.is_default_deny() {
            return Ok(false);
        }
        let file_columns = file_column_names(self.core.table_def());
        if file_columns.is_empty() {
            return Ok(false);
        }
        let scan_context = SharedScanContext {
            snapshot_commit_seq: None,
            policies,
        };
        if !self.authorization.pre_authorize_scan(self, &scan_context, None).await? {
            return Ok(false);
        }
        let rows = <Self as base::DeferredMvccScanProvider<_, _>>::scan_kvs_with_context(
            self,
            &scan_context,
            None,
            None,
            None,
            false,
            Some(file_columns.as_slice()),
        )
        .await?;
        for (row_id, row) in rows {
            if !row_contains_file_path(&row.fields, &file_columns, subfolder, stored_name) {
                continue;
            }
            let authorized = self
                .authorization
                .authorize_resolved_rows(self, &scan_context, vec![(row_id, row)])
                .await?;
            if !authorized.is_empty() {
                return Ok(true);
            }
        }
        Ok(false)
    }

    pub(super) fn replicates_shared_writes(&self) -> bool {
        self.core.services.cluster_coordinator.replicates_shared_writes()
    }
}

fn file_column_names(table_def: &kalamdb_commons::models::schemas::TableDefinition) -> Vec<String> {
    table_def
        .columns
        .iter()
        .filter(|column| column.data_type() == kalamdb_commons::KalamDataType::File)
        .map(|column| column.column_name.clone())
        .collect()
}

fn row_contains_file_path(
    row: &Row,
    file_columns: &[String],
    subfolder: &str,
    stored_name: &str,
) -> bool {
    file_columns.iter().any(|column| {
        let Some(value) = row.values.get(column) else {
            return false;
        };
        let json = match value {
            ScalarValue::Utf8(Some(json)) | ScalarValue::LargeUtf8(Some(json)) => json.as_str(),
            _ => return false,
        };
        kalamdb_commons::FileRef::try_from_json(json).is_ok_and(|file_ref| {
            file_ref.sub == subfolder && file_ref.stored_name() == stored_name
        })
    })
}
