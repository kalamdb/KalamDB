//! Direct DML for `system.migrations` (literal INSERT/UPDATE, no DataFusion).

use std::collections::HashMap;

use kalamdb_commons::{
    models::{MigrationId, TableId},
    SystemTable,
};
use kalamdb_system::Migration;
use sqlparser::ast::{
    Assignment, AssignmentTarget, BinaryOperator, Expr, ObjectName, ObjectNamePart, Statement,
    TableFactor, TableObject, Value,
};

use super::DmlKind;
use crate::{
    error::KalamDbError,
    sql::{
        executor::{PreparedExecutionStatement, SqlExecutor},
        ExecutionResult,
    },
};

impl SqlExecutor {
    pub(super) fn is_system_migrations_table(table_id: Option<&TableId>) -> bool {
        table_id
            .filter(|table_id| {
                table_id.namespace_id().is_system_namespace()
                    && SystemTable::from_name(table_id.table_name().as_str())
                        .is_ok_and(|table| table == SystemTable::Migrations)
            })
            .is_some()
    }

    pub(super) async fn try_execute_system_migrations_dml(
        &self,
        metadata: &PreparedExecutionStatement,
        dml_kind: DmlKind,
    ) -> Result<Option<ExecutionResult>, KalamDbError> {
        if !Self::is_system_migrations_table(metadata.table_id.as_ref()) {
            return Ok(None);
        }

        match dml_kind {
            DmlKind::Insert => {
                let statement = metadata.parsed_dml.as_ref().ok_or_else(|| {
                    KalamDbError::InvalidSql(
                        "Missing prepared DML metadata for system.migrations INSERT".to_string(),
                    )
                })?;
                let Statement::Insert(insert) = statement else {
                    return Ok(None);
                };
                match &insert.table {
                    TableObject::TableName(name) => expect_system_migrations_object(name)?,
                    TableObject::TableFunction(_) | TableObject::TableQuery(_) => {
                        return Err(KalamDbError::InvalidSql(
                            "INSERT system.migrations requires a table name".to_string(),
                        ));
                    },
                }

                let Some(source) = insert.source.as_ref() else {
                    return Err(KalamDbError::InvalidSql(
                        "INSERT system.migrations requires VALUES".to_string(),
                    ));
                };
                if source.with.is_some()
                    || source.order_by.is_some()
                    || source.limit_clause.is_some()
                {
                    return Err(KalamDbError::InvalidSql(
                        "INSERT system.migrations supports VALUES only".to_string(),
                    ));
                }
                let row = kalamdb_sql::single_values_insert_row(&insert).map_err(|reason| {
                    KalamDbError::InvalidSql(match reason {
                        "insert requires values" => {
                            "INSERT system.migrations requires VALUES".to_string()
                        },
                        "insert supports exactly one row" => {
                            "INSERT system.migrations supports exactly one row".to_string()
                        },
                        other => other.to_string(),
                    })
                })?;
                if insert.columns.len() != row.len() {
                    return Err(KalamDbError::InvalidSql(
                        "INSERT system.migrations column count does not match value count"
                            .to_string(),
                    ));
                }

                let values = insert
                    .columns
                    .iter()
                    .zip(row.iter())
                    .map(|(column, value)| {
                        (
                            kalamdb_sql::object_name_to_string(column)
                                .unwrap_or_default()
                                .to_ascii_lowercase(),
                            value,
                        )
                    })
                    .collect::<HashMap<_, _>>();
                let migration = migration_from_insert_values(&values)?;
                self.app_context
                    .system_tables()
                    .migrations()
                    .upsert_migration_async(migration)
                    .await?;
                Ok(Some(ExecutionResult::Inserted { rows_affected: 1 }))
            },
            DmlKind::Update => {
                let statement = metadata.parsed_dml.as_ref().ok_or_else(|| {
                    KalamDbError::InvalidSql(
                        "Missing prepared DML metadata for system.migrations UPDATE".to_string(),
                    )
                })?;
                let Statement::Update(update) = statement else {
                    return Ok(None);
                };
                if !update.table.joins.is_empty() {
                    return Err(KalamDbError::InvalidSql(
                        "UPDATE system.migrations does not support joins".to_string(),
                    ));
                }
                match &update.table.relation {
                    TableFactor::Table { name, .. } => expect_system_migrations_object(name)?,
                    _ => {
                        return Err(KalamDbError::InvalidSql(
                            "UPDATE system.migrations requires a table name".to_string(),
                        ));
                    },
                }

                let migration_key = migration_key_from_update_selection(update.selection.as_ref())?;
                let Some(mut migration) = self
                    .app_context
                    .system_tables()
                    .migrations()
                    .get_migration_async(&migration_key)
                    .await?
                else {
                    return Ok(Some(ExecutionResult::Updated { rows_affected: 0 }));
                };

                for assignment in &update.assignments {
                    apply_migration_assignment(&mut migration, assignment)?;
                }
                self.app_context
                    .system_tables()
                    .migrations()
                    .upsert_migration_async(migration)
                    .await?;
                Ok(Some(ExecutionResult::Updated { rows_affected: 1 }))
            },
            DmlKind::Delete => Err(KalamDbError::InvalidOperation(
                "DELETE system.migrations is not supported".to_string(),
            )),
        }
    }
}

