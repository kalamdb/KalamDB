//! Shared-table FORCE RLS e2e over the real HTTP SQL API.
//!
//! Covers role-targeted policies, authorization-set caching, membership revocation,
//! PostgreSQL-style NULL/NOT semantics, query-shape bypass attempts, and multi-subscriber
//! conversation membership isolation on SELECT + live fan-out.

use std::time::{Duration, Instant};

use kalam_client::models::{ChangeEvent, ResponseStatus};
use kalam_client::{KalamCellValue, SubscriptionManager};
use kalamdb_commons::{Role, TableId};
use kalamdb_rls::AuthorizationCacheMetrics;
use kalamdb_tables::SharedTableProvider;
use tokio::time::timeout;

use super::test_support::{
    auth_helper::create_user_auth_header,
    consolidated_helpers::{ensure_user_exists, unique_namespace, unique_table},
    query_helpers::assert_query_success,
};

const USER_PASSWORD: &str = "RlsE2ePass123!";

async fn seed_membership_fixture(
    server: &super::test_support::http_server::HttpTestServer,
    ns: &str,
) -> anyhow::Result<(String, String)> {
    let messages = "rls_messages";
    let members = "rls_members";

    for sql in [
        format!("CREATE NAMESPACE IF NOT EXISTS {ns}"),
        format!(
            "CREATE TABLE {ns}.{messages} (id TEXT PRIMARY KEY, group_id TEXT NOT NULL) WITH \
             (TYPE='SHARED')"
        ),
        format!(
            "CREATE TABLE {ns}.{members} (id TEXT PRIMARY KEY, user_id TEXT NOT NULL, group_id \
             TEXT NOT NULL, status TEXT NOT NULL) WITH (TYPE='SHARED')"
        ),
    ] {
        let resp = server.execute_sql(&sql).await?;
        anyhow::ensure!(
            resp.status == ResponseStatus::Success,
            "setup failed for `{sql}`: {:?}",
            resp.error
        );
    }

    for sql in [
        format!(
            "INSERT INTO {ns}.{members} (id, user_id, group_id, status) VALUES ('m1', 'alice', \
             'group-a', 'active')"
        ),
        format!(
            "INSERT INTO {ns}.{messages} (id, group_id) VALUES ('msg-a', 'group-a'), ('msg-b', \
             'group-b')"
        ),
        format!(
            "CREATE POLICY member_read ON {ns}.{messages} FOR SELECT TO user USING (group_id IN \
             (SELECT group_id FROM {ns}.{members} WHERE user_id = CURRENT_USER AND status <> \
             'blocked'))"
        ),
    ] {
        let resp = server.execute_sql(&sql).await?;
        anyhow::ensure!(
            resp.status == ResponseStatus::Success,
            "seed failed for `{sql}`: {:?}",
            resp.error
        );
    }

    Ok((messages.to_string(), members.to_string()))
}

fn row_count(response: &kalam_client::models::QueryResponse) -> usize {
    response.row_count()
}

fn authorization_cache_metrics(
    server: &super::test_support::http_server::HttpTestServer,
    ns: &str,
    table: &str,
) -> AuthorizationCacheMetrics {
    let table_id = TableId::from_strings(ns, table);
    let provider = server
        .app_context()
        .schema_registry()
        .get_provider(&table_id)
        .expect("shared table provider");
    let provider = (provider.as_ref() as &dyn std::any::Any)
        .downcast_ref::<SharedTableProvider>()
        .expect("shared table provider downcast");
    provider.authorization_cache_metrics()
}

