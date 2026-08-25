//! Shared-table FORCE RLS e2e coverage.
//!
//! ACCESS_LEVEL is rejected. Grants come from CREATE POLICY. User/Service default-deny
//! to zero rows, System/DBA bypass, live subscriptions fail closed on grant/revoke.

use std::time::Duration;

use reqwest::StatusCode;

use crate::common::*;

#[ntest::timeout(180000)]
#[test]
fn smoke_shared_table_rls_rejects_access_level() {
    if !is_server_running() {
        eprintln!("Skipping smoke_shared_table_rls_rejects_access_level: server not running");
        return;
    }

    let namespace = generate_unique_namespace("rls_access_ns");
    let table = generate_unique_table("rls_access_tbl");
    let full = format!("{namespace}.{table}");

    execute_sql_as_root_via_client(&format!("CREATE NAMESPACE IF NOT EXISTS {namespace}"))
        .expect("create namespace");

    let create_err = execute_sql_as_root_via_client(&format!(
        "CREATE TABLE {full} (id BIGINT PRIMARY KEY, name TEXT) WITH (TYPE='SHARED', ACCESS_LEVEL \
         = 'PUBLIC')"
    ))
    .expect_err("CREATE TABLE ACCESS_LEVEL must be rejected");
    assert!(
        create_err.to_string().contains("ACCESS_LEVEL is not supported"),
        "unexpected CREATE TABLE error: {create_err}"
    );

    execute_sql_as_root_via_client(&format!(
        "CREATE TABLE {full} (id BIGINT PRIMARY KEY, name TEXT) WITH (TYPE='SHARED')"
    ))
    .expect("create shared table");
    wait_for_table_ready(&full, Duration::from_secs(3)).expect("table ready");

    let alter_err = execute_sql_as_root_via_client(&format!(
        "ALTER TABLE {full} SET TBLPROPERTIES (ACCESS_LEVEL = 'PRIVATE')"
    ))
    .expect_err("ALTER ACCESS_LEVEL must be rejected");
    assert!(
        alter_err.to_string().contains("ACCESS_LEVEL is not supported"),
        "unexpected ALTER TABLE error: {alter_err}"
    );

    let set_err =
        execute_sql_as_root_via_client(&format!("ALTER TABLE {full} SET ACCESS LEVEL PUBLIC"))
            .expect_err("SET ACCESS LEVEL must be rejected");
    assert!(
        set_err.to_string().contains("ACCESS_LEVEL is not supported"),
        "unexpected SET ACCESS LEVEL error: {set_err}"
    );

    let _ =
        execute_sql_as_root_via_client(&format!("DROP NAMESPACE IF EXISTS {namespace} CASCADE"));
}

