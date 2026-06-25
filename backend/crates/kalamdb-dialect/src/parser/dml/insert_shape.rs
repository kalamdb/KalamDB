//! INSERT statement shape detection for the applier fast path.
//!
//! Centralizes sqlparser AST checks for VALUES-only INSERT statements so
//! execution code in `kalamdb-core` does not pattern-match `SetExpr` directly.

use sqlparser::ast::{
    Expr, Insert, OnConflict, OnInsert, Parens, Query, SelectItem, SetExpr, Statement,
};

use super::on_conflict::on_conflict_from_insert;

/// Policy for which INSERT ... VALUES shapes are accepted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ValuesInsertShapeOptions {
    /// Accept `INSERT ... ON CONFLICT`.
    pub allow_on_conflict: bool,
    /// Accept `INSERT ... RETURNING` (when false, RETURNING rejects the shape).
    pub allow_returning: bool,
    /// Accept `INSERT OVERWRITE`.
    pub allow_overwrite: bool,
}

impl ValuesInsertShapeOptions {
    /// Plain literal INSERT (RETURNING allowed, no ON CONFLICT).
    pub const PLAIN: Self = Self {
        allow_on_conflict: false,
        allow_returning: true,
        allow_overwrite: false,
    };

    /// VALUES extraction for `INSERT ... ON CONFLICT` (RETURNING handled by caller).
    pub const ON_CONFLICT_ROWS: Self = Self {
        allow_on_conflict: true,
        allow_returning: true,
        allow_overwrite: false,
    };

    /// Batch INSERT inside an active transaction.
    pub const BATCH: Self = Self {
        allow_on_conflict: false,
        allow_returning: false,
        allow_overwrite: false,
    };
}

/// A VALUES-only INSERT view with its parsed row list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ValuesInsertView<'a> {
    pub insert: &'a Insert,
    pub value_rows: &'a [Parens<Vec<Expr>>],
}

/// Returns the INSERT AST when `statement` is an INSERT.
pub fn insert_from_statement(statement: &Statement) -> Option<&Insert> {
    match statement {
        Statement::Insert(insert) => Some(insert),
        _ => None,
    }
}

/// Returns true when the INSERT has an `ON CONFLICT` clause.
#[inline]
pub fn insert_has_on_conflict(insert: &Insert) -> bool {
    matches!(insert.on.as_ref(), Some(OnInsert::OnConflict(_)))
}

/// Returns the RETURNING projection list when present.
pub fn insert_returning_items(insert: &Insert) -> Option<&[SelectItem]> {
    insert.returning.as_deref()
}

/// Extract VALUES rows from an INSERT when the source is a plain VALUES list.
///
/// Rejects INSERT without source, CTEs, ORDER BY, LIMIT, and non-VALUES bodies.
#[inline]
pub fn values_rows_from_insert(insert: &Insert) -> Option<&[Parens<Vec<Expr>>]> {
    let source = insert.source.as_ref()?;
    values_rows_from_query(source)
}

/// Extract VALUES rows from a query body when it is a plain VALUES list.
pub fn values_rows_from_query(query: &Query) -> Option<&[Parens<Vec<Expr>>]> {
    if query.with.is_some() || query.order_by.is_some() || query.limit_clause.is_some() {
        return None;
    }

    match query.body.as_ref() {
        SetExpr::Values(values) => Some(&values.rows),
        _ => None,
    }
}

/// Returns a VALUES INSERT view when the statement matches `options`.
#[inline]
pub fn values_insert_view<'a>(
    insert: &'a Insert,
    options: ValuesInsertShapeOptions,
) -> Option<ValuesInsertView<'a>> {
    if !options.allow_overwrite && insert.overwrite {
        return None;
    }
    if !options.allow_on_conflict && insert_has_on_conflict(insert) {
        return None;
    }
    if !options.allow_returning && insert.returning.is_some() {
        return None;
    }

    let value_rows = values_rows_from_insert(insert)?;
    Some(ValuesInsertView { insert, value_rows })
}

