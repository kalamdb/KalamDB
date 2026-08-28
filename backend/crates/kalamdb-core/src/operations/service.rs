use std::{collections::BTreeMap, sync::Arc};

use async_trait::async_trait;
use datafusion::prelude::SessionContext;
use kalamdb_backend::session::LiveSessionTransaction;
use kalamdb_commons::{
    models::{
        pg_operations::{
            DeleteRequest, InsertRequest, MutationResult, ScanRequest, ScanResult, UpdateRequest,
        },
        rows::Row,
        OperationKind, ReadContext, Role, TransactionId, TransactionOrigin, UserId,
    },
    NamespaceId, PolicyCommand, TableId, TableType,
};
use kalamdb_pg::OperationExecutor;
use kalamdb_session_datafusion::SessionUserContext;
use kalamdb_tables::SharedTableProvider;
use kalamdb_transactions::{
    build_insert_staged_mutations, TransactionQueryContext, TransactionQueryExtension,
};
use tonic::Status;

use super::scan;
use crate::{
    app_context::AppContext,
    sql::ExecutionContext,
    transactions::{
        CoordinatorAccessValidator, CoordinatorOverlayView, ExecutionOwnerKey, StagedMutation,
    },
};

/// Domain-typed operation executor for Tier-2 (typed) callers.
///
/// Typed callers (PG extension, future transports) skip SQL parsing and DataFusion
/// logical planning entirely:
/// - **Scans**: `TableProvider::scan()` → `collect(plan, task_ctx)` — physical execution only
/// - **Mutations**: `UnifiedApplier` → Raft → `DmlExecutor` — no DataFusion at all
pub struct OperationService {
    app_context: Arc<AppContext>,
}

impl OperationService {
    pub fn new(app_context: Arc<AppContext>) -> Self {
        Self { app_context }
    }

    fn session_with_query_context(
        &self,
        user_id: Option<&UserId>,
        role: Role,
        transaction_query_context: Option<TransactionQueryContext>,
    ) -> SessionContext {
        let base = self.app_context.base_session_context();
        let mut state = base.state();

        let ctx = match user_id {
            Some(uid) => SessionUserContext::new(uid.clone(), role, ReadContext::Client),
            None => SessionUserContext::new(UserId::anonymous(), role, ReadContext::Client),
        };
        state.config_mut().options_mut().extensions.insert(ctx);

        if let Some(transaction_query_context) = transaction_query_context {
            state
                .config_mut()
                .options_mut()
                .extensions
                .insert(TransactionQueryExtension::new(transaction_query_context));
        }

        SessionContext::new_with_state(state)
    }

    /// Bind the typed-path role from table type.
    ///
    /// User/Stream tables use `Role::User`. Shared tables use `Role::Service`,
    /// which is still subject to FORCE RLS (PUBLIC policies include Service).
    fn role_for_table_type(table_type: TableType) -> Role {
        match table_type {
            TableType::User | TableType::Stream => Role::User,
            _ => Role::Service,
        }
    }

    /// Resolve the principal for typed PG RPCs.
    ///
    /// Preference order:
    /// 1. Explicit `user_id` on the request (`kalam.user_id` / RPC field) → table-type role
    /// 2. Authenticated backend session (account_login bridge) → keep System/DBA role so FORCE RLS
    ///    bypasses for DBA connections that omit `kalam.user_id`
    /// 3. Shared tables without either → unauthenticated
    fn resolve_typed_principal(
        &self,
        table_type: TableType,
        request_user_id: Option<UserId>,
        session_id: Option<&str>,
    ) -> Result<(Option<UserId>, Role), Status> {
        if let Some(user_id) = request_user_id {
            return Ok((Some(user_id), Self::role_for_table_type(table_type)));
        }

        if let Some(session_id) = session_id.map(str::trim).filter(|id| !id.is_empty()) {
            if let Some(manager) = self.app_context.try_backend_session_manager() {
                if let Some(snapshot) = manager.get_snapshot(session_id) {
                    if let Some(user_id) = snapshot.authenticated_user_id {
                        let role =
                            if matches!(snapshot.authenticated_role, Role::System | Role::Dba) {
                                snapshot.authenticated_role
                            } else {
                                Self::role_for_table_type(table_type)
                            };
                        return Ok((Some(user_id), role));
                    }
                }
            }
        }

        if table_type == TableType::Shared {
            return Err(Status::unauthenticated(
                "shared-table operations require an authenticated principal",
            ));
        }

        Ok((None, Self::role_for_table_type(table_type)))
    }

    /// Evaluate a typed shared-table mutation against FORCE RLS.
    ///
    /// Typed shared writes fail closed without a principal or a matching policy.
    /// System/DBA principals bypass FORCE RLS.
    async fn authorize_typed_shared_rows(
        &self,
        table_id: &TableId,
        user_id: &UserId,
        role: Role,
        command: PolicyCommand,
        check: bool,
        rows: &[Row],
    ) -> Result<(), Status> {
        let provider = self
            .app_context
            .schema_registry()
            .get_provider(table_id)
            .ok_or_else(|| Status::not_found(format!("shared table {table_id} not found")))?;
        let shared = (provider.as_ref() as &dyn std::any::Any)
            .downcast_ref::<SharedTableProvider>()
            .ok_or_else(|| {
                Status::failed_precondition(format!("{table_id} is not a shared table"))
            })?;
        shared
            .check_rows_authorized(user_id, role, command, check, rows, None)
            .await
            .map_err(|error| Status::permission_denied(error.to_string()))
    }