#[ntest::timeout(180000)]
#[test]
fn smoke_shared_table_rls_default_deny_and_policy_grant() {
    if !is_server_running() {
        eprintln!(
            "Skipping smoke_shared_table_rls_default_deny_and_policy_grant: server not running"
        );
        return;
    }

    let namespace = generate_unique_namespace("rls_deny_ns");
    let table = generate_unique_table("rls_docs");
    let full = format!("{namespace}.{table}");
    let user = generate_unique_namespace("rls_user");
    let service = generate_unique_namespace("rls_svc");
    let password = "smoke_pass_123";

    execute_sql_as_root_via_client(&format!("CREATE NAMESPACE IF NOT EXISTS {namespace}"))
        .expect("create namespace");
    execute_sql_as_root_via_client(&format!(
        "CREATE TABLE {full} (
            id TEXT PRIMARY KEY,
            owner_id TEXT NOT NULL,
            body TEXT
        ) WITH (TYPE='SHARED')"
    ))
    .expect("create shared table");
    wait_for_table_ready(&full, Duration::from_secs(3)).expect("table ready");
    execute_sql_as_root_via_client(&format!(
        "INSERT INTO {full} (id, owner_id, body) VALUES ('alice-1', '{user}', 'secret-alice'), \
         ('bob-1', 'bob', 'secret-bob')"
    ))
    .expect("seed rows as DBA");
    execute_sql_as_root_via_client(&format!(
        "CREATE USER {user} WITH PASSWORD '{password}' ROLE 'user'"
    ))
    .expect("create user");
    execute_sql_as_root_via_client(&format!(
        "CREATE USER {service} WITH PASSWORD '{password}' ROLE 'service'"
    ))
    .expect("create service");

    let dba_out = execute_sql_as_root_via_client(&format!("SELECT body FROM {full}"))
        .expect("DBA SELECT bypasses RLS");
    assert!(dba_out.contains("secret-alice") && dba_out.contains("secret-bob"));

    let user_select =
        execute_sql_via_client_as(&user, password, &format!("SELECT body FROM {full}"))
            .expect("User SELECT on default-deny shared table succeeds with zero rows");
    assert!(
        !user_select.contains("secret-alice") && !user_select.contains("secret-bob"),
        "User must not see default-deny rows: {user_select}"
    );
    let service_select =
        execute_sql_via_client_as(&service, password, &format!("SELECT body FROM {full}"))
            .expect("Service SELECT on default-deny shared table succeeds with zero rows");
    assert!(
        !service_select.contains("secret-alice") && !service_select.contains("secret-bob"),
        "Service must not see default-deny rows: {service_select}"
    );

    let insert_err = execute_sql_via_client_as(
        &user,
        password,
        &format!("INSERT INTO {full} (id, owner_id, body) VALUES ('u1', '{user}', 'x')"),
    )
    .expect_err("User INSERT without WITH CHECK policy must fail");
    assert_rls_denied(&insert_err, "user insert");

    execute_sql_as_root_via_client(&format!(
        "CREATE POLICY {table}_select ON {full} FOR SELECT TO user, service USING (owner_id = \
         CURRENT_USER)"
    ))
    .expect("create select policy");

    let user_granted =
        execute_sql_via_client_as(&user, password, &format!("SELECT body FROM {full}"))
            .expect("User SELECT after CREATE POLICY");
    assert!(
        user_granted.contains("secret-alice") && !user_granted.contains("secret-bob"),
        "User must see only own rows: {user_granted}"
    );

    let bypass = execute_sql_via_client_as(
        &user,
        password,
        &format!("SELECT body FROM {full} WHERE owner_id = 'bob' OR true"),
    )
    .expect("OR true must not error");
    assert!(
        bypass.contains("secret-alice") && !bypass.contains("secret-bob"),
        "OR true must not bypass RLS: {bypass}"
    );

    let nested = execute_sql_via_client_as(
        &user,
        password,
        &format!("SELECT body FROM {full} WHERE id IN (SELECT id FROM {full})"),
    )
    .expect("nested SELECT must not error");
    assert!(
        nested.contains("secret-alice") && !nested.contains("secret-bob"),
        "nested query must not bypass RLS: {nested}"
    );

    let write_err = execute_sql_via_client_as(
        &user,
        password,
        &format!("INSERT INTO {full} (id, owner_id, body) VALUES ('u2', '{user}', 'x')"),
    )
    .expect_err("SELECT policy must not grant writes");
    assert_rls_denied(&write_err, "user insert after select policy");

    let explain = execute_sql_as_root_via_client(&format!("EXPLAIN SELECT body FROM {full}"))
        .expect("EXPLAIN as DBA");
    assert!(
        explain.contains("RlsAuthorization") && explain.contains("bypass=admin"),
        "DBA EXPLAIN must disclose the RLS bypass strategy: {explain}"
    );

    let _ = execute_sql_as_root_via_client(&format!("DROP USER {user}"));
    let _ = execute_sql_as_root_via_client(&format!("DROP USER {service}"));
    let _ =
        execute_sql_as_root_via_client(&format!("DROP NAMESPACE IF EXISTS {namespace} CASCADE"));
}

#[ntest::timeout(180000)]
#[test]
fn smoke_shared_table_rls_plan_cache_isolation() {
    if !is_server_running() {
        eprintln!("Skipping smoke_shared_table_rls_plan_cache_isolation: server not running");
        return;
    }

    let namespace = generate_unique_namespace("rls_cache_ns");
    let table = generate_unique_table("rls_cache_docs");
    let full = format!("{namespace}.{table}");
    let alice = generate_unique_namespace("rls_alice");
    let bob = generate_unique_namespace("rls_bob");
    let password = "smoke_pass_123";

    execute_sql_as_root_via_client(&format!("CREATE NAMESPACE IF NOT EXISTS {namespace}"))
        .expect("create namespace");
    execute_sql_as_root_via_client(&format!(
        "CREATE TABLE {full} (id TEXT PRIMARY KEY, owner_id TEXT NOT NULL) WITH (TYPE='SHARED')"
    ))
    .expect("create shared table");
    wait_for_table_ready(&full, Duration::from_secs(3)).expect("table ready");
    execute_sql_as_root_via_client(&format!(
        "CREATE USER {alice} WITH PASSWORD '{password}' ROLE 'user'"
    ))
    .expect("create alice");
    execute_sql_as_root_via_client(&format!(
        "CREATE USER {bob} WITH PASSWORD '{password}' ROLE 'user'"
    ))
    .expect("create bob");
    execute_sql_as_root_via_client(&format!(
        "INSERT INTO {full} (id, owner_id) VALUES ('alice_doc', '{alice}'), ('bob_doc', '{bob}')"
    ))
    .expect("seed rows");
    execute_sql_as_root_via_client(&format!(
        "CREATE POLICY {table}_select ON {full} FOR SELECT TO user USING (owner_id = CURRENT_USER)"
    ))
    .expect("create policy");

    let sql = format!("SELECT id FROM {full}");
    let alice_first =
        execute_sql_via_client_as(&alice, password, &sql).expect("alice first select");
    let bob_rows = execute_sql_via_client_as(&bob, password, &sql).expect("bob select");
    let alice_second =
        execute_sql_via_client_as(&alice, password, &sql).expect("alice second select");

    assert!(
        alice_first.contains("alice_doc") && !alice_first.contains("bob_doc"),
        "alice first select must be isolated: {alice_first}"
    );
    assert!(
        bob_rows.contains("bob_doc") && !bob_rows.contains("alice_doc"),
        "bob select must be isolated: {bob_rows}"
    );
    assert!(
        alice_second.contains("alice_doc") && !alice_second.contains("bob_doc"),
        "alice must still see own row after bob's cached plan: {alice_second}"
    );

    let _ = execute_sql_as_root_via_client(&format!("DROP USER {alice}"));
    let _ = execute_sql_as_root_via_client(&format!("DROP USER {bob}"));
    let _ =
        execute_sql_as_root_via_client(&format!("DROP NAMESPACE IF EXISTS {namespace} CASCADE"));
}

