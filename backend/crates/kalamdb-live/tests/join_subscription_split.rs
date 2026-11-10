//! Spike: one joined live-query SQL becomes one single-table subscription per input.
//!
//! DataFusion plans the join. This walker reads that plan and emits the subscriptions the
//! existing live path already knows how to open. It does not join rows, and it is not wired
//! into the WebSocket handler.
//!
//! A filter stays on the table whose columns it uses. It is not copied onto the other side of
//! the join. On a shared table, a side with no filter of its own would subscribe to every row,
//! so the split fails. User tables are already stored per session user, so an unfiltered side
//! stays inside that user.

use std::{collections::BTreeSet, sync::Arc};

use datafusion::{
    arrow::datatypes::{DataType, Field, Schema},
    catalog::{MemorySchemaProvider, SchemaProvider, TableProvider},
    common::{Column, JoinType, ScalarValue},
    datasource::MemTable,
    logical_expr::{Expr, LogicalPlan, Operator},
    prelude::SessionContext,
};

#[derive(Debug, Clone, PartialEq, Eq)]
struct TableSubscription {
    table: String,
    columns: Vec<String>,
    filter: Option<String>,
    sql: String,
}

#[derive(Debug)]
struct TableScanSubscription {
    table: String,
    qualifier: String,
    columns: BTreeSet<String>,
    filters: Vec<String>,
}

impl TableScanSubscription {
    fn into_subscription(self) -> Result<TableSubscription, String> {
        if self.columns.is_empty() {
            return Err(format!("table {} has no columns to subscribe to", self.table));
        }
        let columns: Vec<String> = self.columns.into_iter().collect();
        let filter = (!self.filters.is_empty()).then(|| self.filters.join(" AND "));
        let mut sql = format!("SELECT {} FROM {}", columns.join(", "), self.table);
        if let Some(filter) = &filter {
            sql.push_str(" WHERE ");
            sql.push_str(filter);
        }
        Ok(TableSubscription {
            table: self.table,
            columns,
            filter,
            sql,
        })
    }
}

async fn split_sql(sql: &str, shared_tables: &[&str]) -> Result<Vec<TableSubscription>, String> {
    let context = subscription_context()?;
    let plan = context
        .sql(sql)
        .await
        .map_err(|error| error.to_string())?
        .into_unoptimized_plan();
    split_plan(&plan, shared_tables).map_err(|error| format!("{error}\nplan:\n{plan}"))
}

fn subscription_context() -> Result<SessionContext, String> {
    let context = SessionContext::new();
    let schema = Arc::new(MemorySchemaProvider::new());
    schema
        .register_table(
            "messages".to_string(),
            text_table(&["id", "conversation_id", "body", "author_id"])?,
        )
        .map_err(|error| error.to_string())?;
    schema
        .register_table("conversations".to_string(), text_table(&["id", "title", "owner_id"])?)
        .map_err(|error| error.to_string())?;
    schema
        .register_table("members".to_string(), text_table(&["user_id", "conversation_id"])?)
        .map_err(|error| error.to_string())?;
    context
        .catalog("datafusion")
        .ok_or_else(|| "default DataFusion catalog is missing".to_string())?
        .register_schema("chat", schema)
        .map_err(|error| error.to_string())?;
    Ok(context)
}

fn text_table(columns: &[&str]) -> Result<Arc<dyn TableProvider>, String> {
    let schema = Arc::new(Schema::new(
        columns
            .iter()
            .map(|name| Field::new(*name, DataType::Utf8, true))
            .collect::<Vec<_>>(),
    ));
    let table = MemTable::try_new(schema, vec![vec![]]).map_err(|error| error.to_string())?;
    Ok(Arc::new(table))
}

