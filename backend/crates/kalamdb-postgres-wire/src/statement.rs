use std::sync::Arc;

use async_trait::async_trait;
use kalamdb_core::{
    app_context::AppContext,
    sql::{
        context::ExecutionContext,
        executor::{
            parameter_binding::max_placeholder_index, PreparedExecutionStatement, SqlExecutor,
        },
        functions::classify_postgres_show,
    },
};
use pgwire::{
    api::{
        portal::Format,
        results::{FieldFormat, FieldInfo},
        stmt::QueryParser,
        ClientInfo, Type,
    },
    error::{ErrorInfo, PgWireError, PgWireResult},
};

use crate::{
    connection::WireConnectionState,
    pg_types::{pg_type_for_arrow, pg_type_for_field},
    sql_exec::pg_error,
};

/// Prepared statement cached at PostgreSQL wire Parse time.
#[derive(Debug, Clone)]
pub struct WireCachedStatement {
    pub metadata:        PreparedExecutionStatement,
    pub parameter_types: Vec<Type>,
    /// Result columns resolved at Parse/Describe time (empty = NoData).
    pub result_columns:  Vec<WireResultColumn>,
}

/// Name + PostgreSQL type for one result column.
#[derive(Debug, Clone)]
pub struct WireResultColumn {
    pub name:     String,
    pub datatype: Type,
}

#[derive(Clone)]
pub struct KalamQueryParser {
    app_context:  Arc<AppContext>,
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
    ) -> PgWireResult<Option<Self::Statement>>
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
            state.current_schema(),
            self.app_context.base_session_context(),
        )
        .with_request_id(format!("wire:{}", state.session_id()));

        let metadata = self
            .sql_executor
            .prepare_statement_metadata(sql, &exec_ctx)
            .map_err(classification_error_to_pg)?;
        let (parameter_types, result_columns) =
            self.prepared_metadata(&metadata, &exec_ctx, _types).await;

        Ok(Some(WireCachedStatement {
            metadata,
            parameter_types,
            result_columns,
        }))
    }

    fn get_parameter_types(&self, stmt: &Self::Statement) -> PgWireResult<Vec<Type>> {
        Ok(stmt.parameter_types.clone())
    }

    fn get_result_schema(
        &self,
        stmt: &Self::Statement,
        column_format: Option<&Format>,
    ) -> PgWireResult<Vec<FieldInfo>> {
        Ok(stmt
            .result_columns
            .iter()
            .enumerate()
            .map(|(index, column)| {
                let format =
                    column_format.map(|fmt| fmt.format_for(index)).unwrap_or(FieldFormat::Text);
                FieldInfo::new(column.name.clone(), None, None, column.datatype.clone(), format)
            })
            .collect())
    }
}

impl KalamQueryParser {
    /// Parameter types and result columns from one executor plan.
    async fn prepared_metadata(
        &self,
        metadata: &PreparedExecutionStatement,
        exec_ctx: &ExecutionContext,
        hinted_types: &[Option<Type>],
    ) -> (Vec<Type>, Vec<WireResultColumn>) {
        let sql = metadata.sql.as_str();
        let shown = classify_postgres_show(sql);
        let count = max_placeholder_index(sql);
        if count == 0 {
            if let Some(shown) = shown.as_ref() {
                return (
                    Vec::new(),
                    vec![WireResultColumn {
                        name:     shown.name.clone(),
                        datatype: Type::TEXT,
                    }],
                );
            }
        }

        let Ok(plan) = self.sql_executor.logical_plan_for_metadata(metadata, exec_ctx).await else {
            let columns = shown
                .map(|shown| WireResultColumn {
                    name:     shown.name,
                    datatype: Type::TEXT,
                })
                .into_iter()
                .collect();
            let parameter_types = (0..count)
                .map(|index| {
                    hinted_types.get(index).and_then(|ty| ty.clone()).unwrap_or(Type::TEXT)
                })
                .collect();
            return (parameter_types, columns);
        };

        let parameter_types = if count == 0 {
            Vec::new()
        } else {
            let mapped = plan.get_parameter_types().ok();
            let mut types = Vec::with_capacity(count);
            for index in 1..=count {
                let hinted = hinted_types.get(index - 1).and_then(|ty| ty.clone());
                let parameter_id = format!("${index}");
                types.push(
                    hinted.unwrap_or_else(|| {
                        mapped
                            .as_ref()
                            .and_then(|parameter_types| parameter_types.get(&parameter_id))
                            .and_then(|data_type| data_type.as_ref())
                            .map(pg_type_for_arrow)
                            .transpose()
                            .ok()
                            .flatten()
                            .unwrap_or(Type::TEXT)
                    }),
                );
            }
            types
        };

        let columns = if let Some(shown) = shown {
            vec![WireResultColumn {
                name:     shown.name,
                datatype: Type::TEXT,
            }]
        } else {
            plan.schema()
                .fields()
                .iter()
                .map(|field| WireResultColumn {
                    name:     field.name().to_string(),
                    datatype: pg_type_for_field(field).unwrap_or(Type::TEXT),
                })
                .collect()
        };
        (parameter_types, columns)
    }
}

fn classification_error_to_pg(
    error: kalamdb_sql::classifier::StatementClassificationError,
) -> PgWireError {
    let message = error.user_message().to_string();
    PgWireError::UserError(Box::new(ErrorInfo::new(
        "ERROR".to_string(),
        "42601".to_string(),
        message,
    )))
}
