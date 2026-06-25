//! PostgreSQL-style `ON CONFLICT` syntax parsing for INSERT statements.

use datafusion_common::ScalarValue;
use sqlparser::ast::{
    Assignment, AssignmentTarget, ConflictTarget, Expr, Ident, Insert, OnConflict,
    OnConflictAction, OnInsert, Value,
};

use super::literals::expr_to_scalar;

pub type DmlParseResult<T> = Result<T, String>;

#[derive(Debug, Clone, PartialEq)]
pub enum OnConflictUpdateValue {
    InsertedColumn(String),
    Literal(ScalarValue),
}

#[derive(Debug, Clone, PartialEq)]
pub struct OnConflictUpdateAssignment {
    pub column_name: String,
    pub value: OnConflictUpdateValue,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ParsedOnConflictAction {
    DoUpdate {
        assignments: Vec<OnConflictUpdateAssignment>,
        where_clause: Option<Expr>,
    },
    DoNothing,
}

/// Returns true when the conflict target names the table's single primary key column.
pub fn conflict_target_is_primary_key(columns: &[Ident], primary_key_column: &str) -> bool {
    columns.len() == 1 && columns[0].value.eq_ignore_ascii_case(primary_key_column)
}

/// Validate that an INSERT ON CONFLICT clause names the primary key conflict target.
pub fn validate_primary_key_conflict_target(
    on_conflict: &OnConflict,
    primary_key_column: &str,
) -> DmlParseResult<()> {
    match on_conflict.conflict_target.as_ref() {
        Some(ConflictTarget::Columns(columns))
            if conflict_target_is_primary_key(columns, primary_key_column) =>
        {
            Ok(())
        },
        Some(ConflictTarget::Columns(_)) => Err(format!(
            "ON CONFLICT only supports the primary key conflict target ({primary_key_column})"
        )),
        Some(ConflictTarget::OnConstraint(_)) => {
            Err("ON CONFLICT ON CONSTRAINT is not supported".to_string())
        },
        None => Err("ON CONFLICT requires a primary key conflict target".to_string()),
    }
}

/// Parse the action clause for INSERT ... ON CONFLICT.
pub fn parse_on_conflict_action(
    on_conflict: &OnConflict,
) -> DmlParseResult<ParsedOnConflictAction> {
    match &on_conflict.action {
        OnConflictAction::DoUpdate(do_update) => Ok(ParsedOnConflictAction::DoUpdate {
            assignments: build_on_conflict_update_assignments(&do_update.assignments)?,
            where_clause: do_update.selection.clone(),
        }),
        OnConflictAction::DoNothing => Ok(ParsedOnConflictAction::DoNothing),
    }
}

/// Returns whether a DO UPDATE WHERE clause allows the update to proceed.
pub fn on_conflict_update_should_apply(where_clause: &Option<Expr>) -> DmlParseResult<bool> {
    match where_clause {
        None => Ok(true),
        Some(Expr::Value(value)) => {
            match &value.value {
                Value::Boolean(value) => Ok(*value),
                _ => Err("ON CONFLICT DO UPDATE only supports literal TRUE/FALSE WHERE clauses"
                    .to_string()),
            }
        },
        Some(Expr::Identifier(ident)) if ident.value.eq_ignore_ascii_case("true") => Ok(true),
        Some(Expr::Identifier(ident)) if ident.value.eq_ignore_ascii_case("false") => Ok(false),
        Some(_) => {
            Err("ON CONFLICT DO UPDATE only supports literal TRUE/FALSE WHERE clauses".to_string())
        },
    }
}

pub fn build_on_conflict_update_assignments(
    assignments: &[Assignment],
) -> DmlParseResult<Vec<OnConflictUpdateAssignment>> {
    assignments
        .iter()
        .map(|assignment| {
            let column_name = match &assignment.target {
                AssignmentTarget::ColumnName(name) => crate::parser::object_name_to_string(name)
                    .ok_or_else(|| format!("Unsupported ON CONFLICT assignment target '{name}'"))?,
                AssignmentTarget::Tuple(_) => {
                    return Err(
                        "ON CONFLICT DO UPDATE tuple assignments are not supported".to_string()
                    );
                },
            };

            Ok(OnConflictUpdateAssignment {
                column_name,
                value: parse_on_conflict_update_value(&assignment.value)?,
            })
        })
        .collect()
}

fn parse_on_conflict_update_value(expr: &Expr) -> DmlParseResult<OnConflictUpdateValue> {
    match expr {
        Expr::CompoundIdentifier(parts) if parts.len() == 2 => {
            let [qualifier, column] = parts.as_slice() else {
                unreachable!("compound identifier length checked above");
            };
            if qualifier.value.eq_ignore_ascii_case("excluded") {
                Ok(OnConflictUpdateValue::InsertedColumn(column.value.clone()))
            } else {
                Err(format!(
                    "ON CONFLICT DO UPDATE only supports EXCLUDED.<column> references, got {}.{}",
                    qualifier.value, column.value
                ))
            }
        },
        _ => expr_to_scalar(expr).map(OnConflictUpdateValue::Literal).map_err(|_| {
            "ON CONFLICT DO UPDATE assignments support literals or EXCLUDED.<column>".to_string()
        }),
    }
}

pub fn on_conflict_clause(insert_on: Option<&OnInsert>) -> Option<&OnConflict> {
    match insert_on {
        Some(OnInsert::OnConflict(on_conflict)) => Some(on_conflict),
        _ => None,
    }
}

/// Extract the ON CONFLICT clause from an INSERT statement, if present.
#[inline]
pub fn on_conflict_from_insert(insert: &Insert) -> Option<&OnConflict> {
    on_conflict_clause(insert.on.as_ref())
}

#[cfg(test)]
mod tests {
    use sqlparser::{dialect::GenericDialect, parser::Parser};