fn split_plan(
    plan: &LogicalPlan,
    shared_tables: &[&str],
) -> Result<Vec<TableSubscription>, String> {
    let scans = walk(plan, true)?;
    reject_unbounded_shared_tables(&scans, shared_tables)?;
    let mut subscriptions = scans
        .into_iter()
        .map(TableScanSubscription::into_subscription)
        .collect::<Result<Vec<_>, _>>()?;
    if subscriptions.len() < 2 {
        return Err(
            "joined subscription needs at least two tables; a single table is already one \
             subscription"
                .to_string(),
        );
    }
    subscriptions.sort_by(|left, right| left.table.cmp(&right.table));
    Ok(subscriptions)
}

fn reject_unbounded_shared_tables(
    scans: &[TableScanSubscription],
    shared_tables: &[&str],
) -> Result<(), String> {
    let unbounded: Vec<&str> = scans
        .iter()
        .filter(|scan| scan.filters.is_empty() && shared_tables.contains(&scan.table.as_str()))
        .map(|scan| scan.table.as_str())
        .collect();
    if unbounded.is_empty() {
        return Ok(());
    }
    let tables = unbounded.join(", ");
    Err(format!(
        "joined live query cannot subscribe to shared table {tables}: no filter uses only that \
         table's columns, so the subscription would include every row. A predicate on the other \
         side of the join does not follow changes to this table"
    ))
}

fn walk(plan: &LogicalPlan, take_projection: bool) -> Result<Vec<TableScanSubscription>, String> {
    match plan {
        LogicalPlan::Projection(projection) => {
            let mut scans = walk(&projection.input, false)?;
            if take_projection {
                for expr in &projection.expr {
                    for column in column_refs(expr)? {
                        add_column(&mut scans, column)?;
                    }
                }
            }
            Ok(scans)
        },
        LogicalPlan::Filter(filter) => {
            let mut scans = walk(&filter.input, take_projection)?;
            for conjunct in conjuncts(&filter.predicate) {
                push_conjunct(&mut scans, conjunct)?;
            }
            Ok(scans)
        },
        LogicalPlan::Join(join) => {
            if join.join_type != JoinType::Inner {
                return Err(format!(
                    "only INNER JOIN can be split into table subscriptions, found {}",
                    join.join_type
                ));
            }
            let mut scans = walk(&join.left, false)?;
            scans.extend(walk(&join.right, false)?);
            let mut saw_equality = false;
            for (left, right) in &join.on {
                saw_equality = true;
                for column in column_refs(left)?.into_iter().chain(column_refs(right)?) {
                    add_column(&mut scans, column)?;
                }
            }
            if let Some(filter) = &join.filter {
                for conjunct in conjuncts(filter) {
                    if !is_column_equality(conjunct) {
                        return Err(
                            "join condition must be column equalities; non-equi joins stay one \
                             query"
                                .to_string(),
                        );
                    }
                    saw_equality = true;
                    for column in column_refs(conjunct)? {
                        add_column(&mut scans, column)?;
                    }
                }
            }
            if !saw_equality {
                return Err(
                    "join condition must be column equalities; non-equi joins stay one query"
                        .to_string(),
                );
            }
            Ok(scans)
        },
        LogicalPlan::SubqueryAlias(alias) => {
            let mut scans = walk(&alias.input, false)?;
            if scans.len() == 1 {
                scans[0].qualifier = alias.alias.to_string();
            }
            Ok(scans)
        },
        LogicalPlan::TableScan(scan) => Ok(vec![TableScanSubscription {
            qualifier: scan.table_name.to_string(),
            table: scan.table_name.to_string(),
            columns: BTreeSet::new(),
            filters: Vec::new(),
        }]),
        other => Err(format!("subscription join split does not support {}", other.display())),
    }
}

fn push_conjunct(scans: &mut [TableScanSubscription], conjunct: &Expr) -> Result<(), String> {
    let columns = column_refs(conjunct)?;
    let mut qualifiers = BTreeSet::new();
    for column in &columns {
        let Some(relation) = column.relation.as_ref() else {
            return Err(format!("filter column {column} is not qualified by a table"));
        };
        qualifiers.insert(relation.to_string());
    }
    match qualifiers.len() {
        0 => Err(format!("filter {conjunct} does not reference a subscribed table")),
        1 => {
            let qualifier = qualifiers.into_iter().next().expect("one qualifier");
            let scan = scan_mut(scans, &qualifier)?;
            scan.filters.push(render_expr(conjunct)?);
            Ok(())
        },
        2 if is_column_equality(conjunct) => {
            for column in columns {
                add_column(scans, column)?;
            }
            Ok(())
        },
        _ => Err(format!(
            "filter {conjunct} spans more than one table and is not a join equality"
        )),
    }
}