fn object_name_parts(name: &ObjectName) -> Option<Vec<&str>> {
    name.0
        .iter()
        .map(|part| match part {
            ObjectNamePart::Identifier(ident) => Some(ident.value.as_str()),
            ObjectNamePart::Function(_) => None,
        })
        .collect()
}

fn expect_system_migrations_object(name: &ObjectName) -> Result<(), KalamDbError> {
    let parts = object_name_parts(name).ok_or_else(|| {
        KalamDbError::InvalidSql("Invalid system.migrations table name".to_string())
    })?;
    if parts.len() == 2
        && parts[0].eq_ignore_ascii_case("system")
        && parts[1].eq_ignore_ascii_case("migrations")
    {
        return Ok(());
    }
    Err(KalamDbError::InvalidSql(format!(
        "Expected system.migrations table, got {}",
        name
    )))
}

fn assignment_column_name(assignment: &Assignment) -> Result<String, KalamDbError> {
    match &assignment.target {
        AssignmentTarget::ColumnName(name) => {
            let parts = object_name_parts(name)
                .ok_or_else(|| KalamDbError::InvalidSql("Invalid assignment target".to_string()))?;
            if parts.len() == 1 {
                Ok(parts[0].to_ascii_lowercase())
            } else {
                Err(KalamDbError::InvalidSql(format!(
                    "Expected unqualified system.migrations column, got {}",
                    name
                )))
            }
        },
        AssignmentTarget::Tuple(_) => Err(KalamDbError::InvalidSql(
            "Tuple assignments are not supported for system.migrations".to_string(),
        )),
    }
}

fn expr_string(expr: &Expr, column: &str) -> Result<String, KalamDbError> {
    match expr {
        Expr::Value(value) => match &value.value {
            Value::SingleQuotedString(value)
            | Value::DoubleQuotedString(value)
            | Value::TripleSingleQuotedString(value)
            | Value::TripleDoubleQuotedString(value)
            | Value::EscapedStringLiteral(value)
            | Value::UnicodeStringLiteral(value)
            | Value::NationalStringLiteral(value)
            | Value::HexStringLiteral(value) => Ok(value.clone()),
            Value::Number(value, _) => Ok(value.clone()),
            Value::Null => Err(KalamDbError::InvalidSql(format!(
                "system.migrations column '{}' cannot be NULL",
                column
            ))),
            other => Err(KalamDbError::InvalidSql(format!(
                "Unsupported literal for system.migrations column '{}': {}",
                column, other
            ))),
        },
        other => Err(KalamDbError::InvalidSql(format!(
            "Expected literal for system.migrations column '{}', got {}",
            column, other
        ))),
    }
}

fn expr_optional_string(expr: &Expr, column: &str) -> Result<Option<String>, KalamDbError> {
    match expr {
        Expr::Value(value) if matches!(value.value, Value::Null) => Ok(None),
        _ => expr_string(expr, column).map(Some),
    }
}

fn expr_optional_i64(expr: &Expr, column: &str) -> Result<Option<i64>, KalamDbError> {
    match expr {
        Expr::Value(value) => match &value.value {
            Value::Null => Ok(None),
            Value::Number(value, _) => value.parse::<i64>().map(Some).map_err(|error| {
                KalamDbError::InvalidSql(format!(
                    "Invalid integer for system.migrations column '{}': {}",
                    column, error
                ))
            }),
            other => Err(KalamDbError::InvalidSql(format!(
                "Unsupported timestamp literal for system.migrations column '{}': {}",
                column, other
            ))),
        },
        other => Err(KalamDbError::InvalidSql(format!(
            "Expected timestamp literal for system.migrations column '{}', got {}",
            column, other
        ))),
    }
}

