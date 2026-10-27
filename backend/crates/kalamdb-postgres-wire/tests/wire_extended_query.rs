use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicU16, Ordering},
        Arc, Mutex,
    },
};

use async_trait::async_trait;
use chrono::Utc;
use kalamdb_auth::{
    repository::user_repo::UserRepository, security::password::hash_password, AuthError, AuthResult,
};
use kalamdb_backend::session::BackendAuth;
use kalamdb_commons::{
    models::{Role, SchemaId, StorageId, UserId},
    AuthType, NodeId,
};
use kalamdb_configs::ServerConfig;
use kalamdb_core::{
    app_context::AppContext,
    sql::executor::{handler_registry::HandlerRegistry, SqlExecutor},
};
use kalamdb_postgres_wire::{
    connection::{
        WireConnectionState, WireConnectionStateError, WirePortal, WirePreparedStatement,
    },
    PostgresWireListener, PostgresWireRuntimeDeps,
};
use kalamdb_store::test_utils::TestDb;
use kalamdb_system::{providers::storages::models::StorageType, Storage, User};
use tokio::net::TcpListener;
use tokio_postgres::{NoTls, SimpleQueryMessage};

static WIRE_TEST_PORT_OFFSET: AtomicU16 = AtomicU16::new(0);

async fn connect_settings_client(url: &str) -> tokio_postgres::Client {
    let (client, connection) =
        tokio_postgres::connect(url, NoTls).await.expect("connect settings client");
    tokio::spawn(async move {
        let _ = connection.await;
    });
    client
}

async fn assert_application_name(client: &tokio_postgres::Client, expected: &str) {
    for sql in [
        "SELECT current_setting('application_name')",
        "SHOW application_name",
        "SELECT setting FROM pg_catalog.pg_settings WHERE name = 'application_name'",
    ] {
        let messages = client.simple_query(sql).await.expect("read session setting");
        let values: Vec<_> = messages
            .iter()
            .filter_map(|message| match message {
                SimpleQueryMessage::Row(row) => row.get(0),
                _ => None,
            })
            .collect();
        assert_eq!(values, vec![expected], "setting must agree for {sql}");
    }
}

#[tokio::test]
#[ntest::timeout(20000)]
async fn wire_settings_set_and_set_config_share_all_read_surfaces() {
    let (url, listener, _app_ctx, _test_db) = spawn_wire_test_server().await;
    let client = connect_settings_client(&url).await;

    client
        .batch_execute("SET application_name = 'Tabularis'")
        .await
        .expect("simple SET");
    assert_application_name(&client, "Tabularis").await;

    let statement = client
        .prepare("SET application_name = 'extended-client'")
        .await
        .expect("prepare SET");
    client.execute(&statement, &[]).await.expect("extended SET");
    assert_application_name(&client, "extended-client").await;

    let row = client
        .query_one("SELECT set_config('application_name', 'function-client', false)", &[])
        .await
        .expect("set_config");
    assert_eq!(row.get::<_, String>(0), "function-client");
    assert_application_name(&client, "function-client").await;

    drop(client);
    listener.shutdown().await.expect("stop wire listener");
}

#[tokio::test]
#[ntest::timeout(20000)]
async fn wire_settings_transaction_and_savepoint_scope() {
    let (url, listener, _app_ctx, _test_db) = spawn_wire_test_server().await;
    let client = connect_settings_client(&url).await;
    client
        .batch_execute("SET application_name = 'baseline'")
        .await
        .expect("set baseline");

    for terminal in ["COMMIT", "ROLLBACK"] {
        client.batch_execute("BEGIN").await.expect("begin local scope");
        client
            .query_one("SELECT set_config('application_name', 'transaction-local', true)", &[])
            .await
            .expect("set local setting");
        assert_application_name(&client, "transaction-local").await;
        client.batch_execute(terminal).await.expect("end local scope");
        assert_application_name(&client, "baseline").await;
    }

    client.batch_execute("BEGIN").await.expect("begin savepoint scope");
    client
        .batch_execute("SET LOCAL application_name = 'outer-local'")
        .await
        .expect("SET LOCAL");
    client
        .batch_execute("SAVEPOINT settings_before_change")
        .await
        .expect("savepoint");
    client
        .query_one("SELECT set_config('application_name', 'inner-local', true)", &[])
        .await
        .expect("set local after savepoint");
    assert_application_name(&client, "inner-local").await;
    client
        .batch_execute("ROLLBACK TO SAVEPOINT settings_before_change")
        .await
        .expect("restore savepoint");
    assert_application_name(&client, "outer-local").await;
    client.batch_execute("COMMIT").await.expect("commit local scope");
    assert_application_name(&client, "baseline").await;

    for (terminal, expected) in [("ROLLBACK", "baseline"), ("COMMIT", "session-change")] {
        client.batch_execute("BEGIN").await.expect("begin session setting scope");
        client
            .query_one("SELECT set_config('application_name', 'session-change', false)", &[])
            .await
            .expect("set session value inside transaction");
        assert_application_name(&client, "session-change").await;
        client.batch_execute(terminal).await.expect("end session setting scope");
        assert_application_name(&client, expected).await;
    }

    drop(client);
    listener.shutdown().await.expect("stop wire listener");
}