    async fn authorize_typed_shared_insert(
        &self,
        table_id: &TableId,
        user_id: &UserId,
        role: Role,
        rows: &[Row],
    ) -> Result<(), Status> {
        self.authorize_typed_shared_rows(table_id, user_id, role, PolicyCommand::Insert, true, rows)
            .await
    }

    async fn current_shared_row(
        &self,
        table_id: &TableId,
        pk_value: &str,
    ) -> Result<Option<Row>, Status> {
        let provider = self
            .app_context
            .schema_registry()
            .get_provider(table_id)
            .ok_or_else(|| Status::not_found(format!("shared table {table_id} not found")))?;
        let shared = (provider.as_ref() as &dyn std::any::Any)
            .downcast_ref::<SharedTableProvider>()
            .ok_or_else(|| {
                Status::failed_precondition(format!("{table_id} is not a shared table"))
            })?;
        shared
            .row_by_pk_value(pk_value)
            .await
            .map_err(|error| Status::internal(error.to_string()))
    }

    async fn authorize_typed_shared_update(
        &self,
        table_id: &TableId,
        user_id: &UserId,
        role: Role,
        pk_value: &str,
        updates: &[Row],
    ) -> Result<(), Status> {
        let current = self.current_shared_row(table_id, pk_value).await?;
        if let Some(old_row) = current.as_ref() {
            self.authorize_typed_shared_rows(
                table_id,
                user_id,
                role,
                PolicyCommand::Update,
                false,
                std::slice::from_ref(old_row),
            )
            .await?;
        }

        let mut new_row = current.unwrap_or_else(|| Row::new(BTreeMap::new()));
        for update in updates {
            for (column, value) in &update.values {
                new_row.values.insert(column.clone(), value.clone());
            }
        }
        self.authorize_typed_shared_rows(
            table_id,
            user_id,
            role,
            PolicyCommand::Update,
            true,
            std::slice::from_ref(&new_row),
        )
        .await
    }

    async fn authorize_typed_shared_delete(
        &self,
        table_id: &TableId,
        user_id: &UserId,
        role: Role,
        pk_value: &str,
    ) -> Result<(), Status> {
        let Some(old_row) = self.current_shared_row(table_id, pk_value).await? else {
            return Ok(());
        };
        self.authorize_typed_shared_rows(
            table_id,
            user_id,
            role,
            PolicyCommand::Delete,
            false,
            std::slice::from_ref(&old_row),
        )
        .await
    }

    fn active_transaction_for_session(
        &self,
        session_id: Option<&str>,
    ) -> Result<Option<TransactionId>, Status> {
        // Autocommit typed DML stays on the hot path here: no transaction handle means
        // one session-id parse plus one coordinator owner-key lookup, with no overlay,
        // query-context, or staged-write allocation.
        let Some(session_id) =
            session_id.map(str::trim).filter(|session_id| !session_id.is_empty())
        else {
            return Ok(None);
        };

        let owner_key = ExecutionOwnerKey::from_pg_session_id(session_id)
            .map_err(|e| Status::invalid_argument(e.to_string()))?;
        Ok(self.app_context.transaction_coordinator().active_for_owner(&owner_key))
    }

    /// Resolve the active transaction id + handle for a PG session in one shot.
    /// Returns `None` if the session has no active transaction. Returns
    /// `FailedPrecondition` if the id is present but the handle is missing.
    fn active_transaction_handle_for_session(
        &self,
        session_id: Option<&str>,
    ) -> Result<Option<(TransactionId, crate::transactions::TransactionHandle)>, Status> {
        let Some(transaction_id) = self.active_transaction_for_session(session_id)? else {
            return Ok(None);
        };
        let handle = self
            .app_context
            .transaction_coordinator()
            .get_handle(&transaction_id)
            .ok_or_else(|| {
                Status::failed_precondition(format!(
                    "active transaction '{}' has no handle",
                    transaction_id
                ))
            })?;
        Ok(Some((transaction_id, handle)))
    }

    fn transaction_query_context_for_session(
        &self,
        session_id: Option<&str>,
    ) -> Result<Option<TransactionQueryContext>, Status> {
        // Autocommit reads return from this helper without constructing an overlay view
        // or mutation sink unless an active transaction handle is actually present.
        let Some((transaction_id, handle)) =
            self.active_transaction_handle_for_session(session_id)?
        else {
            return Ok(None);
        };

        if !handle.state.is_open() {
            return Err(Status::failed_precondition(format!(
                "transaction '{}' is {}",
                transaction_id, handle.state
            )));
        }

        let coordinator = self.app_context.transaction_coordinator();
        Ok(Some(TransactionQueryContext::new(
            transaction_id.clone(),
            handle.snapshot_commit_seq,
            Arc::new(CoordinatorOverlayView::new(Arc::clone(&coordinator), transaction_id.clone())),
            Arc::new(crate::transactions::CoordinatorMutationSink::new(coordinator)),
            Arc::new(CoordinatorAccessValidator::new(self.app_context.transaction_coordinator())),
        )))
    }

    async fn stage_insert(
        &self,
        transaction_id: &TransactionId,
        request: InsertRequest,
    ) -> Result<MutationResult, Status> {
        let coordinator = self.app_context.transaction_coordinator();
        let affected_rows = request.rows.len() as u64;

        let mutations = build_insert_staged_mutations(
            transaction_id,
            &request.table_id,
            request.table_type,
            request.user_id.clone(),
            "id",
            request.rows,
        )
        .map_err(|e| Status::invalid_argument(e.to_string()))?;

        coordinator
            .stage_batch(transaction_id, mutations)
            .map_err(|e| Status::failed_precondition(e.to_string()))?;

        Ok(MutationResult { affected_rows })
    }