fn is_column_equality(expr: &Expr) -> bool {
    matches!(
        expr,
        Expr::BinaryExpr(binary)
            if binary.op == Operator::Eq
                && matches!(binary.left.as_ref(), Expr::Column(_))
                && matches!(binary.right.as_ref(), Expr::Column(_))
    )
}

fn add_column(scans: &mut [TableScanSubscription], column: &Column) -> Result<(), String> {
    let Some(relation) = column.relation.as_ref() else {
        return Err(format!("column {column} is not qualified by a table"));
    };
    let scan = scan_mut(scans, &relation.to_string())?;
    scan.columns.insert(column.name.clone());
    Ok(())
}

fn scan_mut<'a>(
    scans: &'a mut [TableScanSubscription],
    qualifier: &str,
) -> Result<&'a mut TableScanSubscription, String> {
    if let Some(index) = scans.iter().position(|scan| scan.qualifier == qualifier) {
        return Ok(&mut scans[index]);
    }
    let known: Vec<&str> = scans.iter().map(|scan| scan.qualifier.as_str()).collect();
    Err(format!(
        "column qualifier {qualifier} is not one of the subscribed tables {known:?}"
    ))
}

fn conjuncts(expr: &Expr) -> Vec<&Expr> {
    match expr {
        Expr::BinaryExpr(binary) if binary.op == Operator::And => {
            let mut parts = conjuncts(&binary.left);
            parts.extend(conjuncts(&binary.right));
            parts
        },
        _ => vec![expr],
    }
}

fn column_refs(expr: &Expr) -> Result<Vec<&Column>, String> {
    let mut columns = Vec::new();
    collect_columns(expr, &mut columns)?;
    Ok(columns)
}

fn collect_columns<'a>(expr: &'a Expr, columns: &mut Vec<&'a Column>) -> Result<(), String> {
    match expr {
        Expr::Column(column) => columns.push(column),
        Expr::Alias(alias) => collect_columns(&alias.expr, columns)?,
        Expr::Cast(cast) => collect_columns(&cast.expr, columns)?,
        Expr::TryCast(cast) => collect_columns(&cast.expr, columns)?,
        Expr::BinaryExpr(binary) => {
            collect_columns(&binary.left, columns)?;
            collect_columns(&binary.right, columns)?;
        },
        Expr::IsNull(inner) | Expr::IsNotNull(inner) | Expr::Not(inner) | Expr::Negative(inner) => {
            collect_columns(inner, columns)?
        },
        Expr::Literal(_, _) => {},
        other => {
            return Err(format!("subscription join split does not support expression {other}"));
        },
    }
    Ok(())
}

fn render_expr(expr: &Expr) -> Result<String, String> {
    match expr {
        Expr::Column(column) => Ok(column.name.clone()),
        Expr::Literal(value, _) => render_literal(value),
        Expr::BinaryExpr(binary) => Ok(format!(
            "{} {} {}",
            render_expr(&binary.left)?,
            binary.op,
            render_expr(&binary.right)?
        )),
        Expr::IsNull(inner) => Ok(format!("{} IS NULL", render_expr(inner)?)),
        Expr::IsNotNull(inner) => Ok(format!("{} IS NOT NULL", render_expr(inner)?)),
        other => Err(format!("cannot render subscription filter {other}")),
    }
}