#[tokio::test]
#[ntest::timeout(20000)]
async fn wire_settings_do_not_leak_between_connections() {
    let (url, listener, _app_ctx, _test_db) = spawn_wire_test_server().await;
    let client_a = connect_settings_client(&url).await;
    let client_b = connect_settings_client(&url).await;

    client_a
        .batch_execute("SET application_name = 'client-a'")
        .await
        .expect("set client a");
    client_b
        .batch_execute("SET application_name = 'client-b'")
        .await
        .expect("set client b");
    assert_application_name(&client_a, "client-a").await;
    assert_application_name(&client_b, "client-b").await;

    client_a.batch_execute("BEGIN").await.expect("begin client a transaction");
    client_a
        .query_one("SELECT set_config('application_name', 'client-a-local', true)", &[])
        .await
        .expect("set client a local value");
    assert_application_name(&client_a, "client-a-local").await;
    assert_application_name(&client_b, "client-b").await;
    client_a.batch_execute("ROLLBACK").await.expect("rollback client a");
    assert_application_name(&client_a, "client-a").await;
    assert_application_name(&client_b, "client-b").await;

    drop(client_a);
    drop(client_b);
    listener.shutdown().await.expect("stop wire listener");
}

#[tokio::test]
#[ntest::timeout(20000)]
async fn wire_settings_unknown_names_report_undefined_object() {
    let (url, listener, _app_ctx, _test_db) = spawn_wire_test_server().await;
    let client = connect_settings_client(&url).await;

    for sql in [
        "SET kalam_missing_setting = 'value'",
        "SHOW kalam_missing_setting",
        "SELECT current_setting('kalam_missing_setting')",
        "SELECT set_config('kalam_missing_setting', 'value', false)",
    ] {
        let error = client.simple_query(sql).await.expect_err("unknown setting must fail");
        assert_eq!(
            error.code().map(|code| code.code()),
            Some("42704"),
            "SQLSTATE for {sql}: {error}"
        );
    }
    let row = client
        .query_one("SELECT current_setting('kalam_missing_setting', true)", &[])
        .await
        .expect("missing_ok suppresses unknown-setting error");
    assert_eq!(row.get::<_, Option<String>>(0), None);

    drop(client);
    listener.shutdown().await.expect("stop wire listener");
}

#[derive(Default)]
struct InMemoryUserRepo {
    users: Mutex<HashMap<UserId, User>>,
}

#[async_trait]
impl UserRepository for InMemoryUserRepo {
    async fn get_user_by_id(&self, user_id: &UserId) -> AuthResult<User> {
        self.users
            .lock()
            .expect("users lock")
            .get(user_id)
            .cloned()
            .ok_or_else(|| AuthError::UserNotFound(user_id.to_string()))
    }

    async fn get_active_oidc_invite_by_email(
        &self,
        _email: &str,
        _now_millis: i64,
    ) -> AuthResult<Option<User>> {
        Ok(None)
    }

    async fn update_user(&self, user: &User) -> AuthResult<()> {
        self.users
            .lock()
            .expect("users lock")
            .insert(user.user_id.clone(), user.clone());
        Ok(())
    }

    async fn create_user(&self, user: User) -> AuthResult<()> {
        self.users.lock().expect("users lock").insert(user.user_id.clone(), user);
        Ok(())
    }

    async fn accept_oidc_invite(&self, _invite_user_id: &UserId, _user: User) -> AuthResult<()> {
        Ok(())
    }
}

fn auth() -> BackendAuth {
    BackendAuth::new(UserId::new("wire_user"), Role::User, "password", i64::MAX)
}