    async fn stage_update(
        &self,
        transaction_id: &TransactionId,
        request: UpdateRequest,
    ) -> Result<MutationResult, Status> {
        let coordinator = self.app_context.transaction_coordinator();
        let payload =
            request.updates.into_iter().next().unwrap_or_else(|| Row::new(BTreeMap::new()));
        let mutation = StagedMutation::new(
            transaction_id.clone(),
            request.table_id,
            request.table_type,
            request.user_id,
            OperationKind::Update,
            request.pk_value,
            payload,
            false,
        );

        coordinator
            .stage(transaction_id, mutation)
            .map_err(|e| Status::failed_precondition(e.to_string()))?;

        Ok(MutationResult { affected_rows: 1 })
    }

    async fn stage_delete(
        &self,
        transaction_id: &TransactionId,
        request: DeleteRequest,
    ) -> Result<MutationResult, Status> {
        let coordinator = self.app_context.transaction_coordinator();
        let mutation = StagedMutation::new(
            transaction_id.clone(),
            request.table_id,
            request.table_type,
            request.user_id,
            OperationKind::Delete,
            request.pk_value,
            Row::new(BTreeMap::new()),
            true,
        );

        coordinator
            .stage(transaction_id, mutation)
            .map_err(|e| Status::failed_precondition(e.to_string()))?;

        Ok(MutationResult { affected_rows: 1 })
    }
}

#[async_trait]
impl OperationExecutor for OperationService {
    async fn active_transaction(
        &self,
        session_id: &str,
    ) -> Result<Option<LiveSessionTransaction>, Status> {
        let Some((transaction_id, handle)) =
            self.active_transaction_handle_for_session(Some(session_id))?
        else {
            return Ok(None);
        };

        Ok(Some(LiveSessionTransaction::new(
            session_id.to_string(),
            transaction_id,
            handle.state,
            handle.has_write_set,
        )))
    }

    async fn begin_transaction(&self, session_id: &str) -> Result<Option<TransactionId>, Status> {
        let owner_key = ExecutionOwnerKey::from_pg_session_id(session_id)
            .map_err(|e| Status::invalid_argument(e.to_string()))?;
        let transaction_id = self
            .app_context
            .transaction_coordinator()
            .begin(owner_key, session_id.to_string().into(), TransactionOrigin::PgRpc)
            .map_err(|e| Status::failed_precondition(e.to_string()))?;
        Ok(Some(transaction_id))
    }

    async fn commit_transaction(
        &self,
        _session_id: &str,
        transaction_id: &TransactionId,
    ) -> Result<Option<TransactionId>, Status> {
        let result = self
            .app_context
            .transaction_coordinator()
            .commit(transaction_id)
            .await
            .map_err(|e| Status::failed_precondition(e.to_string()))?;
        Ok(Some(result.transaction_id))
    }

    async fn rollback_transaction(
        &self,
        _session_id: &str,
        transaction_id: &TransactionId,
    ) -> Result<Option<TransactionId>, Status> {
        self.app_context
            .transaction_coordinator()
            .rollback(transaction_id)
            .map_err(|e| Status::failed_precondition(e.to_string()))?;
        Ok(Some(transaction_id.clone()))
    }

    async fn execute_scan(&self, request: ScanRequest) -> Result<ScanResult, Status> {
        let (user_id, role) = self.resolve_typed_principal(
            request.table_type,
            request.user_id.clone(),
            request.session_id.as_deref(),
        )?;
        // Non-transactional scans pay only the idle-session lookup above. The
        // transaction query extension is attached only when a live transaction exists.
        let transaction_query_context =
            self.transaction_query_context_for_session(request.session_id.as_deref())?;
        let session =
            self.session_with_query_context(user_id.as_ref(), role, transaction_query_context);
        let batches = scan::execute_scan(
            &self.app_context.schema_registry(),
            &session,
            &request.table_id,
            &request.columns,
            request.limit,
            &request.filters,
        )
        .await
        .map_err(|e| -> Status { e.into() })?;
        Ok(ScanResult { batches })
    }

    async fn execute_insert(&self, request: InsertRequest) -> Result<MutationResult, Status> {
        let (resolved_user_id, role) = self.resolve_typed_principal(
            request.table_type,
            request.user_id.clone(),
            request.session_id.as_deref(),
        )?;
        if request.table_type == TableType::Shared {
            let user_id = resolved_user_id.as_ref().ok_or_else(|| {
                Status::permission_denied(
                    "typed shared-table writes require an authenticated principal and WITH CHECK \
                     policy; use SQL",
                )
            })?;
            self.authorize_typed_shared_insert(&request.table_id, user_id, role, &request.rows)
                .await?;
        }
        // Autocommit requests pay only the owner-key lookup here; we do not allocate
        // transaction overlays or staged write buffers unless an explicit transaction is active.
        if let Some(transaction_id) =
            self.active_transaction_for_session(request.session_id.as_deref())?
        {
            return self.stage_insert(&transaction_id, request).await;
        }

        let applier = self.app_context.applier();
        let affected = match request.table_type {
            TableType::User | TableType::Stream => {
                let user_id = require_user_id(resolved_user_id, "inserts")?;
                let resp = applier
                    .insert_user_data(request.table_id, user_id, request.rows)
                    .await
                    .map_err(|e| Status::internal(e.to_string()))?;
                resp.rows_affected()
            },
            TableType::Shared => {
                let resp = applier
                    .insert_shared_data(request.table_id, resolved_user_id, request.rows)
                    .await
                    .map_err(|e| Status::internal(e.to_string()))?;
                resp.rows_affected()
            },
            TableType::System => {
                return Err(Status::permission_denied("cannot insert into system tables"));
            },
        };
        Ok(MutationResult {
            affected_rows: affected as u64,
        })
    }