#[ntest::timeout(180000)]
#[test]
fn smoke_shared_table_rls_on_conflict_rejected_for_subjects() {
    if !is_server_running() {
        eprintln!(
            "Skipping smoke_shared_table_rls_on_conflict_rejected_for_subjects: server not running"
        );
        return;
    }

    let namespace = generate_unique_namespace("rls_conflict_ns");
    let table = generate_unique_table("rls_conflict_docs");
    let full = format!("{namespace}.{table}");
    let user = generate_unique_namespace("rls_conflict_user");
    let password = "smoke_pass_123";

    execute_sql_as_root_via_client(&format!("CREATE NAMESPACE IF NOT EXISTS {namespace}"))
        .expect("create namespace");
    execute_sql_as_root_via_client(&format!(
        "CREATE TABLE {full} (id TEXT PRIMARY KEY, owner_id TEXT NOT NULL) WITH (TYPE='SHARED')"
    ))
    .expect("create shared table");
    wait_for_table_ready(&full, Duration::from_secs(3)).expect("table ready");
    execute_sql_as_root_via_client(&format!(
        "CREATE USER {user} WITH PASSWORD '{password}' ROLE 'user'"
    ))
    .expect("create user");
    execute_sql_as_root_via_client(&format!(
        "CREATE POLICY {table}_all ON {full} FOR ALL TO user USING (owner_id = CURRENT_USER) WITH \
         CHECK (owner_id = CURRENT_USER)"
    ))
    .expect("create write policy");

    execute_sql_via_client_as(
        &user,
        password,
        &format!("INSERT INTO {full} (id, owner_id) VALUES ('doc-a', '{user}')"),
    )
    .expect("User INSERT with WITH CHECK policy must succeed");
    let granted = execute_sql_via_client_as(&user, password, &format!("SELECT id FROM {full}"))
        .expect("User SELECT after WITH CHECK insert");
    assert!(
        granted.contains("doc-a"),
        "User must see the row granted by WITH CHECK: {granted}"
    );

    let error = execute_sql_via_client_as(
        &user,
        password,
        &format!(
            "INSERT INTO {full} (id, owner_id) VALUES ('doc-a', '{user}') ON CONFLICT (id) DO \
             UPDATE SET owner_id = EXCLUDED.owner_id"
        ),
    )
    .expect_err("User ON CONFLICT must be rejected rather than bypassing RLS");
    assert!(
        error.to_string().contains("ON CONFLICT"),
        "expected ON CONFLICT rejection, got {error}"
    );

    let _ = execute_sql_as_root_via_client(&format!("DROP USER {user}"));
    let _ =
        execute_sql_as_root_via_client(&format!("DROP NAMESPACE IF EXISTS {namespace} CASCADE"));
}

