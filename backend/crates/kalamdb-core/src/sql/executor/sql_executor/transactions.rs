//! Request transactions, savepoints, and session search path.

use std::sync::Arc;

use datafusion::{execution::context::SessionState, prelude::SessionContext};
use kalamdb_commons::{
    models::{ReadContext, SchemaId, TableId, TransactionId},
    schemas::TableType,
};
use kalamdb_session_datafusion::install_extension;
use kalamdb_sql::classifier::{SqlStatement, SqlStatementKind};
use kalamdb_transactions::{
    primary_key_claims, TransactionQueryContext, TransactionQueryExtension,
};
use uuid::Uuid;

use crate::{
    error::KalamDbError,
    sql::{
        executor::{
            point_read_session_cache_key::PointReadSessionCacheKey,
            request_transaction_state::{
                map_request_transaction_error, AppContextRequestTransactionCoordinator,
                RequestTransactionState,
            },
            PreparedExecutionStatement, SqlExecutor,
        },
        ExecutionContext, ExecutionResult,
    },
    transactions::CoordinatorAccessValidator,
};

fn extract_select_from_table_id(sql: &str, default_namespace: &SchemaId) -> Option<TableId> {
    let lowered = sql.to_ascii_lowercase();
    let from_idx = lowered.find(" from ")?;
    let rest = sql[from_idx + 6..].trim_start();
    let ident: String = rest
        .chars()
        .take_while(|ch| ch.is_ascii_alphanumeric() || *ch == '_' || *ch == '.')
        .collect();
    if ident.is_empty() {
        return None;
    }
    let mut parts = ident.split('.');
    match (parts.next(), parts.next(), parts.next()) {
        (Some(table), None, None) => {
            Some(TableId::from_namespace(default_namespace.clone(), table))
        },
        (Some(namespace), Some(table), None) => Some(TableId::from_strings(namespace, table)),
        _ => None,
    }
}

fn extract_sql_target_table_id(sql: &str, default_namespace: &SchemaId) -> Option<TableId> {
    let trimmed = sql.trim_start();
    if trimmed.len() >= 6 && trimmed[..6].eq_ignore_ascii_case("select") {
        return extract_select_from_table_id(trimmed, default_namespace);
    }
    kalamdb_sql::extract_dml_table_id_fast(sql, default_namespace)
        .or_else(|| kalamdb_sql::extract_dml_table_id(sql, default_namespace))
        .or_else(|| extract_select_from_table_id(sql, default_namespace))
}

impl SqlExecutor {
    pub(super) fn is_ddl_statement(kind: &SqlStatementKind) -> bool {
        matches!(
            kind,
            SqlStatementKind::AlterSchema(_)
                | SqlStatementKind::DropSchema(_)
                | SqlStatementKind::CreateStorage(_)
                | SqlStatementKind::AlterStorage(_)
                | SqlStatementKind::DropStorage(_)
                | SqlStatementKind::CreateTable(_)
                | SqlStatementKind::CreateView(_)
                | SqlStatementKind::AlterTable(_)
                | SqlStatementKind::DropTable(_)
                | SqlStatementKind::CreatePolicy(_)
                | SqlStatementKind::AlterPolicy(_)
                | SqlStatementKind::DropPolicy(_)
                | SqlStatementKind::CreateType(_)
                | SqlStatementKind::AlterType(_)
                | SqlStatementKind::DropType(_)
                | SqlStatementKind::CreateDomain(_)
                | SqlStatementKind::AlterDomain(_)
                | SqlStatementKind::DropDomain(_)
                | SqlStatementKind::CreateProcedure(_)
                | SqlStatementKind::AlterRoutine(_)
                | SqlStatementKind::DropProcedure(_)
                | SqlStatementKind::CommentOn(_)
                | SqlStatementKind::CreateTrigger(_)
                | SqlStatementKind::CreateSchedule(_)
                | SqlStatementKind::DropTrigger(_)
                | SqlStatementKind::DropSchedule(_)
                | SqlStatementKind::AlterTrigger(_)
                | SqlStatementKind::AlterSchedule(_)
                | SqlStatementKind::GrantExecute(_)
                | SqlStatementKind::RevokeExecute(_)
                | SqlStatementKind::CreateSchema(_)
        )
    }