    async fn execute_update(&self, request: UpdateRequest) -> Result<MutationResult, Status> {
        let (resolved_user_id, role) = self.resolve_typed_principal(
            request.table_type,
            request.user_id.clone(),
            request.session_id.as_deref(),
        )?;
        if request.table_type == TableType::Shared {
            let user_id = resolved_user_id.as_ref().ok_or_else(|| {
                Status::permission_denied(
                    "typed shared-table writes require an authenticated principal and WITH CHECK \
                     policy; use SQL",
                )
            })?;
            self.authorize_typed_shared_update(
                &request.table_id,
                user_id,
                role,
                &request.pk_value,
                &request.updates,
            )
            .await?;
        }
        // Preserve the autocommit fast path: one presence check, then go straight to the applier.
        if let Some(transaction_id) =
            self.active_transaction_for_session(request.session_id.as_deref())?
        {
            return self.stage_update(&transaction_id, request).await;
        }

        let applier = self.app_context.applier();
        let affected = match request.table_type {
            TableType::User | TableType::Stream => {
                let user_id = require_user_id(resolved_user_id, "updates")?;
                let resp = applier
                    .update_user_data(
                        request.table_id,
                        user_id,
                        request.updates,
                        Some(request.pk_value),
                    )
                    .await
                    .map_err(|e| Status::internal(e.to_string()))?;
                resp.rows_affected()
            },
            TableType::Shared => {
                let resp = applier
                    .update_shared_data(
                        request.table_id,
                        resolved_user_id,
                        request.updates,
                        Some(request.pk_value),
                    )
                    .await
                    .map_err(|e| Status::internal(e.to_string()))?;
                resp.rows_affected()
            },
            TableType::System => {
                return Err(Status::permission_denied("cannot update system tables"));
            },
        };
        Ok(MutationResult {
            affected_rows: affected as u64,
        })
    }

    async fn execute_delete(&self, request: DeleteRequest) -> Result<MutationResult, Status> {
        let (resolved_user_id, role) = self.resolve_typed_principal(
            request.table_type,
            request.user_id.clone(),
            request.session_id.as_deref(),
        )?;
        if request.table_type == TableType::Shared {
            let user_id = resolved_user_id.as_ref().ok_or_else(|| {
                Status::permission_denied(
                    "typed shared-table writes require an authenticated principal and WITH CHECK \
                     policy; use SQL",
                )
            })?;
            self.authorize_typed_shared_delete(&request.table_id, user_id, role, &request.pk_value)
                .await?;
        }
        // Preserve the autocommit fast path: avoid transaction-specific allocations when absent.
        if let Some(transaction_id) =
            self.active_transaction_for_session(request.session_id.as_deref())?
        {
            return self.stage_delete(&transaction_id, request).await;
        }

        let applier = self.app_context.applier();
        let affected = match request.table_type {
            TableType::User | TableType::Stream => {
                let user_id = require_user_id(resolved_user_id, "deletes")?;
                let resp = applier
                    .delete_user_data(request.table_id, user_id, Some(vec![request.pk_value]))
                    .await
                    .map_err(|e| Status::internal(e.to_string()))?;
                resp.rows_affected()
            },
            TableType::Shared => {
                let resp = applier
                    .delete_shared_data(
                        request.table_id,
                        resolved_user_id,
                        Some(vec![request.pk_value]),
                    )
                    .await
                    .map_err(|e| Status::internal(e.to_string()))?;
                resp.rows_affected()
            },
            TableType::System => {
                return Err(Status::permission_denied("cannot delete from system tables"));
            },
        };
        Ok(MutationResult {
            affected_rows: affected as u64,
        })
    }

    async fn execute_sql(&self, sql: &str) -> Result<String, Status> {
        let base = self.app_context.base_session_context();
        let exec_ctx = ExecutionContext::with_namespace(
            UserId::new("pg-extension"),
            Role::Dba,
            NamespaceId::new("default"),
            base,
        );

        let sql_executor = self.app_context.sql_executor();
        let result = sql_executor
            .execute(sql, &exec_ctx, Vec::new())
            .await
            .map_err(|e| Status::internal(e.to_string()))?;

        let message = match result {
            crate::sql::ExecutionResult::Success { message } => message,
            other => format!("OK (affected: {})", other.affected_rows()),
        };
        Ok(message)
    }

    async fn execute_query(&self, sql: &str) -> Result<(String, Vec<bytes::Bytes>), Status> {
        let base = self.app_context.base_session_context();
        let exec_ctx = ExecutionContext::with_namespace(
            UserId::new("pg-extension"),
            Role::Dba,
            NamespaceId::new("default"),
            base,
        );

        let sql_executor = self.app_context.sql_executor();
        let result = sql_executor
            .execute(sql, &exec_ctx, Vec::new())
            .await
            .map_err(|e| Status::internal(e.to_string()))?;

        match result {
            crate::sql::ExecutionResult::Rows {
                batches, row_count, ..
            } => {
                let (ipc_batches, _) = kalamdb_pg::encode_batches(&batches)?;
                Ok((format!("{} row(s)", row_count), ipc_batches))
            },
            crate::sql::ExecutionResult::Success { message } => Ok((message, Vec::new())),
            other => Ok((format!("OK (affected: {})", other.affected_rows()), Vec::new())),
        }
    }
}

