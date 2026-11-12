use crate::support::{
    create_counting_cluster_app_context, create_user_table, row, unique_namespace,
};
use datafusion_common::ScalarValue;
use kalamdb_commons::{
    models::{OperationKind, TransactionOrigin, UserId},
    TableType,
};
use kalamdb_core::transactions::{ExecutionOwnerKey, StagedMutation};
use kalamdb_raft::{DataResponse, KalamStateMachine, RaftCommand, RaftExecutor};
use kalamdb_sharding::ShardRouter;
use kalamdb_store::EntityStore;
use kalamdb_tables::KalamTableProvider;

fn insert_mutation(
    transaction_id: &kalamdb_commons::models::TransactionId,
    table_id: &kalamdb_commons::models::TableId,
    user_id: &UserId,
    pk_value: &str,
    row: kalamdb_commons::models::rows::Row,
) -> StagedMutation {
    StagedMutation::new(
        transaction_id.clone(),
        table_id.clone(),
        TableType::User,
        Some(user_id.clone()),
        OperationKind::Insert,
        pk_value,
        row,
        false,
    )
}

fn other_shard_user(router: &ShardRouter, first: &UserId) -> UserId {
    let shard = router.user_shard_id(first);
    for index in 0..4096 {
        let candidate = UserId::new(format!("tx-other-shard-{index}"));
        if router.user_shard_id(&candidate) != shard {
            return candidate;
        }
    }
    panic!("failed to find a user id on another shard");
}

async fn user_pk_present(provider: &KalamTableProvider, user_id: &UserId, id: i64) -> bool {
    provider
        .find_user_by_pk(user_id, &ScalarValue::Int64(Some(id)))
        .await
        .expect("user lookup")
        .is_some()
}

#[tokio::test]
#[ntest::timeout(15_000)]
async fn interrupted_durable_commit_is_wholly_present_or_absent() {
    let (app_ctx, _test_db, batches) = create_counting_cluster_app_context().await;
    let user_id = UserId::new("tx-replay-crash");
    let table_id = create_user_table(&app_ctx, &unique_namespace("tx_replay_crash"), "items").await;
    let provider = app_ctx.schema_registry().get_kalam_provider(&table_id).expect("user provider");
    let partition = provider.user_store().expect("user store").partition().name().to_string();
    let batches_before = batches.batches_touching(&partition);

    let coordinator = app_ctx.transaction_coordinator();
    let transaction_id = coordinator
        .begin(
            ExecutionOwnerKey::internal(41),
            "replay-crash".to_string().into(),
            TransactionOrigin::Internal,
        )
        .expect("begin");
    coordinator
        .stage(
            &transaction_id,
            insert_mutation(&transaction_id, &table_id, &user_id, "1", row(1, "alpha")),
        )
        .expect("stage alpha");
    coordinator
        .stage(
            &transaction_id,
            insert_mutation(&transaction_id, &table_id, &user_id, "2", row(2, "beta")),
        )
        .expect("stage beta");

    batches.fail_next_durable_batch_touching(&partition);
    let commit = coordinator.commit(&transaction_id).await;
    assert_eq!(
        batches.pending_durable_failures(),
        0,
        "the injected durable batch failure did not run"
    );

    let alpha = user_pk_present(provider.as_ref(), &user_id, 1).await;
    let beta = user_pk_present(provider.as_ref(), &user_id, 2).await;
    assert_eq!(alpha, beta, "an interrupted commit must not leave one of the rows");

    match commit {
        Ok(_) => {
            assert!(alpha, "a successful commit must contain both rows");
            assert_eq!(
                batches.batches_touching(&partition) - batches_before,
                1,
                "replay after the failed flush must write the commit once"
            );
        },
        Err(error) => {
            let error = error.to_string();
            assert!(
                error.contains("injected durable batch failure"),
                "interrupted commit failed for a different reason: {error}"
            );
            assert!(!alpha, "failed commit left rows: {error}");
            assert_eq!(
                batches.batches_touching(&partition),
                batches_before,
                "a failed flush must not count as a durable batch"
            );

            let router =
                ShardRouter::from_optional_cluster_config(app_ctx.config().cluster.as_ref());
            let executor = app_ctx.executor();
            let raft = executor.as_any().downcast_ref::<RaftExecutor>().expect("raft executor");
            let state_machine = raft
                .manager()
                .user_data_state_machine(router.user_shard_id(&user_id))
                .expect("user state machine");
            let index = state_machine.last_applied_index() + 1;
            let replay = RaftCommand::TransactionCommit {
                required_meta_index: 0,
                transaction_id: transaction_id.clone(),
                mutations: vec![
                    insert_mutation(&transaction_id, &table_id, &user_id, "1", row(1, "alpha")),
                    insert_mutation(&transaction_id, &table_id, &user_id, "2", row(2, "beta")),
                ],
            };
            let bytes = kalamdb_raft::codec::command_codec::encode_raft_command(&replay)
                .expect("encode replay");
            let result = state_machine.apply(index, 1, &bytes).await.expect("replay apply");
            let kalamdb_raft::ApplyResult::Ok(data) = result else {
                panic!("replay should apply the absent commit, got {result:?}");
            };
            let response = kalamdb_raft::codec::command_codec::decode_data_response(&data)
                .expect("decode replay");
            assert!(
                matches!(response, DataResponse::TransactionCommitted(_)),
                "replay did not commit the rows: {response:?}"
            );
            assert!(user_pk_present(provider.as_ref(), &user_id, 1).await);
            assert!(user_pk_present(provider.as_ref(), &user_id, 2).await);
            assert_eq!(
                batches.batches_touching(&partition) - batches_before,
                1,
                "replay writes the interrupted commit once"
            );

            let duplicate = RaftCommand::TransactionCommit {
                required_meta_index: 0,
                transaction_id: transaction_id.clone(),
                mutations: vec![insert_mutation(
                    &transaction_id,
                    &table_id,
                    &user_id,
                    "3",
                    row(3, "gamma"),
                )],
            };
            let duplicate_bytes =
                kalamdb_raft::codec::command_codec::encode_raft_command(&duplicate)
                    .expect("encode duplicate replay");
            let duplicate_result =
                state_machine.apply(index, 1, &duplicate_bytes).await.expect("duplicate replay");
            assert!(
                matches!(duplicate_result, kalamdb_raft::ApplyResult::NoOp),
                "replaying the same index must not apply again: {duplicate_result:?}"
            );
            assert!(!user_pk_present(provider.as_ref(), &user_id, 3).await);
            assert_eq!(batches.batches_touching(&partition) - batches_before, 1);
        },
    }
}

