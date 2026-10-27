//! Catalog routine calls inside SELECT, including `SELECT * FROM schema.func()`.

use std::sync::Arc;

use arrow::{
    array::{Array, RecordBatch},
    datatypes::{Field, Schema},
};
use datafusion::scalar::ScalarValue;
use kalamdb_commons::models::SchemaId;
use sqlparser::ast::Statement;

use crate::{
    error::KalamDbError,
    sql::{executor::SqlExecutor, ExecutionContext, ExecutionResult},
};

fn literal_limit_offset(
    clause: Option<&sqlparser::ast::LimitClause>,
) -> Result<(u64, Option<u64>), KalamDbError> {
    use sqlparser::ast::LimitClause;

    let Some(clause) = clause else {
        return Ok((0, None));
    };
    let (limit, offset) = match clause {
        LimitClause::LimitOffset {
            limit,
            offset,
            limit_by,
        } => {
            if !limit_by.is_empty() {
                return Err(KalamDbError::InvalidSql(
                    "LIMIT BY is not supported on a routine preview".to_string(),
                ));
            }
            (limit.as_ref(), offset.as_ref().map(|offset| &offset.value))
        },
        LimitClause::OffsetCommaLimit { offset, limit } => (Some(limit), Some(offset)),
    };
    let limit = limit.map(literal_row_count).transpose()?;
    let offset = offset.map(literal_row_count).transpose()?.unwrap_or(0);
    Ok((offset, limit))
}

fn literal_row_count(expr: &sqlparser::ast::Expr) -> Result<u64, KalamDbError> {
    use sqlparser::ast::{Expr, Value};

    match expr {
        Expr::Value(value) => match &value.value {
            Value::Number(text, false) => text.parse::<u64>().map_err(|_| {
                KalamDbError::InvalidSql(format!("invalid LIMIT or OFFSET value '{text}'"))
            }),
            _ => Err(KalamDbError::InvalidSql(
                "LIMIT and OFFSET on a routine preview must be integer literals".to_string(),
            )),
        },
        _ => Err(KalamDbError::InvalidSql(
            "LIMIT and OFFSET on a routine preview must be integer literals".to_string(),
        )),
    }
}