#[tokio::test]
#[ntest::timeout(90_000)]
async fn test_rls_role_targets_user_service_and_public() -> anyhow::Result<()> {
    let _guard = super::test_support::http_server::acquire_test_lock().await;
    let server = super::test_support::http_server::get_global_server().await;
    let ns = unique_namespace("rls_roles");
    let table = unique_table("role_docs");
    let full = format!("{ns}.{table}");

    let user_auth =
        create_user_auth_header(&server, "rls_role_user", USER_PASSWORD, &Role::User).await?;
    let service_auth =
        create_user_auth_header(&server, "rls_role_svc", USER_PASSWORD, &Role::Service).await?;
    let dba_auth =
        create_user_auth_header(&server, "rls_role_dba", USER_PASSWORD, &Role::Dba).await?;

    for sql in [
        format!("CREATE NAMESPACE IF NOT EXISTS {ns}"),
        format!(
            "CREATE TABLE {full} (id TEXT PRIMARY KEY, owner_id TEXT NOT NULL, label TEXT) WITH \
             (TYPE='SHARED')"
        ),
        format!(
            "INSERT INTO {full} (id, owner_id, label) VALUES ('u1', 'rls_role_user', 'user-row'), \
             ('s1', 'rls_role_svc', 'service-row')"
        ),
        format!(
            "CREATE POLICY user_only ON {full} FOR SELECT TO user USING (owner_id = CURRENT_USER)"
        ),
    ] {
        let resp = server.execute_sql(&sql).await?;
        anyhow::ensure!(resp.status == ResponseStatus::Success, "{sql}: {:?}", resp.error);
    }

    let user_rows = server
        .execute_sql_with_auth(&format!("SELECT id FROM {full} ORDER BY id"), &user_auth)
        .await?;
    assert_query_success(&user_rows, "user select");
    assert_eq!(row_count(&user_rows), 1);

    let service_rows = server
        .execute_sql_with_auth(&format!("SELECT id FROM {full}"), &service_auth)
        .await?;
    assert_query_success(&service_rows, "service select");
    assert_eq!(row_count(&service_rows), 0, "TO user policy must not apply to service role");

    let dba_rows = server
        .execute_sql_with_auth(&format!("SELECT id FROM {full}"), &dba_auth)
        .await?;
    assert_query_success(&dba_rows, "dba select");
    assert_eq!(
        row_count(&dba_rows),
        2,
        "DBA must bypass FORCE RLS even without a matching policy target"
    );

    let public_policy = server
        .execute_sql(&format!(
            "CREATE POLICY service_read ON {full} FOR SELECT TO service USING (owner_id = \
             CURRENT_USER)"
        ))
        .await?;
    assert_query_success(&public_policy, "create service policy");

    let service_granted = server
        .execute_sql_with_auth(&format!("SELECT id FROM {full}"), &service_auth)
        .await?;
    assert_eq!(row_count(&service_granted), 1);

    let public_all = server
        .execute_sql(&format!(
            "CREATE POLICY public_read ON {full} FOR SELECT TO PUBLIC USING (label = 'user-row')"
        ))
        .await?;
    assert_query_success(&public_all, "create public policy");

    let service_public = server
        .execute_sql_with_auth(&format!("SELECT id FROM {full} ORDER BY id"), &service_auth)
        .await?;
    assert_eq!(
        row_count(&service_public),
        2,
        "PUBLIC policy must combine permissively with service-targeted policy"
    );

    let _ = server.execute_sql(&format!("DROP NAMESPACE IF EXISTS {ns} CASCADE")).await;
    Ok(())
}

#[tokio::test]
#[ntest::timeout(90_000)]
async fn test_rls_membership_authorization_cache_warms_on_repeat_select() -> anyhow::Result<()> {
    let _guard = super::test_support::http_server::acquire_test_lock().await;
    let server = super::test_support::http_server::get_global_server().await;
    let ns = unique_namespace("rls_cache");
    let (messages, _) = seed_membership_fixture(&server, &ns).await?;
    let full = format!("{ns}.{messages}");

    ensure_user_exists(&server, "alice", USER_PASSWORD, &Role::User).await?;
    let alice_auth = create_user_auth_header(&server, "alice", USER_PASSWORD, &Role::User).await?;

    let select_sql = format!("SELECT id FROM {full} ORDER BY id");
    let metrics_before = authorization_cache_metrics(&server, &ns, &messages);
    let first = server.execute_sql_with_auth(&select_sql, &alice_auth).await?;
    assert_query_success(&first, "first membership select");
    assert_eq!(row_count(&first), 1);
    let metrics_after_first = authorization_cache_metrics(&server, &ns, &messages);
    assert!(
        metrics_after_first.misses > metrics_before.misses,
        "first membership select must populate the authorization cache"
    );

    let second = server.execute_sql_with_auth(&select_sql, &alice_auth).await?;
    assert_query_success(&second, "second membership select");
    assert_eq!(row_count(&second), 1);
    let metrics_after_second = authorization_cache_metrics(&server, &ns, &messages);
    assert!(
        metrics_after_second.hits > metrics_after_first.hits,
        "second membership select must hit the authorization cache"
    );

    let _ = server.execute_sql(&format!("DROP NAMESPACE IF EXISTS {ns} CASCADE")).await;
    Ok(())
}

