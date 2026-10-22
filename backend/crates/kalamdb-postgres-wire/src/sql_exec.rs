use std::sync::Arc;

use kalamdb_backend::manager::{BackendSessionError, BackendSessionManager};
use kalamdb_core::{
    app_context::AppContext,
    sql::{ExecutionContext, ScalarValue, SqlExecutor, executor::PreparedExecutionStatement},
};
use kalamdb_sql::classifier::SqlStatementKind;
use pgwire::{
    api::{
        portal::Format,
        results::{Response, Tag},
    },
    error::{ErrorInfo, PgWireError, PgWireResult},
};

use crate::{
    connection::WireConnectionState,
    row_encoder::execution_result_to_responses_with_format,
    tx_control::{WireTransactionControl, WireTransactionOutcome, execute_transaction_control},
};

const IN_FAILED_SQLSTATE: &str = "25P02";
const SQL_ERROR_SQLSTATE: &str = "XX000";

/// Session for one wire connection.
///
/// `SELECT *` includes storage columns unless `postgres_wire.show_system_columns`
/// is false. An explicit `SELECT _version` still resolves either way.
pub(crate) fn execution_context_for_wire(
    app_context: &AppContext,
    state: &WireConnectionState,
) -> ExecutionContext {
    let context = ExecutionContext::with_namespace(
        state.auth().user_id.clone(),
        state.auth().role,
        state.current_schema(),
        app_context.base_session_context(),
    )
    .with_request_id(format!("wire:{}", state.session_id()))
    .with_session_config(state.session_config());
    if app_context.config().postgres_wire.show_system_columns {
        context
    } else {
        context.with_postgres_star()
    }
}

#[derive(Clone)]
pub struct WireSqlExecutor {
    app_context:     Arc<AppContext>,
    sql_executor:    Arc<SqlExecutor>,
    session_manager: Arc<BackendSessionManager>,
}

impl WireSqlExecutor {
    pub fn new(
        app_context: Arc<AppContext>,
        sql_executor: Arc<SqlExecutor>,
        session_manager: Arc<BackendSessionManager>,
    ) -> Self {
        Self {
            app_context,
            sql_executor,
            session_manager,
        }
    }

    pub fn session_manager(&self) -> &BackendSessionManager {
        self.session_manager.as_ref()
    }

    pub fn app_context(&self) -> Arc<AppContext> {
        self.app_context.clone()
    }

    pub fn core_sql_executor(&self) -> Arc<SqlExecutor> {
        self.sql_executor.clone()
    }

    pub async fn execute_metadata(
        &self,
        state: &WireConnectionState,
        metadata: &PreparedExecutionStatement,
        params: Vec<ScalarValue>,
        column_format: Option<&Format>,
    ) -> PgWireResult<Vec<Response>> {
        if let Some(control) = connection_transaction_control(metadata) {
            return self.execute_connection_transaction(state, control).await;
        }

        let is_rollback_to_savepoint = metadata
            .classified_statement
            .as_ref()
            .is_some_and(|stmt| matches!(stmt.kind(), SqlStatementKind::RollbackToSavepoint(_)));

        if !is_rollback_to_savepoint {
            if let Err(error) = self.session_manager.ensure_block_allows_work(state.session_id()) {
                return Ok(vec![Response::Error(Box::new(error_info(
                    "ERROR",
                    IN_FAILED_SQLSTATE,
                    error.to_string(),
                )))]);
            }
        }

        let mut exec_ctx = self.execution_context(state);
        if let Some(transaction_id) = self
            .session_manager
            .transaction_id(state.session_id())
            .map_err(|error| pg_error(error.to_string()))?
        {
            exec_ctx = exec_ctx.with_transaction_id(transaction_id);
        }

        kalamdb_observability::kdb_await_in_info_span!(
            async {
                match self.sql_executor.execute_with_metadata(metadata, &exec_ctx, params).await {
                    Ok(result) => {
                        if is_rollback_to_savepoint {
                            let _ =
                                self.session_manager.recover_failed_transaction(state.session_id());
                        }
                        let _span = kalamdb_observability::kdb_info_span_entered!("wire.encode");
                        execution_result_to_responses_with_format(result, column_format)
                    },
                    Err(error) => {
                        self.mark_statement_failed_if_in_block(state);
                        Ok(vec![Response::Error(Box::new(sql_error_info(&error)))])
                    },
                }
            },
            "wire.execute"
        )
    }

