use std::sync::Arc;

use kalamdb_backend::manager::{BackendSessionError, BackendSessionManager};
use kalamdb_commons::models::NamespaceId;
use kalamdb_core::{
    app_context::AppContext,
    sql::{ExecutionContext, SqlExecutor},
};
use kalamdb_core::sql::ScalarValue;
use pgwire::{
    api::results::Response,
    error::{ErrorInfo, PgWireError, PgWireResult},
};

use crate::{connection::WireConnectionState, row_encoder::execution_result_to_responses};

const IN_FAILED_SQLSTATE: &str = "25P02";
const SQL_ERROR_SQLSTATE: &str = "XX000";

#[derive(Clone)]
pub struct WireSqlExecutor {
    app_context: Arc<AppContext>,
    sql_executor: Arc<SqlExecutor>,
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
            Ok(result) => execution_result_to_responses(result),
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
            Ok(result) => execution_result_to_responses(result),
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

    fn execution_context(&self, state: &WireConnectionState) -> ExecutionContext {
        ExecutionContext::with_namespace(
            state.auth().user_id.clone(),
            state.auth().role,
            NamespaceId::new(state.current_schema().as_str()),
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