#[tokio::test]
#[ntest::timeout(90_000)]
async fn test_rls_repeat_membership_scan_performance_stays_bounded() -> anyhow::Result<()> {
    let _guard = super::test_support::http_server::acquire_test_lock().await;
    let server = super::test_support::http_server::get_global_server().await;
    let ns = unique_namespace("rls_perf");
    let (messages, _) = seed_membership_fixture(&server, &ns).await?;
    let full = format!("{ns}.{messages}");

    ensure_user_exists(&server, "alice", USER_PASSWORD, &Role::User).await?;
    let alice_auth = create_user_auth_header(&server, "alice", USER_PASSWORD, &Role::User).await?;
    let select_sql = format!("SELECT id FROM {full}");

    let metrics_before = authorization_cache_metrics(&server, &ns, &messages);
    let cold_start = Instant::now();
    let cold = server.execute_sql_with_auth(&select_sql, &alice_auth).await?;
    let cold_elapsed = cold_start.elapsed();
    assert_query_success(&cold, "cold membership select");
    assert_eq!(row_count(&cold), 1);

    let warm_start = Instant::now();
    for iteration in 0..12 {
        let resp = server.execute_sql_with_auth(&select_sql, &alice_auth).await?;
        assert_query_success(&resp, &format!("warm membership select #{iteration}"));
        assert_eq!(row_count(&resp), 1);
    }
    let warm_elapsed = warm_start.elapsed();
    let warm_per_query = warm_elapsed / 12;

    assert!(
        warm_per_query <= cold_elapsed.mul_f32(3.0).max(Duration::from_millis(250)),
        "cached membership scans should stay near cold latency (cold={cold_elapsed:?}, \
         warm_avg={warm_per_query:?})"
    );

    let metrics_after = authorization_cache_metrics(&server, &ns, &messages);
    assert!(
        metrics_after.hits > metrics_before.hits,
        "warmed membership scans must record authorization cache hits"
    );

    let _ = server.execute_sql(&format!("DROP NAMESPACE IF EXISTS {ns} CASCADE")).await;
    Ok(())
}

#[tokio::test]
#[ntest::timeout(90_000)]
async fn test_rls_membership_revoke_invalidates_authorization_cache() -> anyhow::Result<()> {
    let _guard = super::test_support::http_server::acquire_test_lock().await;
    let server = super::test_support::http_server::get_global_server().await;
    let ns = unique_namespace("rls_revoke");
    let (messages, members) = seed_membership_fixture(&server, &ns).await?;
    let messages_full = format!("{ns}.{messages}");
    let members_full = format!("{ns}.{members}");

    ensure_user_exists(&server, "alice", USER_PASSWORD, &Role::User).await?;
    let alice_auth = create_user_auth_header(&server, "alice", USER_PASSWORD, &Role::User).await?;
    let select_sql = format!("SELECT id FROM {messages_full}");

    let metrics_before = authorization_cache_metrics(&server, &ns, &messages);
    let visible = server.execute_sql_with_auth(&select_sql, &alice_auth).await?;
    assert_eq!(row_count(&visible), 1);

    let _ = server.execute_sql_with_auth(&select_sql, &alice_auth).await?;
    let metrics_warmed = authorization_cache_metrics(&server, &ns, &messages);
    assert!(metrics_warmed.hits > metrics_before.hits);

    let revoke = server
        .execute_sql(&format!("DELETE FROM {members_full} WHERE id = 'm1'"))
        .await?;
    assert_query_success(&revoke, "delete membership");

    let hidden = server.execute_sql_with_auth(&select_sql, &alice_auth).await?;
    assert_eq!(row_count(&hidden), 0, "membership revocation must hide previously visible rows");

    let metrics_after_revoke = authorization_cache_metrics(&server, &ns, &messages);
    assert!(
        metrics_after_revoke.misses > metrics_warmed.misses,
        "revoked membership must rebuild authorization state"
    );

    let _ = server.execute_sql(&format!("DROP NAMESPACE IF EXISTS {ns} CASCADE")).await;
    Ok(())
}