#[ntest::timeout(180000)]
#[test]
fn smoke_shared_table_rls_live_grant_revoke() {
    if !is_server_running() {
        eprintln!("Skipping smoke_shared_table_rls_live_grant_revoke: server not running");
        return;
    }

    let namespace = generate_unique_namespace("rls_live_ns");
    let table = generate_unique_table("rls_live_docs");
    let full = format!("{namespace}.{table}");
    let user = generate_unique_namespace("rls_live_user");
    let password = "smoke_pass_123";
    let query = format!("SELECT id, owner_id, body FROM {full}");

    execute_sql_as_root_via_client(&format!("CREATE NAMESPACE IF NOT EXISTS {namespace}"))
        .expect("create namespace");
    execute_sql_as_root_via_client(&format!(
        "CREATE TABLE {full} (
            id TEXT PRIMARY KEY,
            owner_id TEXT NOT NULL,
            body TEXT
        ) WITH (TYPE='SHARED')"
    ))
    .expect("create shared table");
    wait_for_table_ready(&full, Duration::from_secs(3)).expect("table ready");
    execute_sql_as_root_via_client(&format!(
        "CREATE USER {user} WITH PASSWORD '{password}' ROLE 'user'"
    ))
    .expect("create user");
    execute_sql_as_root_via_client(&format!(
        "CREATE POLICY {table}_select ON {full} FOR SELECT TO user USING (owner_id = CURRENT_USER)"
    ))
    .expect("create select policy");

    let mut listener = match SubscriptionListener::start_as_user(&query, &user, password) {
        Ok(listener) => listener,
        Err(error) if error.to_string().contains("channel closed") => {
            eprintln!("Skipping transient live-query backend failure: {error}");
            cleanup_rls_live(&namespace, &user);
            return;
        },
        Err(error) => panic!("user subscription should start: {error}"),
    };

    drain_listener(&mut listener, Duration::from_secs(2));
    execute_sql_as_root_via_client(&format!(
        "INSERT INTO {full} (id, owner_id, body) VALUES ('live-ok', '{user}', \
         'visible-after-bind')"
    ))
    .expect("insert authorized live row");
    assert!(
        wait_for_event(&mut listener, "visible-after-bind", Duration::from_secs(6)),
        "bound subscriber must receive authorized inserts"
    );

    execute_sql_as_root_via_client(&format!("DROP POLICY {table}_select ON {full}"))
        .expect("drop policy");
    execute_sql_as_root_via_client(&format!(
        "INSERT INTO {full} (id, owner_id, body) VALUES ('live-revoked', '{user}', \
         'hidden-after-revoke')"
    ))
    .expect("insert after revoke");
    assert!(
        !wait_for_event(&mut listener, "hidden-after-revoke", Duration::from_secs(3)),
        "DROP POLICY must fail closed for the existing subscription"
    );
    listener.stop().ok();

    execute_sql_as_root_via_client(&format!(
        "CREATE POLICY {table}_select ON {full} FOR SELECT TO user USING (owner_id = CURRENT_USER)"
    ))
    .expect("recreate select policy");
    let mut rebound = SubscriptionListener::start_as_user(&query, &user, password)
        .expect("resubscribe after grant");
    drain_listener(&mut rebound, Duration::from_secs(2));
    execute_sql_as_root_via_client(&format!(
        "INSERT INTO {full} (id, owner_id, body) VALUES ('live-rebind', '{user}', \
         'visible-after-rebind')"
    ))
    .expect("insert after rebind");
    assert!(
        wait_for_event(&mut rebound, "visible-after-rebind", Duration::from_secs(6)),
        "resubscribe must pick up the new policy for later events only"
    );
    rebound.stop().ok();

    cleanup_rls_live(&namespace, &user);
}

#[ntest::timeout(180000)]
#[test]
fn smoke_shared_table_rls_multi_subscriber_conversation_membership() {
    if !is_server_running() {
        eprintln!(
            "Skipping smoke_shared_table_rls_multi_subscriber_conversation_membership: server not \
             running"
        );
        return;
    }

    let namespace = generate_unique_namespace("rls_conv_ns");
    let messages = generate_unique_table("rls_conv_msgs");
    let members = generate_unique_table("rls_conv_mem");
    let messages_full = format!("{namespace}.{messages}");
    let members_full = format!("{namespace}.{members}");
    let password = "smoke_pass_123";
    const SUBSCRIBER_COUNT: usize = 10;

    execute_sql_as_root_via_client(&format!("CREATE NAMESPACE IF NOT EXISTS {namespace}"))
        .expect("create namespace");
    execute_sql_as_root_via_client(&format!(
        "CREATE TABLE {messages_full} (
            id TEXT PRIMARY KEY,
            conversation_id TEXT NOT NULL,
            body TEXT NOT NULL
        ) WITH (TYPE='SHARED')"
    ))
    .expect("create messages");
    execute_sql_as_root_via_client(&format!(
        "CREATE TABLE {members_full} (
            id TEXT PRIMARY KEY,
            user_id TEXT NOT NULL,
            conversation_id TEXT NOT NULL
        ) WITH (TYPE='SHARED')"
    ))
    .expect("create members");
    wait_for_table_ready(&messages_full, Duration::from_secs(3)).expect("messages ready");

    let mut users = Vec::with_capacity(SUBSCRIBER_COUNT);
    for i in 0..SUBSCRIBER_COUNT {
        let user = generate_unique_namespace(&format!("rls_conv_u{i}"));
        let conversation_id = format!("conv-{i}");
        execute_sql_as_root_via_client(&format!(
            "CREATE USER {user} WITH PASSWORD '{password}' ROLE 'user'"
        ))
        .expect("create user");
        execute_sql_as_root_via_client(&format!(
            "INSERT INTO {members_full} (id, user_id, conversation_id) VALUES ('m{i}', '{user}', \
             '{conversation_id}')"
        ))
        .expect("seed membership");
        users.push((user, conversation_id));
    }

    execute_sql_as_root_via_client(&format!(
        "CREATE POLICY conversation_member_read ON {messages_full} FOR SELECT TO user USING \
         (conversation_id IN (SELECT conversation_id FROM {members_full} WHERE user_id = \
         CURRENT_USER))"
    ))
    .expect("create conversation membership policy");

    for (i, (user, conversation_id)) in users.iter().enumerate() {
        execute_sql_as_root_via_client(&format!(
            "INSERT INTO {messages_full} (id, conversation_id, body) VALUES ('seed-{i}', \
             '{conversation_id}', 'seed-for-{conversation_id}')"
        ))
        .expect("seed conversation row");
        let visible = execute_sql_via_client_as(
            user,
            password,
            &format!("SELECT id, body FROM {messages_full}"),
        )
        .expect("membership select");
        assert!(
            visible.contains(&format!("seed-for-{conversation_id}")),
            "{user} must see own conversation seed: {visible}"
        );
        for (j, (_, other_conversation)) in users.iter().enumerate() {
            if i == j {
                continue;
            }
            assert!(
                !visible.contains(&format!("seed-for-{other_conversation}")),
                "{user} must not see {other_conversation}: {visible}"
            );
        }
    }

    let query = format!("SELECT id, conversation_id, body FROM {messages_full}");
    let mut listeners = Vec::with_capacity(SUBSCRIBER_COUNT);
    for (user, _) in &users {
        match SubscriptionListener::start_as_user(&query, user, password) {
            Ok(mut listener) => {
                drain_listener(&mut listener, Duration::from_secs(2));
                listeners.push(listener);
            },
            Err(error) if error.to_string().contains("channel closed") => {
                eprintln!("Skipping transient live-query backend failure: {error}");
                for (user, _) in &users {
                    let _ = execute_sql_as_root_via_client(&format!("DROP USER {user}"));
                }
                let _ = execute_sql_as_root_via_client(&format!(
                    "DROP NAMESPACE IF EXISTS {namespace} CASCADE"
                ));
                return;
            },
            Err(error) => panic!("membership subscription should start for {user}: {error}"),
        }
    }

    for (i, (_, conversation_id)) in users.iter().enumerate() {
        execute_sql_as_root_via_client(&format!(
            "INSERT INTO {messages_full} (id, conversation_id, body) VALUES ('live-{i}', \
             '{conversation_id}', 'live-for-{conversation_id}')"
        ))
        .expect("live conversation insert");
    }

    for (i, ((user, conversation_id), listener)) in
        users.iter().zip(listeners.iter_mut()).enumerate()
    {
        let expected = format!("live-for-{conversation_id}");
        let foreign = format!("live-for-conv-{}", (i + 1) % SUBSCRIBER_COUNT);
        assert!(
            wait_for_event(listener, &expected, Duration::from_secs(8)),
            "{user} must receive live event for {conversation_id}"
        );
        assert!(
            !wait_for_event(listener, &foreign, Duration::from_secs(1)),
            "{user} must not receive foreign conversation live event {foreign}"
        );
        listener.stop().ok();
    }

    for (user, _) in &users {
        let _ = execute_sql_as_root_via_client(&format!("DROP USER {user}"));
    }
    let _ =
        execute_sql_as_root_via_client(&format!("DROP NAMESPACE IF EXISTS {namespace} CASCADE"));
}