async fn spawn_wire_test_server() -> (String, PostgresWireListener, Arc<AppContext>, TestDb) {
    let probe = TcpListener::bind(("127.0.0.1", 0)).await.expect("probe free port");
    let wire_port = probe.local_addr().expect("local addr").port();
    drop(probe);

    // Stay under Linux ip_local_port_range (32768-60999).
    const RPC_BASE: u16 = 10_000;
    const API_BASE: u16 = 20_000;
    const SLOTS: u16 = 9_000;
    let ticket = WIRE_TEST_PORT_OFFSET.fetch_add(1, Ordering::Relaxed);
    let slot = (std::process::id() as u16).wrapping_mul(251).wrapping_add(ticket) % SLOTS;
    let rpc_port = RPC_BASE + slot;
    let api_port = API_BASE + slot;

    let mut config = ServerConfig::default();
    config.postgres_wire.enabled = true;
    config.postgres_wire.host = "127.0.0.1".to_string();
    config.postgres_wire.port = wire_port;
    config.cluster = Some(kalamdb_configs::ClusterConfig {
        cluster_id: "wire-test-cluster".to_string(),
        node_id: 1,
        rpc_addr: format!("127.0.0.1:{rpc_port}"),
        api_addr: format!("http://127.0.0.1:{api_port}"),
        peers: Vec::new(),
        user_shards: 4,
        shared_shards: 1,
        heartbeat_interval_ms: 50,
        election_timeout_ms: (150, 300),
        snapshot_policy: "LogsSinceLast(1000)".to_string(),
        max_snapshots_to_keep: 3,
        replication_timeout_ms: 5_000,
        reconnect_interval_ms: 3_000,
        peer_wait_max_retries: None,
        peer_wait_initial_delay_ms: None,
        peer_wait_max_delay_ms: None,
    });

    let test_db = TestDb::with_system_tables().expect("create test db");
    let storage_base_path = test_db.storage_dir().expect("storage dir");
    let app_ctx = AppContext::create_isolated(
        test_db.backend(),
        NodeId::new(1),
        storage_base_path.to_string_lossy().into_owned(),
        config.clone(),
    );
    app_ctx.executor().start().await.expect("start raft");
    app_ctx.executor().initialize_cluster().await.expect("initialize raft cluster");
    app_ctx.wire_raft_appliers();
    kalamdb_jobs::init_job_manager(&app_ctx);

    let storages = app_ctx.system_tables().storages();
    let storage_id = StorageId::from("local");
    if storages.get_storage_by_id(&storage_id).unwrap().is_none() {
        storages
            .create_storage(Storage {
                storage_id,
                storage_name: "Local Storage".to_string(),
                description: Some("Default local storage for wire tests".to_string()),
                storage_type: StorageType::Filesystem,
                base_directory: storage_base_path.to_string_lossy().to_string(),
                credentials: None,
                config_json: None,
                shared_tables_template: "shared/{namespace}/{table}".to_string(),
                user_tables_template: "user/{namespace}/{table}/{userId}".to_string(),
                created_at: Utc::now().timestamp_millis(),
                updated_at: Utc::now().timestamp_millis(),
            })
            .expect("create default local storage");
    }

    let registry = Arc::new(HandlerRegistry::new());
    kalamdb_handlers::register_all_handlers(&registry, Arc::clone(&app_ctx), false);
    let sql_executor = Arc::new(SqlExecutor::new(Arc::clone(&app_ctx), registry));

    let user_repo = Arc::new(InMemoryUserRepo::default());
    let password_hash = hash_password("secret", Some(4)).await.expect("hash password");
    user_repo
        .create_user(User {
            user_id: UserId::new("admin"),
            password_hash: password_hash.clone(),
            role: Role::Dba,
            name: None,
            email: None,
            auth_type: AuthType::Password,
            auth_data: None,
            storage_mode: kalamdb_system::providers::storages::models::StorageMode::Table,
            storage_id: None,
            failed_login_attempts: 0,
            locked_until: None,
            last_login_at: None,
            created_at: 0,
            updated_at: 0,
            last_seen: None,
            deleted_at: None,
            invite_expires_at: None,
            invited_by: None,
        })
        .await
        .expect("create admin user");

    let listener = PostgresWireListener::start(
        &config,
        PostgresWireRuntimeDeps {
            app_context: Arc::clone(&app_ctx),
            sql_executor,
            user_repo,
        },
    )
    .await
    .expect("start wire listener")
    .expect("wire listener enabled");

    let url = format!("host=127.0.0.1 port={wire_port} user=admin password=secret dbname=kalam");
    (url, listener, app_ctx, test_db)
}