#[tokio::test]
#[ntest::timeout(90_000)]
async fn test_rls_null_owner_hidden_under_not_policy() -> anyhow::Result<()> {
    let _guard = super::test_support::http_server::acquire_test_lock().await;
    let server = super::test_support::http_server::get_global_server().await;
    let ns = unique_namespace("rls_null");
    let table = unique_table("null_owner");
    let full = format!("{ns}.{table}");

    ensure_user_exists(&server, "alice", USER_PASSWORD, &Role::User).await?;
    let alice_auth = create_user_auth_header(&server, "alice", USER_PASSWORD, &Role::User).await?;

    for sql in [
        format!("CREATE NAMESPACE IF NOT EXISTS {ns}"),
        format!("CREATE TABLE {full} (id TEXT PRIMARY KEY, owner_id TEXT) WITH (TYPE='SHARED')"),
        format!("INSERT INTO {full} (id, owner_id) VALUES ('null-row', NULL), ('bob-row', 'bob')"),
        format!(
            "CREATE POLICY not_owner ON {full} FOR SELECT TO user USING (NOT (owner_id = \
             CURRENT_USER))"
        ),
    ] {
        let resp = server.execute_sql(&sql).await?;
        anyhow::ensure!(resp.status == ResponseStatus::Success, "{sql}: {:?}", resp.error);
    }

    let visible = server
        .execute_sql_with_auth(&format!("SELECT id FROM {full} ORDER BY id"), &alice_auth)
        .await?;
    assert_query_success(&visible, "NOT policy select");
    assert_eq!(
        row_count(&visible),
        1,
        "NULL = CURRENT_USER is unknown, so NOT(...) must not leak the null-owner row"
    );

    let _ = server.execute_sql(&format!("DROP NAMESPACE IF EXISTS {ns} CASCADE")).await;
    Ok(())
}

#[tokio::test]
#[ntest::timeout(90_000)]
async fn test_rls_union_and_except_cannot_bypass_membership() -> anyhow::Result<()> {
    let _guard = super::test_support::http_server::acquire_test_lock().await;
    let server = super::test_support::http_server::get_global_server().await;
    let ns = unique_namespace("rls_union");
    let (messages, _) = seed_membership_fixture(&server, &ns).await?;
    let full = format!("{ns}.{messages}");

    ensure_user_exists(&server, "alice", USER_PASSWORD, &Role::User).await?;
    let alice_auth = create_user_auth_header(&server, "alice", USER_PASSWORD, &Role::User).await?;

    let union_sql =
        format!("SELECT id FROM {full} WHERE group_id = 'group-b' UNION SELECT id FROM {full}");
    let union_rows = server.execute_sql_with_auth(&union_sql, &alice_auth).await?;
    assert_eq!(row_count(&union_rows), 1);

    let except_sql =
        format!("SELECT id FROM {full} EXCEPT SELECT id FROM {full} WHERE group_id = 'group-a'");
    let except_rows = server.execute_sql_with_auth(&except_sql, &alice_auth).await?;
    assert_eq!(
        row_count(&except_rows),
        0,
        "EXCEPT must not surface rows outside membership authorization"
    );

    let _ = server.execute_sql(&format!("DROP NAMESPACE IF EXISTS {ns} CASCADE")).await;
    Ok(())
}

#[tokio::test]
#[ntest::timeout(90_000)]
async fn test_rls_plan_cache_stays_isolated_across_roles_and_principals() -> anyhow::Result<()> {
    let _guard = super::test_support::http_server::acquire_test_lock().await;
    let server = super::test_support::http_server::get_global_server().await;
    let ns = unique_namespace("rls_plan");
    let table = unique_table("plan_docs");
    let full = format!("{ns}.{table}");

    for sql in [
        format!("CREATE NAMESPACE IF NOT EXISTS {ns}"),
        format!(
            "CREATE TABLE {full} (id TEXT PRIMARY KEY, owner_id TEXT NOT NULL) WITH \
             (TYPE='SHARED')"
        ),
        format!("INSERT INTO {full} (id, owner_id) VALUES ('a', 'alice'), ('b', 'bob')"),
        format!(
            "CREATE POLICY owner_read ON {full} FOR SELECT TO user USING (owner_id = CURRENT_USER)"
        ),
    ] {
        let resp = server.execute_sql(&sql).await?;
        anyhow::ensure!(resp.status == ResponseStatus::Success, "{sql}: {:?}", resp.error);
    }

    for (name, role) in [("alice", Role::User), ("bob", Role::User)] {
        ensure_user_exists(&server, name, USER_PASSWORD, &role).await?;
    }
    let alice_auth = create_user_auth_header(&server, "alice", USER_PASSWORD, &Role::User).await?;
    let bob_auth = create_user_auth_header(&server, "bob", USER_PASSWORD, &Role::User).await?;
    let sql = format!("SELECT id FROM {full}");

    let alice_first = server.execute_sql_with_auth(&sql, &alice_auth).await?;
    let bob_rows = server.execute_sql_with_auth(&sql, &bob_auth).await?;
    let alice_second = server.execute_sql_with_auth(&sql, &alice_auth).await?;

    assert_eq!(row_count(&alice_first), 1);
    assert_eq!(row_count(&bob_rows), 1);
    assert_eq!(row_count(&alice_second), 1);
    let alice_ids: Vec<String> = alice_first
        .rows_as_maps()
        .into_iter()
        .filter_map(|row| row.get("id").and_then(|v| v.as_text()).map(str::to_string))
        .collect();
    let bob_ids: Vec<String> = bob_rows
        .rows_as_maps()
        .into_iter()
        .filter_map(|row| row.get("id").and_then(|v| v.as_text()).map(str::to_string))
        .collect();
    assert_ne!(
        alice_ids, bob_ids,
        "plan cache must bind CURRENT_USER independently per principal"
    );

    let _ = server.execute_sql(&format!("DROP NAMESPACE IF EXISTS {ns} CASCADE")).await;
    Ok(())
}