fn require_user_id(user_id: Option<UserId>, operation: &str) -> Result<UserId, Status> {
    user_id.ok_or_else(|| {
        Status::invalid_argument(format!("user_id required for user/stream table {}", operation))
    })
}

#[cfg(test)]
mod tests {
    use std::{collections::BTreeMap, sync::Arc};

    use arrow::{
        array::{Int64Array, StringArray},
        datatypes::{DataType, Field, Schema},
        record_batch::RecordBatch,
    };
    use datafusion::datasource::MemTable;
    use datafusion_common::ScalarValue;
    use kalamdb_commons::{
        datatypes::KalamDataType,
        models::{
            rows::Row,
            schemas::{ColumnDefinition, TableDefinition, TableOptions},
            NamespaceId, TableId, TableName,
        },
        TableType,
    };
    use kalamdb_pg::OperationExecutor;

    use super::*;
    use crate::{
        schema_registry::cached_table_data::CachedTableData, test_helpers::test_app_context_simple,
    };

    fn empty_row() -> Row {
        Row {
            values: BTreeMap::new(),
        }
    }

    /// Helper: create an AppContext and OperationService for tests.
    fn setup() -> (Arc<AppContext>, OperationService) {
        let app_ctx = test_app_context_simple();
        let svc = OperationService::new(Arc::clone(&app_ctx));
        (app_ctx, svc)
    }

    /// Helper: register an in-memory table with two columns (id INT64, name UTF8)
    /// and optional seed data into the SchemaRegistry.
    fn register_mem_table(
        app_ctx: &AppContext,
        table_id: &TableId,
        batches: Vec<RecordBatch>,
    ) -> Arc<Schema> {
        let arrow_schema = Arc::new(Schema::new(vec![
            Field::new("id", DataType::Int64, false),
            Field::new("name", DataType::Utf8, true),
        ]));

        // Create a TableDefinition so the CachedTableData can be built
        let table_def = TableDefinition::new(
            table_id.namespace_id().clone(),
            table_id.table_name().clone(),
            TableType::Shared,
            vec![
                ColumnDefinition::primary_key(1, "id", 1, KalamDataType::BigInt),
                ColumnDefinition::simple(2, "name", 2, KalamDataType::Text),
            ],
            TableOptions::shared(),
            None,
        )
        .expect("table definition");

        let cached = Arc::new(CachedTableData::new(Arc::new(table_def)));

        // Wrap seed data in a MemTable provider
        let mem =
            MemTable::try_new(Arc::clone(&arrow_schema), vec![batches]).expect("MemTable creation");
        cached.set_provider(Arc::new(mem));

        app_ctx.schema_registry().insert_cached(table_id.clone(), cached);
        arrow_schema
    }

    // ---------------------------------------------------------------
    // Scan tests
    // ---------------------------------------------------------------

    #[tokio::test]
    async fn scan_nonexistent_table_returns_not_found() {
        let (_app_ctx, svc) = setup();
        let req = ScanRequest {
            table_id:   TableId::new(NamespaceId::new("no_ns"), TableName::new("no_table")),
            table_type: TableType::Shared,
            session_id: None,
            columns:    vec![],
            limit:      None,
            user_id:    Some(UserId::new("service")),
            filters:    vec![],
        };
        let err = svc.execute_scan(req).await.unwrap_err();
        assert_eq!(err.code(), tonic::Code::NotFound);
    }

    #[tokio::test]
    async fn scan_shared_without_principal_returns_unauthenticated() {
        let (app_ctx, svc) = setup();
        let table_id = TableId::new(NamespaceId::new("default"), TableName::new("deny_tbl"));
        register_mem_table(&app_ctx, &table_id, vec![]);

        let err = svc
            .execute_scan(ScanRequest {
                table_id,
                table_type: TableType::Shared,
                session_id: None,
                columns: vec![],
                limit: None,
                user_id: None,
                filters: vec![],
            })
            .await
            .unwrap_err();
        assert_eq!(err.code(), tonic::Code::Unauthenticated);
    }

    #[tokio::test]
    async fn scan_shared_uses_bridge_session_principal_when_request_user_id_omitted() {
        let (app_ctx, svc) = setup();
        let table_id = TableId::new(NamespaceId::new("default"), TableName::new("bridge_tbl"));
        register_mem_table(&app_ctx, &table_id, vec![]);

        let session_id = "pg-12345-abcdef01";
        app_ctx
            .backend_session_manager()
            .open_session(
                kalamdb_commons::models::SessionOrigin::ExtensionBridge,
                session_id,
                kalamdb_backend::session::BackendAuth::new(
                    UserId::new("root"),
                    Role::System,
                    "account_login",
                    i64::MAX,
                ),
                None,
                None,
            )
            .expect("open bridge session");

        let res = svc
            .execute_scan(ScanRequest {
                table_id,
                table_type: TableType::Shared,
                session_id: Some(session_id.to_string()),
                columns: vec![],
                limit: None,
                user_id: None,
                filters: vec![],
            })
            .await
            .expect("bridge System session should authorize shared scans without request user_id");
        let total_rows: usize = res.batches.iter().map(|b| b.num_rows()).sum();
        assert_eq!(total_rows, 0);
    }

    #[tokio::test]
    async fn scan_empty_table_returns_zero_batches() {
        let (app_ctx, svc) = setup();
        let table_id = TableId::new(NamespaceId::new("default"), TableName::new("empty_tbl"));
        register_mem_table(&app_ctx, &table_id, vec![]);

        let res = svc
            .execute_scan(ScanRequest {
                table_id,
                table_type: TableType::Shared,
                session_id: None,
                columns: vec![],
                limit: None,
                user_id: Some(UserId::new("service")),
                filters: vec![],
            })
            .await
            .expect("scan should succeed");
        let total_rows: usize = res.batches.iter().map(|b| b.num_rows()).sum();
        assert_eq!(total_rows, 0);
    }