    pub(super) fn request_transaction_state<'a>(
        &self,
        exec_ctx: &'a ExecutionContext,
    ) -> Result<Option<RequestTransactionState<'a>>, KalamDbError> {
        let request_transaction_coordinator =
            AppContextRequestTransactionCoordinator::new(self.app_context.as_ref());
        let mut request_state = RequestTransactionState::from_request_id(exec_ctx.request_id());
        if let Some(state) = request_state.as_mut() {
            state.sync(&request_transaction_coordinator);
        }
        Ok(request_state)
    }

    pub(super) fn active_request_transaction_id(
        &self,
        exec_ctx: &ExecutionContext,
    ) -> Result<Option<TransactionId>, KalamDbError> {
        if let Some(tx_id) = exec_ctx.transaction_id() {
            return Ok(Some(tx_id.clone()));
        }
        Ok(self
            .request_transaction_state(exec_ctx)?
            .and_then(|state| state.active_transaction_id().cloned()))
    }

    pub(super) async fn resolve_prepared_table_type(
        &self,
        metadata: &PreparedExecutionStatement,
    ) -> Result<Option<TableType>, KalamDbError> {
        if let Some(table_type) = metadata.table_type {
            return Ok(Some(table_type));
        }

        let Some(table_id) = metadata.table_id.as_ref() else {
            return Ok(None);
        };

        if let Some(cached) = self.app_context.schema_registry().get(table_id) {
            return Ok(Some(cached.table_entry().table_type));
        }

        Ok(self
            .app_context
            .schema_registry()
            .get_table_if_exists_async(table_id)
            .await?
            .map(|table_def| table_def.table_type))
    }

    pub(super) fn transaction_query_context_for_request(
        &self,
        exec_ctx: &ExecutionContext,
    ) -> Result<Option<TransactionQueryContext>, KalamDbError> {
        let transaction_id = match exec_ctx.transaction_id().cloned() {
            Some(transaction_id) => Some(transaction_id),
            None => self.active_request_transaction_id(exec_ctx)?,
        };
        let Some(transaction_id) = transaction_id else {
            return Ok(None);
        };

        let coordinator = self.app_context.transaction_coordinator();
        let handle = coordinator.get_handle(&transaction_id).ok_or_else(|| {
            KalamDbError::InvalidOperation(format!(
                "active SQL transaction '{}' has no handle",
                transaction_id
            ))
        })?;

        if !handle.state.is_open() {
            return Err(KalamDbError::InvalidOperation(format!(
                "transaction '{}' is {}",
                transaction_id, handle.state
            )));
        }

        Ok(Some(
            TransactionQueryContext::new(
                transaction_id.clone(),
                Arc::clone(&handle.snapshot_log_index),
                Arc::new(crate::transactions::CoordinatorOverlayView::new(
                    Arc::clone(&coordinator),
                    transaction_id.clone(),
                )),
                Arc::new(crate::transactions::CoordinatorMutationSink::new(coordinator)),
                Arc::new(CoordinatorAccessValidator::new(
                    self.app_context.transaction_coordinator(),
                )),
            )
            .with_pk_claims(primary_key_claims()),
        ))
    }

    pub(super) fn user_session_state(
        exec_ctx: &ExecutionContext,
        transaction: Option<TransactionQueryContext>,
    ) -> SessionState {
        let mut state = exec_ctx.build_user_session_state();
        if let Some(transaction) = transaction {
            install_extension(&mut state, TransactionQueryExtension::new(transaction));
        }
        state
    }

    pub(super) fn create_session_with_transaction_context(
        &self,
        exec_ctx: &ExecutionContext,
    ) -> Result<SessionContext, KalamDbError> {
        let Some(transaction) = self.transaction_query_context_for_request(exec_ctx)? else {
            return Ok(exec_ctx.create_session_with_user());
        };
        Ok(SessionContext::new_with_state(Self::user_session_state(
            exec_ctx,
            Some(transaction),
        )))
    }

    pub(super) fn point_read_session_state(
        &self,
        exec_ctx: &ExecutionContext,
    ) -> Result<Arc<SessionState>, KalamDbError> {
        if let Some(transaction) = self.transaction_query_context_for_request(exec_ctx)? {
            let _span = kalamdb_observability::kdb_info_span_entered!("point_get.tx_session_state");
            return Ok(Arc::new(Self::user_session_state(exec_ctx, Some(transaction))));
        }

        let key = PointReadSessionCacheKey::new(
            exec_ctx.user_id().clone(),
            exec_ctx.user_role(),
            exec_ctx.default_namespace(),
            exec_ctx.read_context(),
        );
        if let Some(state) = self.point_read_session_cache.get(&key) {
            return Ok(state);
        }

        let _span = kalamdb_observability::kdb_info_span_entered!("point_get.cold_session_state");
        let state = Arc::new(Self::user_session_state(exec_ctx, None));
        self.point_read_session_cache.insert(key, Arc::clone(&state));
        Ok(state)
    }

    pub(super) fn apply_set_search_path(
        &self,
        stmt: &kalamdb_sql::ddl::SetSearchPathStatement,
        exec_ctx: &ExecutionContext,
    ) {
        let target_schema = stmt
            .schemas
            .iter()
            .find_map(|schema| {
                if schema.as_str() == "$user" {
                    return None;
                }
                if schema.is_default_namespace()
                    || schema.is_system_schema()
                    || schema.is_pg_catalog()
                    || schema.is_information_schema()
                {
                    return Some(schema.clone());
                }
                if self
                    .app_context
                    .system_tables()
                    .namespaces()
                    .get_namespace(schema)
                    .ok()
                    .flatten()
                    .is_some()
                {
                    return Some(schema.clone());
                }
                Some(schema.clone())
            })
            .unwrap_or_else(|| SchemaId::new("default"));

        let session = exec_ctx.create_session_with_user();
        session.state_ref().write().config_mut().options_mut().catalog.default_schema =
            target_schema.as_str().to_string();
    }

    pub(super) async fn abort_active_transaction_on_statement_failure(
        &self,
        classified: &SqlStatement,
        metadata: &PreparedExecutionStatement,
        error: &KalamDbError,
        exec_ctx: &ExecutionContext,
    ) {
        if Self::is_ddl_statement(classified.kind()) {
            return;
        }
        if let Ok(Some(TableType::Stream | TableType::System)) =
            self.resolve_prepared_table_type(metadata).await
        {
            return;
        }
        if error.to_string().contains("is already bound to data raft group") {
            return;
        }
        let transaction_id = match exec_ctx.transaction_id().cloned() {
            Some(tx_id) => Some(tx_id),
            None => self.active_request_transaction_id(exec_ctx).ok().flatten(),
        };
        if let Some(transaction_id) = transaction_id {
            self.app_context
                .transaction_coordinator()
                .mark_transaction_aborted(&transaction_id);
        }
    }

    pub(super) fn ensure_active_transaction_not_aborted(
        &self,
        classified: &SqlStatement,
        exec_ctx: &ExecutionContext,
    ) -> Result<(), KalamDbError> {
        if matches!(
            classified.kind(),
            SqlStatementKind::RollbackTransaction | SqlStatementKind::RollbackToSavepoint(_)
        ) {
            return Ok(());
        }
        let transaction_id = match exec_ctx.transaction_id().cloned() {
            Some(tx_id) => Some(tx_id),
            None => self.active_request_transaction_id(exec_ctx).ok().flatten(),
        };
        if let Some(transaction_id) = transaction_id {
            if let Some(handle) =
                self.app_context.transaction_coordinator().get_handle(&transaction_id)
            {
                if matches!(handle.state, kalamdb_commons::models::TransactionState::Aborted) {
                    return Err(KalamDbError::InvalidOperation(
                        "current transaction is aborted, commands ignored until end of \
                         transaction block (SQLSTATE 25P02)"
                            .to_string(),
                    ));
                }
            }
        }
        Ok(())
    }

    pub(super) fn execute_savepoint(
        &self,
        name: &str,
        exec_ctx: &ExecutionContext,
    ) -> Result<ExecutionResult, KalamDbError> {
        let transaction_id = match exec_ctx.transaction_id().cloned() {
            Some(tx_id) => tx_id,
            None => self.active_request_transaction_id(exec_ctx)?.ok_or_else(|| {
                KalamDbError::InvalidOperation(
                    "SAVEPOINT can only be used in transaction blocks".to_string(),
                )
            })?,
        };
        exec_ctx
            .session_config()
            .savepoint(name)
            .map_err(KalamDbError::InvalidOperation)?;
        self.app_context.transaction_coordinator().savepoint(&transaction_id, name)?;
        Ok(ExecutionResult::Success {
            message: "SAVEPOINT".to_string(),
        })
    }

    pub(super) fn execute_release_savepoint(
        &self,
        name: &str,
        exec_ctx: &ExecutionContext,
    ) -> Result<ExecutionResult, KalamDbError> {
        let transaction_id = match exec_ctx.transaction_id().cloned() {
            Some(tx_id) => tx_id,
            None => self.active_request_transaction_id(exec_ctx)?.ok_or_else(|| {
                KalamDbError::InvalidOperation(
                    "RELEASE SAVEPOINT can only be used in transaction blocks".to_string(),
                )
            })?,
        };
        self.app_context
            .transaction_coordinator()
            .release_savepoint(&transaction_id, name)?;
        exec_ctx.session_config().release(name);
        Ok(ExecutionResult::Success {
            message: "RELEASE".to_string(),
        })
    }

    pub(super) fn execute_rollback_to_savepoint(
        &self,
        name: &str,
        exec_ctx: &ExecutionContext,
    ) -> Result<ExecutionResult, KalamDbError> {
        let transaction_id = match exec_ctx.transaction_id().cloned() {
            Some(tx_id) => tx_id,
            None => self.active_request_transaction_id(exec_ctx)?.ok_or_else(|| {
                KalamDbError::InvalidOperation(
                    "ROLLBACK TO SAVEPOINT can only be used in transaction blocks".to_string(),
                )
            })?,
        };
        self.app_context
            .transaction_coordinator()
            .rollback_to_savepoint(&transaction_id, name)?;
        exec_ctx.session_config().rollback_to(name);
        Ok(ExecutionResult::Success {
            message: "ROLLBACK".to_string(),
        })
    }

    pub(super) async fn execute_begin_transaction(
        &self,
        exec_ctx: &ExecutionContext,
    ) -> Result<ExecutionResult, KalamDbError> {
        let request_transaction_coordinator =
            AppContextRequestTransactionCoordinator::new(self.app_context.as_ref());
        let mut request_state = RequestTransactionState::from_request_id(exec_ctx.request_id())
            .ok_or_else(|| {
                KalamDbError::InvalidOperation(
                    "BEGIN requires a request-scoped execution context".to_string(),
                )
            })?;
        request_state.sync(&request_transaction_coordinator);
        let transaction_id = request_state
            .begin(&request_transaction_coordinator)
            .map_err(map_request_transaction_error)?;
        Ok(ExecutionResult::Success {
            message: {
                exec_ctx.session_config().begin();
                format!("Transaction started ({})", transaction_id)
            },
        })
    }

    pub(super) async fn execute_commit_transaction(
        &self,
        exec_ctx: &ExecutionContext,
    ) -> Result<ExecutionResult, KalamDbError> {
        let request_transaction_coordinator =
            AppContextRequestTransactionCoordinator::new(self.app_context.as_ref());
        let mut request_state = RequestTransactionState::from_request_id(exec_ctx.request_id())
            .ok_or_else(|| {
                KalamDbError::InvalidOperation(
                    "COMMIT requires a request-scoped execution context".to_string(),
                )
            })?;
        request_state.sync(&request_transaction_coordinator);
        let transaction_id = request_state
            .commit(&request_transaction_coordinator)
            .await
            .map_err(map_request_transaction_error)?;
        Ok(ExecutionResult::Success {
            message: {
                exec_ctx.session_config().commit();
                format!("Transaction committed ({})", transaction_id)
            },
        })
    }

    pub(super) fn execute_rollback_transaction(
        &self,
        exec_ctx: &ExecutionContext,
    ) -> Result<ExecutionResult, KalamDbError> {
        let request_transaction_coordinator =
            AppContextRequestTransactionCoordinator::new(self.app_context.as_ref());
        let mut request_state = RequestTransactionState::from_request_id(exec_ctx.request_id())
            .ok_or_else(|| {
                KalamDbError::InvalidOperation(
                    "ROLLBACK requires a request-scoped execution context".to_string(),
                )
            })?;
        request_state.sync(&request_transaction_coordinator);
        let transaction_id = request_state
            .rollback(&request_transaction_coordinator)
            .map_err(map_request_transaction_error)?;
        Ok(ExecutionResult::Success {
            message: {
                exec_ctx.session_config().rollback();
                format!("Transaction rolled back ({})", transaction_id)
            },
        })
    }

    pub(super) fn reject_ddl_in_active_request_transaction(
        &self,
        classified: &SqlStatement,
        exec_ctx: &ExecutionContext,
    ) -> Result<(), KalamDbError> {
        if !Self::is_ddl_statement(classified.kind()) {
            return Ok(());
        }
        // ALTER TABLE, CREATE TABLE, and DROP TABLE are applied immediately.
        // PostgreSQL installers such as Directus create, alter, and drop
        // collections inside an explicit transaction. The statements are not
        // rolled back with the transaction.
        if matches!(
            classified.kind(),
            SqlStatementKind::AlterTable(_)
                | SqlStatementKind::CreateTable(_)
                | SqlStatementKind::DropTable(_)
        ) {
            return Ok(());
        }

        if let Some(transaction_id) = self.active_request_transaction_id(exec_ctx)? {
            self.app_context
                .transaction_coordinator()
                .reject_ddl_in_transaction(&transaction_id)?;
        }

        Ok(())
    }

    pub(super) async fn reject_unsupported_dml_in_active_request_transaction(
        &self,
        metadata: &PreparedExecutionStatement,
        exec_ctx: &ExecutionContext,
    ) -> Result<(), KalamDbError> {
        let Some(transaction_id) = self.active_request_transaction_id(exec_ctx)? else {
            return Ok(());
        };

        match self.resolve_prepared_table_type(metadata).await? {
            Some(TableType::Stream) => Err(KalamDbError::InvalidOperation(format!(
                "transaction '{}' failed: stream tables are not supported inside explicit \
                 transactions",
                transaction_id
            ))),
            Some(TableType::System) => Err(KalamDbError::InvalidOperation(format!(
                "transaction '{}' failed: system tables are not supported inside explicit \
                 transactions",
                transaction_id
            ))),
            _ => Ok(()),
        }
    }

    pub(super) async fn should_autocommit_stream_dml(
        &self,
        metadata: &PreparedExecutionStatement,
        exec_ctx: &ExecutionContext,
    ) -> Result<bool, KalamDbError> {
        if !exec_ctx.allows_stream_autocommit() {
            return Ok(false);
        }
        if matches!(self.resolve_prepared_table_type(metadata).await?, Some(TableType::Stream)) {
            return Ok(true);
        }
        self.is_stream_target_sql(metadata.sql.as_str(), metadata.table_id.as_ref(), exec_ctx)
            .await
    }

    pub(super) async fn is_stream_target_sql(
        &self,
        sql: &str,
        table_id: Option<&TableId>,
        exec_ctx: &ExecutionContext,
    ) -> Result<bool, KalamDbError> {
        if !exec_ctx.allows_stream_autocommit() {
            return Ok(false);
        }
        let Some(table_id) = table_id
            .cloned()
            .or_else(|| extract_sql_target_table_id(sql, &exec_ctx.default_namespace()))
        else {
            return Ok(false);
        };
        if let Some(cached) = self.app_context.schema_registry().get(&table_id) {
            return Ok(cached.table_entry().table_type == TableType::Stream);
        }
        Ok(self
            .app_context
            .schema_registry()
            .get_table_if_exists_async(&table_id)
            .await?
            .is_some_and(|table| table.table_type == TableType::Stream))
    }

    pub(super) fn isolated_stream_exec_ctx(exec_ctx: &ExecutionContext) -> ExecutionContext {
        // Procedure SQL runs on the meta leader, which is often a follower of the
        // user shard. Stream DML waits until that shard applies the entry locally
        // before returning, so the read in the same procedure must use that replica
        // instead of failing the leader check.
        exec_ctx
            .clone()
            .without_transaction_id()
            .with_request_id(format!("stream-{}", Uuid::now_v7()))
            .with_read_context(ReadContext::Internal)
    }
}
