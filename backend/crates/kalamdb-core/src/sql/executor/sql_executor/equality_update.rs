//! Literal secondary-index updates.
//!
//! Parameter-free `UPDATE` statements miss the plan cache: the cache key is the
//! full SQL text, and the benchmark issues a distinct literal each time.
//! A single equality on an indexed column is a prefix seek plus one staged
//! mutation. Expressions, ranges, cold segments, and seek columns already
//! assigned in the transaction stay on the DataFusion path.

use std::collections::HashSet;

use datafusion::scalar::ScalarValue;
use kalamdb_commons::{ids::VersionId, models::TransactionId, schemas::TableType};
use kalamdb_sql::{expr_to_scalar, strip_nested_expr};
use sqlparser::ast::{
    Assignment, AssignmentTarget, BinaryOperator, Expr, Ident, ObjectNamePart, Statement,
};

use crate::{
    error::KalamDbError,
    sql::{
        executor::{PreparedExecutionStatement, SqlExecutor},
        ExecutionContext, ExecutionResult,
    },
};

struct LiteralEqualityUpdate {
    seek_column: String,
    seek_value: ScalarValue,
    assignments: Vec<(String, ScalarValue)>,
}

impl SqlExecutor {
    pub(super) async fn try_execute_literal_equality_update(
        &self,
        metadata: &PreparedExecutionStatement,
        params: &[ScalarValue],
        exec_ctx: &ExecutionContext,
    ) -> Result<Option<ExecutionResult>, KalamDbError> {
        if !params.is_empty() || !exec_ctx.is_admin() {
            return Ok(None);
        }
        if !metadata.table_type.is_some_and(literal_equality_fast_path) {
            return Ok(None);
        }
        let Some(table_id) = metadata.table_id.as_ref() else {
            return Ok(None);
        };
        let Some(statement) = metadata.parsed_dml.as_deref() else {
            return Ok(None);
        };
        let Some(update) = literal_equality_update(statement) else {
            return Ok(None);
        };
        let Some(transaction_id) = self.active_request_transaction_id(exec_ctx)? else {
            return Ok(None);
        };

        let provider = self.app_context.schema_registry().get_kalam_provider(table_id);
        let Some(provider) = provider else {
            return Ok(None);
        };

        let overlay = self
            .app_context
            .transaction_coordinator()
            .get_table_overlay(&transaction_id, table_id);
        let mutations = provider
            .plan_literal_equality_updates(
                &transaction_id,
                exec_ctx.user_id(),
                snapshot_commit_seq(self, &transaction_id),
                overlay.as_ref(),
                &update.seek_column,
                &update.seek_value,
                &update.assignments,
            )
            .await?;
        drop(overlay);
        let Some(mutations) = mutations else {
            return Ok(None);
        };
        let rows_affected = mutations.len();
        self.app_context
            .transaction_coordinator()
            .stage_batch(&transaction_id, mutations)?;
        Ok(Some(ExecutionResult::Updated { rows_affected }))
    }
}

fn snapshot_commit_seq(
    executor: &SqlExecutor,
    transaction_id: &TransactionId,
) -> Option<VersionId> {
    executor
        .app_context
        .transaction_coordinator()
        .get_handle(transaction_id)
        .and_then(|handle| handle.snapshot_commit_seq())
        .and_then(|log_index| {
            kalamdb_commons::ids::RaftVersionId::try_new(
                log_index,
                kalamdb_commons::ids::MAX_ORDINAL,
            )
            .ok()
            .map(|packed| packed.version())
        })
}

fn literal_equality_fast_path(table_type: TableType) -> bool {
    matches!(table_type, TableType::User | TableType::Shared)
}

