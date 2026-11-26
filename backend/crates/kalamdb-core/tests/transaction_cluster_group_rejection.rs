use crate::support::{
    create_cluster_app_context, create_cluster_app_context_with_shared_shards, create_executor,
    create_shared_table, create_shared_table_in_shard, create_standalone_app_context,
    create_user_table, execute_err, execute_ok, insert_sql, request_exec_ctx,
    request_transaction_coordinator, request_transaction_state, row, select_names,
    unique_namespace,
};
use datafusion_common::ScalarValue;
use kalamdb_commons::{
    models::{OperationKind, TransactionOrigin, UserId},
    TableType,
};
use kalamdb_configs::ServerConfig;
use kalamdb_core::transactions::{StagedMutation, TransactionRaftBinding};
use kalamdb_transactions::ExecutionOwnerKey;
use ntest::timeout;

#[tokio::test]
#[timeout(10000)]
async fn sql_request_transaction_rejects_cross_group_access_without_aborting_bound_group() {
    let (app_ctx, _test_db) = create_cluster_app_context().await;
    let namespace = unique_namespace("tx_cluster_groups");
    let user_table = create_user_table(&app_ctx, &namespace, "user_items").await;
    let shared_table = create_shared_table(&app_ctx, &namespace, "shared_items").await;
    let executor = create_executor(app_ctx.clone());
    let request_ctx = request_exec_ctx(&app_ctx, "sql-cluster-groups-active");
    let observer_ctx = request_exec_ctx(&app_ctx, "sql-cluster-groups-observer");

    execute_ok(&executor, &request_ctx, "BEGIN").await;
    execute_ok(&executor, &request_ctx, &insert_sql(&user_table, 1, "alpha")).await;

    let tx_coordinator = request_transaction_coordinator(app_ctx.as_ref());
    let mut request_state = request_transaction_state(&request_ctx);
    request_state.sync(&tx_coordinator);
    let transaction_id = request_state
        .active_transaction_id()
        .cloned()
        .expect("transaction remains active after BEGIN");

    let handle = app_ctx
        .transaction_coordinator()
        .get_handle(&transaction_id)
        .expect("transaction handle exists");
    let bound_group = match handle.raft_binding {
        TransactionRaftBinding::BoundCluster { group_id, .. } => group_id,
        other => panic!("expected bound cluster transaction, got {:?}", other),
    };

    let error = execute_err(&executor, &request_ctx, &insert_sql(&shared_table, 2, "beta")).await;
    assert!(
        error.contains("already bound to data raft group"),
        "expected cross-group rejection, got: {error}"
    );
    assert!(
        error.contains(&bound_group.to_string()),
        "expected bound group in error: {error}"
    );
    assert!(
        error.contains("data:shared:00"),
        "expected requested shared group in error: {error}"
    );

    assert_eq!(
        select_names(&executor, &request_ctx, &user_table).await,
        vec!["alpha".to_string()]
    );
    assert!(select_names(&executor, &observer_ctx, &user_table).await.is_empty());
    assert!(select_names(&executor, &observer_ctx, &shared_table).await.is_empty());

    execute_ok(&executor, &request_ctx, "ROLLBACK").await;

    assert!(select_names(&executor, &observer_ctx, &user_table).await.is_empty());
    assert!(select_names(&executor, &observer_ctx, &shared_table).await.is_empty());
}

#[tokio::test]
#[timeout(15000)]
async fn standalone_transaction_rejects_second_group_before_staging_and_can_commit_first() {
    let (app_ctx, _test_db) = create_standalone_app_context().await;
    let namespace = unique_namespace("tx_standalone_groups");
    let user_table = create_user_table(&app_ctx, &namespace, "user_items").await;
    let shared_table = create_shared_table(&app_ctx, &namespace, "shared_items").await;
    let executor = create_executor(app_ctx.clone());
    let request_ctx = request_exec_ctx(&app_ctx, "sql-standalone-groups-active");
    let observer_ctx = request_exec_ctx(&app_ctx, "sql-standalone-groups-observer");

    execute_ok(&executor, &request_ctx, "BEGIN").await;
    execute_ok(&executor, &request_ctx, &insert_sql(&user_table, 1, "alpha")).await;

    let tx_coordinator = request_transaction_coordinator(app_ctx.as_ref());
    let mut request_state = request_transaction_state(&request_ctx);
    request_state.sync(&tx_coordinator);
    let transaction_id =
        request_state.active_transaction_id().cloned().expect("active transaction");
    let handle = app_ctx.transaction_coordinator().get_handle(&transaction_id).expect("handle");
    let bound_group = match handle.raft_binding {
        TransactionRaftBinding::LocalSingleNode {
            group_id: Some(group_id),
        } => group_id,
        other => panic!("expected bound standalone transaction, got {other:?}"),
    };

    let error = execute_err(&executor, &request_ctx, &insert_sql(&shared_table, 2, "beta")).await;
    assert!(error.contains(&bound_group.to_string()), "{error}");
    assert!(error.contains("data:shared:00"), "{error}");
    assert_eq!(
        select_names(&executor, &request_ctx, &user_table).await,
        vec!["alpha".to_string()]
    );
    assert!(select_names(&executor, &observer_ctx, &user_table).await.is_empty());
    assert!(select_names(&executor, &observer_ctx, &shared_table).await.is_empty());

    execute_ok(&executor, &request_ctx, "COMMIT").await;
    assert_eq!(
        select_names(&executor, &observer_ctx, &user_table).await,
        vec!["alpha".to_string()]
    );
    assert!(select_names(&executor, &observer_ctx, &shared_table).await.is_empty());
}