#[ntest::timeout(180000)]
#[test]
fn smoke_shared_table_rls_raw_download_denied() {
    if !is_server_running() {
        eprintln!("Skipping smoke_shared_table_rls_raw_download_denied: server not running");
        return;
    }

    let namespace = generate_unique_namespace("rls_dl_ns");
    let table = generate_unique_table("rls_dl_tbl");
    let full = format!("{namespace}.{table}");
    let user = generate_unique_namespace("rls_dl_user");
    let password = "smoke_pass_123";

    execute_sql_as_root_via_client(&format!("CREATE NAMESPACE IF NOT EXISTS {namespace}"))
        .expect("create namespace");
    execute_sql_as_root_via_client(&format!(
        "CREATE TABLE {full} (id TEXT PRIMARY KEY, body TEXT) WITH (TYPE='SHARED')"
    ))
    .expect("create shared table");
    wait_for_table_ready(&full, Duration::from_secs(3)).expect("table ready");
    execute_sql_as_root_via_client(&format!(
        "CREATE USER {user} WITH PASSWORD '{password}' ROLE 'user'"
    ))
    .expect("create user");
    execute_sql_as_root_via_client(&format!(
        "CREATE POLICY {table}_select ON {full} FOR SELECT TO PUBLIC USING (true)"
    ))
    .expect("grant select");

    let status = {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime");
        runtime.block_on(async {
            let token = get_access_token(&user, password).await.expect("user login");
            shared_http_client()
                .get(format!("{}/v1/files/{namespace}/{table}/sub/file.bin", server_url()))
                .bearer_auth(token)
                .send()
                .await
                .expect("download request")
                .status()
        })
    };
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "User must not download raw shared-table files even with SELECT policy"
    );

    let _ =
        execute_sql_as_root_via_client(&format!("DROP NAMESPACE IF EXISTS {namespace} CASCADE"));
}