#[tokio::test]
#[ntest::timeout(12_000)]
async fn replay_of_applied_commit_does_not_duplicate_rows() {
    let (app_ctx, _test_db, _batches) = create_counting_cluster_app_context().await;
    let user_id = UserId::new("tx-replay-once");
    let table_id = create_user_table(&app_ctx, &unique_namespace("tx_replay_once"), "items").await;
    let provider = app_ctx.schema_registry().get_kalam_provider(&table_id).expect("user provider");

    let coordinator = app_ctx.transaction_coordinator();
    let transaction_id = coordinator
        .begin(
            ExecutionOwnerKey::internal(43),
            "replay-once".to_string().into(),
            TransactionOrigin::Internal,
        )
        .expect("begin");
    coordinator
        .stage(
            &transaction_id,
            insert_mutation(&transaction_id, &table_id, &user_id, "1", row(1, "alpha")),
        )
        .expect("stage");
    coordinator.commit(&transaction_id).await.expect("commit");
    assert!(user_pk_present(provider.as_ref(), &user_id, 1).await);

    let router = ShardRouter::from_optional_cluster_config(app_ctx.config().cluster.as_ref());
    let shard = router.user_shard_id(&user_id);
    let executor = app_ctx.executor();
    let raft = executor.as_any().downcast_ref::<RaftExecutor>().expect("raft executor");
    let state_machine = raft.manager().user_data_state_machine(shard).expect("user state machine");
    let applied = state_machine.last_applied_index();
    let replay = RaftCommand::TransactionCommit {
        required_meta_index: 0,
        transaction_id: transaction_id.clone(),
        mutations: vec![insert_mutation(
            &transaction_id,
            &table_id,
            &user_id,
            "3",
            row(3, "gamma"),
        )],
    };
    let bytes =
        kalamdb_raft::codec::command_codec::encode_raft_command(&replay).expect("encode replay");
    let result = state_machine
        .apply(applied, state_machine.last_applied_term(), &bytes)
        .await
        .expect("replay apply");
    assert!(
        matches!(result, kalamdb_raft::ApplyResult::NoOp),
        "an applied index must not run again: {result:?}"
    );
    assert!(user_pk_present(provider.as_ref(), &user_id, 1).await);
    assert!(
        !user_pk_present(provider.as_ref(), &user_id, 3).await,
        "replaying the applied commit added a row"
    );
}

#[tokio::test]
#[ntest::timeout(12_000)]
async fn cross_domain_commit_is_rejected_before_a_storage_batch() {
    let (app_ctx, _test_db, batches) = create_counting_cluster_app_context().await;
    let router = ShardRouter::from_optional_cluster_config(app_ctx.config().cluster.as_ref());
    let first_user = UserId::new("tx-cross-domain-a");
    let second_user = other_shard_user(&router, &first_user);
    let table_id = create_user_table(&app_ctx, &unique_namespace("tx_cross_domain"), "items").await;
    let provider = app_ctx.schema_registry().get_kalam_provider(&table_id).expect("user provider");
    let partition = provider.user_store().expect("user store").partition().name().to_string();
    let batches_before = batches.batches_touching(&partition);

    let executor = app_ctx.executor();
    let raft = executor.as_any().downcast_ref::<RaftExecutor>().expect("raft executor");
    let state_machine = raft
        .manager()
        .user_data_state_machine(router.user_shard_id(&first_user))
        .expect("user state machine");
    let transaction_id =
        kalamdb_commons::models::TransactionId::new("01960f7b-3d15-7d6d-b26c-7e4db6f25f8d");
    let command = RaftCommand::TransactionCommit {
        required_meta_index: 0,
        transaction_id: transaction_id.clone(),
        mutations: vec![
            insert_mutation(&transaction_id, &table_id, &first_user, "1", row(1, "alpha")),
            insert_mutation(&transaction_id, &table_id, &second_user, "1", row(1, "beta")),
        ],
    };
    let bytes =
        kalamdb_raft::codec::command_codec::encode_raft_command(&command).expect("encode commit");
    let result = state_machine
        .apply(state_machine.last_applied_index() + 1, 1, &bytes)
        .await
        .expect("apply cross-domain commit");
    let kalamdb_raft::ApplyResult::Ok(data) = result else {
        panic!("cross-domain apply should return the admission error, got {result:?}");
    };
    let response =
        kalamdb_raft::codec::command_codec::decode_data_response(&data).expect("decode response");
    match response {
        DataResponse::Error { message } => {
            assert!(message.contains("one data raft group"), "{message}");
        },
        other => panic!("expected an admission error, got {other:?}"),
    }
    assert_eq!(
        batches.batches_touching(&partition),
        batches_before,
        "a cross-domain commit must not submit a storage batch"
    );
    assert!(!user_pk_present(provider.as_ref(), &first_user, 1).await);
    assert!(!user_pk_present(provider.as_ref(), &second_user, 1).await);
}