    #[tokio::test]
    async fn scan_with_data_returns_rows() {
        let (app_ctx, svc) = setup();
        let table_id = TableId::new(NamespaceId::new("default"), TableName::new("data_tbl"));
        let schema = register_mem_table(
            &app_ctx,
            &table_id,
            vec![RecordBatch::try_new(
                Arc::new(Schema::new(vec![
                    Field::new("id", DataType::Int64, false),
                    Field::new("name", DataType::Utf8, true),
                ])),
                vec![
                    Arc::new(Int64Array::from(vec![1, 2, 3])),
                    Arc::new(StringArray::from(vec!["a", "b", "c"])),
                ],
            )
            .unwrap()],
        );

        let res = svc
            .execute_scan(ScanRequest {
                table_id,
                table_type: TableType::Shared,
                session_id: None,
                columns: vec![],
                limit: None,
                user_id: Some(UserId::new("service")),
                filters: vec![],
            })
            .await
            .expect("scan should succeed");
        let total_rows: usize = res.batches.iter().map(|b| b.num_rows()).sum();
        assert_eq!(total_rows, 3);
        // Verify schema when returning all columns
        assert_eq!(res.batches[0].schema().as_ref(), schema.as_ref());
    }

    #[tokio::test]
    async fn scan_with_column_projection() {
        let (app_ctx, svc) = setup();
        let table_id = TableId::new(NamespaceId::new("default"), TableName::new("proj_tbl"));
        register_mem_table(
            &app_ctx,
            &table_id,
            vec![RecordBatch::try_new(
                Arc::new(Schema::new(vec![
                    Field::new("id", DataType::Int64, false),
                    Field::new("name", DataType::Utf8, true),
                ])),
                vec![
                    Arc::new(Int64Array::from(vec![10])),
                    Arc::new(StringArray::from(vec!["x"])),
                ],
            )
            .unwrap()],
        );

        let res = svc
            .execute_scan(ScanRequest {
                table_id,
                table_type: TableType::Shared,
                session_id: None,
                columns: vec!["name".to_string()],
                limit: None,
                user_id: Some(UserId::new("service")),
                filters: vec![],
            })
            .await
            .expect("scan with projection");
        assert_eq!(res.batches[0].num_columns(), 1);
        assert_eq!(res.batches[0].schema().field(0).name(), "name");
    }

    #[tokio::test]
    async fn scan_with_invalid_column_returns_error() {
        let (app_ctx, svc) = setup();
        let table_id = TableId::new(NamespaceId::new("default"), TableName::new("badcol_tbl"));
        register_mem_table(&app_ctx, &table_id, vec![]);

        let err = svc
            .execute_scan(ScanRequest {
                table_id,
                table_type: TableType::Shared,
                session_id: None,
                columns: vec!["nonexistent_col".to_string()],
                limit: None,
                user_id: Some(UserId::new("service")),
                filters: vec![],
            })
            .await
            .unwrap_err();
        assert_eq!(err.code(), tonic::Code::InvalidArgument);
        assert!(err.message().contains("nonexistent_col"));
    }

    #[tokio::test]
    async fn scan_with_limit() {
        let (app_ctx, svc) = setup();
        let table_id = TableId::new(NamespaceId::new("default"), TableName::new("limit_tbl"));
        register_mem_table(
            &app_ctx,
            &table_id,
            vec![RecordBatch::try_new(
                Arc::new(Schema::new(vec![
                    Field::new("id", DataType::Int64, false),
                    Field::new("name", DataType::Utf8, true),
                ])),
                vec![
                    Arc::new(Int64Array::from(vec![1, 2, 3, 4, 5])),
                    Arc::new(StringArray::from(vec!["a", "b", "c", "d", "e"])),
                ],
            )
            .unwrap()],
        );

        // Limit is passed as a hint to TableProvider::scan(); MemTable may not
        // enforce it at the physical level, so we only verify the call succeeds.
        let res = svc
            .execute_scan(ScanRequest {
                table_id,
                table_type: TableType::Shared,
                session_id: None,
                columns: vec![],
                limit: Some(2),
                user_id: Some(UserId::new("service")),
                filters: vec![],
            })
            .await
            .expect("scan with limit should succeed");
        let total_rows: usize = res.batches.iter().map(|b| b.num_rows()).sum();
        assert!(total_rows > 0, "scan should return some rows");
    }

    // ---------------------------------------------------------------
    // DML rejection tests (system tables — no applier needed)
    // ---------------------------------------------------------------

    #[tokio::test]
    async fn insert_system_table_rejected() {
        let (_app_ctx, svc) = setup();
        let err = svc
            .execute_insert(InsertRequest {
                table_id:   TableId::new(NamespaceId::new("system"), TableName::new("users")),
                table_type: TableType::System,
                session_id: None,
                rows:       vec![empty_row()],
                user_id:    None,
            })
            .await
            .unwrap_err();
        assert_eq!(err.code(), tonic::Code::PermissionDenied);
    }

    #[tokio::test]
    async fn update_system_table_rejected() {
        let (_app_ctx, svc) = setup();
        let err = svc
            .execute_update(UpdateRequest {
                table_id:   TableId::new(NamespaceId::new("system"), TableName::new("users")),
                table_type: TableType::System,
                session_id: None,
                updates:    vec![empty_row()],
                pk_value:   "some_pk".to_string(),
                user_id:    None,
            })
            .await
            .unwrap_err();
        assert_eq!(err.code(), tonic::Code::PermissionDenied);
    }