impl SqlExecutor {
    pub(super) async fn try_execute_standalone_catalog_function_select(
        &self,
        statement: &mut Statement,
        exec_ctx: &ExecutionContext,
        params: &[ScalarValue],
        had_catalog_func: &mut bool,
    ) -> Result<Option<ExecutionResult>, KalamDbError> {
        if !self.app_context.system_tables().catalog_stores().has_user_routines() {
            return Ok(None);
        }
        let Statement::Query(query) = statement else {
            return Ok(None);
        };
        if query.with.is_some()
            || query.order_by.is_some()
            || query.fetch.is_some()
            || !query.locks.is_empty()
        {
            return Ok(None);
        }
        let limit_clause = query.limit_clause.clone();
        let sqlparser::ast::SetExpr::Select(select) = query.body.as_mut() else {
            return Ok(None);
        };
        if let Some(result) = self
            .try_execute_catalog_function_from_clause(
                limit_clause.as_ref(),
                select,
                exec_ctx,
                params,
            )
            .await?
        {
            *had_catalog_func = true;
            return Ok(Some(result));
        }
        if query.limit_clause.is_some() {
            return Ok(None);
        }
        if select.distinct.is_some()
            || select.top.is_some()
            || !select.from.is_empty()
            || !select.lateral_views.is_empty()
            || select.selection.is_some()
            || !matches!(
                &select.group_by,
                sqlparser::ast::GroupByExpr::Expressions(exprs, _) if exprs.is_empty()
            )
            || !select.cluster_by.is_empty()
            || !select.distribute_by.is_empty()
            || !select.sort_by.is_empty()
            || select.having.is_some()
            || !select.named_window.is_empty()
            || select.qualify.is_some()
            || select.projection.len() != 1
        {
            return Ok(None);
        }

        let (col_name, func) = match &mut select.projection[0] {
            sqlparser::ast::SelectItem::UnnamedExpr(sqlparser::ast::Expr::Function(func)) => {
                let name = func
                    .name
                    .0
                    .last()
                    .map(|part| part.to_string().trim_matches('"').to_ascii_lowercase())
                    .unwrap_or_else(|| "result".to_string());
                (name, func)
            },
            sqlparser::ast::SelectItem::ExprWithAlias {
                expr: sqlparser::ast::Expr::Function(func),
                alias,
            } => (alias.value.clone(), func),
            _ => return Ok(None),
        };

        let Some(routine_id) =
            Self::resolve_catalog_routine_id(&func.name, &exec_ctx.default_namespace())
        else {
            return Ok(None);
        };
        let stores = self.app_context.system_tables().catalog_stores();
        let Ok(Some(routine)) = stores.get_routine(&routine_id) else {
            return Ok(None);
        };
        if routine.language.as_deref() == Some("INTERNAL") {
            return Ok(None);
        }
        if routine.kind == kalamdb_commons::models::RoutineKind::Procedure {
            return Err(KalamDbError::InvalidSql(format!(
                "{} is a procedure, not a function; invoke it with CALL",
                routine_id
            )));
        }

        if let sqlparser::ast::FunctionArguments::List(list) = &mut func.args {
            for arg in &mut list.args {
                if let sqlparser::ast::FunctionArg::Unnamed(
                    sqlparser::ast::FunctionArgExpr::Expr(arg_expr),
                ) = arg
                {
                    self.evaluate_catalog_functions_in_expr(
                        arg_expr,
                        exec_ctx,
                        params,
                        had_catalog_func,
                    )
                    .await?;
                }
            }
        }

        let call = kalamdb_sql::ddl::column_default::function_to_routine_call_with_placeholders(
            func,
            &exec_ctx.default_namespace(),
        )
        .map_err(KalamDbError::InvalidSql)?;

        let rv = crate::functions::FunctionService::execute_scalar_function_call(
            Arc::clone(&self.app_context),
            exec_ctx,
            &call,
            params,
        )
        .await?;
        *had_catalog_func = true;

        if matches!(
            &rv.value,
            ScalarValue::Struct(_)
                | ScalarValue::List(_)
                | ScalarValue::LargeList(_)
                | ScalarValue::FixedSizeList(_)
        ) {
            let array = rv
                .value
                .to_array()
                .map_err(|error| KalamDbError::ExecutionError(error.to_string()))?;
            let field = arrow::datatypes::Field::new(col_name, array.data_type().clone(), true);
            let schema = Arc::new(arrow::datatypes::Schema::new(vec![field]));
            let batch = arrow::array::RecordBatch::try_new(Arc::clone(&schema), vec![array])
                .map_err(|error| KalamDbError::ExecutionError(error.to_string()))?;
            return Ok(Some(ExecutionResult::Rows {
                row_count: batch.num_rows(),
                batches: vec![batch],
                schema: Some(schema),
            }));
        }

        let ast_expr = Self::routine_value_to_ast_expr(&rv, &routine)?;
        select.projection[0] = sqlparser::ast::SelectItem::ExprWithAlias {
            expr: ast_expr,
            alias: sqlparser::ast::Ident::new(col_name),
        };
        Ok(None)
    }

    async fn evaluate_returning_items(
        &self,
        returning: &mut Option<Vec<sqlparser::ast::SelectItem>>,
        exec_ctx: &ExecutionContext,
        params: &[ScalarValue],
        changed: &mut bool,
    ) -> Result<(), KalamDbError> {
        if let Some(returning) = returning {
            for item in returning {
                self.evaluate_catalog_functions_in_select_item(item, exec_ctx, params, changed)
                    .await?;
            }
        }
        Ok(())
    }