fn literal_equality_update(statement: &Statement) -> Option<LiteralEqualityUpdate> {
    let Statement::Update(update) = statement else {
        return None;
    };
    if update.returning.is_some()
        || update.from.is_some()
        || update.output.is_some()
        || update.or.is_some()
        || update.limit.is_some()
        || !update.order_by.is_empty()
        || !update.optimizer_hints.is_empty()
        || !update.table.joins.is_empty()
    {
        return None;
    }

    let (seek_column, seek_value) = equality_literal(update.selection.as_ref()?)?;
    if seek_value.is_null() {
        return None;
    }

    let mut seen = HashSet::with_capacity(update.assignments.len());
    let mut assignments = Vec::with_capacity(update.assignments.len());
    for assignment in &update.assignments {
        let column = assignment_column(assignment)?;
        if column == seek_column || !seen.insert(column.clone()) {
            return None;
        }
        let value = expr_to_scalar(&assignment.value).ok()?;
        assignments.push((column, value));
    }
    if assignments.is_empty() {
        return None;
    }

    Some(LiteralEqualityUpdate {
        seek_column,
        seek_value,
        assignments,
    })
}

fn equality_literal(expr: &Expr) -> Option<(String, ScalarValue)> {
    let Expr::BinaryOp { left, op, right } = strip_nested_expr(expr) else {
        return None;
    };
    if op != &BinaryOperator::Eq {
        return None;
    }
    if let (Some(column), Ok(value)) = (expr_column_name(left), expr_to_scalar(right)) {
        return Some((column, value));
    }
    if let (Some(column), Ok(value)) = (expr_column_name(right), expr_to_scalar(left)) {
        return Some((column, value));
    }
    None
}

fn assignment_column(assignment: &Assignment) -> Option<String> {
    match &assignment.target {
        AssignmentTarget::ColumnName(name) => match name.0.last()? {
            ObjectNamePart::Identifier(ident) => Some(normalize_ident(ident)),
            ObjectNamePart::Function(_) => None,
        },
        AssignmentTarget::Tuple(_) => None,
    }
}

fn expr_column_name(expr: &Expr) -> Option<String> {
    match strip_nested_expr(expr) {
        Expr::Identifier(ident) => Some(normalize_ident(ident)),
        Expr::CompoundIdentifier(idents) => idents.last().map(normalize_ident),
        _ => None,
    }
}

fn normalize_ident(ident: &Ident) -> String {
    if ident.quote_style.is_some() {
        ident.value.clone()
    } else {
        ident.value.to_ascii_lowercase()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parsed(sql: &str) -> Statement {
        kalamdb_sql::parse_single_statement(sql).expect("parse").expect("one statement")
    }

    #[test]
    fn parses_literal_secondary_index_update() {
        let update = literal_equality_update(&parsed("UPDATE t2 SET b = 6669 WHERE a = 1"))
            .expect("simple update");
        assert_eq!(update.seek_column, "a");
        assert_eq!(update.seek_value, ScalarValue::Int64(Some(1)));
        assert_eq!(update.assignments, vec![("b".to_string(), ScalarValue::Int64(Some(6669)))]);

        let text = literal_equality_update(&parsed("UPDATE t2 SET c = 'hello' WHERE a = 4"))
            .expect("text assignment");
        assert_eq!(
            text.assignments,
            vec![("c".to_string(), ScalarValue::Utf8(Some("hello".to_string())))]
        );
    }

    #[test]
    fn rejects_shapes_that_need_the_planner() {
        let rejected = [
            "UPDATE t2 SET b = b * 2 WHERE a >= 0 AND a < 10",
            "UPDATE t2 SET b = 1 WHERE a = 1 AND c = 'x'",
            "UPDATE t2 SET a = 2 WHERE a = 1",
            "UPDATE t2 SET b = 1",
            "UPDATE t2 SET b = 1 WHERE a = 1 RETURNING b",
        ];
        for sql in rejected {
            assert!(literal_equality_update(&parsed(sql)).is_none(), "{sql}");
        }
    }

    #[test]
    fn user_and_shared_tables_use_the_equality_fast_path() {
        assert!(literal_equality_fast_path(TableType::User));
        assert!(literal_equality_fast_path(TableType::Shared));
        assert!(!literal_equality_fast_path(TableType::Stream));
        assert!(!literal_equality_fast_path(TableType::System));
    }
}