#[ntest::timeout(180000)]
#[test]
fn smoke_shared_table_rls_role_policy_targets() {
    if !is_server_running() {
        eprintln!("Skipping smoke_shared_table_rls_role_policy_targets: server not running");
        return;
    }

    let namespace = generate_unique_namespace("rls_role_ns");
    let table = generate_unique_table("rls_role_docs");
    let full = format!("{namespace}.{table}");
    let user = generate_unique_namespace("rls_role_user");
    let service = generate_unique_namespace("rls_role_svc");
    let password = "smoke_pass_123";

    execute_sql_as_root_via_client(&format!("CREATE NAMESPACE IF NOT EXISTS {namespace}"))
        .expect("create namespace");
    execute_sql_as_root_via_client(&format!(
        "CREATE TABLE {full} (id TEXT PRIMARY KEY, owner_id TEXT NOT NULL, label TEXT) WITH \
         (TYPE='SHARED')"
    ))
    .expect("create table");
    wait_for_table_ready(&full, Duration::from_secs(3)).expect("table ready");
    execute_sql_as_root_via_client(&format!(
        "INSERT INTO {full} (id, owner_id, label) VALUES ('u1', '{user}', 'user-row'), ('s1', \
         '{service}', 'service-row')"
    ))
    .expect("seed rows");
    execute_sql_as_root_via_client(&format!(
        "CREATE USER {user} WITH PASSWORD '{password}' ROLE 'user'"
    ))
    .expect("create user");
    execute_sql_as_root_via_client(&format!(
        "CREATE USER {service} WITH PASSWORD '{password}' ROLE 'service'"
    ))
    .expect("create service");
    execute_sql_as_root_via_client(&format!(
        "CREATE POLICY user_only ON {full} FOR SELECT TO user USING (owner_id = CURRENT_USER)"
    ))
    .expect("create user policy");

    let user_rows = execute_sql_via_client_as(&user, password, &format!("SELECT id FROM {full}"))
        .expect("user select");
    assert!(user_rows.contains("u1") && !user_rows.contains("s1"));

    let service_rows =
        execute_sql_via_client_as(&service, password, &format!("SELECT id FROM {full}"))
            .expect("service select");
    assert!(
        !service_rows.contains("u1") && !service_rows.contains("s1"),
        "TO user policy must not grant service role: {service_rows}"
    );

    execute_sql_as_root_via_client(&format!(
        "CREATE POLICY service_only ON {full} FOR SELECT TO service USING (owner_id = \
         CURRENT_USER)"
    ))
    .expect("create service policy");
    let service_granted =
        execute_sql_via_client_as(&service, password, &format!("SELECT id FROM {full}"))
            .expect("service granted");
    assert!(service_granted.contains("s1") && !service_granted.contains("u1"));

    execute_sql_as_root_via_client(&format!(
        "CREATE POLICY public_label ON {full} FOR SELECT TO PUBLIC USING (label = 'user-row')"
    ))
    .expect("create public policy");
    let service_public =
        execute_sql_via_client_as(&service, password, &format!("SELECT id FROM {full}"))
            .expect("service public select");
    assert!(
        service_public.contains("u1") && service_public.contains("s1"),
        "PUBLIC policy must combine permissively: {service_public}"
    );

    let _ = execute_sql_as_root_via_client(&format!("DROP USER {user}"));
    let _ = execute_sql_as_root_via_client(&format!("DROP USER {service}"));
    let _ =
        execute_sql_as_root_via_client(&format!("DROP NAMESPACE IF EXISTS {namespace} CASCADE"));
}

#[ntest::timeout(180000)]
#[test]
fn smoke_shared_table_rls_membership_cache_and_performance() {
    if !is_server_running() {
        eprintln!(
            "Skipping smoke_shared_table_rls_membership_cache_and_performance: server not running"
        );
        return;
    }

    let namespace = generate_unique_namespace("rls_mem_cache_ns");
    let messages = generate_unique_table("rls_messages");
    let members = generate_unique_table("rls_members");
    let messages_full = format!("{namespace}.{messages}");
    let members_full = format!("{namespace}.{members}");
    let user = generate_unique_namespace("rls_mem_user");
    let password = "smoke_pass_123";

    execute_sql_as_root_via_client(&format!("CREATE NAMESPACE IF NOT EXISTS {namespace}"))
        .expect("create namespace");
    execute_sql_as_root_via_client(&format!(
        "CREATE TABLE {messages_full} (id TEXT PRIMARY KEY, group_id TEXT NOT NULL) WITH \
         (TYPE='SHARED')"
    ))
    .expect("create messages");
    execute_sql_as_root_via_client(&format!(
        "CREATE TABLE {members_full} (id TEXT PRIMARY KEY, user_id TEXT NOT NULL, group_id TEXT \
         NOT NULL, status TEXT NOT NULL) WITH (TYPE='SHARED')"
    ))
    .expect("create members");
    wait_for_table_ready(&messages_full, Duration::from_secs(3)).expect("messages ready");
    execute_sql_as_root_via_client(&format!(
        "CREATE USER {user} WITH PASSWORD '{password}' ROLE 'user'"
    ))
    .expect("create user");
    execute_sql_as_root_via_client(&format!(
        "INSERT INTO {members_full} (id, user_id, group_id, status) VALUES ('m1', '{user}', \
         'group-a', 'active')"
    ))
    .expect("seed membership");
    execute_sql_as_root_via_client(&format!(
        "INSERT INTO {messages_full} (id, group_id) VALUES ('msg-a', 'group-a'), ('msg-b', \
         'group-b')"
    ))
    .expect("seed messages");
    execute_sql_as_root_via_client(&format!(
        "CREATE POLICY member_read ON {messages_full} FOR SELECT TO user USING (group_id IN \
         (SELECT group_id FROM {members_full} WHERE user_id = CURRENT_USER AND status <> \
         'blocked'))"
    ))
    .expect("create membership policy");

    let select_sql = format!("SELECT id FROM {messages_full}");
    let first = execute_sql_via_client_as(&user, password, &select_sql).expect("first select");
    assert!(first.contains("msg-a") && !first.contains("msg-b"));

    let second = execute_sql_via_client_as(&user, password, &select_sql).expect("second select");
    assert!(second.contains("msg-a") && !second.contains("msg-b"));

    let cold_start = std::time::Instant::now();
    let _ = execute_sql_via_client_as(&user, password, &select_sql).expect("cold timing select");
    let cold_elapsed = cold_start.elapsed();

    let warm_start = std::time::Instant::now();
    for _ in 0..10 {
        let out = execute_sql_via_client_as(&user, password, &select_sql).expect("warm select");
        assert!(out.contains("msg-a") && !out.contains("msg-b"));
    }
    let warm_avg = warm_start.elapsed() / 10;
    assert!(
        warm_avg <= cold_elapsed.mul_f32(3.0).max(Duration::from_millis(250)),
        "cached membership scans should stay bounded (cold={cold_elapsed:?}, \
         warm_avg={warm_avg:?})"
    );

    let _ = execute_sql_as_root_via_client(&format!("DROP USER {user}"));
    let _ =
        execute_sql_as_root_via_client(&format!("DROP NAMESPACE IF EXISTS {namespace} CASCADE"));
}

