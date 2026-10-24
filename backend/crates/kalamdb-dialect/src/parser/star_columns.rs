//! PostgreSQL `SELECT *` must not project storage columns.
//!
//! `_version` and `_deleted` stay selectable by name. The Kalam SQL API still
//! returns them from a star. The planner runs this rewrite only for a
//! PostgreSQL session, where every returned column is table data.

use std::collections::{HashMap, HashSet};

use kalamdb_commons::constants::SystemColumnNames;
use sqlparser::ast::{
    Expr, Ident, ObjectName, ObjectNamePart, Query, Select, SelectItem,
    SelectItemQualifiedWildcardKind, SetExpr, Statement, TableFactor, VisitMut, VisitorMut,
    WildcardAdditionalOptions,
};

use super::utils::object_name_to_string;

/// Replace plain `*` and `table.*` on a single base table with that table's
/// non-storage columns. `columns_by_table` is keyed by [`object_name_to_string`].
/// Missing tables and stars with `EXCLUDE` / `EXCEPT` / `REPLACE` stay as-is.
pub fn expand_stars_without_storage_columns(
    statement: &mut Statement,
    columns_by_table: &HashMap<String, Vec<String>>,
) {
    let _ = statement.visit(&mut StarExpander { columns_by_table });
}

/// Base tables whose star this module can expand. Callers resolve schemas for
/// these names before [`expand_stars_without_storage_columns`].
pub fn storage_star_table_names(statement: &mut Statement) -> Vec<ObjectName> {
    let mut names = Vec::new();
    let mut seen = HashSet::new();
    let _ = statement.visit(&mut StarTableCollector {
        names: &mut names,
        seen:  &mut seen,
    });
    names
}

struct StarExpander<'a> {
    columns_by_table: &'a HashMap<String, Vec<String>>,
}

impl VisitorMut for StarExpander<'_> {
    type Break = ();

    fn pre_visit_query(&mut self, query: &mut Query) -> std::ops::ControlFlow<Self::Break> {
        rewrite_set_expr(&mut query.body, self.columns_by_table);
        std::ops::ControlFlow::Continue(())
    }
}

struct StarTableCollector<'a> {
    names: &'a mut Vec<ObjectName>,
    seen:  &'a mut HashSet<String>,
}

impl VisitorMut for StarTableCollector<'_> {
    type Break = ();

    fn pre_visit_query(&mut self, query: &mut Query) -> std::ops::ControlFlow<Self::Break> {
        collect_set_expr(&query.body, self.names, self.seen);
        std::ops::ControlFlow::Continue(())
    }
}

fn rewrite_set_expr(expr: &mut SetExpr, columns_by_table: &HashMap<String, Vec<String>>) {
    match expr {
        SetExpr::Select(select) => rewrite_select(select, columns_by_table),
        SetExpr::SetOperation { left, right, .. } => {
            rewrite_set_expr(left, columns_by_table);
            rewrite_set_expr(right, columns_by_table);
        },
        _ => {},
    }
}

fn collect_set_expr(expr: &SetExpr, names: &mut Vec<ObjectName>, seen: &mut HashSet<String>) {
    match expr {
        SetExpr::Select(select) => {
            let Some(table) = single_base_table(select) else {
                return;
            };
            if !select.projection.iter().any(is_plain_star) {
                return;
            }
            let Some(key) = object_name_to_string(&table.name) else {
                return;
            };
            if seen.insert(key) {
                names.push(table.name);
            }
        },
        SetExpr::SetOperation { left, right, .. } => {
            collect_set_expr(left, names, seen);
            collect_set_expr(right, names, seen);
        },
        _ => {},
    }
}

struct BaseTable {
    name:  ObjectName,
    alias: Option<String>,
}

fn single_base_table(select: &Select) -> Option<BaseTable> {
    if select.from.len() != 1 || !select.from[0].joins.is_empty() {
        return None;
    }
    let TableFactor::Table {
        name,
        alias,
        args,
        with_hints,
        version,
        partitions,
        with_ordinality,
        ..
    } = &select.from[0].relation
    else {
        return None;
    };
    if args.is_some()
        || !with_hints.is_empty()
        || version.is_some()
        || !partitions.is_empty()
        || *with_ordinality
        || alias.as_ref().is_some_and(|alias| !alias.columns.is_empty())
    {
        return None;
    }
    Some(BaseTable {
        name:  name.clone(),
        alias: alias.as_ref().map(|alias| alias.name.value.clone()),
    })
}

fn rewrite_select(select: &mut Select, columns_by_table: &HashMap<String, Vec<String>>) {
    let Some(table) = single_base_table(select) else {
        return;
    };
    if !select.projection.iter().any(is_plain_star) {
        return;
    }
    let Some(key) = object_name_to_string(&table.name) else {
        return;
    };
    let Some(columns) = columns_by_table.get(&key) else {
        return;
    };
    let visible: Vec<SelectItem> = columns
        .iter()
        .filter(|name| !SystemColumnNames::is_system_column(name))
        .map(|name| SelectItem::UnnamedExpr(Expr::Identifier(Ident::new(name.clone()))))
        .collect();
    if visible.is_empty() {
        return;
    }
    let table_leaf = object_name_leaf(&table.name);
    let mut rewritten = Vec::with_capacity(select.projection.len() + visible.len());
    for item in select.projection.drain(..) {
        if star_targets_table(&item, table_leaf, table.alias.as_deref()) {
            rewritten.extend(visible.iter().cloned());
        } else {
            rewritten.push(item);
        }
    }
    select.projection = rewritten;
}