    pub(super) async fn evaluate_catalog_functions_in_statement(
        &self,
        statement: &mut Statement,
        exec_ctx: &ExecutionContext,
        params: &[ScalarValue],
    ) -> Result<bool, KalamDbError> {
        if !self.app_context.system_tables().catalog_stores().has_user_routines() {
            return Ok(false);
        }
        let mut changed = false;
        self.evaluate_catalog_functions_in_statement_inner(
            statement,
            exec_ctx,
            params,
            &mut changed,
        )
        .await?;
        Ok(changed)
    }

    pub(super) async fn evaluate_catalog_functions_in_statement_inner(
        &self,
        statement: &mut Statement,
        exec_ctx: &ExecutionContext,
        params: &[ScalarValue],
        changed: &mut bool,
    ) -> Result<(), KalamDbError> {
        match statement {
            Statement::Query(query) => {
                self.evaluate_catalog_functions_in_query(query, exec_ctx, params, changed)
                    .await?;
            },
            Statement::Insert(insert) => {
                if let Some(source) = &mut insert.source {
                    self.evaluate_catalog_functions_in_query(source, exec_ctx, params, changed)
                        .await?;
                }
                self.evaluate_returning_items(&mut insert.returning, exec_ctx, params, changed)
                    .await?;
            },
            Statement::Update(update) => {
                for assignment in &mut update.assignments {
                    self.evaluate_catalog_functions_in_expr(
                        &mut assignment.value,
                        exec_ctx,
                        params,
                        changed,
                    )
                    .await?;
                }
                if let Some(selection) = &mut update.selection {
                    self.evaluate_catalog_functions_in_expr(selection, exec_ctx, params, changed)
                        .await?;
                }
                self.evaluate_returning_items(&mut update.returning, exec_ctx, params, changed)
                    .await?;
            },
            Statement::Delete(delete) => {
                if let Some(selection) = &mut delete.selection {
                    self.evaluate_catalog_functions_in_expr(selection, exec_ctx, params, changed)
                        .await?;
                }
                self.evaluate_returning_items(&mut delete.returning, exec_ctx, params, changed)
                    .await?;
            },
            _ => {},
        }
        Ok(())
    }

    pub(super) async fn evaluate_catalog_functions_in_query(
        &self,
        query: &mut sqlparser::ast::Query,
        exec_ctx: &ExecutionContext,
        params: &[ScalarValue],
        changed: &mut bool,
    ) -> Result<(), KalamDbError> {
        if let Some(with) = &mut query.with {
            for cte in &mut with.cte_tables {
                Box::pin(self.evaluate_catalog_functions_in_query(
                    &mut cte.query,
                    exec_ctx,
                    params,
                    changed,
                ))
                .await?;
            }
        }
        Box::pin(self.evaluate_catalog_functions_in_set_expr(
            &mut query.body,
            exec_ctx,
            params,
            changed,
        ))
        .await?;
        Ok(())
    }

    pub(super) async fn evaluate_catalog_functions_in_set_expr(
        &self,
        set_expr: &mut sqlparser::ast::SetExpr,
        exec_ctx: &ExecutionContext,
        params: &[ScalarValue],
        changed: &mut bool,
    ) -> Result<(), KalamDbError> {
        match set_expr {
            sqlparser::ast::SetExpr::Select(select) => {
                for item in &mut select.projection {
                    self.evaluate_catalog_functions_in_select_item(item, exec_ctx, params, changed)
                        .await?;
                }
                if let Some(selection) = &mut select.selection {
                    self.evaluate_catalog_functions_in_expr(selection, exec_ctx, params, changed)
                        .await?;
                }
                if let Some(having) = &mut select.having {
                    self.evaluate_catalog_functions_in_expr(having, exec_ctx, params, changed)
                        .await?;
                }
            },
            sqlparser::ast::SetExpr::Values(values) => {
                for row in &mut values.rows {
                    for expr in row.iter_mut() {
                        self.evaluate_catalog_functions_in_expr(expr, exec_ctx, params, changed)
                            .await?;
                    }
                }
            },
            sqlparser::ast::SetExpr::Query(inner) => {
                Box::pin(
                    self.evaluate_catalog_functions_in_query(inner, exec_ctx, params, changed),
                )
                .await?;
            },
            sqlparser::ast::SetExpr::SetOperation { left, right, .. } => {
                Box::pin(
                    self.evaluate_catalog_functions_in_set_expr(left, exec_ctx, params, changed),
                )
                .await?;
                Box::pin(
                    self.evaluate_catalog_functions_in_set_expr(right, exec_ctx, params, changed),
                )
                .await?;
            },
            _ => {},
        }
        Ok(())
    }