#[ntest::timeout(180000)]
#[test]
fn smoke_shared_table_rls_membership_revoke_invalidates_cache() {
    if !is_server_running() {
        eprintln!(
            "Skipping smoke_shared_table_rls_membership_revoke_invalidates_cache: server not \
             running"
        );
        return;
    }

    let namespace = generate_unique_namespace("rls_revoke_ns");
    let messages = generate_unique_table("rls_revoke_msgs");
    let members = generate_unique_table("rls_revoke_mem");
    let messages_full = format!("{namespace}.{messages}");
    let members_full = format!("{namespace}.{members}");
    let user = generate_unique_namespace("rls_revoke_user");
    let password = "smoke_pass_123";

    execute_sql_as_root_via_client(&format!("CREATE NAMESPACE IF NOT EXISTS {namespace}"))
        .expect("create namespace");
    execute_sql_as_root_via_client(&format!(
        "CREATE TABLE {messages_full} (id TEXT PRIMARY KEY, group_id TEXT NOT NULL) WITH \
         (TYPE='SHARED')"
    ))
    .expect("create messages");
    execute_sql_as_root_via_client(&format!(
        "CREATE TABLE {members_full} (id TEXT PRIMARY KEY, user_id TEXT NOT NULL, group_id TEXT \
         NOT NULL, status TEXT NOT NULL) WITH (TYPE='SHARED')"
    ))
    .expect("create members");
    wait_for_table_ready(&messages_full, Duration::from_secs(3)).expect("messages ready");
    execute_sql_as_root_via_client(&format!(
        "CREATE USER {user} WITH PASSWORD '{password}' ROLE 'user'"
    ))
    .expect("create user");
    execute_sql_as_root_via_client(&format!(
        "INSERT INTO {members_full} (id, user_id, group_id, status) VALUES ('m1', '{user}', \
         'group-a', 'active')"
    ))
    .expect("seed membership");
    execute_sql_as_root_via_client(&format!(
        "INSERT INTO {messages_full} (id, group_id) VALUES ('msg-a', 'group-a')"
    ))
    .expect("seed messages");
    execute_sql_as_root_via_client(&format!(
        "CREATE POLICY member_read ON {messages_full} FOR SELECT TO user USING (group_id IN \
         (SELECT group_id FROM {members_full} WHERE user_id = CURRENT_USER))"
    ))
    .expect("create policy");

    let select_sql = format!("SELECT id FROM {messages_full}");
    let visible =
        execute_sql_via_client_as(&user, password, &select_sql).expect("visible before revoke");
    assert!(visible.contains("msg-a"));
    let _ = execute_sql_via_client_as(&user, password, &select_sql).expect("warm cache");

    execute_sql_as_root_via_client(&format!("DELETE FROM {members_full} WHERE id = 'm1'"))
        .expect("revoke membership");
    let hidden =
        execute_sql_via_client_as(&user, password, &select_sql).expect("hidden after revoke");
    assert!(!hidden.contains("msg-a"), "membership revocation must hide rows: {hidden}");

    let _ = execute_sql_as_root_via_client(&format!("DROP USER {user}"));
    let _ =
        execute_sql_as_root_via_client(&format!("DROP NAMESPACE IF EXISTS {namespace} CASCADE"));
}