/// Ten subscribers, each authorized for a distinct conversation_id via membership RLS.
/// Asserts SELECT isolation and that live fan-out delivers only authorized conversation events.
#[tokio::test]
#[ntest::timeout(120_000)]
async fn test_rls_multi_subscriber_conversation_membership_isolation() -> anyhow::Result<()> {
    let _guard = super::test_support::http_server::acquire_test_lock().await;
    let server = super::test_support::http_server::get_global_server().await;
    let ns = unique_namespace("rls_conv_fanout");
    let messages = unique_table("messages");
    let members = unique_table("members");
    let messages_full = format!("{ns}.{messages}");
    let members_full = format!("{ns}.{members}");
    const SUBSCRIBER_COUNT: usize = 10;

    for sql in [
        format!("CREATE NAMESPACE IF NOT EXISTS {ns}"),
        format!(
            "CREATE TABLE {messages_full} (
                id TEXT PRIMARY KEY,
                conversation_id TEXT NOT NULL,
                body TEXT NOT NULL
            ) WITH (TYPE='SHARED')"
        ),
        format!(
            "CREATE TABLE {members_full} (
                id TEXT PRIMARY KEY,
                user_id TEXT NOT NULL,
                conversation_id TEXT NOT NULL
            ) WITH (TYPE='SHARED')"
        ),
    ] {
        let resp = server.execute_sql(&sql).await?;
        anyhow::ensure!(resp.status == ResponseStatus::Success, "{sql}: {:?}", resp.error);
    }

    let mut users = Vec::with_capacity(SUBSCRIBER_COUNT);
    for i in 0..SUBSCRIBER_COUNT {
        let user = format!("{ns}_u{i}");
        ensure_user_exists(&server, &user, USER_PASSWORD, &Role::User).await?;
        let conversation_id = format!("conv-{i}");
        let member_sql = format!(
            "INSERT INTO {members_full} (id, user_id, conversation_id) VALUES ('m{i}', '{user}', \
             '{conversation_id}')"
        );
        let resp = server.execute_sql(&member_sql).await?;
        anyhow::ensure!(
            resp.status == ResponseStatus::Success,
            "{member_sql}: {:?}",
            resp.error
        );
        users.push((user, conversation_id));
    }

    let policy = server
        .execute_sql(&format!(
            "CREATE POLICY conversation_member_read ON {messages_full} FOR SELECT TO user USING \
             (conversation_id IN (SELECT conversation_id FROM {members_full} WHERE user_id = \
             CURRENT_USER))"
        ))
        .await?;
    assert_query_success(&policy, "create conversation membership policy");

    // Seed one historical row per conversation and verify SELECT isolation.
    for (i, (user, conversation_id)) in users.iter().enumerate() {
        let seed = server
            .execute_sql(&format!(
                "INSERT INTO {messages_full} (id, conversation_id, body) VALUES ('seed-{i}', \
                 '{conversation_id}', 'seed-for-{conversation_id}')"
            ))
            .await?;
        assert_query_success(&seed, "seed conversation row");

        let auth = create_user_auth_header(&server, user, USER_PASSWORD, &Role::User).await?;
        let visible = server
            .execute_sql_with_auth(
                &format!("SELECT id, conversation_id, body FROM {messages_full}"),
                &auth,
            )
            .await?;
        assert_query_success(&visible, "membership select");
        assert_eq!(
            row_count(&visible),
            1,
            "{user} must see exactly their conversation row"
        );
        let body = visible
            .rows_as_maps()
            .into_iter()
            .filter_map(|row| row.get("body").and_then(|v| v.as_text()).map(str::to_string))
            .collect::<Vec<_>>();
        assert_eq!(body, vec![format!("seed-for-{conversation_id}")]);
    }

    // Subscribe all principals, then insert one live row per conversation.
    // Keep clients alive for the full fan-out window — dropping the client closes the socket.
    let mut clients = Vec::with_capacity(SUBSCRIBER_COUNT);
    let mut subscriptions = Vec::with_capacity(SUBSCRIBER_COUNT);
    for (user, _) in &users {
        let client = server.link_client(user);
        let mut subscription = client
            .live_events(&format!(
                "SELECT id, conversation_id, body FROM {messages_full}"
            ))
            .await
            .expect("membership live subscribe");
        drain_subscription_snapshot(&mut subscription, Duration::from_secs(5)).await?;
        clients.push(client);
        subscriptions.push(subscription);
    }
    let _clients = clients;

    for (i, (_, conversation_id)) in users.iter().enumerate() {
        let insert = server
            .execute_sql(&format!(
                "INSERT INTO {messages_full} (id, conversation_id, body) VALUES ('live-{i}', \
                 '{conversation_id}', 'live-for-{conversation_id}')"
            ))
            .await?;
        assert_query_success(&insert, "live conversation insert");
    }

    for (i, ((user, conversation_id), subscription)) in
        users.iter().zip(subscriptions.iter_mut()).enumerate()
    {
        let expected = format!("live-for-{conversation_id}");
        let foreign = format!("live-for-conv-{}", (i + 1) % SUBSCRIBER_COUNT);
        assert!(
            wait_for_insert_body(subscription, &expected, Duration::from_secs(8)).await?,
            "{user} must receive live event for {conversation_id}"
        );
        assert!(
            !wait_for_insert_body(subscription, &foreign, Duration::from_secs(1)).await?,
            "{user} must not receive foreign conversation live event {foreign}"
        );
    }

    let _ = server.execute_sql(&format!("DROP NAMESPACE IF EXISTS {ns} CASCADE")).await;
    Ok(())
}

