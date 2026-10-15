use std::sync::Arc;

use kalamdb_backend::manager::{BackendSessionError, BackendSessionManager};
use kalamdb_commons::models::SchemaId;
use kalamdb_core::{
    app_context::AppContext,
    sql::{ExecutionContext, ScalarValue, SqlExecutor},
};
use kalamdb_session::{catalog_entries_for_stored_keys, Catalog, SearchPath};
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
        let is_rollback_to_savepoint = metadata
            .classified_statement
            .as_ref()
            .is_some_and(|stmt| {
                matches!(
                    stmt.kind(),
                    kalamdb_sql::classifier::SqlStatementKind::RollbackToSavepoint(_)
                )
            })
            || is_rollback_to_savepoint_sql(metadata.sql.as_str());

        if !is_rollback_to_savepoint {
            if let Err(error) = self.session_manager.ensure_block_allows_work(state.session_id()) {
                return Ok(vec![Response::Error(Box::new(error_info(
                    "ERROR",
                    IN_FAILED_SQLSTATE,
                    error.to_string(),
                )))]);
            }
        }

        if let Some(response) = self.apply_postgres_client_set(state, metadata.sql.as_str()) {
            return Ok(response);
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
        let is_rollback_to_savepoint = is_rollback_to_savepoint_sql(sql);
        if !is_rollback_to_savepoint {
            if let Err(error) = self.session_manager.ensure_block_allows_work(state.session_id()) {
                return Ok(vec![Response::Error(Box::new(error_info(
                    "ERROR",
                    IN_FAILED_SQLSTATE,
                    error.to_string(),
                )))]);
            }
        }

        if let Some(response) = self.apply_postgres_client_set(state, sql) {
            return Ok(response);
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
                if is_rollback_to_savepoint {
                    let _ = self.session_manager.recover_failed_transaction(state.session_id());
                }
                execution_result_to_responses(result)
            },
            Err(error) => {
                self.mark_statement_failed_if_in_block(state);
                Ok(vec![Response::Error(Box::new(sql_error_info(&error)))])
            },
        }
    }

    fn apply_postgres_client_set(
        &self,
        state: &WireConnectionState,
        sql: &str,
    ) -> Option<Vec<Response>> {
        match classify_postgres_set(sql)? {
            PostgresSetAction::SetSearchPath { path } => {
                self.persist_search_path(state, &path);
                Some(vec![Response::Execution(Tag::new("SET"))])
            },
            PostgresSetAction::NoOp => Some(vec![Response::Execution(Tag::new("SET"))]),
        }
    }

    fn persist_search_path(&self, state: &WireConnectionState, path_text: &str) {
        let Ok(path) = SearchPath::parse(path_text) else {
            return;
        };
        let Ok(namespaces) = self.app_context.system_tables().namespaces().list_namespaces() else {
            return;
        };
        let stored_keys: Vec<(&str, bool)> = namespaces
            .iter()
            .map(|namespace| (namespace.namespace_id.as_str(), true))
            .collect();
        let entries = catalog_entries_for_stored_keys(&stored_keys);
        let catalog = Catalog {
            user:        state.auth().user_id.as_str(),
            temp_schema: None,
            schemas:     &entries,
        };
        if let Some(stored_key) = path.current_stored_key(&catalog) {
            state.set_current_schema(SchemaId::new(stored_key));
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

    pub fn statement_supports_implicit_batch_transaction(
        &self,
        sql: &str,
        state: &WireConnectionState,
    ) -> bool {
        if classify_postgres_set(sql).is_some() {
            return true;
        }
        let exec_ctx = self.execution_context(state);
        let Ok(metadata) = self.sql_executor.prepare_statement_metadata(sql, &exec_ctx) else {
            return true;
        };
        let Some(classified) = metadata.classified_statement.as_ref() else {
            return true;
        };
        use kalamdb_sql::classifier::SqlStatementKind;
        matches!(
            classified.kind(),
            SqlStatementKind::Select
                | SqlStatementKind::Insert(_)
                | SqlStatementKind::Update(_)
                | SqlStatementKind::Delete(_)
                | SqlStatementKind::SetSearchPath(_)
                | SqlStatementKind::Savepoint(_)
                | SqlStatementKind::ReleaseSavepoint(_)
                | SqlStatementKind::RollbackToSavepoint(_)
                | SqlStatementKind::SetTransaction
                | SqlStatementKind::Call(_)
                | SqlStatementKind::DataFusionMetaCommand
        )
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
    error_info("ERROR", error.sqlstate(), error.user_message())
}

fn is_rollback_to_savepoint_sql(sql: &str) -> bool {
    let words: Vec<String> = sql
        .trim()
        .trim_end_matches(';')
        .split_whitespace()
        .take(3)
        .map(|w| w.to_ascii_uppercase())
        .collect();
    matches!(
        words.iter().map(String::as_str).collect::<Vec<_>>().as_slice(),
        ["ROLLBACK", "TO", ..] | ["ROLLBACK", "WORK", "TO"]
    )
}

fn error_info(
    severity: impl Into<String>,
    code: impl Into<String>,
    message: impl Into<String>,
) -> ErrorInfo {
    ErrorInfo::new(severity.into(), code.into(), message.into())
}