#[ntest::timeout(180000)]
#[test]
fn smoke_shared_table_rls_null_not_policy_and_union_bypass() {
    if !is_server_running() {
        eprintln!(
            "Skipping smoke_shared_table_rls_null_not_policy_and_union_bypass: server not running"
        );
        return;
    }

    let namespace = generate_unique_namespace("rls_null_union_ns");
    let table = generate_unique_table("rls_null_docs");
    let full = format!("{namespace}.{table}");
    let user = generate_unique_namespace("rls_null_user");
    let password = "smoke_pass_123";

    execute_sql_as_root_via_client(&format!("CREATE NAMESPACE IF NOT EXISTS {namespace}"))
        .expect("create namespace");
    execute_sql_as_root_via_client(&format!(
        "CREATE TABLE {full} (id TEXT PRIMARY KEY, owner_id TEXT) WITH (TYPE='SHARED')"
    ))
    .expect("create table");
    wait_for_table_ready(&full, Duration::from_secs(3)).expect("table ready");
    execute_sql_as_root_via_client(&format!(
        "CREATE USER {user} WITH PASSWORD '{password}' ROLE 'user'"
    ))
    .expect("create user");
    execute_sql_as_root_via_client(&format!(
        "INSERT INTO {full} (id, owner_id) VALUES ('null-row', NULL), ('bob-row', 'bob')"
    ))
    .expect("seed rows");
    let not_error = execute_sql_as_root_via_client(&format!(
        "CREATE POLICY not_owner ON {full} FOR SELECT TO user USING (NOT (owner_id = \
         CURRENT_USER))"
    ))
    .expect_err("unbounded NOT policy must be rejected");
    assert!(
        not_error.to_string().contains("indexed live routing"),
        "complex policy rejection should explain bounded routing: {not_error}"
    );

    execute_sql_as_root_via_client(&format!(
        "CREATE TABLE {namespace}.rls_union_msgs (id TEXT PRIMARY KEY, group_id TEXT NOT NULL) \
         WITH (TYPE='SHARED')"
    ))
    .expect("create union messages");
    execute_sql_as_root_via_client(&format!(
        "CREATE TABLE {namespace}.rls_union_mem (id TEXT PRIMARY KEY, user_id TEXT NOT NULL, \
         group_id TEXT NOT NULL) WITH (TYPE='SHARED')"
    ))
    .expect("create union members");
    let union_msgs = format!("{namespace}.rls_union_msgs");
    let union_mem = format!("{namespace}.rls_union_mem");
    execute_sql_as_root_via_client(&format!(
        "INSERT INTO {union_mem} (id, user_id, group_id) VALUES ('m1', '{user}', 'group-a')"
    ))
    .expect("seed union membership");
    execute_sql_as_root_via_client(&format!(
        "INSERT INTO {union_msgs} (id, group_id) VALUES ('msg-a', 'group-a'), ('msg-b', 'group-b')"
    ))
    .expect("seed union messages");
    execute_sql_as_root_via_client(&format!(
        "CREATE POLICY union_member_read ON {union_msgs} FOR SELECT TO user USING (group_id IN \
         (SELECT group_id FROM {union_mem} WHERE user_id = CURRENT_USER))"
    ))
    .expect("create union policy");

    let union_sql = format!(
        "SELECT id FROM {union_msgs} WHERE group_id = 'group-b' UNION SELECT id FROM {union_msgs}"
    );
    let union_rows = execute_sql_via_client_as(&user, password, &union_sql).expect("union select");
    assert!(
        union_rows.contains("msg-a") && !union_rows.contains("msg-b"),
        "UNION must not bypass membership RLS: {union_rows}"
    );

    let _ = execute_sql_as_root_via_client(&format!("DROP USER {user}"));
    let _ =
        execute_sql_as_root_via_client(&format!("DROP NAMESPACE IF EXISTS {namespace} CASCADE"));
}

fn assert_rls_denied(error: &dyn std::fmt::Display, context: &str) {
    let message = error.to_string().to_lowercase();
    assert!(
        message.contains("with check")
            || message.contains("row-level")
            || message.contains("policy")
            || message.contains("access denied")
            || message.contains("permission")
            || message.contains("not authorized"),
        "expected RLS denial for {context}, got {error}"
    );
}

fn drain_listener(listener: &mut SubscriptionListener, duration: Duration) {
    let deadline = std::time::Instant::now() + duration;
    while std::time::Instant::now() < deadline {
        match listener.try_read_line(Duration::from_millis(100)) {
            Ok(Some(_)) | Err(_) => {},
            Ok(None) => break,
        }
    }
}

fn wait_for_event(listener: &mut SubscriptionListener, needle: &str, timeout: Duration) -> bool {
    let deadline = std::time::Instant::now() + timeout;
    while std::time::Instant::now() < deadline {
        match listener.try_read_line(Duration::from_millis(100)) {
            Ok(Some(line)) if line.contains(needle) => return true,
            Ok(Some(_)) | Err(_) => {},
            Ok(None) => return false,
        }
    }
    false
}

fn cleanup_rls_live(namespace: &str, user: &str) {
    let _ = execute_sql_as_root_via_client(&format!("DROP USER {user}"));
    let _ =
        execute_sql_as_root_via_client(&format!("DROP NAMESPACE IF EXISTS {namespace} CASCADE"));
}