fn star_targets_table(item: &SelectItem, table_leaf: &str, alias: Option<&str>) -> bool {
    match item {
        SelectItem::Wildcard(options) => plain_star_options(options),
        SelectItem::QualifiedWildcard(
            SelectItemQualifiedWildcardKind::ObjectName(name),
            options,
        ) => plain_star_options(options) && qualifier_matches(name, table_leaf, alias),
        _ => false,
    }
}

fn is_plain_star(item: &SelectItem) -> bool {
    match item {
        SelectItem::Wildcard(options) | SelectItem::QualifiedWildcard(_, options) => {
            plain_star_options(options)
        },
        _ => false,
    }
}

fn plain_star_options(options: &WildcardAdditionalOptions) -> bool {
    options.opt_ilike.is_none()
        && options.opt_exclude.is_none()
        && options.opt_except.is_none()
        && options.opt_replace.is_none()
        && options.opt_rename.is_none()
        && options.opt_alias.is_none()
}

fn qualifier_matches(name: &ObjectName, table_leaf: &str, alias: Option<&str>) -> bool {
    let Some(leaf) = object_name_leaf_owned(name) else {
        return false;
    };
    leaf.eq_ignore_ascii_case(table_leaf)
        || alias.is_some_and(|alias| leaf.eq_ignore_ascii_case(alias))
}

fn object_name_leaf(name: &ObjectName) -> &str {
    name.0
        .last()
        .and_then(|part| match part {
            ObjectNamePart::Identifier(ident) => Some(ident.value.as_str()),
            ObjectNamePart::Function(_) => None,
        })
        .unwrap_or("")
}

fn object_name_leaf_owned(name: &ObjectName) -> Option<&str> {
    match name.0.last()? {
        ObjectNamePart::Identifier(ident) => Some(ident.value.as_str()),
        ObjectNamePart::Function(_) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::KalamParser;

    fn columns() -> HashMap<String, Vec<String>> {
        HashMap::from([(
            "directus_fields".to_string(),
            vec![
                "id".to_string(),
                "field".to_string(),
                "_version".to_string(),
                "_deleted".to_string(),
                "_timestamp".to_string(),
            ],
        )])
    }

    fn projection_names(statement: &Statement) -> Vec<String> {
        let Statement::Query(query) = statement else {
            panic!("expected a query");
        };
        let SetExpr::Select(select) = query.body.as_ref() else {
            panic!("expected a select");
        };
        select
            .projection
            .iter()
            .map(|item| match item {
                SelectItem::UnnamedExpr(Expr::Identifier(ident)) => ident.value.clone(),
                other => panic!("unexpected projection item: {other:?}"),
            })
            .collect()
    }

    #[test]
    fn star_omits_storage_columns() {
        let mut statement =
            KalamParser::parse_one(r#"SELECT * FROM "directus_fields" WHERE "field" = 'country'"#)
                .unwrap();
        expand_stars_without_storage_columns(&mut statement, &columns());
        assert_eq!(projection_names(&statement), vec!["id".to_string(), "field".to_string()]);
    }

    #[test]
    fn qualified_star_and_alias_omit_storage_columns() {
        let mut statement =
            KalamParser::parse_one(r#"SELECT f.* FROM "directus_fields" AS f"#).unwrap();
        expand_stars_without_storage_columns(&mut statement, &columns());
        assert_eq!(projection_names(&statement), vec!["id".to_string(), "field".to_string()]);
    }

    #[test]
    fn explicit_storage_column_stays_selectable() {
        let mut statement =
            KalamParser::parse_one(r#"SELECT "_version", "id" FROM "directus_fields""#).unwrap();
        expand_stars_without_storage_columns(&mut statement, &columns());
        assert_eq!(projection_names(&statement), vec!["_version".to_string(), "id".to_string()]);
    }

    #[test]
    fn unresolved_star_is_left_alone() {
        let mut statement = KalamParser::parse_one("SELECT * FROM other_table").unwrap();
        let before = statement.to_string();
        expand_stars_without_storage_columns(&mut statement, &columns());
        assert_eq!(statement.to_string(), before);
    }

    #[test]
    fn joined_star_is_left_alone() {
        let mut statement = KalamParser::parse_one(
            r#"SELECT * FROM "directus_fields" f JOIN "directus_collections" c ON c.collection = f.collection"#,
        )
        .unwrap();
        let before = statement.to_string();
        expand_stars_without_storage_columns(&mut statement, &columns());
        assert_eq!(statement.to_string(), before);
    }

    #[test]
    fn star_with_alias_column_list_is_left_alone() {
        let mut statement =
            KalamParser::parse_one(r#"SELECT * FROM "directus_fields" AS f("id", "field")"#)
                .unwrap();
        let before = statement.to_string();
        expand_stars_without_storage_columns(&mut statement, &columns());
        assert_eq!(statement.to_string(), before);
    }
}
