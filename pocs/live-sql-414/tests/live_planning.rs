use std::sync::Arc;

use datafusion::{
    arrow::datatypes::{DataType, Field, Schema},
    catalog::{MemorySchemaProvider, SchemaProvider, TableProvider},
    datasource::MemTable,
    prelude::SessionContext,
};
use kalamdb_live_sql_414_poc::{
    prepare_live_plan, DynamicJoinKeys, ExecutionMode, KeyAction, RefreshCoordinator,
    RefreshPolicy, RowImage, TableChange,
};
use datafusion::common::ScalarValue;

fn table(columns: &[(&str, DataType)]) -> Arc<dyn TableProvider> {
    let schema = Arc::new(Schema::new(columns.iter().map(|(name, dtype)| {
        Field::new(*name, dtype.clone(), true)
    }).collect::<Vec<_>>()));
    Arc::new(MemTable::try_new(schema, vec![vec![]]).unwrap())
}

fn context() -> SessionContext {
    let ctx = SessionContext::new();
    let chat = Arc::new(MemorySchemaProvider::new());
    chat.register_table(
        "conversations".into(),
        table(&[
            ("id", DataType::Utf8),
            ("owner_id", DataType::Utf8),
            ("title", DataType::Utf8),
        ]),
    ).unwrap();
    chat.register_table(
        "messages".into(),
        table(&[
            ("id", DataType::Utf8),
            ("conversation_id", DataType::Utf8),
            ("author_id", DataType::Utf8),
            ("body", DataType::Utf8),
            ("score", DataType::Int64),
            ("last_updated", DataType::Int64),
        ]),
    ).unwrap();
    ctx.catalog("datafusion").unwrap().register_schema("chat", chat).unwrap();
    ctx
}

async fn plan(sql: &str) -> kalamdb_live_sql_414_poc::LivePlan {
    let logical = context().sql(sql).await.unwrap().into_unoptimized_plan();
    prepare_live_plan(sql, &logical).unwrap()
}

#[tokio::test]
async fn simple_select_preserves_old_delta_path() {
    let p = plan("SELECT id, body FROM chat.messages WHERE author_id = 'alice'").await;
    assert_eq!(p.mode, ExecutionMode::SingleTableDelta);
    assert_eq!(p.sources.len(), 1);
    assert!(p.sources[0].local_filters.iter().any(|s| s.contains("alice")));
}

#[tokio::test]
async fn filtered_join_discovers_sources_keys_and_one_sided_filter() {
    let p = plan(
        "SELECT c.id, c.title, m.id, m.body \
         FROM chat.conversations AS c \
         INNER JOIN chat.messages AS m ON m.conversation_id = c.id \
         WHERE c.owner_id = 'alice' AND m.last_updated > 10"
    ).await;
    assert_eq!(p.mode, ExecutionMode::ClientJoinCandidate);
    assert_eq!(p.sources.len(), 2);
    assert_eq!(p.join_keys.len(), 1);
    assert_eq!(p.join_keys[0].left.name, "id");
    assert_eq!(p.join_keys[0].right.name, "conversation_id");
    assert!(p.sources.iter().any(|s| s.table == "chat.conversations" && s.local_filters.iter().any(|s| s.contains("alice"))));
    assert!(p.sources.iter().any(|s| s.table == "chat.messages" && s.local_filters.iter().any(|s| s.contains("10"))));
}

#[tokio::test]
async fn one_sided_shared_filter_is_not_rejected_by_splitter() {
    let p = plan(
        "SELECT c.title, m.body FROM chat.conversations c \
         JOIN chat.messages m ON m.conversation_id = c.id WHERE c.owner_id = 'alice'"
    ).await;
    assert_eq!(p.mode, ExecutionMode::ClientJoinCandidate);
    assert!(p.reason.contains("dynamic"));
}

#[tokio::test]
async fn aggregate_and_group_by_choose_server_refresh() {
    let p = plan(
        "SELECT conversation_id, COUNT(*) AS total, SUM(score) AS points, \
         AVG(score) AS average, MIN(score) AS minimum, MAX(score) AS maximum \
         FROM chat.messages GROUP BY conversation_id"
    ).await;
    assert_eq!(p.mode, ExecutionMode::ServerQueryRefresh);
    assert_eq!(p.group_by.len(), 1);
    assert_eq!(p.sources.len(), 1);
    assert!(!p.aggregate_expressions.is_empty());
}

