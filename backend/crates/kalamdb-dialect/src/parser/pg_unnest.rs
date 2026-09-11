//! Rewrite PostgreSQL `UNNEST` table factors so DataFusion's DuckDB dialect can plan them.
//!
//! DataFusion sessions use the DuckDB SQL dialect (needed for lambdas). DuckDB does not
//! treat `UNNEST(...)` / `unnest(...)` as `TableFactor::UNNEST`; it looks them up as
//! table-valued functions and fails with `table function 'UNNEST' not found`.
//!
//! SELECT-list `unnest()` and `LATERAL` derived tables are supported, so GUI catalog
//! probes of the form:
//!
//! ```sql
//! JOIN unnest(pk_con.conkey) AS pk_col(attnum) ON true
//! CROSS JOIN LATERAL unnest(expr) WITH ORDINALITY AS k(attnum, n)
//! JOIN unnest(con.conkey, con.confkey) AS cols(src, ref) ON true
//! ```
//!
//! become correlated `LATERAL (SELECT unnest(...) AS col, ...) AS alias`.

use core::ops::ControlFlow;

use sqlparser::ast::{
    ArrayElemTypeDef, BinaryOperator, DataType, Expr, FunctionArg, FunctionArgExpr,
    FunctionArguments, Ident, ObjectName, ObjectNamePart, Query, SetExpr, Statement, TableAlias,
    TableFactor, UnaryOperator, Value, Visit, VisitMut, Visitor, VisitorMut,
};

use crate::{dialect::KalamDbDialect, parser::utils::parse_sql_statements};

pub fn apply_pg_table_function_rewrites(statements: &mut Vec<Statement>) {
    let mut int_alias_visitor = PgIntAliasRewriter;
    let _ = statements.visit(&mut int_alias_visitor);

    let mut indkey_visitor = PgIndkeyExpandRewriter;
    let _ = statements.visit(&mut indkey_visitor);

    // Replace Tabularis `EXISTS (… pg_constraint … unnest(conkey) …)` before UNNEST
    // rewrite: DataFusion plans EXISTS logically but cannot execute it physically.
    let mut pk_exists_visitor = PgPkExistsRewriter;
    let _ = statements.visit(&mut pk_exists_visitor);

    let mut unnest_visitor = PgUnnestRewriter;
    let _ = statements.visit(&mut unnest_visitor);
}

struct PgIntAliasRewriter;

impl VisitorMut for PgIntAliasRewriter {
    type Break = ();

    fn post_visit_expr(&mut self, expr: &mut Expr) -> ControlFlow<Self::Break> {
        match expr {
            Expr::Cast { data_type, .. } => rewrite_pg_int_aliases_in_data_type(data_type),
            Expr::TypedString(typed) => rewrite_pg_int_aliases_in_data_type(&mut typed.data_type),
            _ => {},
        }
        ControlFlow::Continue(())
    }
}

struct PgIndkeyExpandRewriter;

impl VisitorMut for PgIndkeyExpandRewriter {
    type Break = ();

    fn post_visit_expr(&mut self, expr: &mut Expr) -> ControlFlow<Self::Break> {
        if let Some(simplified) = simplify_indkey_string_to_array(expr) {
            *expr = simplified;
        }
        ControlFlow::Continue(())
    }
}

struct PgPkExistsRewriter;

impl VisitorMut for PgPkExistsRewriter {
    type Break = ();

    fn post_visit_expr(&mut self, expr: &mut Expr) -> ControlFlow<Self::Break> {
        if let Some(rewritten) = rewrite_pg_pk_exists(expr) {
            *expr = rewritten;
        }
        ControlFlow::Continue(())
    }
}

struct PgUnnestRewriter;

impl VisitorMut for PgUnnestRewriter {
    type Break = ();

    fn post_visit_table_factor(
        &mut self,
        table_factor: &mut TableFactor,
    ) -> ControlFlow<Self::Break> {
        if let Some(rewritten) = rewrite_unnest_table_factor(table_factor) {
            *table_factor = rewritten;
        }
        ControlFlow::Continue(())
    }
}

fn rewrite_pg_int_aliases_in_data_type(data_type: &mut DataType) {
    match data_type {
        DataType::Int2(width) => *data_type = DataType::SmallInt(*width),
        DataType::Int4(width) => *data_type = DataType::Int(*width),
        DataType::Int8(width) => *data_type = DataType::BigInt(*width),
        DataType::Custom(name, _) => {
            if let Some(rewritten) = pg_int_custom_alias(name) {
                *data_type = rewritten;
            }
        },
        DataType::Array(def) => match def {
            ArrayElemTypeDef::SquareBracket(inner, _)
            | ArrayElemTypeDef::AngleBracket(inner)
            | ArrayElemTypeDef::Parenthesis(inner) => {
                rewrite_pg_int_aliases_in_data_type(inner);
            },
            ArrayElemTypeDef::None => {},
        },
        _ => {},
    }
}