#[test]
fn prepared_statement_limit_rejects_new_entries_after_cap() {
    let state = WireConnectionState::with_limits("session", auth(), SchemaId::default(), 1, 1);

    state
        .put_prepared_statement(WirePreparedStatement {
            name: "stmt_one".to_string(),
            sql: "SELECT 1".to_string(),
        })
        .expect("first statement fits");

    let error = state
        .ensure_prepared_statement_capacity("stmt_two")
        .expect_err("second statement exceeds cap");
    assert_eq!(error, WireConnectionStateError::PreparedStatementLimitExceeded { limit: 1 });
}

#[test]
fn prepared_statement_limit_allows_replacing_existing_name() {
    let state = WireConnectionState::with_limits("session", auth(), SchemaId::default(), 1, 1);

    state
        .put_prepared_statement(WirePreparedStatement {
            name: "stmt".to_string(),
            sql: "SELECT 1".to_string(),
        })
        .expect("first statement fits");
    state
        .ensure_prepared_statement_capacity("stmt")
        .expect("replacing a statement should not consume another slot");
    state
        .put_prepared_statement(WirePreparedStatement {
            name: "stmt".to_string(),
            sql: "SELECT 2".to_string(),
        })
        .expect("replacement fits");

    assert_eq!(state.prepared_statement_count(), 1);
}

#[test]
fn portal_limit_rejects_new_entries_after_cap_and_close_releases_slot() {
    let state = WireConnectionState::with_limits("session", auth(), SchemaId::default(), 1, 1);

    state
        .put_portal(WirePortal {
            name: "portal_one".to_string(),
            statement_name: "stmt".to_string(),
        })
        .expect("first portal fits");

    let error = state
        .ensure_portal_capacity("portal_two")
        .expect_err("second portal exceeds cap");
    assert_eq!(error, WireConnectionStateError::PortalLimitExceeded { limit: 1 });

    state.remove_portal("portal_one");
    state
        .ensure_portal_capacity("portal_two")
        .expect("removing a portal releases the slot");
}

#[test]
fn prepared_statement_and_sql_byte_budgets_reject_oversized_entries() {
    let state = WireConnectionState::with_resource_limits(
        "session",
        auth(),
        SchemaId::default(),
        4,
        4,
        16,
        24,
    );

    let too_long_sql = "SELECT '0123456789abcdef'";
    assert_eq!(
        state.ensure_statement_sql_bytes(too_long_sql),
        Err(WireConnectionStateError::StatementSqlBytesExceeded {
            limit: 16,
            actual: too_long_sql.len(),
        })
    );

    state
        .put_prepared_statement(WirePreparedStatement {
            name: "s1".to_string(),
            sql: "SELECT 12345".to_string(),
        })
        .expect("first statement fits within 24-byte budget");

    let err = state
        .put_prepared_statement(WirePreparedStatement {
            name: "s2".to_string(),
            sql: "SELECT 67890".to_string(),
        })
        .expect_err("second statement exceeds total byte budget");
    assert!(matches!(
        err,
        WireConnectionStateError::PreparedStatementBytesExceeded { limit: 24, .. }
    ));
}