    pub async fn execute(
        &self,
        state: &WireConnectionState,
        sql: &str,
    ) -> PgWireResult<Vec<Response>> {
        let exec_ctx = self.execution_context(state);
        let metadata = match self.sql_executor.prepare_statement_metadata(sql, &exec_ctx) {
            Ok(metadata) => metadata,
            Err(error) => {
                return Ok(vec![Response::Error(Box::new(error_info(
                    "ERROR",
                    "42601",
                    error.user_message(),
                )))]);
            },
        };
        self.execute_metadata(state, &metadata, Vec::new(), None).await
    }

    fn execution_context(&self, state: &WireConnectionState) -> ExecutionContext {
        execution_context_for_wire(&self.app_context, state)
    }

    fn mark_statement_failed_if_in_block(&self, state: &WireConnectionState) {
        if matches!(self.session_manager.transaction_id(state.session_id()), Ok(Some(_))) {
            let _ = self.session_manager.mark_statement_failed(state.session_id());
        }
    }

    pub fn statement_supports_implicit_batch_transaction(
        &self,
        sql: &str,
        state: &WireConnectionState,
    ) -> bool {
        let exec_ctx = self.execution_context(state);
        let Ok(metadata) = self.sql_executor.prepare_statement_metadata(sql, &exec_ctx) else {
            return true;
        };
        metadata
            .classified_statement
            .as_ref()
            .is_none_or(|statement| statement.kind().joins_implicit_transaction())
    }

    async fn execute_connection_transaction(
        &self,
        state: &WireConnectionState,
        control: WireTransactionControl,
    ) -> PgWireResult<Vec<Response>> {
        let outcome =
            execute_transaction_control(self.session_manager(), state.session_id(), control).await;
        Ok(vec![match outcome {
            Ok(WireTransactionOutcome::Began { .. }) => {
                state.session_config().begin();
                Response::TransactionStart(Tag::new("BEGIN"))
            },
            Ok(WireTransactionOutcome::Committed) => {
                state.session_config().commit();
                Response::TransactionEnd(Tag::new("COMMIT"))
            },
            Ok(WireTransactionOutcome::RolledBack) => {
                state.session_config().rollback();
                Response::TransactionEnd(Tag::new("ROLLBACK"))
            },
            Err(error) => backend_session_error_to_response(error),
        }])
    }
}

fn connection_transaction_control(
    metadata: &PreparedExecutionStatement,
) -> Option<WireTransactionControl> {
    match metadata.classified_statement.as_ref()?.kind() {
        SqlStatementKind::BeginTransaction => Some(WireTransactionControl::Begin),
        SqlStatementKind::CommitTransaction => Some(WireTransactionControl::Commit),
        SqlStatementKind::RollbackTransaction => Some(WireTransactionControl::Rollback),
        _ => None,
    }
}

pub fn backend_session_error_to_response(error: BackendSessionError) -> Response {
    let sqlstate = match &error {
        BackendSessionError::FailedBlock(_) => IN_FAILED_SQLSTATE,
        BackendSessionError::ActiveBlock(msg) if msg.contains("no active") => "25P01",
        BackendSessionError::ActiveBlock(_) => "25001",
        _ => SQL_ERROR_SQLSTATE,
    };
    Response::Error(Box::new(error_info("ERROR", sqlstate, error.to_string())))
}

pub fn pg_error(message: impl Into<String>) -> PgWireError {
    PgWireError::UserError(Box::new(error_info("ERROR", SQL_ERROR_SQLSTATE, message)))
}

fn sql_error_info(error: &kalamdb_core::error::KalamDbError) -> ErrorInfo {
    let message = error.user_message();
    let mut info = error_info("ERROR", error.sqlstate(), message.as_ref());
    if let Some((_, rest)) = message.split_once("foreign key violation on table \"") {
        if let Some((table, after)) = rest.split_once('"') {
            info.table = Some(table.to_string());
            if let Some((_, detail)) = after.split_once(" detail ") {
                let detail = detail.trim();
                if !detail.is_empty() {
                    info.detail = Some(detail.to_string());
                }
            }
        }
    }
    info
}

fn error_info(
    severity: impl Into<String>,
    code: impl Into<String>,
    message: impl Into<String>,
) -> ErrorInfo {
    ErrorInfo::new(severity.into(), code.into(), message.into())
}

#[cfg(test)]
mod tests {
    use kalamdb_core::error::KalamDbError;

    use super::sql_error_info;

    #[test]
    fn foreign_key_error_puts_the_column_in_the_first_detail_group() {
        let message = "foreign key violation on table \"posts\" detail Key (author)=(999) is not \
                       present in table \"authors\".";
        let info = sql_error_info(&KalamDbError::InvalidOperation(message.to_string()));

        assert_eq!(info.code, "23503");
        assert_eq!(info.table.as_deref(), Some("posts"));
        assert_eq!(
            info.detail.as_deref(),
            Some("Key (author)=(999) is not present in table \"authors\".")
        );
    }
}