fn pg_int_custom_alias(name: &ObjectName) -> Option<DataType> {
    let ident = match name.0.last()? {
        ObjectNamePart::Identifier(ident) => ident.value.as_str(),
        ObjectNamePart::Function(_) => return None,
    };
    match ident.to_ascii_lowercase().as_str() {
        "int2" | "smallint" => Some(DataType::SmallInt(None)),
        "int4" | "integer" => Some(DataType::Int(None)),
        "int8" => Some(DataType::BigInt(None)),
        _ => None,
    }
}

fn simplify_indkey_string_to_array(expr: &Expr) -> Option<Expr> {
    let inner = match expr {
        Expr::Cast {
            expr, data_type, ..
        } if is_int_array_type(data_type) => expr.as_ref(),
        _ => expr,
    };
    let args = function_args_named(inner, "string_to_array")?;
    if args.len() < 2 {
        return None;
    }
    let source = function_arg_expr(&args[0])?;
    if !is_space_literal(&args[1]) {
        return None;
    }
    let column = unwrap_text_cast(source);
    is_indkey_column(&column).then_some(column)
}

fn is_int_array_type(data_type: &DataType) -> bool {
    let DataType::Array(ArrayElemTypeDef::SquareBracket(inner, _)) = data_type else {
        return false;
    };
    matches!(
        inner.as_ref(),
        DataType::Int2(_)
            | DataType::SmallInt(_)
            | DataType::Int(_)
            | DataType::Int4(_)
            | DataType::Integer(_)
            | DataType::Int8(_)
            | DataType::BigInt(_)
    )
}

fn unwrap_text_cast(expr: &Expr) -> Expr {
    match expr {
        Expr::Cast {
            expr, data_type, ..
        } if matches!(
            data_type,
            DataType::Text | DataType::Varchar(_) | DataType::Char(_) | DataType::Character(_)
        ) =>
        {
            unwrap_text_cast(expr)
        },
        other => other.clone(),
    }
}

fn is_indkey_column(expr: &Expr) -> bool {
    match expr {
        Expr::Identifier(ident) => ident.value.eq_ignore_ascii_case("indkey"),
        Expr::CompoundIdentifier(parts) => {
            parts.last().is_some_and(|ident| ident.value.eq_ignore_ascii_case("indkey"))
        },
        _ => false,
    }
}

fn is_space_literal(arg: &FunctionArg) -> bool {
    let Some(expr) = function_arg_expr(arg) else {
        return false;
    };
    matches!(expr, Expr::Value(value) if matches!(&value.value, Value::SingleQuotedString(text) if text == " "))
}

fn function_args_named<'a>(expr: &'a Expr, name: &str) -> Option<&'a [FunctionArg]> {
    let Expr::Function(function) = expr else {
        return None;
    };
    if !object_name_eq_ignore_ascii_case(&function.name, name) {
        return None;
    }
    match &function.args {
        FunctionArguments::List(list) => Some(list.args.as_slice()),
        _ => None,
    }
}

fn rewrite_pg_pk_exists(expr: &Expr) -> Option<Expr> {
    let Expr::Exists { subquery, negated } = expr else {
        return None;
    };
    if !subquery_is_pg_pk_exists(subquery) {
        return None;
    }
    let pk_column = match pk_exists_outer_alias(subquery) {
        Some(alias) => Expr::CompoundIdentifier(vec![alias, Ident::new("kdb_primary_key")]),
        None => Expr::Identifier(Ident::new("kdb_primary_key")),
    };
    if *negated {
        Some(Expr::UnaryOp {
            op:   UnaryOperator::Not,
            expr: Box::new(pk_column),
        })
    } else {
        Some(pk_column)
    }
}

fn subquery_is_pg_pk_exists(query: &Query) -> bool {
    let sql = query.to_string().to_ascii_lowercase();
    sql.contains("pg_constraint")
        && sql.contains("unnest")
        && sql.contains("contype")
        && sql.contains("column_name")
}

fn pk_exists_outer_alias(query: &Query) -> Option<Ident> {
    struct Finder {
        alias: Option<Ident>,
    }
    impl Visitor for Finder {
        type Break = ();

        fn pre_visit_expr(&mut self, expr: &Expr) -> ControlFlow<Self::Break> {
            if let Expr::BinaryOp {
                left,
                op: BinaryOperator::Eq,
                right,
            } = expr
            {
                if let Some(alias) = alias_from_attname_column_eq(left, right) {
                    self.alias = Some(alias);
                }
            }
            ControlFlow::Continue(())
        }
    }
    let mut finder = Finder { alias: None };
    let _ = query.visit(&mut finder);
    finder.alias
}

fn alias_from_attname_column_eq(left: &Expr, right: &Expr) -> Option<Ident> {
    if ident_tail_eq(left, "attname") {
        return compound_head_if_tail(right, "column_name");
    }
    if ident_tail_eq(right, "attname") {
        return compound_head_if_tail(left, "column_name");
    }
    None
}