async fn drain_subscription_snapshot(
    subscription: &mut SubscriptionManager,
    timeout_duration: Duration,
) -> anyhow::Result<()> {
    let deadline = Instant::now() + timeout_duration;
    while Instant::now() < deadline {
        match timeout(Duration::from_millis(100), subscription.next()).await {
            Ok(Some(Ok(ChangeEvent::Ack { .. })))
            | Ok(Some(Ok(ChangeEvent::InitialDataBatch { .. }))) => {},
            Ok(Some(Ok(_))) => return Ok(()),
            Ok(Some(Err(error))) => return Err(anyhow::anyhow!("subscription error: {error:?}")),
            Ok(None) => return Ok(()),
            Err(_) => return Ok(()),
        }
    }
    Ok(())
}

async fn wait_for_insert_body(
    subscription: &mut SubscriptionManager,
    needle: &str,
    timeout_duration: Duration,
) -> anyhow::Result<bool> {
    let deadline = Instant::now() + timeout_duration;
    while Instant::now() < deadline {
        match timeout(Duration::from_millis(100), subscription.next()).await {
            Ok(Some(Ok(ChangeEvent::Insert { rows, .. }))) => {
                let hit = rows.iter().any(|row| {
                    row.values().any(|value| cell_contains(value, needle))
                        || format!("{row:?}").contains(needle)
                });
                if hit {
                    return Ok(true);
                }
            },
            Ok(Some(Ok(_))) | Err(_) => {},
            Ok(Some(Err(error))) => return Err(anyhow::anyhow!("subscription error: {error:?}")),
            Ok(None) => return Ok(false),
        }
    }
    Ok(false)
}

fn cell_contains(value: &KalamCellValue, needle: &str) -> bool {
    value.as_text().is_some_and(|text| text.contains(needle))
        || format!("{value}").contains(needle)
}
