mod support;

use std::{sync::Arc, time::Duration};

use datafusion_common::ScalarValue;
use kalamdb_commons::{
    constants::SystemColumnNames,
    ids::VersionId,
    models::{rows::Row, ConnectionId, ConnectionInfo, UserId},
    websocket::{SubscriptionOptions, SubscriptionRequest},
    NamespaceId, Role, TableId,
};
use kalamdb_configs::ServerConfig;
use kalamdb_core::{
    app_context::AppContext,
    live_adapters::RaftApplyBarrierAdapter,
    sql::executor::{handler_registry::HandlerRegistry, SqlExecutor},
};
use kalamdb_live::{traits::LiveApplyBarrier, InitialDataOptions};
use kalamdb_raft::{GroupId, RaftExecutor};
use kalamdb_sharding::ShardRouter;
use support::{
    create_cluster_app_context_with_shared_shards, execute_ok, observer_exec_ctx, result_rows,
};
use tokio::time::timeout;

#[tokio::test]
#[ntest::timeout(8_000)]
async fn shared_table_restart_keeps_owner_and_version_frontier() {
    let (app, db) = create_cluster_app_context_with_shared_shards(ServerConfig::default(), 4).await;
    let executor = sql_executor(&app);
    let observer = observer_exec_ctx(&app);
    execute_ok(&executor, &observer, "CREATE NAMESPACE restart_ns").await;
    execute_ok(
        &executor,
        &observer,
        "CREATE SHARED TABLE restart_ns.items (id BIGINT PRIMARY KEY, name TEXT)",
    )
    .await;
    execute_ok(
        &executor,
        &observer,
        "INSERT INTO restart_ns.items (id, name) VALUES (1, 'a'), (2, 'b')",
    )
    .await;
    let table = TableId::new(NamespaceId::new("restart_ns"), "items".into());
    let owner = app.shared_group_id(&table).unwrap();
    let before = app
        .schema_registry()
        .get_table_if_exists(&table)
        .unwrap()
        .unwrap()
        .shared_version_domain()
        .unwrap();
    let versions = versions_for(
        &executor,
        &observer,
        "SELECT _version FROM restart_ns.items ORDER BY _version LIMIT 20",
    )
    .await;
    assert_eq!(versions.len(), 2);
    assert!(versions[0] < versions[1]);

    let backend = app.storage_backend();
    let storage_path = db.storage_dir().unwrap().to_string_lossy().into_owned();
    let mut config = (**app.config()).clone();
    if let Some(cluster) = config.cluster.as_mut() {
        let port = cluster.rpc_addr.rsplit(':').next().unwrap().parse::<u16>().unwrap();
        cluster.rpc_addr = format!("127.0.0.1:{}", port.saturating_add(2));
    }
    app.executor().shutdown().await.unwrap();
    drop(executor);
    drop(observer);
    drop(app);

    let app =
        AppContext::create_isolated(backend, kalamdb_commons::NodeId::new(1), storage_path, config);
    app.wire_raft_appliers();
    app.executor().start().await.unwrap();
    app.executor().initialize_cluster().await.unwrap();
    let executor = sql_executor(&app);
    executor.load_existing_tables().await.unwrap();
    app.restore_raft_state_machines().await;
    let ready = std::time::Instant::now() + Duration::from_secs(3);
    loop {
        let meta = app.executor().is_leader(GroupId::Meta).await;
        let data = app.executor().is_leader(owner).await;
        if meta && data {
            break;
        }
        assert!(std::time::Instant::now() < ready, "restarted node did not become leader");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    assert_eq!(app.shared_group_id(&table).unwrap(), owner);
    let after = app
        .schema_registry()
        .get_table_if_exists(&table)
        .unwrap()
        .unwrap()
        .shared_version_domain()
        .unwrap();
    assert_eq!(after, before);
    let restored = versions_for(
        &executor,
        &observer_exec_ctx(&app),
        "SELECT _version FROM restart_ns.items ORDER BY _version LIMIT 20",
    )
    .await;
    assert_eq!(restored, versions);

    let grown = ShardRouter::new(32, 8);
    let stored = app.schema_registry().get_table_if_exists(&table).unwrap().unwrap();
    assert_eq!(grown.shared_group_id(&stored).unwrap(), owner);

    let inserted = execute_ok(
        &executor,
        &observer_exec_ctx(&app),
        "INSERT INTO restart_ns.items (id, name) VALUES (3, 'c')",
    )
    .await;
    let continued_rows = result_rows(
        execute_ok(
            &executor,
            &observer_exec_ctx(&app),
            "SELECT id, name, _version FROM restart_ns.items ORDER BY id LIMIT 20",
        )
        .await,
    );
    let continued = versions_for(
        &executor,
        &observer_exec_ctx(&app),
        "SELECT _version FROM restart_ns.items ORDER BY _version LIMIT 20",
    )
    .await;
    match &inserted {
        kalamdb_core::sql::ExecutionResult::Inserted { rows_affected: 1 } => {},
        other => panic!("restarted insert did not persist one row: {other:?}"),
    }
    assert_eq!(continued.len(), 3, "rows={continued_rows:?} versions={continued:?}");
    assert!(continued[2] > continued[1]);
    assert_eq!(app.shared_group_id(&table).unwrap(), owner);
    app.executor().shutdown().await.unwrap();
}

#[tokio::test]
#[ntest::timeout(4000)]
async fn shared_live_resume_sees_every_version_on_owner_group() {
    let (app, _db) =
        create_cluster_app_context_with_shared_shards(ServerConfig::default(), 4).await;
    let executor = sql_executor(&app);
    let observer = observer_exec_ctx(&app);
    execute_ok(&executor, &observer, "CREATE NAMESPACE live_ns").await;
    let mut owner = GroupId::DataSharedShard(0);
    let mut table_name = String::new();
    for i in 0..8 {
        let name = format!("t{i}");
        execute_ok(
            &executor,
            &observer,
            &format!("CREATE SHARED TABLE live_ns.{name} (id BIGINT PRIMARY KEY, name TEXT)"),
        )
        .await;
        let id = TableId::new(NamespaceId::new("live_ns"), name.clone().into());
        let group = app.shared_group_id(&id).unwrap();
        if !matches!(group, GroupId::DataSharedShard(0)) {
            owner = group;
            table_name = name;
            break;
        }
        table_name = name;
        owner = group;
    }
    let table = TableId::new(NamespaceId::new("live_ns"), table_name.clone().into());
    let domain = app
        .schema_registry()
        .get_table_if_exists(&table)
        .unwrap()
        .unwrap()
        .shared_version_domain()
        .unwrap();
    assert_eq!(
        domain.scope_id,
        match owner {
            GroupId::DataSharedShard(shard) => u64::from(shard),
            _ => panic!("shared owner"),
        }
    );

    for id in 1..=3 {
        execute_ok(
            &executor,
            &observer,
            &format!("INSERT INTO live_ns.{table_name} (id, name) VALUES ({id}, 'before')"),
        )
        .await;
    }

    let registry = app.connection_registry();
    let registration = registry
        .register_connection(ConnectionId::new("live-owner"), ConnectionInfo::new(None))
        .unwrap();
    registration.state.mark_authenticated(UserId::new("root"), Role::Dba);
    let request = SubscriptionRequest {
        id:      "sub-live".to_string(),
        sql:     format!("SELECT * FROM live_ns.{table_name}"),
        options: Some(SubscriptionOptions::default()),
    };
    let subscribed = app
        .live_query_manager()
        .register_subscription_with_initial_data(
            &registration.state,
            &request,
            Some(InitialDataOptions::default().with_limit(100)),
        )
        .await
        .unwrap();
    assert_eq!(subscribed.version_domain.as_ref(), Some(&domain));
    let mut seen = versions_of_rows(subscribed.initial_data.unwrap().rows);
    assert_eq!(seen.len(), 3);
    assert!(seen.windows(2).all(|pair| pair[0] < pair[1]));

    execute_ok(
        &executor,
        &observer,
        &format!("INSERT INTO live_ns.{table_name} (id, name) VALUES (4, 'during')"),
    )
    .await;
    let flushed = registration.state.complete_initial_load(&request.id);
    assert!(flushed >= 1, "the insert during the snapshot handoff must be delivered");

    execute_ok(
        &executor,
        &observer,
        &format!("INSERT INTO live_ns.{table_name} (id, name) VALUES (5, 'after')"),
    )
    .await;
    let mut notification_rx = registration.notification_rx;
    let mut notifications = Vec::new();
    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    while notifications.len() < 2 && std::time::Instant::now() < deadline {
        if let Ok(Some(note)) = timeout(Duration::from_millis(200), notification_rx.recv()).await {
            if note.subscription_id.as_ref() == "sub-live" {
                notifications.push(note);
            }
        }
    }
    seen.extend(notification_versions(&notifications));
    seen.sort_unstable();
    seen.dedup();

    let stored = versions_for(
        &executor,
        &observer,
        &format!("SELECT _version FROM live_ns.{table_name} ORDER BY _version LIMIT 20"),
    )
    .await;
    assert_eq!(seen, stored, "snapshot plus live delivery must include every committed version");
    assert!(stored.windows(2).all(|pair| pair[0] < pair[1]));

    let barrier = RaftApplyBarrierAdapter::new(Arc::clone(&app));
    barrier
        .wait_for_table_apply_barrier(
            &table,
            kalamdb_commons::TableType::Shared,
            &UserId::new("root"),
        )
        .await
        .unwrap();
    let executor_handle = app.executor();
    let raft = executor_handle.as_any().downcast_ref::<RaftExecutor>().unwrap();
    let owner_applied = applied_index(raft, owner);
    let idle = if matches!(owner, GroupId::DataSharedShard(0)) {
        GroupId::DataSharedShard(1)
    } else {
        GroupId::DataSharedShard(0)
    };
    let idle_applied = applied_index(raft, idle);
    assert!(
        owner_applied > idle_applied,
        "barrier follows the owner group {owner:?} ({owner_applied}), not {idle:?} \
         ({idle_applied})"
    );

    app.live_query_manager()
        .unregister_subscription(&registration.state, &request.id, &subscribed.live_id)
        .await
        .unwrap();

    let last = VersionId::try_from_i64(*stored.last().unwrap()).unwrap();
    let resume = SubscriptionRequest {
        id:      "sub-resume".to_string(),
        sql:     format!("SELECT * FROM live_ns.{table_name}"),
        options: Some(SubscriptionOptions {
            version_domain: Some(domain.clone()),
            from: Some(last),
            ..SubscriptionOptions::default()
        }),
    };
    app.live_query_manager()
        .register_subscription_with_initial_data(
            &registration.state,
            &resume,
            Some(InitialDataOptions::since(last)),
        )
        .await
        .expect("resume in the same domain");

    let mut foreign = domain.clone();
    foreign.history_incarnation = "rotated-history".to_string();
    let rejected = SubscriptionRequest {
        id:      "sub-foreign".to_string(),
        sql:     format!("SELECT * FROM live_ns.{table_name}"),
        options: Some(SubscriptionOptions {
            version_domain: Some(foreign),
            from: Some(last),
            ..SubscriptionOptions::default()
        }),
    };
    let error = app
        .live_query_manager()
        .register_subscription_with_initial_data(
            &registration.state,
            &rejected,
            Some(InitialDataOptions::since(last)),
        )
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("domain"), "{error}");

    let missing_domain = SubscriptionRequest {
        id:      "sub-bare".to_string(),
        sql:     format!("SELECT * FROM live_ns.{table_name}"),
        options: Some(SubscriptionOptions {
            from: Some(last),
            ..SubscriptionOptions::default()
        }),
    };
    let error = app
        .live_query_manager()
        .register_subscription_with_initial_data(
            &registration.state,
            &missing_domain,
            Some(InitialDataOptions::since(last)),
        )
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("domain"), "{error}");

    app.live_query_manager()
        .unregister_connection(&UserId::new("root"), &ConnectionId::new("live-owner"))
        .await
        .unwrap();
    let left = result_rows(
        execute_ok(&executor, &observer, "SELECT subscription_id FROM system.live").await,
    );
    assert!(left.is_empty(), "unsubscribed live state must be released: {left:?}");
    app.executor().shutdown().await.unwrap();
}