fn ident_tail_eq(expr: &Expr, expected: &str) -> bool {
    match expr {
        Expr::Identifier(ident) => ident.value.eq_ignore_ascii_case(expected),
        Expr::CompoundIdentifier(parts) => {
            parts.last().is_some_and(|ident| ident.value.eq_ignore_ascii_case(expected))
        },
        _ => false,
    }
}

fn compound_head_if_tail(expr: &Expr, expected_tail: &str) -> Option<Ident> {
    let Expr::CompoundIdentifier(parts) = expr else {
        return None;
    };
    if parts.len() < 2 {
        return None;
    }
    parts
        .last()
        .is_some_and(|ident| ident.value.eq_ignore_ascii_case(expected_tail))
        .then(|| parts[0].clone())
}

fn rewrite_unnest_table_factor(table_factor: &TableFactor) -> Option<TableFactor> {
    match table_factor {
        TableFactor::UNNEST {
            alias,
            array_exprs,
            with_offset: false,
            with_ordinality,
            ..
        } => unnest_as_lateral(array_exprs, alias.as_ref(), *with_ordinality),
        TableFactor::Function {
            name,
            args,
            alias,
            with_ordinality,
            ..
        } if is_unnest_name(name) => {
            let exprs = function_args_to_exprs(args)?;
            unnest_as_lateral(&exprs, alias.as_ref(), *with_ordinality)
        },
        TableFactor::Table {
            name,
            args: Some(func_args),
            alias,
            with_ordinality,
            ..
        } if is_unnest_name(name) => {
            let exprs = function_args_to_exprs(&func_args.args)?;
            unnest_as_lateral(&exprs, alias.as_ref(), *with_ordinality)
        },
        _ => None,
    }
}

fn unnest_as_lateral(
    array_exprs: &[Expr],
    alias: Option<&TableAlias>,
    with_ordinality: bool,
) -> Option<TableFactor> {
    if array_exprs.is_empty() {
        return None;
    }

    let alias_name = alias.map(|alias| alias.name.clone()).unwrap_or_else(|| Ident::new("unnest"));
    let column_names: Vec<Ident> = alias
        .map(|alias| alias.columns.iter().map(|column| column.name.clone()).collect())
        .unwrap_or_default();

    let mut projections = Vec::with_capacity(array_exprs.len() + usize::from(with_ordinality));
    for (index, expr) in array_exprs.iter().enumerate() {
        let column = column_names.get(index).cloned().unwrap_or_else(|| {
            if array_exprs.len() == 1 {
                Ident::new("unnest")
            } else {
                Ident::new(format!("unnest_{index}"))
            }
        });
        projections.push(format!("unnest({expr}) AS {column}"));
    }

    let inner_sql = if with_ordinality {
        let first_column = column_names.first().cloned().unwrap_or_else(|| Ident::new("unnest"));
        let ordinal_column = column_names
            .get(array_exprs.len())
            .cloned()
            .unwrap_or_else(|| Ident::new("ordinality"));
        // Number unnested rows in an outer SELECT so ROW_NUMBER() is not computed
        // before DataFusion expands `unnest()`.
        format!(
            "SELECT {first_column}, ROW_NUMBER() OVER () AS {ordinal_column} FROM (SELECT {})",
            projections.join(", ")
        )
    } else {
        format!("SELECT {}", projections.join(", "))
    };

    let sql = format!("SELECT * FROM LATERAL ({inner_sql}) AS {alias_name}");
    parse_single_table_factor(&sql)
}

fn parse_single_table_factor(sql: &str) -> Option<TableFactor> {
    let dialect = KalamDbDialect::default();
    let statements = parse_sql_statements(sql, &dialect).ok()?;
    let Statement::Query(query) = statements.into_iter().next()? else {
        return None;
    };
    let SetExpr::Select(select) = *query.body else {
        return None;
    };
    Some(select.from.into_iter().next()?.relation)
}

fn is_unnest_name(name: &ObjectName) -> bool {
    object_name_eq_ignore_ascii_case(name, "unnest")
}

fn object_name_eq_ignore_ascii_case(name: &ObjectName, expected: &str) -> bool {
    match name.0.last() {
        Some(ObjectNamePart::Identifier(ident)) => ident.value.eq_ignore_ascii_case(expected),
        _ => false,
    }
}

fn function_args_to_exprs(args: &[FunctionArg]) -> Option<Vec<Expr>> {
    let mut exprs = Vec::with_capacity(args.len());
    for arg in args {
        exprs.push(function_arg_expr(arg)?.clone());
    }
    Some(exprs)
}

fn function_arg_expr(arg: &FunctionArg) -> Option<&Expr> {
    match arg {
        FunctionArg::Unnamed(FunctionArgExpr::Expr(expr)) => Some(expr),
        FunctionArg::Named {
            arg: FunctionArgExpr::Expr(expr),
            ..
        } => Some(expr),
        FunctionArg::ExprNamed {
            arg: FunctionArgExpr::Expr(expr),
            ..
        } => Some(expr),
        _ => None,
    }
}