#[tokio::test]
async fn aggregate_over_join_triggers_from_both_tables() {
    let p = plan(
        "SELECT c.id, COUNT(m.id) AS total \
         FROM chat.conversations c LEFT JOIN chat.messages m \
         ON m.conversation_id = c.id WHERE c.owner_id = 'alice' \
         GROUP BY c.id HAVING COUNT(m.id) >= 0"
    ).await;
    assert_eq!(p.mode, ExecutionMode::ServerQueryRefresh);
    let mut registry = RefreshCoordinator::default();
    let query = registry.register(p).unwrap();
    assert_eq!(registry.notify_committed(&TableChange {
        table: "chat.messages".into(), old: None,
        new: Some(RowImage::with_text("conversation_id", "10")),
    }, 1), 1);
    assert_eq!(registry.notify_committed(&TableChange {
        table: "chat.conversations".into(), old: None,
        new: Some(RowImage::with_text("id", "10")),
    }, 2), 1);
    assert_eq!(registry.invalidation_count(query), Some(2));
    let job = registry.take_due(query, 30, RefreshPolicy::default()).unwrap();
    assert_eq!(job.original_sql.contains("LEFT JOIN"), true);
    assert!(job.full_refresh_required);
}

#[tokio::test]
async fn coalesces_burst_and_remembers_invalidation_while_running() {
    let mut registry = RefreshCoordinator::default();
    let query = registry.register(plan(
        "SELECT conversation_id, COUNT(*) AS total FROM chat.messages GROUP BY conversation_id"
    ).await).unwrap();
    for t in 0..100 {
        registry.notify_committed(&TableChange {
            table: "chat.messages".into(), old: None,
            new: Some(RowImage::with_text("conversation_id", "10")),
        }, t);
    }
    assert_eq!(registry.invalidation_count(query), Some(100));
    let policy = RefreshPolicy { debounce_ms: 25, max_wait_ms: 150 };
    assert!(registry.take_due(query, 105, policy).is_none());
    let first = registry.take_due(query, 125, policy).unwrap();
    assert_eq!(first.generation, 1);
    assert_eq!(first.affected_groups_hint.len(), 1);
    assert!(registry.take_due(query, 150, policy).is_none()); // one in flight
    registry.notify_committed(&TableChange {
        table: "chat.messages".into(),
        old: Some(RowImage::with_text("conversation_id", "10")),
        new: Some(RowImage::with_text("conversation_id", "20")),
    }, 151);
    assert!(registry.finish(query, 1, true, 151).unwrap());
    assert!(registry.take_due(query, 160, policy).is_none());
    let second = registry.take_due(query, 180, policy).unwrap();
    assert_eq!(second.generation, 2);
    assert_eq!(second.affected_groups_hint.len(), 2);
    assert!(registry.finish(query, 1, true, 181).is_err());
    assert!(!registry.finish(query, 2, true, 181).unwrap());
    registry.unregister(query);
    assert_eq!(registry.notify_committed(&TableChange {
        table: "chat.messages".into(), old: None, new: None,
    }, 200), 0);
}

#[test]
fn dynamic_keys_backfill_and_revoke_after_owner_transfer() {
    let mut joined = DynamicJoinKeys::default();
    let ten = ScalarValue::Utf8(Some("10".into()));
    let twenty = ScalarValue::Utf8(Some("20".into()));
    assert_eq!(joined.parent_change(None, Some((ten.clone(), true))),
        vec![KeyAction::BackfillAndListen(ten.clone())]);
    assert!(joined.matches_child_key(&ten));
    assert_eq!(joined.parent_change(Some((ten.clone(), true)), Some((ten.clone(), false))),
        vec![KeyAction::Revoke(ten.clone())]);
    assert!(!joined.matches_child_key(&ten));
    assert_eq!(joined.parent_change(None, Some((twenty.clone(), true))),
        vec![KeyAction::BackfillAndListen(twenty.clone())]);
    assert_eq!(joined.child_change(Some(&twenty), Some(&ten)), (true, false));
}