    #[tokio::test]
    async fn delete_system_table_rejected() {
        let (_app_ctx, svc) = setup();
        let err = svc
            .execute_delete(DeleteRequest {
                table_id:   TableId::new(NamespaceId::new("system"), TableName::new("users")),
                table_type: TableType::System,
                session_id: None,
                pk_value:   "some_pk".to_string(),
                user_id:    None,
            })
            .await
            .unwrap_err();
        assert_eq!(err.code(), tonic::Code::PermissionDenied);
    }

    // ---------------------------------------------------------------
    // Validation tests (user_id required — no applier needed)
    // ---------------------------------------------------------------

    #[tokio::test]
    async fn insert_user_table_without_user_id_rejected() {
        let (_app_ctx, svc) = setup();
        let err = svc
            .execute_insert(InsertRequest {
                table_id:   TableId::new(NamespaceId::new("default"), TableName::new("tbl")),
                table_type: TableType::User,
                session_id: None,
                rows:       vec![empty_row()],
                user_id:    None,
            })
            .await
            .unwrap_err();
        assert_eq!(err.code(), tonic::Code::InvalidArgument);
        assert!(err.message().contains("user_id required"));
    }

    #[tokio::test]
    async fn update_user_table_without_user_id_rejected() {
        let (_app_ctx, svc) = setup();
        let err = svc
            .execute_update(UpdateRequest {
                table_id:   TableId::new(NamespaceId::new("default"), TableName::new("tbl")),
                table_type: TableType::User,
                session_id: None,
                updates:    vec![empty_row()],
                pk_value:   "pk".to_string(),
                user_id:    None,
            })
            .await
            .unwrap_err();
        assert_eq!(err.code(), tonic::Code::InvalidArgument);
    }

    #[tokio::test]
    async fn delete_user_table_without_user_id_rejected() {
        let (_app_ctx, svc) = setup();
        let err = svc
            .execute_delete(DeleteRequest {
                table_id:   TableId::new(NamespaceId::new("default"), TableName::new("tbl")),
                table_type: TableType::User,
                session_id: None,
                pk_value:   "pk".to_string(),
                user_id:    None,
            })
            .await
            .unwrap_err();
        assert_eq!(err.code(), tonic::Code::InvalidArgument);
    }

    #[tokio::test]
    async fn insert_stream_table_without_user_id_rejected() {
        let (_app_ctx, svc) = setup();
        let err = svc
            .execute_insert(InsertRequest {
                table_id:   TableId::new(NamespaceId::new("default"), TableName::new("events")),
                table_type: TableType::Stream,
                session_id: None,
                rows:       vec![empty_row()],
                user_id:    None,
            })
            .await
            .unwrap_err();
        assert_eq!(err.code(), tonic::Code::InvalidArgument);
        assert!(err.message().contains("user_id required"));
    }

    #[tokio::test]
    async fn shared_insert_with_active_pg_transaction_fails_closed() {
        let (app_ctx, svc) = setup();
        let session_id = "pg-321-deadbeef";
        let transaction_id = app_ctx
            .transaction_coordinator()
            .begin(
                crate::transactions::ExecutionOwnerKey::from_pg_session_id(session_id).unwrap(),
                session_id.to_string().into(),
                kalamdb_commons::models::TransactionOrigin::PgRpc,
            )
            .unwrap();

        let mut values = BTreeMap::new();
        values.insert("id".to_string(), ScalarValue::Int64(Some(42)));
        values.insert("name".to_string(), ScalarValue::Utf8(Some("staged item".to_string())));

        let error = svc
            .execute_insert(InsertRequest {
                table_id:   TableId::new(NamespaceId::new("default"), TableName::new("items")),
                table_type: TableType::Shared,
                session_id: Some(session_id.to_string()),
                user_id:    None,
                rows:       vec![Row::new(values)],
            })
            .await
            .expect_err("shared typed write must not bypass RLS");
        // No request principal and no authenticated bridge session → fail closed
        // before WITH CHECK / overlay staging (same gate as shared scans).
        assert_eq!(error.code(), tonic::Code::Unauthenticated);

        assert!(
            app_ctx.transaction_coordinator().get_overlay(&transaction_id).is_none(),
            "rejected typed shared writes must not stage an overlay"
        );
    }

    async fn register_shared_table_with_public_all_policy(
        app_ctx: &AppContext,
        table_name: &str,
    ) -> TableId {
        let table_id = TableId::new(NamespaceId::new("default"), TableName::new(table_name));
        let id_col = ColumnDefinition::new(
            1,
            "id".to_string(),
            1,
            KalamDataType::BigInt,
            false,
            true,
            false,
            kalamdb_commons::schemas::ColumnDefault::None,
            None,
        );
        let name_col = ColumnDefinition::simple(2, "name", 2, KalamDataType::Text);
        let mut table_def = TableDefinition::new(
            table_id.namespace_id().clone(),
            table_id.table_name().clone(),
            TableType::Shared,
            vec![id_col, name_col],
            TableOptions::shared(),
            None,
        )
        .expect("shared table definition");
        app_ctx
            .system_columns_service()
            .add_system_columns(&mut table_def)
            .expect("system columns");
        app_ctx
            .schema_registry()
            .register_table(table_def)
            .expect("register shared table");

        app_ctx
            .system_tables()
            .table_policies()
            .create_policy(kalamdb_commons::TablePolicy::new(
                kalamdb_commons::PolicyId::new(table_id.clone(), "public_all").expect("policy id"),
                table_id.clone(),
                "public_all",
                PolicyCommand::All,
                vec![kalamdb_commons::PolicyTarget::Public],
                Some("true".to_string()),
                Some("true".to_string()),
                Some(kalamdb_commons::PolicyProgram::RowLocal {
                    expr: kalamdb_commons::BoundExprShape::Literal(true),
                }),
                Some(kalamdb_commons::PolicyProgram::RowLocal {
                    expr: kalamdb_commons::BoundExprShape::Literal(true),
                }),
                0,
                1,
            ))
            .await
            .expect("create public ALL policy");
        table_id
    }