fn migration_from_insert_values(
    values: &HashMap<String, &Expr>,
) -> Result<Migration, KalamDbError> {
    let required = |column: &str| -> Result<&Expr, KalamDbError> {
        values.get(column).copied().ok_or_else(|| {
            KalamDbError::InvalidSql(format!(
                "Missing required system.migrations column '{}'",
                column
            ))
        })
    };
    let optional = |column: &str| -> Option<&Expr> { values.get(column).copied() };

    Ok(Migration {
        migration_key: MigrationId::new(expr_string(required("migration_key")?, "migration_key")?),
        migration_id:  expr_string(required("migration_id")?, "migration_id")?,
        namespace:     expr_string(required("namespace")?, "namespace")?,
        name:          expr_string(required("name")?, "name")?,
        checksum:      expr_string(required("checksum")?, "checksum")?,
        status:        expr_string(required("status")?, "status")?,
        started_at:    optional("started_at")
            .map(|expr| expr_optional_i64(expr, "started_at"))
            .transpose()?
            .flatten(),
        finished_at:   optional("finished_at")
            .map(|expr| expr_optional_i64(expr, "finished_at"))
            .transpose()?
            .flatten(),
        error_message: optional("error_message")
            .map(|expr| expr_optional_string(expr, "error_message"))
            .transpose()?
            .flatten(),
        source:        optional("source")
            .map(|expr| expr_optional_string(expr, "source"))
            .transpose()?
            .flatten(),
        kalam_version: optional("kalam_version")
            .map(|expr| expr_optional_string(expr, "kalam_version"))
            .transpose()?
            .flatten(),
    })
}

fn apply_migration_assignment(
    migration: &mut Migration,
    assignment: &Assignment,
) -> Result<(), KalamDbError> {
    let column = assignment_column_name(assignment)?;
    match column.as_str() {
        "migration_key" => {
            return Err(KalamDbError::InvalidOperation(
                "system.migrations migration_key cannot be updated".to_string(),
            ));
        },
        "migration_id" => {
            return Err(KalamDbError::InvalidOperation(
                "system.migrations migration_id cannot be updated".to_string(),
            ));
        },
        "namespace" => migration.namespace = expr_string(&assignment.value, &column)?,
        "name" => migration.name = expr_string(&assignment.value, &column)?,
        "checksum" => migration.checksum = expr_string(&assignment.value, &column)?,
        "status" => migration.status = expr_string(&assignment.value, &column)?,
        "started_at" => migration.started_at = expr_optional_i64(&assignment.value, &column)?,
        "finished_at" => migration.finished_at = expr_optional_i64(&assignment.value, &column)?,
        "error_message" => {
            migration.error_message = expr_optional_string(&assignment.value, &column)?
        },
        "source" => migration.source = expr_optional_string(&assignment.value, &column)?,
        "kalam_version" => {
            migration.kalam_version = expr_optional_string(&assignment.value, &column)?
        },
        _ => {
            return Err(KalamDbError::InvalidSql(format!(
                "Unknown system.migrations column '{}'",
                column
            )));
        },
    }
    Ok(())
}

fn migration_key_from_update_selection(
    selection: Option<&Expr>,
) -> Result<MigrationId, KalamDbError> {
    let Some(selection) = selection else {
        return Err(KalamDbError::InvalidSql(
            "UPDATE system.migrations requires WHERE migration_key = <literal>".to_string(),
        ));
    };
    match selection {
        Expr::BinaryOp { left, op, right } if *op == BinaryOperator::Eq => {
            let column_is_migration_key = matches!(
                left.as_ref(),
                Expr::Identifier(ident) if ident.value.eq_ignore_ascii_case("migration_key")
            );
            if column_is_migration_key {
                return Ok(MigrationId::new(expr_string(right, "migration_key")?));
            }
            Err(KalamDbError::InvalidSql(
                "UPDATE system.migrations WHERE clause must target migration_key".to_string(),
            ))
        },
        _ => Err(KalamDbError::InvalidSql(
            "UPDATE system.migrations requires WHERE migration_key = <literal>".to_string(),
        )),
    }
}