    pub(super) async fn evaluate_catalog_functions_in_select_item(
        &self,
        item: &mut sqlparser::ast::SelectItem,
        exec_ctx: &ExecutionContext,
        params: &[ScalarValue],
        changed: &mut bool,
    ) -> Result<(), KalamDbError> {
        match item {
            sqlparser::ast::SelectItem::UnnamedExpr(expr) => {
                let alias = if let sqlparser::ast::Expr::Function(func) = expr {
                    func.name
                        .0
                        .last()
                        .map(|part| part.to_string().trim_matches('"').to_ascii_lowercase())
                } else {
                    None
                };
                let before = *changed;
                self.evaluate_catalog_functions_in_expr(expr, exec_ctx, params, changed).await?;
                if *changed != before {
                    if let Some(alias_name) = alias {
                        let replaced = std::mem::replace(
                            expr,
                            sqlparser::ast::Expr::Value(sqlparser::ast::Value::Null.into()),
                        );
                        *item = sqlparser::ast::SelectItem::ExprWithAlias {
                            expr: replaced,
                            alias: sqlparser::ast::Ident::new(alias_name),
                        };
                    }
                }
            },
            sqlparser::ast::SelectItem::ExprWithAlias { expr, .. } => {
                self.evaluate_catalog_functions_in_expr(expr, exec_ctx, params, changed).await?;
            },
            _ => {},
        }
        Ok(())
    }

    pub(super) async fn evaluate_catalog_functions_in_expr(
        &self,
        expr: &mut sqlparser::ast::Expr,
        exec_ctx: &ExecutionContext,
        params: &[ScalarValue],
        changed: &mut bool,
    ) -> Result<(), KalamDbError> {
        match expr {
            sqlparser::ast::Expr::Nested(inner) => {
                Box::pin(self.evaluate_catalog_functions_in_expr(inner, exec_ctx, params, changed))
                    .await?;
            },
            sqlparser::ast::Expr::BinaryOp { left, right, .. } => {
                Box::pin(self.evaluate_catalog_functions_in_expr(left, exec_ctx, params, changed))
                    .await?;
                Box::pin(self.evaluate_catalog_functions_in_expr(right, exec_ctx, params, changed))
                    .await?;
            },
            sqlparser::ast::Expr::UnaryOp { expr: inner, .. } => {
                Box::pin(self.evaluate_catalog_functions_in_expr(inner, exec_ctx, params, changed))
                    .await?;
            },
            sqlparser::ast::Expr::Cast { expr: inner, .. } => {
                Box::pin(self.evaluate_catalog_functions_in_expr(inner, exec_ctx, params, changed))
                    .await?;
            },
            sqlparser::ast::Expr::IsNull(inner) | sqlparser::ast::Expr::IsNotNull(inner) => {
                Box::pin(self.evaluate_catalog_functions_in_expr(inner, exec_ctx, params, changed))
                    .await?;
            },
            sqlparser::ast::Expr::Function(func) => {
                if let sqlparser::ast::FunctionArguments::List(list) = &mut func.args {
                    for arg in &mut list.args {
                        if let sqlparser::ast::FunctionArg::Unnamed(
                            sqlparser::ast::FunctionArgExpr::Expr(arg_expr),
                        ) = arg
                        {
                            Box::pin(self.evaluate_catalog_functions_in_expr(
                                arg_expr, exec_ctx, params, changed,
                            ))
                            .await?;
                        }
                    }
                }
                if let Some(routine_id) =
                    Self::resolve_catalog_routine_id(&func.name, &exec_ctx.default_namespace())
                {
                    let stores = self.app_context.system_tables().catalog_stores();
                    if let Ok(Some(routine)) = stores.get_routine(&routine_id) {
                        if routine.language.as_deref() != Some("INTERNAL") {
                            if routine.kind == kalamdb_commons::models::RoutineKind::Procedure {
                                return Err(KalamDbError::InvalidSql(format!(
                                    "{} is a procedure, not a function; invoke it with CALL",
                                    routine_id
                                )));
                            }
                            let call = kalamdb_sql::ddl::column_default::function_to_routine_call_with_placeholders(
                                func,
                                &exec_ctx.default_namespace(),
                            )
                            .map_err(KalamDbError::InvalidSql)?;
                            let rv = Box::pin(
                                crate::functions::FunctionService::execute_scalar_function_call(
                                    Arc::clone(&self.app_context),
                                    exec_ctx,
                                    &call,
                                    params,
                                ),
                            )
                            .await?;
                            *expr = Self::routine_value_to_ast_expr(&rv, &routine)?;
                            *changed = true;
                        }
                    }
                }
            },
            _ => {},
        }
        Ok(())
    }