#[tokio::test]
#[timeout(15000)]
async fn standalone_transaction_commits_two_users_in_its_single_user_group() {
    let (app_ctx, _test_db) = create_standalone_app_context().await;
    let namespace = unique_namespace("tx_user_groups");
    let first_table = create_user_table(&app_ctx, &namespace, "first").await;
    let first_user = UserId::new("tx-user-shard-first");
    let other_user = UserId::new("tx-user-shard-other");
    let coordinator = app_ctx.transaction_coordinator();
    let transaction_id = coordinator
        .begin(
            ExecutionOwnerKey::internal(433),
            "user-shard-test".into(),
            TransactionOrigin::Internal,
        )
        .expect("begin");

    for (user_id, id, name) in [(&first_user, 1, "alpha"), (&other_user, 2, "beta")] {
        coordinator
            .stage(
                &transaction_id,
                StagedMutation::new(
                    transaction_id.clone(),
                    first_table.clone(),
                    TableType::User,
                    Some(user_id.clone()),
                    OperationKind::Insert,
                    id.to_string(),
                    row(id, name),
                    false,
                ),
            )
            .expect("stage user write");
    }
    assert_eq!(
        app_ctx
            .transaction_coordinator()
            .get_handle(&transaction_id)
            .unwrap()
            .raft_binding,
        TransactionRaftBinding::LocalSingleNode {
            group_id: Some(kalamdb_sharding::GroupId::DataUserShard(0)),
        }
    );
    coordinator.commit(&transaction_id).await.expect("commit both users");
    let provider = app_ctx.schema_registry().get_kalam_provider(&first_table).expect("provider");
    for (user_id, id) in [(&first_user, 1), (&other_user, 2)] {
        assert!(provider
            .find_user_by_pk(user_id, &ScalarValue::Int64(Some(id)))
            .await
            .expect("lookup")
            .is_some());
    }
}

#[tokio::test]
#[timeout(15000)]
async fn one_node_cluster_transaction_rejects_a_different_shared_shard() {
    let (app_ctx, _test_db) =
        create_cluster_app_context_with_shared_shards(ServerConfig::default(), 2).await;
    let namespace = unique_namespace("tx_shared_groups");
    let first_table = create_shared_table_in_shard(&app_ctx, &namespace, "first", 0).await;
    let same_group_table =
        create_shared_table_in_shard(&app_ctx, &namespace, "same_group", 0).await;
    let other_table = create_shared_table_in_shard(&app_ctx, &namespace, "other", 1).await;
    let coordinator = app_ctx.transaction_coordinator();
    let transaction_id = coordinator
        .begin(
            ExecutionOwnerKey::internal(434),
            "shared-shard-test".into(),
            TransactionOrigin::Internal,
        )
        .expect("begin");

    coordinator
        .validate_table_access(&transaction_id, &first_table, TableType::Shared, None)
        .expect("first group");
    coordinator
        .validate_table_access(&transaction_id, &same_group_table, TableType::Shared, None)
        .expect("same group");
    let error = coordinator
        .validate_table_access(&transaction_id, &other_table, TableType::Shared, None)
        .expect_err("different group");
    assert!(error.to_string().contains("data:shared:00"));
    assert!(error.to_string().contains("data:shared:01"));
    coordinator
        .validate_table_access(&transaction_id, &first_table, TableType::Shared, None)
        .expect("transaction remains usable");
    coordinator.rollback(&transaction_id).expect("rollback");
}