fn sql_executor(app: &Arc<AppContext>) -> Arc<SqlExecutor> {
    let registry = Arc::new(HandlerRegistry::new());
    kalamdb_handlers::register_all_handlers(&registry, Arc::clone(app), false);
    let executor = Arc::new(SqlExecutor::new(Arc::clone(app), registry));
    app.set_sql_executor(Arc::clone(&executor));
    executor
}

fn applied_index(raft: &RaftExecutor, group: GroupId) -> u64 {
    raft.manager()
        .group_metrics(group)
        .and_then(|metrics| metrics.last_applied)
        .map(|id| id.index)
        .unwrap_or(0)
}

async fn versions_for(
    executor: &SqlExecutor,
    observer: &kalamdb_core::sql::context::ExecutionContext,
    sql: &str,
) -> Vec<i64> {
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

fn versions_of_rows(rows: Vec<Row>) -> Vec<i64> {
    rows.iter()
        .map(|row| match row.values.get(SystemColumnNames::VERSION) {
            Some(ScalarValue::Int64(Some(version))) => *version,
            Some(ScalarValue::Utf8(Some(version))) => version.parse().unwrap(),
            other => panic!("row version {other:?}"),
        })
        .collect()
}

fn notification_versions(notes: &[Arc<kalamdb_commons::websocket::WireNotification>]) -> Vec<i64> {
    let mut versions = Vec::new();
    for note in notes {
        let Some(rows) = note.payload.rows.as_ref() else {
            continue;
        };
        for row in rows {
            if let Some(cell) = row.get(SystemColumnNames::VERSION) {
                let version =
                    cell.0.as_i64().or_else(|| cell.0.as_str().and_then(|raw| raw.parse().ok()));
                if let Some(version) = version {
                    versions.push(version);
                }
            }
        }
    }
    versions
}