    use super::*;

    fn parse_on_conflict(sql: &str) -> OnConflict {
        let statements = Parser::parse_sql(&GenericDialect {}, sql).expect("sql should parse");
        let sqlparser::ast::Statement::Insert(insert) = &statements[0] else {
            panic!("expected insert");
        };
        match insert.on.as_ref() {
            Some(OnInsert::OnConflict(on_conflict)) => on_conflict.clone(),
            other => panic!("expected on conflict, got {other:?}"),
        }
    }

    #[test]
    fn validate_primary_key_conflict_target_accepts_pk_column() {
        let on_conflict = parse_on_conflict(
            "INSERT INTO t (id, name) VALUES (1, 'a') ON CONFLICT (id) DO NOTHING",
        );
        assert!(validate_primary_key_conflict_target(&on_conflict, "id").is_ok());
    }

    #[test]
    fn validate_primary_key_conflict_target_rejects_non_pk_column() {
        let on_conflict = parse_on_conflict(
            "INSERT INTO t (id, name) VALUES (1, 'a') ON CONFLICT (name) DO NOTHING",
        );
        let err = validate_primary_key_conflict_target(&on_conflict, "id").unwrap_err();
        assert!(err.contains("primary key conflict target"));
    }

    #[test]
    fn parse_on_conflict_action_parses_excluded_assignments() {
        let on_conflict = parse_on_conflict(
            "INSERT INTO t (id, name) VALUES (1, 'a') \
             ON CONFLICT (id) DO UPDATE SET name = EXCLUDED.name",
        );
        let action = parse_on_conflict_action(&on_conflict).expect("action should parse");
        assert!(matches!(
            action,
            ParsedOnConflictAction::DoUpdate {
                assignments,
                where_clause: None,
            } if assignments.len() == 1
                && assignments[0].column_name == "name"
                && matches!(
                    assignments[0].value,
                    OnConflictUpdateValue::InsertedColumn(ref column) if column == "name"
                )
        ));
    }
}