#[tokio::test]
#[ntest::timeout(20000)]
async fn wire_extended_query_lifecycle_transactions_and_session_isolation() {
    let (url, listener, _app_ctx, _test_db) = spawn_wire_test_server().await;

    let (client_a, conn_a) = tokio_postgres::connect(url.as_str(), NoTls).await.expect("connect a");
    tokio::spawn(async move {
        let _ = conn_a.await;
    });

    let (client_b, conn_b) = tokio_postgres::connect(url.as_str(), NoTls).await.expect("connect b");
    tokio::spawn(async move {
        let _ = conn_b.await;
    });

    client_a
        .batch_execute(
            "CREATE SCHEMA wire_s1;
             CREATE SCHEMA wire_s2;
             CREATE TABLE wire_s1.items (id BIGINT PRIMARY KEY, label TEXT, flag BOOLEAN) WITH \
             (TYPE = 'SHARED');
             CREATE TABLE wire_s2.items (id BIGINT PRIMARY KEY, label TEXT, flag BOOLEAN) WITH \
             (TYPE = 'SHARED');",
        )
        .await
        .expect("create schemas and tables");

    // 1. Session isolation for SET search_path across concurrent connections
    client_a
        .batch_execute("SET search_path TO wire_s1")
        .await
        .expect("set search_path s1");
    client_b
        .batch_execute("SET search_path TO wire_s2")
        .await
        .expect("set search_path s2");

    // 2. Extended query Parse / Describe / Bind / Execute / repeated Bind & NULL parameters
    let insert_stmt = client_a
        .prepare("INSERT INTO items (id, label, flag) VALUES ($1, $2, $3)")
        .await
        .expect("prepare insert in wire_s1");
    assert_eq!(insert_stmt.params().len(), 3);

    let inserted = client_a
        .execute(&insert_stmt, &[&1_i64, &Some("alpha"), &Some(true)])
        .await
        .expect("bind and execute first row");
    assert_eq!(inserted, 1);

    let inserted_null = client_a
        .execute(&insert_stmt, &[&2_i64, &Option::<&str>::None, &Option::<bool>::None])
        .await
        .expect("repeated bind with NULL parameters");
    assert_eq!(inserted_null, 1);

    let select_stmt = client_a
        .prepare("SELECT id, label, flag FROM items WHERE id = $1")
        .await
        .expect("prepare select in wire_s1");
    assert_eq!(select_stmt.columns().len(), 3);

    let row1 = client_a.query_one(&select_stmt, &[&1_i64]).await.expect("query row 1");
    assert_eq!(row1.get::<_, i64>(0), 1);
    assert_eq!(row1.get::<_, Option<String>>(1).as_deref(), Some("alpha"));
    assert_eq!(row1.get::<_, Option<bool>>(2), Some(true));

    let row2 = client_a.query_one(&select_stmt, &[&2_i64]).await.expect("query row 2");
    assert_eq!(row2.get::<_, i64>(0), 2);
    assert_eq!(row2.get::<_, Option<String>>(1), None);
    assert_eq!(row2.get::<_, Option<bool>>(2), None);

    // Verify client_b sees wire_s2.items (which is empty), proving search_path isolation
    let rows_b = client_b
        .query("SELECT id FROM items", &[])
        .await
        .expect("query items in client_b");
    assert!(rows_b.is_empty(), "client_b search_path must remain wire_s2");

    // 3. Explicit transaction error -> 25P02 failed transaction state -> SAVEPOINT recovery /
    //    ROLLBACK
    client_a.batch_execute("BEGIN").await.expect("BEGIN on client_a");
    client_a
        .execute(&insert_stmt, &[&3_i64, &Some("in_tx"), &Some(false)])
        .await
        .expect("insert row 3 in tx");

    let bad_err = client_a
        .query("SELECT nonexistent_col FROM items", &[])
        .await
        .expect_err("query on nonexistent column inside tx must fail");
    assert_eq!(bad_err.code().map(|c| c.code()), Some("42703"));

    let aborted_err = client_a
        .query("SELECT id FROM items", &[])
        .await
        .expect_err("subsequent statement in failed tx must return 25P02");
    assert_eq!(aborted_err.code().map(|c| c.code()), Some("25P02"));

    client_a.batch_execute("ROLLBACK").await.expect("ROLLBACK recovers failed tx");

    let rows_after_rollback = client_a
        .query(&select_stmt, &[&3_i64])
        .await
        .expect("query after rollback succeeds");
    assert!(rows_after_rollback.is_empty(), "row 3 from rolled back tx must not persist");

    // 4. Multi-statement simple query implicit transaction atomicity on error
    let multi_err = client_a
        .simple_query(
            "INSERT INTO items (id, label, flag) VALUES (10, 'ten', true);
             SELECT nonexistent_col FROM items;
             INSERT INTO items (id, label, flag) VALUES (11, 'eleven', true);",
        )
        .await;
    assert!(multi_err.is_err(), "multi-statement simple query must fail on bad statement");

    let row10 = client_a
        .query(&select_stmt, &[&10_i64])
        .await
        .expect("check row 10 after failed multi-statement batch");
    assert!(
        row10.is_empty(),
        "earlier statement in failed multi-statement simple query must roll back atomically"
    );

    // 5. Simple query returning multiple result sets
    let messages = client_a
        .simple_query("SELECT 1 AS a; SELECT 2 AS b;")
        .await
        .expect("multi-statement simple query succeeds");
    let row_values: Vec<String> = messages
        .into_iter()
        .filter_map(|msg| match msg {
            SimpleQueryMessage::Row(row) => row.get(0).map(str::to_string),
            _ => None,
        })
        .collect();
    assert_eq!(row_values, vec!["1".to_string(), "2".to_string()]);

    let _ = listener.shutdown().await;
}