    /// `SELECT * FROM schema.func(args) [LIMIT n OFFSET m]`.
    ///
    /// PostgreSQL clients preview a routine the same way they preview a table.
    /// DataFusion plans that `FROM` item as a table function and keeps only the
    /// first name part, so `"schema"."health"()` is looked up as the schema.
    /// A catalog routine runs here, on the shared SQL path, before that lookup.
    pub(super) async fn try_execute_catalog_function_from_clause(
        &self,
        limit_clause: Option<&sqlparser::ast::LimitClause>,
        select: &sqlparser::ast::Select,
        exec_ctx: &ExecutionContext,
        params: &[ScalarValue],
    ) -> Result<Option<ExecutionResult>, KalamDbError> {
        use sqlparser::ast::{
            Function, FunctionArgumentList, FunctionArguments, ObjectNamePart, SelectItem,
            TableFactor,
        };

        if select.distinct.is_some()
            || select.top.is_some()
            || !select.lateral_views.is_empty()
            || select.selection.is_some()
            || !matches!(
                &select.group_by,
                sqlparser::ast::GroupByExpr::Expressions(exprs, _) if exprs.is_empty()
            )
            || !select.cluster_by.is_empty()
            || !select.distribute_by.is_empty()
            || !select.sort_by.is_empty()
            || select.having.is_some()
            || !select.named_window.is_empty()
            || select.qualify.is_some()
            || !matches!(select.projection.as_slice(), [SelectItem::Wildcard(_)])
        {
            return Ok(None);
        }
        let [from] = select.from.as_slice() else {
            return Ok(None);
        };
        if !from.joins.is_empty() {
            return Ok(None);
        }
        let TableFactor::Table {
            name,
            args: Some(func_args),
            with_ordinality: false,
            ..
        } = &from.relation
        else {
            return Ok(None);
        };
        if func_args.settings.is_some() {
            return Ok(None);
        }
        let Some(routine_id) =
            Self::resolve_catalog_routine_id(name, &exec_ctx.default_namespace())
        else {
            return Ok(None);
        };
        let stores = self.app_context.system_tables().catalog_stores();
        let Ok(Some(routine)) = stores.get_routine(&routine_id) else {
            return Ok(None);
        };
        if routine.language.as_deref() == Some("INTERNAL") {
            return Ok(None);
        }
        if routine.kind == kalamdb_commons::models::RoutineKind::Procedure {
            return Err(KalamDbError::InvalidSql(format!(
                "{routine_id} is a procedure, not a function; invoke it with CALL"
            )));
        }
        let func = Function {
            name: name.clone(),
            uses_odbc_syntax: false,
            parameters: FunctionArguments::None,
            args: FunctionArguments::List(FunctionArgumentList {
                duplicate_treatment: None,
                args: func_args.args.clone(),
                clauses: Vec::new(),
            }),
            filter: None,
            null_treatment: None,
            over: None,
            within_group: Vec::new(),
        };
        let call = kalamdb_sql::ddl::column_default::function_to_routine_call_with_placeholders(
            &func,
            &exec_ctx.default_namespace(),
        )
        .map_err(KalamDbError::InvalidSql)?;
        let rv = crate::functions::FunctionService::execute_scalar_function_call(
            Arc::clone(&self.app_context),
            exec_ctx,
            &call,
            params,
        )
        .await?;
        let column_name = name
            .0
            .last()
            .and_then(|part| match part {
                ObjectNamePart::Identifier(ident) => Some(ident.value.clone()),
                _ => None,
            })
            .unwrap_or_else(|| "result".to_string());
        let array = rv.value.to_array().map_err(|err| KalamDbError::InvalidSql(err.to_string()))?;
        let schema =
            Arc::new(Schema::new(vec![Field::new(column_name, array.data_type().clone(), true)]));
        let (offset, limit) = literal_limit_offset(limit_clause)?;
        let include = offset == 0 && limit != Some(0);
        let batch = if include {
            RecordBatch::try_new(Arc::clone(&schema), vec![array])
                .map_err(|err| KalamDbError::InvalidSql(err.to_string()))?
        } else {
            RecordBatch::new_empty(Arc::clone(&schema))
        };
        Ok(Some(ExecutionResult::Rows {
            row_count: usize::from(include),
            batches: vec![batch],
            schema: Some(schema),
        }))
    }