fn render_literal(value: &ScalarValue) -> Result<String, String> {
    match value {
        ScalarValue::Utf8(Some(text))
        | ScalarValue::LargeUtf8(Some(text))
        | ScalarValue::Utf8View(Some(text)) => Ok(format!("'{}'", text.replace('\'', "''"))),
        ScalarValue::Boolean(Some(flag)) => Ok(flag.to_string()),
        ScalarValue::Int64(Some(number)) => Ok(number.to_string()),
        other => Err(format!("cannot render literal {other:?} in a subscription filter")),
    }
}

const SHARED_CHAT: &[&str] = &["chat.messages", "chat.conversations", "chat.members"];

#[tokio::test]
async fn shared_join_rejects_a_side_that_only_the_other_table_constrains() {
    let error = split_sql(
        "SELECT m.id, m.body, c.title \
         FROM chat.messages AS m \
         INNER JOIN chat.conversations AS c ON m.conversation_id = c.id \
         WHERE c.owner_id = 'alice'",
        SHARED_CHAT,
    )
    .await
    .expect_err("messages would subscribe to every shared row");

    assert!(
        error.contains("chat.messages"),
        "the unfiltered shared table must be named, got {error}"
    );
    assert!(
        !error.contains("SELECT body"),
        "the split must not open an unfiltered messages subscription, got {error}"
    );
}

#[tokio::test]
async fn shared_join_rejects_every_side_without_its_own_filter() {
    let error = split_sql(
        "SELECT m.id, m.body, c.title, mem.user_id \
         FROM chat.messages AS m \
         INNER JOIN chat.conversations AS c ON m.conversation_id = c.id \
         INNER JOIN chat.members AS mem ON mem.conversation_id = c.id \
         WHERE mem.user_id = 'alice'",
        SHARED_CHAT,
    )
    .await
    .expect_err("messages and conversations are not limited by their own columns");

    let reason = error.lines().next().unwrap_or(error.as_str());
    assert!(reason.contains("chat.conversations"), "got {reason}");
    assert!(reason.contains("chat.messages"), "got {reason}");
    assert!(!reason.contains("chat.members"), "members has its own filter, got {reason}");
}

#[tokio::test]
async fn user_tables_can_split_without_a_filter_on_every_side() {
    let subscriptions = split_sql(
        "SELECT m.id, m.body, c.title \
         FROM chat.messages AS m \
         INNER JOIN chat.conversations AS c ON m.conversation_id = c.id \
         WHERE c.owner_id = 'alice'",
        &[],
    )
    .await
    .expect("user tables are already scoped to the session user");

    assert_eq!(
        subscriptions
            .iter()
            .map(|subscription| subscription.sql.as_str())
            .collect::<Vec<_>>(),
        vec![
            "SELECT id, title FROM chat.conversations WHERE owner_id = 'alice'",
            "SELECT body, conversation_id, id FROM chat.messages",
        ]
    );
}

#[tokio::test]
async fn shared_join_splits_when_each_table_filters_its_own_columns() {
    let subscriptions = split_sql(
        "SELECT m.id, m.body, c.title \
         FROM chat.messages AS m \
         INNER JOIN chat.conversations AS c ON m.conversation_id = c.id \
         WHERE c.owner_id = 'alice' AND m.author_id = 'alice'",
        SHARED_CHAT,
    )
    .await
    .expect("each shared table has a row-local filter");

    assert_eq!(
        subscriptions
            .iter()
            .map(|subscription| subscription.sql.as_str())
            .collect::<Vec<_>>(),
        vec![
            "SELECT id, title FROM chat.conversations WHERE owner_id = 'alice'",
            "SELECT body, conversation_id, id FROM chat.messages WHERE author_id = 'alice'",
        ]
    );
}

#[tokio::test]
async fn left_join_is_not_split() {
    let error = split_sql(
        "SELECT m.id, c.title \
         FROM chat.messages AS m \
         LEFT JOIN chat.conversations AS c ON m.conversation_id = c.id",
        SHARED_CHAT,
    )
    .await
    .expect_err("outer join is outside this split");

    assert!(
        error.contains("only INNER JOIN"),
        "expected an inner-join rejection, got {error}"
    );
}
