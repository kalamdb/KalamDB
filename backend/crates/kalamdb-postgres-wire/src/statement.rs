use std::sync::Arc;

use arrow::datatypes::{DataType, Field};
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
    connection::WireConnectionState, pg_types::pg_type_for_named_field, sql_exec::pg_error,
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
        let (parameter_types, result_columns) = self.prepared_metadata(sql).await;

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
    async fn prepared_metadata(&self, sql: &str) -> (Vec<Type>, Vec<WireResultColumn>) {
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

        let Ok(plan) = self.sql_executor.logical_plan(sql).await else {
            let columns = shown
                .map(|shown| WireResultColumn {
                    name:     shown.name,
                    datatype: Type::TEXT,
                })
                .into_iter()
                .collect();
            return (vec![Type::TEXT; count], columns);
        };

        let parameter_types = if count == 0 {
            Vec::new()
        } else {
            let mapped = plan.get_parameter_types().ok();
            let mut types = Vec::with_capacity(count);
            for index in 1..=count {
                let parameter_id = format!("${index}");
                types.push(
                    mapped
                        .as_ref()
                        .and_then(|parameter_types| parameter_types.get(&parameter_id))
                        .and_then(|data_type| data_type.as_ref())
                        .map(pg_type_for_data_type)
                        .transpose()
                        .ok()
                        .flatten()
                        .unwrap_or(Type::TEXT),
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

fn pg_type_for_field(field: &Field) -> PgWireResult<Type> {
    if let Some(pg_type) = pg_type_for_named_field(field) {
        return Ok(pg_type);
    }
    pg_type_for_data_type(field.data_type())
}

fn pg_type_for_data_type(data_type: &DataType) -> PgWireResult<Type> {
    Ok(match data_type {
        DataType::Boolean => Type::BOOL,
        DataType::Int8 | DataType::Int16 => Type::INT2,
        DataType::Int32 | DataType::UInt8 | DataType::UInt16 => Type::INT4,
        DataType::Int64 | DataType::UInt32 => Type::INT8,
        DataType::Float32 => Type::FLOAT4,
        DataType::Float64 => Type::FLOAT8,
        DataType::Date32 | DataType::Date64 => Type::DATE,
        DataType::Timestamp(_, _) => Type::TIMESTAMP,
        DataType::Utf8 | DataType::LargeUtf8 | DataType::Utf8View | DataType::Null => Type::TEXT,
        _ => Type::TEXT,
    })
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
