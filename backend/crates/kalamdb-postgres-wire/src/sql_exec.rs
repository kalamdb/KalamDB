use std::sync::Arc;

use kalamdb_backend::manager::{BackendSessionError, BackendSessionManager};
use kalamdb_commons::models::NamespaceId;
use kalamdb_core::{
    app_context::AppContext,
    sql::{ExecutionContext, ScalarValue, SqlExecutor},
};
use pgwire::{
    api::{
        portal::Format,
        results::{Response, Tag},
    },
    error::{ErrorInfo, PgWireError, PgWireResult},
};

use crate::{
    client_catalog::{classify_postgres_set, PostgresSetAction},
    connection::WireConnectionState,
    row_encoder::{execution_result_to_responses, execution_result_to_responses_with_format},
};

const IN_FAILED_SQLSTATE: &str = "25P02";
const SQL_ERROR_SQLSTATE: &str = "XX000";

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
        metadata: &kalamdb_core::sql::executor::PreparedExecutionStatement,
        params: Vec<ScalarValue>,
        column_format: Option<&Format>,
    ) -> PgWireResult<Vec<Response>> {
        if let Err(error) = self.session_manager.ensure_block_allows_work(state.session_id()) {
            return Ok(vec![Response::Error(Box::new(error_info(
                "ERROR",
                IN_FAILED_SQLSTATE,
                error.to_string(),
            )))]);
        }

        let mut exec_ctx = self.execution_context(state);
        if let Some(transaction_id) = self
            .session_manager
            .transaction_id(state.session_id())
            .map_err(|error| pg_error(error.to_string()))?
        {
            exec_ctx = exec_ctx.with_transaction_id(transaction_id);
        }

        match self.sql_executor.execute_with_metadata(metadata, &exec_ctx, params).await {
            Ok(result) => {
                self.persist_search_path_if_needed(state, metadata.sql.as_str());
                execution_result_to_responses_with_format(result, column_format)
            },
            Err(error) => {
                self.mark_statement_failed_if_in_block(state);
                Ok(vec![Response::Error(Box::new(error_info(
                    "ERROR",
                    SQL_ERROR_SQLSTATE,
                    error.to_string(),
                )))])
            },
        }
    }

    pub async fn execute(
        &self,
        state: &WireConnectionState,
        sql: &str,
    ) -> PgWireResult<Vec<Response>> {
        if let Err(error) = self.session_manager.ensure_block_allows_work(state.session_id()) {
            return Ok(vec![Response::Error(Box::new(error_info(
                "ERROR",
                IN_FAILED_SQLSTATE,
                error.to_string(),
            )))]);
        }

        let mut exec_ctx = self.execution_context(state);
        if let Some(transaction_id) = self
            .session_manager
            .transaction_id(state.session_id())
            .map_err(|error| pg_error(error.to_string()))?
        {
            exec_ctx = exec_ctx.with_transaction_id(transaction_id);
        }
        match self.sql_executor.execute(sql, &exec_ctx, Vec::new()).await {
            Ok(result) => {
                self.persist_search_path_if_needed(state, sql);
                // SET returns Success — map to Execution tag clients expect.
                if matches!(
                    classify_postgres_set(sql),
                    Some(PostgresSetAction::NoOp | PostgresSetAction::SetSearchPath { .. })
                ) {
                    return Ok(vec![Response::Execution(Tag::new("SET"))]);
                }
                execution_result_to_responses(result)
            },
            Err(error) => {
                self.mark_statement_failed_if_in_block(state);
                Ok(vec![Response::Error(Box::new(error_info(
                    "ERROR",
                    SQL_ERROR_SQLSTATE,
                    error.to_string(),
                )))])
            },
        }
    }

    fn persist_search_path_if_needed(&self, state: &WireConnectionState, sql: &str) {
        let Some(PostgresSetAction::SetSearchPath { schema }) = classify_postgres_set(sql) else {
            return;
        };
        let namespace = NamespaceId::new(schema.as_str());
        let exists = self
            .app_context
            .system_tables()
            .namespaces()
            .get_namespace(&namespace)
            .map(|entry| entry.is_some())
            .unwrap_or(false);
        if exists {
            state.set_current_schema(namespace);
        }
    }

    fn execution_context(&self, state: &WireConnectionState) -> ExecutionContext {
        ExecutionContext::with_namespace(
            state.auth().user_id.clone(),
            state.auth().role,
            state.current_schema(),
            self.app_context.base_session_context(),
        )
        .with_request_id(format!("wire:{}", state.session_id()))
    }

    fn mark_statement_failed_if_in_block(&self, state: &WireConnectionState) {
        if matches!(self.session_manager.transaction_id(state.session_id()), Ok(Some(_))) {
            let _ = self.session_manager.mark_statement_failed(state.session_id());
        }
    }
}

pub fn backend_session_error_to_response(error: BackendSessionError) -> Response {
    Response::Error(Box::new(error_info("ERROR", SQL_ERROR_SQLSTATE, error.to_string())))
}

pub fn pg_error(message: impl Into<String>) -> PgWireError {
    PgWireError::UserError(Box::new(error_info("ERROR", SQL_ERROR_SQLSTATE, message)))
}

fn error_info(
    severity: impl Into<String>,
    code: impl Into<String>,
    message: impl Into<String>,
) -> ErrorInfo {
    ErrorInfo::new(severity.into(), code.into(), message.into())
}
