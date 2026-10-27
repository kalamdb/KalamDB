//! Lift an `ORDER BY` source column out from under a same-named select alias.
//!
//! DataFusion adds a missing `ORDER BY` column to the projection schema. A qualified source
//! column such as `articles.id` then sits beside the unqualified alias `id` produced by
//! `CASE ... END AS id`, and schema validation rejects the pair as ambiguous. The sort
//! expression is projected under `kalam_sort_N` and ordered from an outer query whose select
//! list is the original output names.

use sqlparser::ast::{
    Expr, Ident, OrderByKind, Query, Select, SelectItem, SetExpr, Statement, TableAlias,
    TableFactor, TableWithJoins,
};

pub(crate) fn disambiguate_order_by_output_names(statement: &mut Statement) {
    let Statement::Query(query) = statement else {
        return;
    };
    lift_colliding_order_by(query);
}

fn lift_colliding_order_by(query: &mut Query) {
    if query.for_clause.is_some()
        || query.settings.is_some()
        || query.format_clause.is_some()
        || !query.pipe_operators.is_empty()
    {
        return;
    }
    let (outputs, collisions) = {
        let Some(order_by) = query.order_by.as_ref() else {
            return;
        };
        let OrderByKind::Expressions(_) = &order_by.kind else {
            return;
        };
        let SetExpr::Select(select) = query.body.as_ref() else {
            return;
        };
        let Some(outputs) = named_outputs(&select.projection) else {
            return;
        };
        let collisions = sort_exprs_to_lift(order_by, &select.projection);
        (outputs, collisions)
    };
    if collisions.is_empty() {
        return;
    }
    let Some(mut shell) = select_shell() else {
        return;
    };

    if let SetExpr::Select(select) = query.body.as_mut() {
        for (offset, (_, expr)) in collisions.iter().enumerate() {
            select.projection.push(SelectItem::ExprWithAlias {
                expr: expr.clone(),
                alias: Ident::new(format!("kalam_sort_{offset}")),
            });
        }
    }
    if let Some(order_by) = query.order_by.as_mut() {
        if let OrderByKind::Expressions(exprs) = &mut order_by.kind {
            for (offset, (index, _)) in collisions.iter().enumerate() {
                exprs[*index].expr = Expr::Identifier(Ident::new(format!("kalam_sort_{offset}")));
            }
        }
    }

    let order_by = query.order_by.take();
    let limit_clause = query.limit_clause.take();
    let fetch = query.fetch.take();
    let inner = query.clone();

    shell.projection = outputs
        .into_iter()
        .map(|ident| SelectItem::UnnamedExpr(Expr::Identifier(ident)))
        .collect();
    shell.from = vec![TableWithJoins {
        relation: TableFactor::Derived {
            lateral: false,
            subquery: Box::new(inner),
            alias: Some(TableAlias {
                explicit: true,
                name: Ident::new("kalam_sort_wrap"),
                columns: Vec::new(),
                at: None,
            }),
            sample: None,
        },
        joins: Vec::new(),
    }];

    *query = Query {
        with: None,
        body: Box::new(SetExpr::Select(Box::new(shell))),
        order_by,
        limit_clause,
        fetch,
        locks: Vec::new(),
        for_clause: None,
        settings: None,
        format_clause: None,
        pipe_operators: Vec::new(),
    };
}

fn named_outputs(projection: &[SelectItem]) -> Option<Vec<Ident>> {
    projection.iter().map(output_ident).collect()
}

fn output_ident(item: &SelectItem) -> Option<Ident> {
    match item {
        SelectItem::ExprWithAlias { alias, .. } => Some(alias.clone()),
        SelectItem::UnnamedExpr(expr) => column_ident(expr).cloned(),
        _ => None,
    }
}

fn column_ident(expr: &Expr) -> Option<&Ident> {
    match expr {
        Expr::Identifier(ident) => Some(ident),
        Expr::CompoundIdentifier(parts) => parts.last(),
        _ => None,
    }
}

fn is_plain_column(item: &SelectItem) -> bool {
    match item {
        SelectItem::UnnamedExpr(expr) => column_ident(expr).is_some(),
        SelectItem::ExprWithAlias { expr, .. } => column_ident(expr).is_some(),
        _ => false,
    }
}