    fn begin_pg_transaction(app_ctx: &AppContext, session_id: &str) -> TransactionId {
        app_ctx
            .transaction_coordinator()
            .begin(
                crate::transactions::ExecutionOwnerKey::from_pg_session_id(session_id).unwrap(),
                session_id.to_string().into(),
                kalamdb_commons::models::TransactionOrigin::PgRpc,
            )
            .unwrap()
    }

    #[tokio::test]
    async fn shared_insert_with_public_policy_stages_overlay() {
        let (app_ctx, svc) = setup();
        let table_id = register_shared_table_with_public_all_policy(&app_ctx, "policy_items").await;
        let session_id = "pg-322-cafef00d";
        let transaction_id = begin_pg_transaction(&app_ctx, session_id);

        let mut values = BTreeMap::new();
        values.insert("id".to_string(), ScalarValue::Int64(Some(1)));
        values.insert("name".to_string(), ScalarValue::Utf8(Some("granted".to_string())));

        svc.execute_insert(InsertRequest {
            table_id,
            table_type: TableType::Shared,
            session_id: Some(session_id.to_string()),
            user_id: Some(UserId::new("policy-writer")),
            rows: vec![Row::new(values)],
        })
        .await
        .expect("WITH CHECK true must allow typed shared insert");

        assert!(
            app_ctx.transaction_coordinator().get_overlay(&transaction_id).is_some(),
            "authorized typed shared insert must stage an overlay"
        );
    }

    #[tokio::test]
    async fn shared_update_without_principal_fails_closed() {
        let (app_ctx, svc) = setup();
        let session_id = "pg-323-aabbccdd";
        let transaction_id = begin_pg_transaction(&app_ctx, session_id);

        let error = svc
            .execute_update(UpdateRequest {
                table_id:   TableId::new(NamespaceId::new("default"), TableName::new("items")),
                table_type: TableType::Shared,
                session_id: Some(session_id.to_string()),
                user_id:    None,
                updates:    vec![empty_row()],
                pk_value:   "1".to_string(),
            })
            .await
            .expect_err("shared typed update must not bypass RLS");
        assert_eq!(error.code(), tonic::Code::Unauthenticated);
        assert!(
            app_ctx.transaction_coordinator().get_overlay(&transaction_id).is_none(),
            "rejected typed shared updates must not stage an overlay"
        );
    }

    #[tokio::test]
    async fn shared_delete_without_principal_fails_closed() {
        let (app_ctx, svc) = setup();
        let session_id = "pg-324-deadbeef";
        let transaction_id = begin_pg_transaction(&app_ctx, session_id);

        let error = svc
            .execute_delete(DeleteRequest {
                table_id:   TableId::new(NamespaceId::new("default"), TableName::new("items")),
                table_type: TableType::Shared,
                session_id: Some(session_id.to_string()),
                user_id:    None,
                pk_value:   "1".to_string(),
            })
            .await
            .expect_err("shared typed delete must not bypass RLS");
        assert_eq!(error.code(), tonic::Code::Unauthenticated);
        assert!(
            app_ctx.transaction_coordinator().get_overlay(&transaction_id).is_none(),
            "rejected typed shared deletes must not stage an overlay"
        );
    }

    #[tokio::test]
    async fn shared_update_with_public_policy_stages_overlay() {
        let (app_ctx, svc) = setup();
        let table_id =
            register_shared_table_with_public_all_policy(&app_ctx, "policy_update_items").await;
        let session_id = "pg-325-cafef00d";
        let transaction_id = begin_pg_transaction(&app_ctx, session_id);

        let mut updates = BTreeMap::new();
        updates.insert("name".to_string(), ScalarValue::Utf8(Some("renamed".to_string())));

        svc.execute_update(UpdateRequest {
            table_id,
            table_type: TableType::Shared,
            session_id: Some(session_id.to_string()),
            user_id: Some(UserId::new("policy-writer")),
            updates: vec![Row::new(updates)],
            pk_value: "1".to_string(),
        })
        .await
        .expect("USING/WITH CHECK true must allow typed shared update");

        assert!(
            app_ctx.transaction_coordinator().get_overlay(&transaction_id).is_some(),
            "authorized typed shared update must stage an overlay"
        );
    }

    #[tokio::test]
    async fn shared_delete_with_public_policy_stages_overlay() {
        let (app_ctx, svc) = setup();
        let table_id =
            register_shared_table_with_public_all_policy(&app_ctx, "policy_delete_items").await;
        let session_id = "pg-326-feedface";
        let transaction_id = begin_pg_transaction(&app_ctx, session_id);

        svc.execute_delete(DeleteRequest {
            table_id,
            table_type: TableType::Shared,
            session_id: Some(session_id.to_string()),
            user_id: Some(UserId::new("policy-writer")),
            pk_value: "1".to_string(),
        })
        .await
        .expect("USING true must allow typed shared delete");

        assert!(
            app_ctx.transaction_coordinator().get_overlay(&transaction_id).is_some(),
            "authorized typed shared delete must stage an overlay"
        );
    }
}
