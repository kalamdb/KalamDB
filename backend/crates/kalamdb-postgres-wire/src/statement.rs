use std::sync::Arc;

use async_trait::async_trait;
use kalamdb_core::{
    app_context::AppContext,
    sql::{
        context::ExecutionContext,
        executor::{PreparedExecutionStatement, SqlExecutor},
    },
};
use pgwire::{
    api::{portal::Format, results::FieldInfo, stmt::QueryParser, ClientInfo, Type},
    error::{ErrorInfo, PgWireError, PgWireResult},
};

use crate::{connection::WireConnectionState, sql_exec::pg_error};

/// Prepared statement cached at PostgreSQL wire Parse time.
#[derive(Debug, Clone)]
pub struct WireCachedStatement {
    pub metadata: PreparedExecutionStatement,
}

#[derive(Clone)]
pub struct KalamQueryParser {
    app_context: Arc<AppContext>,
    sql_executor: Arc<SqlExecutor>,
}

impl KalamQueryParser {
    pub fn new(app_context: Arc<AppContext>, sql_executor: Arc<SqlExecutor>) -> Self {
        Self {
            app_context,
            sql_executor,
        }
    }
}

#[async_trait]
impl QueryParser for KalamQueryParser {
    type Statement = WireCachedStatement;

    async fn parse_sql<C>(
        &self,
        client: &C,
        sql: &str,
        _types: &[Option<Type>],
    ) -> PgWireResult<Self::Statement>
    where
        C: ClientInfo + Unpin + Send + Sync,
    {
        let state = client
            .session_extensions()
            .get::<WireConnectionState>()
            .ok_or_else(|| pg_error("wire connection state is missing"))?;

        let exec_ctx = ExecutionContext::with_namespace(
            state.auth().user_id.clone(),
            state.auth().role,
            state.current_schema().clone(),
            self.app_context.base_session_context(),
        )
        .with_request_id(format!("wire:{}", state.session_id()));

        let metadata = self
            .sql_executor
            .prepare_statement_metadata(sql, &exec_ctx)
            .map_err(classification_error_to_pg)?;

        Ok(WireCachedStatement { metadata })
    }

    fn get_parameter_types(&self, _stmt: &Self::Statement) -> PgWireResult<Vec<Type>> {
        Ok(vec![])
    }

    fn get_result_schema(
        &self,
        _stmt: &Self::Statement,
        _column_format: Option<&Format>,
    ) -> PgWireResult<Vec<FieldInfo>> {
        Ok(vec![])
    }
}

fn classification_error_to_pg(
    error: kalamdb_sql::classifier::StatementClassificationError,
) -> PgWireError {
    let message = match error {
        kalamdb_sql::classifier::StatementClassificationError::Unauthorized(msg) => msg,
        kalamdb_sql::classifier::StatementClassificationError::InvalidSql { message, .. } => {
            message
        },
    };
    PgWireError::UserError(Box::new(ErrorInfo::new(
        "ERROR".to_string(),
        "42601".to_string(),
        message,
    )))
}
