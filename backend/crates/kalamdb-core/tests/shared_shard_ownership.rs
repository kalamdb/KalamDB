mod support;

use std::{collections::HashSet, sync::Arc};

use kalamdb_commons::{
    ids::{same_domain_cmp, VersionError, VersionId},
    NamespaceId, TableId,
};
use kalamdb_configs::ServerConfig;
use kalamdb_core::sql::{
    context::ExecutionContext,
    executor::{handler_registry::HandlerRegistry, SqlExecutor},
};
use kalamdb_sharding::{GroupId, ShardRouter};
use support::{
    create_cluster_app_context_with_shared_shards, execute_err, execute_ok, observer_exec_ctx,
    request_exec_ctx, result_rows,
};

#[tokio::test]
#[ntest::timeout(1000)]
async fn shared_owners_concurrent_versions_transactions_and_catalog_recovery() {
    let (app, _db) =
        create_cluster_app_context_with_shared_shards(ServerConfig::default(), 4).await;
    let registry = Arc::new(HandlerRegistry::new());
    kalamdb_handlers::register_all_handlers(&registry, Arc::clone(&app), false);
    let executor = Arc::new(SqlExecutor::new(Arc::clone(&app), registry));
    let observer = observer_exec_ctx(&app);
    execute_ok(&executor, &observer, "CREATE NAMESPACE ownership").await;
    let mut tables = Vec::new();
    let mut owners = HashSet::new();
    for i in 0..16 {
        execute_ok(
            &executor,
            &observer,
            &format!("CREATE SHARED TABLE ownership.t{i} (id BIGINT PRIMARY KEY, name TEXT)"),
        )
        .await;
        let id = TableId::new(NamespaceId::new("ownership"), format!("t{i}").into());
        let owner = app.shared_group_id(&id).unwrap();
        owners.insert(owner);
        tables.push((id, owner));
    }
    assert!(owners.len() > 1, "shared tables must land on more than one owner: {owners:?}");
    let mut tasks = Vec::new();
    // Many clients share each table; each task performs sequential commits.
    for client in 0..64 {
        let executor = Arc::clone(&executor);
        let app = Arc::clone(&app);
        let (table, owner) = tables[client % tables.len()].clone();
        tasks.push(tokio::spawn(async move {
            let ctx = observer_exec_ctx(&app);
            for n in 0..16 {
                execute_ok(
                    &executor,
                    &ctx,
                    &format!(
                        "INSERT INTO {}.{} (id, name) VALUES ({}, 'value')",
                        table.namespace_id(),
                        table.table_name(),
                        client * 16 + n
                    ),
                )
                .await;
                assert_eq!(app.shared_group_id(&table).unwrap(), owner);
            }
        }));
    }
    for task in tasks {
        task.await.unwrap();
    }
    for (table, _) in &tables {
        let rows = result_rows(
            execute_ok(
                &executor,
                &observer,
                &format!(
                    "SELECT _version FROM {}.{} ORDER BY _version LIMIT 200",
                    table.namespace_id(),
                    table.table_name()
                ),
            )
            .await,
        );
        let versions: Vec<i64> = rows
            .iter()
            .map(|row| {
                row["_version"]
                    .0
                    .as_str()
                    .unwrap_or_else(|| panic!("version {:?}", row["_version"]))
                    .parse()
                    .unwrap()
            })
            .collect();
        assert_eq!(versions.len(), 64);
        assert!(versions.windows(2).all(|pair| pair[0] < pair[1]));
    }

    execute_ok(
        &executor,
        &observer,
        "CREATE SHARED TABLE ownership.hot (id BIGINT PRIMARY KEY, name TEXT)",
    )
    .await;
    let hot = TableId::new(NamespaceId::new("ownership"), "hot".into());
    let hot_owner = app.shared_group_id(&hot).unwrap();
    let mut hot_tasks = Vec::new();
    for client in 0..50 {
        let executor = Arc::clone(&executor);
        let app = Arc::clone(&app);
        let hot = hot.clone();
        hot_tasks.push(tokio::spawn(async move {
            let ctx = observer_exec_ctx(&app);
            for n in 0..20 {
                execute_ok(
                    &executor,
                    &ctx,
                    &format!(
                        "INSERT INTO ownership.hot (id, name) VALUES ({}, 'hot')",
                        20_000 + client * 20 + n
                    ),
                )
                .await;
                assert_eq!(app.shared_group_id(&hot).unwrap(), hot_owner);
            }
        }));
    }
    for task in hot_tasks {
        task.await.unwrap();
    }
    let hot_versions = versions_for(
        &executor,
        &observer,
        "SELECT _version FROM ownership.hot ORDER BY _version LIMIT 1000",
    )
    .await;
    assert_eq!(hot_versions.len(), 1000);
    assert!(hot_versions.windows(2).all(|pair| pair[0] < pair[1]));

    let (first, owner) = &tables[0];
    let (different, _) = tables.iter().find(|(_, other)| other != owner).unwrap();
    let tx = request_exec_ctx(&app, "cross-owner");
    execute_ok(&executor, &tx, "BEGIN").await;
    execute_ok(
        &executor,
        &tx,
        &format!("INSERT INTO ownership.{} (id, name) VALUES (1000, 'tx')", first.table_name()),
    )
    .await;
    let error = execute_err(
        &executor,
        &tx,
        &format!(
            "INSERT INTO ownership.{} (id, name) VALUES (1001, 'tx')",
            different.table_name()
        ),
    )
    .await;
    assert!(error.contains("group") || error.contains("shard"), "{error}");
    execute_ok(&executor, &tx, "ROLLBACK").await;

    let (left, owner) = tables
        .iter()
        .find(|(table, owner)| {
            tables.iter().any(|(other, other_owner)| other != table && other_owner == owner)
        })
        .cloned()
        .expect("two shared tables share an owner");
    let (right, _) = tables
        .iter()
        .find(|(table, other)| table != &left && *other == owner)
        .cloned()
        .unwrap();
    execute_ok(
        &executor,
        &observer,
        &format!("INSERT INTO ownership.{} (id, name) VALUES (9000, 'a0')", left.table_name()),
    )
    .await;
    execute_ok(
        &executor,
        &observer,
        &format!("INSERT INTO ownership.{} (id, name) VALUES (9001, 'b')", right.table_name()),
    )
    .await;
    execute_ok(
        &executor,
        &observer,
        &format!("INSERT INTO ownership.{} (id, name) VALUES (9002, 'a1')", left.table_name()),
    )
    .await;
    let left_gap = versions_for(
        &executor,
        &observer,
        &format!(
            "SELECT _version FROM ownership.{} WHERE id IN (9000, 9002) ORDER BY _version",
            left.table_name()
        ),
    )
    .await;
    let right_between = versions_for(
        &executor,
        &observer,
        &format!("SELECT _version FROM ownership.{} WHERE id = 9001", right.table_name()),
    )
    .await;
    assert!(left_gap[0] < right_between[0] && right_between[0] < left_gap[1]);

    execute_ok(
        &executor,
        &observer,
        &format!(
            "INSERT INTO ownership.{} (id, name) VALUES (9300, 'old'), (9301, 'old')",
            left.table_name()
        ),
    )
    .await;
    execute_ok(
        &executor,
        &observer,
        &format!(
            "UPDATE ownership.{} SET name = 'bulk' WHERE id IN (9300, 9301)",
            left.table_name()
        ),
    )
    .await;
    let bulk = versions_for(
        &executor,
        &observer,
        &format!(
            "SELECT _version FROM ownership.{} WHERE id IN (9300, 9301) ORDER BY _version",
            left.table_name()
        ),
    )
    .await;
    assert_eq!(bulk.len(), 2);
    assert!(bulk[0] < bulk[1]);
    assert_eq!(bulk[0] >> 16, bulk[1] >> 16, "one update statement shares a log index");
    assert_ne!(bulk[0] & 0xffff, bulk[1] & 0xffff);

    execute_ok(
        &executor,
        &observer,
        &format!("INSERT INTO ownership.{} (id, name) VALUES (9200, 'old')", left.table_name()),
    )
    .await;
    let inserted = versions_for(
        &executor,
        &observer,
        &format!("SELECT _version FROM ownership.{} WHERE id = 9200", left.table_name()),
    )
    .await;
    execute_ok(
        &executor,
        &observer,
        &format!("UPDATE ownership.{} SET name = 'new' WHERE id = 9200", left.table_name()),
    )
    .await;
    let updated = versions_for(
        &executor,
        &observer,
        &format!("SELECT _version FROM ownership.{} WHERE id = 9200", left.table_name()),
    )
    .await;
    assert!(updated[0] > inserted[0]);
    execute_ok(
        &executor,
        &observer,
        &format!("UPDATE ownership.{} SET name = 'newer' WHERE id = 9200", left.table_name()),
    )
    .await;
    let updated_again = versions_for(
        &executor,
        &observer,
        &format!("SELECT _version FROM ownership.{} WHERE id = 9200", left.table_name()),
    )
    .await;
    assert!(updated_again[0] > updated[0]);
    execute_ok(
        &executor,
        &observer,
        &format!("DELETE FROM ownership.{} WHERE id = 9200", left.table_name()),
    )
    .await;
    let deleted = versions_for(
        &executor,
        &observer,
        &format!("SELECT _version FROM ownership.{} WHERE id = 9200", left.table_name()),
    )
    .await;
    assert!(deleted.is_empty());

    let same = request_exec_ctx(&app, "same-owner");
    execute_ok(&executor, &same, "BEGIN").await;
    execute_ok(
        &executor,
        &same,
        &format!("INSERT INTO ownership.{} (id, name) VALUES (9400, 'tx')", left.table_name()),
    )
    .await;
    execute_ok(
        &executor,
        &same,
        &format!("INSERT INTO ownership.{} (id, name) VALUES (9401, 'tx')", right.table_name()),
    )
    .await;
    execute_ok(&executor, &same, "COMMIT").await;
    assert_eq!(
        versions_for(
            &executor,
            &observer,
            &format!("SELECT _version FROM ownership.{} WHERE id = 9400", left.table_name()),
        )
        .await
        .len(),
        1
    );
    assert_eq!(
        versions_for(
            &executor,
            &observer,
            &format!("SELECT _version FROM ownership.{} WHERE id = 9401", right.table_name()),
        )
        .await
        .len(),
        1
    );

    let left_domain =
        app.schema_registry().get(&left).unwrap().table.shared_version_domain().unwrap();
    let other_table = tables.iter().find(|(_, other)| *other != owner).unwrap().0.clone();
    let other_domain = app
        .schema_registry()
        .get(&other_table)
        .unwrap()
        .table
        .shared_version_domain()
        .unwrap();
    let GroupId::DataSharedShard(shard) = owner else {
        panic!("shared owner must be a data shard");
    };
    assert_eq!(left_domain.scope_id, u64::from(shard));
    let sample = VersionId::try_from_i64(left_gap[0]).unwrap();
    assert_eq!(
        same_domain_cmp(&left_domain, sample, &other_domain, sample),
        Err(VersionError::DomainMismatch)
    );

    // Durable catalog keeps the owner when the configured shard count grows.
    let expanded = ShardRouter::new(32, 8);
    for (table, owner) in tables {
        let before = app.schema_registry().get(&table).unwrap().table.shared_version_domain();
        let stored = app
            .system_tables()
            .tables()
            .get_table_by_id(&table)
            .unwrap()
            .expect("persisted table definition");
        assert_eq!(expanded.shared_group_id(&stored).unwrap(), owner);
        assert_eq!(stored.shared_version_domain(), before);
    }
    app.executor().shutdown().await.unwrap();
}

async fn versions_for(executor: &SqlExecutor, observer: &ExecutionContext, sql: &str) -> Vec<i64> {
    result_rows(execute_ok(executor, observer, sql).await)
        .iter()
        .map(|row| {
            row["_version"]
                .0
                .as_str()
                .unwrap_or_else(|| panic!("version {:?}", row["_version"]))
                .parse()
                .unwrap()
        })
        .collect()
}