/// Sort keys that must be computed in the inner projection.
///
/// A colliding alias hides the source column. A scalar subquery in `ORDER BY`
/// is rejected by DataFusion (`correlated scalar subquery can only be used in
/// Projection, Filter, Aggregate`). Projecting it lets the decorrelator turn
/// `count(*)` into a join, and the outer query still returns the original columns.
fn sort_exprs_to_lift(
    order_by: &sqlparser::ast::OrderBy,
    projection: &[SelectItem],
) -> Vec<(usize, Expr)> {
    let mut lifted = colliding_sort_exprs(order_by, projection);
    let OrderByKind::Expressions(exprs) = &order_by.kind else {
        return lifted;
    };
    for (index, item) in exprs.iter().enumerate() {
        if lifted.iter().any(|(lifted_index, _)| *lifted_index == index) {
            continue;
        }
        if matches!(item.expr, Expr::Subquery(_)) {
            lifted.push((index, item.expr.clone()));
        }
    }
    lifted
}

fn colliding_sort_exprs(
    order_by: &sqlparser::ast::OrderBy,
    projection: &[SelectItem],
) -> Vec<(usize, Expr)> {
    let OrderByKind::Expressions(exprs) = &order_by.kind else {
        return Vec::new();
    };
    exprs
        .iter()
        .enumerate()
        .filter_map(|(index, item)| {
            expr_collides(projection, &item.expr).then(|| (index, item.expr.clone()))
        })
        .collect()
}

fn expr_collides(projection: &[SelectItem], expr: &Expr) -> bool {
    let Expr::CompoundIdentifier(parts) = expr else {
        return false;
    };
    if parts.len() < 2 {
        return false;
    }
    let Some(column) = parts.last() else {
        return false;
    };
    projection.iter().any(|item| {
        output_ident(item).is_some_and(|name| name.value.eq_ignore_ascii_case(&column.value))
            && !is_plain_column(item)
    })
}

fn select_shell() -> Option<Select> {
    let statement = super::KalamParser::parse_one("SELECT 1").ok()?;
    let Statement::Query(query) = statement else {
        return None;
    };
    let SetExpr::Select(select) = *query.body else {
        return None;
    };
    Some(*select)
}

#[cfg(test)]
mod tests {
    use super::super::parse_statement_for_datafusion;

    fn planned(sql: &str) -> String {
        parse_statement_for_datafusion(sql).expect("parse").to_string()
    }

    #[test]
    fn case_alias_order_by_source_column_is_lifted() {
        let sql = planned(
            r#"SELECT (CASE WHEN "xtpdl"."id" = $1 THEN "case_when"."id" END) AS "id"
               FROM "case_when"
               LEFT JOIN "directus_users" AS "xtpdl" ON "case_when"."user_created" = "xtpdl"."id"
               ORDER BY "case_when"."id" ASC
               LIMIT $2"#,
        );
        assert!(sql.contains("kalam_sort_0"), "{sql}");
        assert!(sql.contains("kalam_sort_wrap"), "{sql}");
        assert!(sql.to_ascii_lowercase().contains("order by kalam_sort_0"), "{sql}");
    }

    #[test]
    fn order_by_count_subquery_is_projected_inside() {
        let sql = planned(
            r#"SELECT "articles"."id", "articles"."title"
               FROM "articles"
               WHERE "articles"."title" LIKE $1
               ORDER BY (SELECT count(*) FROM "links" AS "yzesc" WHERE "yzesc"."article_id" = "articles"."id") ASC
               LIMIT $2"#,
        );
        assert!(sql.contains("kalam_sort_0"), "{sql}");
        assert!(sql.contains("kalam_sort_wrap"), "{sql}");
        assert!(sql.to_ascii_lowercase().contains("order by kalam_sort_0"), "{sql}");
        assert!(sql.to_ascii_lowercase().contains("select count(*)"), "{sql}");
    }

    #[test]
    fn plain_column_and_unqualified_order_by_stay_put() {
        let by_other = planned("SELECT name FROM t ORDER BY t.id");
        assert!(!by_other.contains("kalam_sort_wrap"), "{by_other}");

        let unqualified = planned("SELECT id FROM t ORDER BY id");
        assert!(!unqualified.contains("kalam_sort_wrap"), "{unqualified}");

        let same_column = planned("SELECT id FROM t ORDER BY t.id");
        assert!(!same_column.contains("kalam_sort_wrap"), "{same_column}");
    }
}