/// Like [`values_insert_view`], but accepts any statement.
pub fn values_insert_view_from_statement<'a>(
    statement: &'a Statement,
    options: ValuesInsertShapeOptions,
) -> Option<ValuesInsertView<'a>> {
    insert_from_statement(statement).and_then(|insert| values_insert_view(insert, options))
}

/// VALUES INSERT with `ON CONFLICT` when the statement matches the applier shape.
pub fn on_conflict_values_insert<'a>(
    statement: &'a Statement,
) -> Option<(ValuesInsertView<'a>, &'a OnConflict)> {
    let insert = insert_from_statement(statement)?;
    let on_conflict = on_conflict_from_insert(insert)?;
    let view = values_insert_view(insert, ValuesInsertShapeOptions::ON_CONFLICT_ROWS)?;
    Some((view, on_conflict))
}

/// Single-row VALUES INSERT; used for constrained system-table inserts.
pub fn single_values_insert_row(insert: &Insert) -> Result<&[Expr], &'static str> {
    let rows = values_rows_from_insert(insert).ok_or("insert requires values")?;
    if rows.len() != 1 {
        return Err("insert supports exactly one row");
    }
    Ok(&rows[0].content)
}

#[cfg(test)]
mod tests {
    use sqlparser::{dialect::GenericDialect, parser::Parser};

    use super::*;

    fn parse_insert(sql: &str) -> Insert {
        let statements = Parser::parse_sql(&GenericDialect {}, sql).expect("sql should parse");
        match statements.into_iter().next().expect("one statement") {
            Statement::Insert(insert) => insert,
            other => panic!("expected insert, got {other:?}"),
        }
    }

    fn parse_statement(sql: &str) -> Statement {
        Parser::parse_sql(&GenericDialect {}, sql)
            .expect("sql should parse")
            .into_iter()
            .next()
            .expect("one statement")
    }

    #[test]
    fn values_rows_from_insert_accepts_plain_values() {
        let insert = parse_insert("INSERT INTO t (id, name) VALUES (1, 'a'), (2, 'b')");
        let rows = values_rows_from_insert(&insert).expect("values rows");
        assert_eq!(rows.len(), 2);
    }

    #[test]
    fn values_rows_from_insert_rejects_select_body() {
        let insert = parse_insert("INSERT INTO t (id) SELECT 1");
        assert!(values_rows_from_insert(&insert).is_none());
    }

    #[test]
    fn plain_shape_rejects_on_conflict() {
        let statement = parse_statement(
            "INSERT INTO t (id) VALUES (1) ON CONFLICT (id) DO NOTHING RETURNING id",
        );
        assert!(values_insert_view_from_statement(&statement, ValuesInsertShapeOptions::PLAIN)
            .is_none());
    }

    #[test]
    fn plain_shape_accepts_returning() {
        let statement = parse_statement("INSERT INTO t (id) VALUES (1) RETURNING id");
        assert!(values_insert_view_from_statement(&statement, ValuesInsertShapeOptions::PLAIN)
            .is_some());
    }

    #[test]
    fn batch_shape_rejects_returning() {
        let statement = parse_statement("INSERT INTO t (id) VALUES (1) RETURNING id");
        assert!(values_insert_view_from_statement(&statement, ValuesInsertShapeOptions::BATCH)
            .is_none());
    }

    #[test]
    fn on_conflict_values_insert_accepts_conflict_with_returning() {
        let statement = parse_statement(
            "INSERT INTO t (id, name) VALUES (1, 'a') \
             ON CONFLICT (id) DO UPDATE SET name = EXCLUDED.name RETURNING id, name",
        );
        let (view, _on_conflict) =
            on_conflict_values_insert(&statement).expect("on conflict insert");
        assert_eq!(view.value_rows.len(), 1);
        assert!(insert_returning_items(view.insert).is_some());
    }

    #[test]
    fn single_values_insert_row_requires_one_row() {
        let insert = parse_insert("INSERT INTO t (id) VALUES (1), (2)");
        assert_eq!(single_values_insert_row(&insert), Err("insert supports exactly one row"));

        let insert = parse_insert("INSERT INTO t (id) VALUES (1)");
        assert_eq!(single_values_insert_row(&insert).unwrap().len(), 1);
    }
}