    pub(super) fn resolve_catalog_routine_id(
        name: &sqlparser::ast::ObjectName,
        default_namespace: &SchemaId,
    ) -> Option<kalamdb_commons::models::RoutineId> {
        let parts: Vec<String> = name
            .0
            .iter()
            .filter_map(|part| match part {
                sqlparser::ast::ObjectNamePart::Identifier(ident) => {
                    if ident.quote_style.is_some() {
                        Some(ident.value.clone())
                    } else {
                        Some(ident.value.to_ascii_lowercase())
                    }
                },
                _ => None,
            })
            .collect();
        match parts.as_slice() {
            [unqualified] => {
                if kalamdb_commons::models::is_builtin_default_name(unqualified) {
                    None
                } else {
                    Some(kalamdb_commons::models::RoutineId::from_parts(
                        Some(default_namespace),
                        unqualified,
                    ))
                }
            },
            [schema, unqualified] => Some(kalamdb_commons::models::RoutineId::from_parts(
                Some(&SchemaId::new(schema)),
                unqualified,
            )),
            _ => None,
        }
    }

    pub(super) fn routine_value_to_ast_expr(
        rv: &kalamdb_functions::RoutineValue,
        routine: &kalamdb_system::CatalogRoutine,
    ) -> Result<sqlparser::ast::Expr, KalamDbError> {
        use kalamdb_commons::KalamDataType;
        use sqlparser::ast::{DataType as AstDataType, Expr as AstExpr, Value as AstValue};
        let scalar = &rv.value;
        let base_expr = match scalar {
            ScalarValue::Null
            | ScalarValue::Boolean(None)
            | ScalarValue::Int8(None)
            | ScalarValue::Int16(None)
            | ScalarValue::Int32(None)
            | ScalarValue::Int64(None)
            | ScalarValue::UInt8(None)
            | ScalarValue::UInt16(None)
            | ScalarValue::UInt32(None)
            | ScalarValue::UInt64(None)
            | ScalarValue::Float32(None)
            | ScalarValue::Float64(None)
            | ScalarValue::Utf8(None)
            | ScalarValue::LargeUtf8(None)
            | ScalarValue::Utf8View(None) => match routine.return_data_type {
                Some(KalamDataType::Boolean) => AstExpr::Cast {
                    kind: sqlparser::ast::CastKind::Cast,
                    expr: Box::new(AstExpr::Value(AstValue::Null.into())),
                    data_type: AstDataType::Boolean,
                    array: false,
                    format: None,
                },
                Some(KalamDataType::SmallInt) => AstExpr::Cast {
                    kind: sqlparser::ast::CastKind::Cast,
                    expr: Box::new(AstExpr::Value(AstValue::Null.into())),
                    data_type: AstDataType::SmallInt(None),
                    array: false,
                    format: None,
                },
                Some(KalamDataType::Int) => AstExpr::Cast {
                    kind: sqlparser::ast::CastKind::Cast,
                    expr: Box::new(AstExpr::Value(AstValue::Null.into())),
                    data_type: AstDataType::Int(None),
                    array: false,
                    format: None,
                },
                Some(KalamDataType::BigInt) => AstExpr::Cast {
                    kind: sqlparser::ast::CastKind::Cast,
                    expr: Box::new(AstExpr::Value(AstValue::Null.into())),
                    data_type: AstDataType::BigInt(None),
                    array: false,
                    format: None,
                },
                Some(KalamDataType::Double | KalamDataType::Float) => AstExpr::Cast {
                    kind: sqlparser::ast::CastKind::Cast,
                    expr: Box::new(AstExpr::Value(AstValue::Null.into())),
                    data_type: AstDataType::Double(sqlparser::ast::ExactNumberInfo::None),
                    array: false,
                    format: None,
                },
                _ => AstExpr::Cast {
                    kind: sqlparser::ast::CastKind::Cast,
                    expr: Box::new(AstExpr::Value(AstValue::Null.into())),
                    data_type: AstDataType::Text,
                    array: false,
                    format: None,
                },
            },
            ScalarValue::Boolean(Some(b)) => AstExpr::Value(AstValue::Boolean(*b).into()),
            ScalarValue::Int8(Some(n)) => {
                AstExpr::Value(AstValue::Number(n.to_string(), false).into())
            },
            ScalarValue::Int16(Some(n)) => {
                AstExpr::Value(AstValue::Number(n.to_string(), false).into())
            },
            ScalarValue::Int32(Some(n)) => {
                AstExpr::Value(AstValue::Number(n.to_string(), false).into())
            },
            ScalarValue::Int64(Some(n)) => {
                AstExpr::Value(AstValue::Number(n.to_string(), false).into())
            },
            ScalarValue::UInt8(Some(n)) => {
                AstExpr::Value(AstValue::Number(n.to_string(), false).into())
            },
            ScalarValue::UInt16(Some(n)) => {
                AstExpr::Value(AstValue::Number(n.to_string(), false).into())
            },
            ScalarValue::UInt32(Some(n)) => {
                AstExpr::Value(AstValue::Number(n.to_string(), false).into())
            },
            ScalarValue::UInt64(Some(n)) => {
                AstExpr::Value(AstValue::Number(n.to_string(), false).into())
            },
            ScalarValue::Float32(Some(n)) => {
                AstExpr::Value(AstValue::Number(n.to_string(), false).into())
            },
            ScalarValue::Float64(Some(n)) => {
                AstExpr::Value(AstValue::Number(n.to_string(), false).into())
            },
            ScalarValue::Utf8(Some(s))
            | ScalarValue::LargeUtf8(Some(s))
            | ScalarValue::Utf8View(Some(s)) => {
                AstExpr::Value(AstValue::SingleQuotedString(s.clone()).into())
            },
            other => {
                if let Some(json) = crate::functions::routine_value_as_json(rv) {
                    match json {
                        serde_json::Value::Null => AstExpr::Value(AstValue::Null.into()),
                        serde_json::Value::Bool(b) => AstExpr::Value(AstValue::Boolean(b).into()),
                        serde_json::Value::Number(num) => {
                            AstExpr::Value(AstValue::Number(num.to_string(), false).into())
                        },
                        serde_json::Value::String(s) => {
                            AstExpr::Value(AstValue::SingleQuotedString(s).into())
                        },
                        other_json => AstExpr::Value(
                            AstValue::SingleQuotedString(other_json.to_string()).into(),
                        ),
                    }
                } else {
                    AstExpr::Value(AstValue::SingleQuotedString(other.to_string()).into())
                }
            },
        };
        Ok(base_expr)
    }
}
