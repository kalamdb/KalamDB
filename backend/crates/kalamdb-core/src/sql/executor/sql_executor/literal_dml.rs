//! Literal INSERT and ON CONFLICT updates that write through table appliers.

use std::{collections::HashMap, sync::Arc};

use datafusion::scalar::ScalarValue;
use kalamdb_commons::{
    models::{rows::row::Row, TableId, TransactionId, UserId},
    schemas::TableType,
    try_pk_bucket_key, PkBucketKey, Role,
};
use kalamdb_tables::KalamTableProvider;
use sqlparser::ast::Statement;
use uuid::Uuid;

use crate::{
    error::KalamDbError,
    sql::{
        executor::{
            request_transaction_state::{
                map_request_transaction_error, AppContextRequestTransactionCoordinator,
                RequestTransactionState,
            },
            PreparedExecutionStatement, SqlExecutor,
        },
        ExecutionContext, ExecutionResult,
    },
};

struct OnConflictStageResult {
    rows_affected: usize,
    returned_rows: Vec<Row>,
}

struct OnConflictExistingRow {
    row: Row,
    mutation_user_id: Option<UserId>,
}

enum OnConflictTableProvider<'a> {
    User(&'a KalamTableProvider),
    Shared(&'a KalamTableProvider),
}

impl SqlExecutor {
    pub(super) async fn try_execute_literal_insert_via_applier(
        &self,
        _sql: &str,
        metadata: &PreparedExecutionStatement,
        exec_ctx: &ExecutionContext,
        params: &[ScalarValue],
    ) -> Result<Option<ExecutionResult>, KalamDbError> {
        if !params.is_empty() {
            crate::sql::executor::parameter_binding::validate_params(params)?;
        }

        let Some(table_id) = metadata.table_id.as_ref() else {
            return Ok(None);
        };

        let parsed_statement = match metadata.parsed_dml.as_deref() {
            Some(statement) => statement,
            None => return Ok(None),
        };

        let has_active_tx = self.active_request_transaction_id(exec_ctx)?.is_some();

        // ON CONFLICT must be handled before the active-transaction bail-out below.
        if let Statement::Insert(insert) = parsed_statement {
            if kalamdb_sql::insert_has_on_conflict(insert) {
                if let Some(on_conflict_rows) =
                    crate::sql::executor::transaction_batch_insert::try_build_literal_on_conflict_update_rows(
                        parsed_statement,
                        Arc::clone(&self.app_context),
                        self.sql_cache_registry.as_ref(),
                        exec_ctx,
                        table_id,
                        params,
                    )
                    .await?
                {
                    return self
                        .execute_literal_on_conflict_update(table_id, exec_ctx, on_conflict_rows)
                        .await
                        .map(Some);
                }
            }
        }

        let Some(insert_rows) = kalamdb_observability::kdb_await_in_info_span!(
            crate::sql::executor::transaction_batch_insert::try_build_literal_insert_rows(
                parsed_statement,
                Arc::clone(&self.app_context),
                self.sql_cache_registry.as_ref(),
                exec_ctx,
                table_id,
                params,
            ),
            "sql.insert_bind"
        )?
        else {
            return Ok(None);
        };

        let foreign_keys = self
            .app_context
            .schema_registry()
            .get_table_if_exists(table_id)?
            .map(|definition| definition.foreign_keys.clone())
            .unwrap_or_default();
        if !foreign_keys.is_empty() {
            let session = self.create_session_with_transaction_context(exec_ctx)?;
            let state = session.state();
            let registry = self.app_context.schema_registry();
            kalamdb_tables::utils::datafusion_dml::reject_missing_foreign_keys(
                registry.as_ref(),
                &state,
                table_id,
                &foreign_keys,
                &insert_rows.rows,
            )
            .await
            .map_err(Self::datafusion_to_execution_error)?;
        }

        if has_active_tx {
            let Some(transaction_id) = self.active_request_transaction_id(exec_ctx)? else {
                return Ok(None);
            };
            if !matches!(insert_rows.table_type, TableType::User | TableType::Shared) {
                return Ok(None);
            }
            let Some(pk_column) = insert_rows.primary_key_column.as_deref() else {
                return Ok(None);
            };
            if insert_rows.table_type == TableType::Shared {
                let provider =
                    self.app_context.schema_registry().get_kalam_provider(table_id).ok_or_else(
                        || {
                            KalamDbError::InvalidOperation(format!(
                                "Table provider not found for RLS check: {}",
                                table_id
                            ))
                        },
                    )?;
                if !provider.is_shared() {
                    return Err(KalamDbError::InvalidOperation(format!(
                        "Shared table provider type mismatch for RLS check: {}",
                        table_id
                    )));
                }
                let snapshot_commit_seq = self
                    .app_context
                    .transaction_coordinator()
                    .get_handle(&transaction_id)
                    .and_then(|handle| handle.snapshot_commit_seq())
                    .and_then(|log_index| {
                        kalamdb_commons::ids::RaftVersionId::try_new(
                            log_index,
                            kalamdb_commons::ids::MAX_ORDINAL,
                        )
                        .ok()
                        .map(|packed| packed.version())
                    });
                kalamdb_observability::kdb_await_in_info_span!(
                    provider.check_shared_rows_authorized(
                        exec_ctx.user_id(),
                        exec_ctx.user_role(),
                        kalamdb_commons::PolicyCommand::Insert,
                        true,
                        &insert_rows.rows,
                        snapshot_commit_seq,
                    ),
                    "table.rls_insert"
                )?;
            }
            let rows_affected = insert_rows.rows.len();
            let returning = insert_rows.returning.clone();
            let returning_rows = returning.as_ref().map(|_| insert_rows.rows.clone());
            let user_id = crate::sql::executor::transaction_batch_insert::staged_mutation_user_id(
                insert_rows.table_type,
                exec_ctx,
            );
            let mutations = kalamdb_transactions::staged_mutation::build_insert_staged_mutations(
                &transaction_id,
                table_id,
                insert_rows.table_type,
                user_id,
                pk_column,
                insert_rows.rows,
            )
            .map_err(|error| KalamDbError::InvalidOperation(error.to_string()))?;
            self.app_context
                .transaction_coordinator()
                .stage_batch(&transaction_id, mutations)?;
            if let (Some(returning), Some(returning_rows)) = (returning.as_ref(), returning_rows) {
                return self
                    .build_on_conflict_returning_result(table_id, returning, returning_rows)
                    .await
                    .map(Some);
            }
            return Ok(Some(ExecutionResult::Inserted { rows_affected }));
        }

        if let Some(returning) = insert_rows.returning.as_ref() {
            let returning_rows = insert_rows.rows.clone();
            let Some(_) = self
                .insert_literal_rows_via_applier(
                    insert_rows.table_type,
                    table_id,
                    exec_ctx,
                    insert_rows.rows,
                )
                .await?
            else {
                return Ok(None);
            };

            self.build_on_conflict_returning_result(table_id, returning, returning_rows)
                .await
                .map(Some)
        } else {
            let Some(rows_affected) = self
                .insert_literal_rows_via_applier(
                    insert_rows.table_type,
                    table_id,
                    exec_ctx,
                    insert_rows.rows,
                )
                .await?
            else {
                return Ok(None);
            };

            Ok(Some(ExecutionResult::Inserted { rows_affected }))
        }
    }

    pub(super) async fn insert_literal_rows_via_applier(
        &self,
        table_type: TableType,
        table_id: &TableId,
        exec_ctx: &ExecutionContext,
        rows: Vec<Row>,
    ) -> Result<Option<usize>, KalamDbError> {
        if table_type == TableType::System {
            return Ok(None);
        }

        if table_type == TableType::Shared {
            let provider =
                self.app_context.schema_registry().get_kalam_provider(table_id).ok_or_else(
                    || {
                        KalamDbError::InvalidOperation(format!(
                            "Table provider not found for RLS check: {}",
                            table_id
                        ))
                    },
                )?;
            if !provider.is_shared() {
                return Err(KalamDbError::InvalidOperation(format!(
                    "Shared table provider type mismatch for RLS check: {}",
                    table_id
                )));
            }
            kalamdb_observability::kdb_await_in_info_span!(
                provider.check_shared_rows_authorized(
                    exec_ctx.user_id(),
                    exec_ctx.user_role(),
                    kalamdb_commons::PolicyCommand::Insert,
                    true,
                    &rows,
                    None,
                ),
                "table.rls_insert"
            )?;
        }

        let applier = self.app_context.applier();
        let rows_affected = match table_type {
            TableType::Shared => applier
                .insert_shared_data(table_id.clone(), Some(exec_ctx.user_id().clone()), rows)
                .await
                .map_err(KalamDbError::from)?
                .rows_affected(),
            TableType::User | TableType::Stream => applier
                .insert_user_data(table_id.clone(), exec_ctx.user_id().clone(), rows)
                .await
                .map_err(KalamDbError::from)?
                .rows_affected(),
            TableType::System => unreachable!("system tables handled above"),
        };

        Ok(Some(rows_affected as usize))
    }

    pub(super) async fn run_with_request_transaction<'a, T, F, Fut>(
        &'a self,
        exec_ctx: &'a ExecutionContext,
        request_prefix: &str,
        context_error: &str,
        op: F,
    ) -> Result<T, KalamDbError>
    where
        F: FnOnce(&'a ExecutionContext, TransactionId) -> Fut,
        Fut: std::future::Future<Output = Result<T, KalamDbError>>,
    {
        let request_transaction_coordinator =
            AppContextRequestTransactionCoordinator::new(self.app_context.as_ref());
        let mut request_state = RequestTransactionState::from_request_id(exec_ctx.request_id())
            .ok_or_else(|| {
                KalamDbError::InvalidOperation(format!(
                    "{context_error} requires a request-scoped execution context \
                     (prefix={request_prefix})"
                ))
            })?;
        request_state.sync(&request_transaction_coordinator);

        let started_transaction = if request_state.is_active() {
            false
        } else {
            request_state
                .begin(&request_transaction_coordinator)
                .map_err(map_request_transaction_error)?;
            true
        };

        let transaction_id = request_state.active_transaction_id().cloned().ok_or_else(|| {
            KalamDbError::InvalidOperation(format!("{context_error} could not start a transaction"))
        })?;

        match op(exec_ctx, transaction_id).await {
            Ok(value) => {
                if started_transaction {
                    if let Err(error) = request_state.commit(&request_transaction_coordinator).await
                    {
                        let _ = request_state.rollback_if_active(&request_transaction_coordinator);
                        return Err(map_request_transaction_error(error));
                    }
                }
                Ok(value)
            },
            Err(error) => {
                if started_transaction {
                    let _ = request_state.rollback_if_active(&request_transaction_coordinator);
                }
                Err(error)
            },
        }
    }

    pub(super) async fn execute_literal_on_conflict_update(
        &self,
        table_id: &TableId,
        exec_ctx: &ExecutionContext,
        on_conflict_rows: crate::sql::executor::transaction_batch_insert::LiteralOnConflictUpdateRows,
    ) -> Result<ExecutionResult, KalamDbError> {
        if on_conflict_rows.table_type == TableType::Shared
            && matches!(exec_ctx.user_role(), Role::User | Role::Service)
        {
            return Err(KalamDbError::InvalidOperation(
                "ON CONFLICT on RLS-protected shared tables is not supported until both conflict \
                 branches can be policy-checked"
                    .to_string(),
            ));
        }
        crate::sql::executor::transaction_batch_insert::reject_system_table_dml(
            on_conflict_rows.table_type,
        )?;

        let owned_exec_ctx;
        let dml_exec_ctx = if exec_ctx.request_id().is_some() {
            exec_ctx
        } else {
            owned_exec_ctx =
                exec_ctx.clone().with_request_id(format!("sql-on-conflict-{}", Uuid::now_v7()));
            &owned_exec_ctx
        };

        let on_conflict_rows_ref = &on_conflict_rows;
        let stage_result = self
            .run_with_request_transaction(
                dml_exec_ctx,
                "sql-on-conflict",
                "ON CONFLICT DO UPDATE",
                |ctx, transaction_id| async move {
                    self.stage_literal_on_conflict_update(
                        table_id,
                        ctx,
                        &transaction_id,
                        on_conflict_rows_ref,
                    )
                    .await
                },
            )
            .await?;

        if let Some(returning) = on_conflict_rows.returning.as_ref() {
            self.build_on_conflict_returning_result(table_id, returning, stage_result.returned_rows)
                .await
        } else {
            Ok(ExecutionResult::Inserted {
                rows_affected: stage_result.rows_affected,
            })
        }
    }

    async fn stage_literal_on_conflict_update(
        &self,
        table_id: &TableId,
        exec_ctx: &ExecutionContext,
        transaction_id: &TransactionId,
        on_conflict_rows: &crate::sql::executor::transaction_batch_insert::LiteralOnConflictUpdateRows,
    ) -> Result<OnConflictStageResult, KalamDbError> {
        let user_ids = crate::sql::executor::transaction_batch_insert::on_conflict_user_ids(
            on_conflict_rows.table_type,
            exec_ctx,
        )?;

        let provider_arc =
            self.app_context.schema_registry().get_kalam_provider(table_id).ok_or_else(|| {
                KalamDbError::InvalidOperation(format!("Table provider not found: {}", table_id))
            })?;
        let table_provider = match on_conflict_rows.table_type {
            TableType::User | TableType::Stream => {
                if !provider_arc.is_user() {
                    return Err(KalamDbError::InvalidOperation(format!(
                        "Provider type mismatch for user table {}",
                        table_id
                    )));
                }
                OnConflictTableProvider::User(provider_arc.as_ref())
            },
            TableType::Shared => {
                if !provider_arc.is_shared() {
                    return Err(KalamDbError::InvalidOperation(format!(
                        "Provider type mismatch for shared table {}",
                        table_id
                    )));
                }
                OnConflictTableProvider::Shared(provider_arc.as_ref())
            },
            TableType::System => unreachable!("system tables rejected above"),
        };

        let mut mutations = Vec::with_capacity(on_conflict_rows.rows.len());
        let mut returned_rows = Vec::with_capacity(on_conflict_rows.rows.len());
        let mut row_state_by_pk: HashMap<PkBucketKey, OnConflictExistingRow> =
            HashMap::with_capacity(on_conflict_rows.rows.len());
        for row in &on_conflict_rows.rows {
            let primary_key =
                row.values.get(&on_conflict_rows.primary_key_column).ok_or_else(|| {
                    KalamDbError::InvalidOperation(format!(
                        "ON CONFLICT requires primary key column '{}'",
                        on_conflict_rows.primary_key_column
                    ))
                })?;
            let pk_key = try_pk_bucket_key(primary_key).map_err(KalamDbError::InvalidOperation)?;
            let primary_key_string = pk_key.to_string();
            if !row_state_by_pk.contains_key(&pk_key) {
                if let Some(existing_row) = self
                    .on_conflict_existing_row(
                        transaction_id,
                        table_id,
                        on_conflict_rows.table_type,
                        user_ids.lookup_user_id.clone(),
                        primary_key,
                        &table_provider,
                    )
                    .await?
                {
                    row_state_by_pk.insert(pk_key.clone(), existing_row);
                }
            }

            let existing_row = row_state_by_pk.get(&pk_key);
            let mutation_user_id = existing_row
                .as_ref()
                .and_then(|existing| existing.mutation_user_id.clone())
                .or_else(|| user_ids.default_mutation_user_id.clone());
            let existing_row_ref = existing_row.map(|existing| &existing.row);

            let Some(staged) =
                crate::sql::executor::transaction_batch_insert::build_on_conflict_staged_mutation_for_action(
                    &on_conflict_rows.action,
                    transaction_id,
                    table_id,
                    on_conflict_rows.table_type,
                    mutation_user_id,
                    primary_key_string.clone(),
                    row,
                    existing_row_ref,
                )?
            else {
                continue;
            };

            let returned_row = staged.returned_row.clone();
            let mutation_user_id = staged.mutation.user_id.clone();
            mutations.push(staged.mutation);
            returned_rows.push(staged.returned_row);
            row_state_by_pk.insert(
                pk_key,
                OnConflictExistingRow {
                    row: returned_row,
                    mutation_user_id,
                },
            );
        }

        self.app_context
            .transaction_coordinator()
            .stage_batch(transaction_id, mutations)?;

        Ok(OnConflictStageResult {
            rows_affected: returned_rows.len(),
            returned_rows,
        })
    }

    async fn on_conflict_existing_row(
        &self,
        transaction_id: &TransactionId,
        table_id: &TableId,
        table_type: TableType,
        user_id: Option<UserId>,
        primary_key: &ScalarValue,
        table_provider: &OnConflictTableProvider<'_>,
    ) -> Result<Option<OnConflictExistingRow>, KalamDbError> {
        let primary_key_str = primary_key.to_string();
        if let Some(overlay) =
            self.app_context.transaction_coordinator().get_overlay(transaction_id)
        {
            let overlay_entry = match table_type {
                TableType::User | TableType::Stream => overlay.latest_visible_entry_for_scope(
                    table_id,
                    user_id.as_ref(),
                    primary_key_str.as_str(),
                ),
                TableType::Shared => overlay
                    .latest_visible_entry_for_scope(
                        table_id,
                        user_id.as_ref(),
                        primary_key_str.as_str(),
                    )
                    .or_else(|| overlay.latest_visible_entry(table_id, primary_key_str.as_str())),
                TableType::System => {
                    overlay.latest_visible_entry(table_id, primary_key_str.as_str())
                },
            };
            if let Some(entry) = overlay_entry {
                return if entry.is_deleted() {
                    Ok(None)
                } else {
                    Ok(Some(OnConflictExistingRow {
                        row: entry.payload.clone(),
                        mutation_user_id: entry.user_id.clone(),
                    }))
                };
            }
        }

        match (table_type, table_provider) {
            (TableType::User | TableType::Stream, OnConflictTableProvider::User(provider)) => {
                let user_id = user_id.ok_or_else(|| {
                    KalamDbError::InvalidOperation(
                        "user table ON CONFLICT lookup requires user_id".to_string(),
                    )
                })?;
                Ok(provider.find_user_by_pk(&user_id, primary_key).await?.map(|(_, row)| {
                    OnConflictExistingRow {
                        row: row.fields,
                        mutation_user_id: Some(user_id),
                    }
                }))
            },
            (TableType::Shared, OnConflictTableProvider::Shared(provider)) => {
                Ok(provider.find_by_pk(primary_key).await?.map(|(_, row)| OnConflictExistingRow {
                    row: row.fields,
                    mutation_user_id: None,
                }))
            },
            _ => Err(KalamDbError::InvalidOperation(format!(
                "Provider type mismatch for table {}",
                table_id
            ))),
        }
    }
}
