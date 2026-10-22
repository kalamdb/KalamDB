//! Fault-injection and negative-path integration tests for CLI workflow lifecycle (T152).

use std::{collections::HashMap, fs, path::PathBuf};

use kalam_cli::{
    agent_error::{AgentError, AgentErrorCode},
    config::{CLIConfiguration, WorkflowLoggingPolicy},
    output::WorkflowOutput,
    workflow::{
        agent::destructive_schema_objects,
        db::seed::{seed_database, SeedMode},
        deploy::validate_deploy_readiness,
        dev::{
            session::{live_session_exists, DevSession},
            watch::{schema_file_mtime, update_schema_baseline, wait_for_stable_schema_file},
        },
        functions::build_functions,
        instance::{
            instance_is_live, live_instance, new_instance_record, project_layout, save_instance,
            StartedBy, TargetKind,
        },
        lifecycle::{other_running_instance, InstanceKind, InstanceState, InstanceSummary},
        migration::{
            checksum_sql, create::update_draft_migration, MigrationState, MigrationStatus,
            DRAFT_MIGRATION_FILE,
        },
        project::config::{
            ConnectionEnv, DevSection, EnvironmentPurpose, FunctionsSection, KalamProjectConfig,
            LoggingSection, MigrationsSection, ProjectSection, SchemaMode, SchemaSection,
            SchemaTarget,
        },
        WorkflowContext,
    },
};
use kalamdb_commons::SchemaId;
use tempfile::TempDir;

fn make_project_config(env_name: &str, purpose: Option<EnvironmentPurpose>) -> KalamProjectConfig {
    KalamProjectConfig {
        project:    ProjectSection {
            name:            "pg18_demo".into(),
            default_env:     env_name.into(),
            package_manager: None,
            kalam_dir:       "kalam".into(),
            server_version:  None,
        },
        connection: HashMap::from([(
            env_name.into(),
            ConnectionEnv {
                url: "http://127.0.0.1:2900".into(),
                namespace: SchemaId::new("pg18_demo"),
                purpose,
            },
        )]),
        schema:     SchemaSection {
            mode:      SchemaMode::Sql,
            path:      Some("schema.sql".into()),
            watch:     true,
            languages: vec!["typescript".into()],
            targets:   HashMap::from([(
                "typescript".into(),
                SchemaTarget {
                    output:            "src/generated/kalam.ts".into(),
                    unqualified_names: false,
                },
            )]),
        },
        migrations: MigrationsSection {
            dir:         "kalam/migrations".into(),
            auto_create: true,
        },
        dev:        DevSection {
            apply_schema: true,
            generate_types: true,
            watch: true,
            ..DevSection::default()
        },
        logging:    LoggingSection::default(),
        functions:  FunctionsSection::default(),
    }
}

fn make_context(root: &std::path::Path, config: KalamProjectConfig) -> WorkflowContext {
    WorkflowContext {
        project_root: root.to_path_buf(),
        config,
        cli_config: CLIConfiguration::default(),
        use_color: false,
        animations: false,
        agent: false,
        json: false,
        project_dir: None,
        env_override: None,
        namespace_override: None,
        url_override: None,
        global: false,
        host: None,
        port: None,
        instance: None,
    }
}

#[tokio::test]
#[ntest::timeout(10000)]
async fn rapid_multi_save_during_dev_watch_debounces_to_final_schema() {
    let temp = TempDir::new().unwrap();
    let schema_path = temp.path().join("schema.sql");
    fs::write(&schema_path, "CREATE SCHEMA app;\nCREATE TABLE app.items (id BIGINT").unwrap();

    let writer_path = schema_path.clone();
    let writer = std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(35));
        fs::write(
            &writer_path,
            "CREATE SCHEMA app;\nCREATE TABLE app.items (id BIGINT PRIMARY KEY);",
        )
        .unwrap();
        std::thread::sleep(std::time::Duration::from_millis(35));
        fs::write(
            &writer_path,
            "CREATE SCHEMA app;\nCREATE DOMAIN app.qty AS INT CHECK (VALUE > 0);\nCREATE TABLE \
             app.items (id BIGINT PRIMARY KEY, qty app.qty NOT NULL);",
        )
        .unwrap();
    });

    let stable_mtime = wait_for_stable_schema_file(&schema_path).await;
    writer.join().unwrap();

    let final_sql = fs::read_to_string(&schema_path).unwrap();
    assert!(final_sql.contains("CREATE DOMAIN app.qty"));
    assert_eq!(stable_mtime, schema_file_mtime(&schema_path));
}

#[test]
fn concurrent_schema_edit_during_migration_apply_refreshes_draft() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    let config = make_project_config("dev", Some(EnvironmentPurpose::Development));
    let output = WorkflowOutput::new(false, WorkflowLoggingPolicy::disabled());

    // Initial applied schema v1
    let v1 = "CREATE SCHEMA app;\nCREATE TABLE app.orders (id BIGINT PRIMARY KEY);\n";
    fs::write(root.join("schema.sql"), v1).unwrap();
    update_schema_baseline(root, &config, &output).unwrap();

    // Edit to v2 (adds enum + column) -> draft created
    let v2 = "CREATE SCHEMA app;\nCREATE TYPE app.status AS ENUM ('open', 'closed');\nCREATE \
              TABLE app.orders (id BIGINT PRIMARY KEY, status app.status);\n";
    fs::write(root.join("schema.sql"), v2).unwrap();
    let draft_v2 = update_draft_migration(root, &config, &output).unwrap();
    assert!(draft_v2.is_some());

    // Simulate v2 being applied and baseline updated to v2 while user concurrently edits schema.sql
    // to v3
    fs::write(config.schema_baseline_path(root), v2).unwrap();
    let v3 = "CREATE SCHEMA app;\nCREATE TYPE app.status AS ENUM ('open', 'closed', \
              'archived');\nCREATE TABLE app.orders (id BIGINT PRIMARY KEY, status app.status, \
              note TEXT);\n";
    fs::write(root.join("schema.sql"), v3).unwrap();

    // Refreshing the draft after concurrent edit must capture only the v2 -> v3 delta
    let refreshed = update_draft_migration(root, &config, &output).unwrap();
    assert!(refreshed.is_some());
    let draft_sql =
        fs::read_to_string(config.migrations_dir(root).join(DRAFT_MIGRATION_FILE)).unwrap();
    assert!(
        draft_sql.contains("archived"),
        "expected v2->v3 enum delta in draft:\n{draft_sql}"
    );
    assert!(
        draft_sql.contains("ADD COLUMN note"),
        "expected v2->v3 column delta:\n{draft_sql}"
    );
}

#[test]
fn checksum_mismatch_on_applied_migration_fails_clearly() {
    let namespace = SchemaId::new("pg18_demo");
    let original_sql = "CREATE SCHEMA app;\nCREATE TABLE app.users (id BIGINT PRIMARY KEY);";
    let mut state = MigrationState::default();
    state.upsert_applying("0001_init.sql", &namespace, original_sql, "0001_init.sql");
    state.mark_applied("0001_init.sql");

    let tampered_sql =
        "CREATE SCHEMA app;\nCREATE TABLE app.users (id BIGINT PRIMARY KEY, role TEXT);";
    let tampered_checksum = checksum_sql(tampered_sql);

    let err = state
        .validate_applied_checksum("0001_init.sql", &tampered_checksum)
        .expect_err("tampered applied migration must fail checksum validation");
    let msg = err.to_string();
    assert!(msg.contains("migration file was modified after being applied"), "{msg}");
    assert!(msg.contains("0001_init.sql"), "{msg}");
}

#[test]
fn interrupted_applying_record_recovery_retry_skip_abort_and_mark_applied() {
    let namespace = SchemaId::new("pg18_demo");
    let sql = "CREATE DOMAIN app.pos_int AS INT CHECK (VALUE > 0);";
    let mut state = MigrationState::default();

    // 1. Interrupted migration left in `applying` state
    state.upsert_applying("0002_domain.sql", &namespace, sql, "0002_domain.sql");
    assert_eq!(state.applying_records().len(), 1);
    assert!(!state.is_applied("0002_domain.sql"));

    // 2. Mark failed (e.g., interrupted before restart)
    state.mark_failed("0002_domain.sql", "interrupted before restart");
    assert!(state.has_failed_migration_id("0002_domain.sql"));
    assert_eq!(state.failed_records().len(), 1);

    // 3. Retry transitions back to `applying`
    state.upsert_applying("0002_domain.sql", &namespace, sql, "0002_domain.sql");
    assert_eq!(
        state.record("0002_domain.sql").map(|r| r.status),
        Some(MigrationStatus::Applying)
    );

    // 4. `repair --mark-applied` / skip transitions to `applied` and clears error
    state.mark_applied("0002_domain.sql");
    assert!(state.is_applied("0002_domain.sql"));
    assert!(!state.has_failed_migration_id("0002_domain.sql"));
    let record = state.record("0002_domain.sql").unwrap();
    assert_eq!(record.status, MigrationStatus::Applied);
    assert!(record.error_message.is_none());
}

#[test]
fn destructive_draft_blocked_in_agent_mode_without_force() {
    let destructive_sql = r#"
-- UP
DROP TABLE app.users;
DROP TYPE app.order_state;
DROP DOMAIN app.positive_qty;
DROP FUNCTION app.calc_tax( BIGINT );
DROP PROCEDURE app.purge_audit;
ALTER TYPE app.shipping_addr DROP ATTRIBUTE zip;
"#;
    let objects = destructive_schema_objects(destructive_sql);
    assert!(objects.iter().any(|o| o.contains("table `app.users`")), "{objects:?}");
    assert!(objects.iter().any(|o| o.contains("type `app.order_state`")), "{objects:?}");
    assert!(objects.iter().any(|o| o.contains("domain `app.positive_qty`")), "{objects:?}");
    assert!(objects.iter().any(|o| o.contains("function `app.calc_tax`")), "{objects:?}");
    assert!(objects.iter().any(|o| o.contains("procedure `app.purge_audit`")), "{objects:?}");
    assert!(
        objects.iter().any(|o| o.contains("attribute `app.shipping_addr.zip`")),
        "{objects:?}"
    );

    let agent_err = AgentError::destructive_schema_change(&objects.join(", "));
    assert_eq!(agent_err.code, AgentErrorCode::DestructiveSchemaChange);
    assert!(agent_err.message.contains("app.users"));
}

#[tokio::test]
#[ntest::timeout(10000)]
async fn failed_function_build_preserves_prior_active_artifact_and_manifest() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    fs::write(
        root.join("schema.sql"),
        "CREATE SCHEMA api;\nCREATE FUNCTION api.ping() RETURNS TEXT;\n",
    )
    .unwrap();
    fs::create_dir_all(root.join("functions/src/api")).unwrap();
    fs::write(root.join("functions/package.json"), r#"{"dependencies":{}}"#).unwrap();

    let config = make_project_config("dev", Some(EnvironmentPurpose::Development));
    let ctx = make_context(root, config);

    // First build succeeds (no implemented TS handler yet -> valid empty module artifact +
    // manifest)
    build_functions(&ctx).await.expect("initial function build should succeed");
    let artifact_path = root.join("functions/.kalam/build/module.js");
    let manifest_path = root.join("functions/.kalam/build/manifest.json");
    let good_artifact = fs::read_to_string(&artifact_path).unwrap();
    let good_manifest = fs::read_to_string(&manifest_path).unwrap();

    // Introduce forbidden `eval(` in function source -> build must fail and keep prior artifact
    // intact
    fs::write(
        root.join("functions/src/api/bad.ts"),
        "export const bad = () => eval('1 + 1');\n",
    )
    .unwrap();
    let err = build_functions(&ctx).await.expect_err("function build with eval() must fail");
    assert!(err.to_string().contains("must not use eval"), "{err}");

    assert_eq!(fs::read_to_string(&artifact_path).unwrap(), good_artifact);
    assert_eq!(fs::read_to_string(&manifest_path).unwrap(), good_manifest);
}

#[test]
fn stale_pid_in_instance_json_and_dev_session_json_is_not_treated_as_live() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    let layout = project_layout(root);
    layout.ensure_dirs().unwrap();

    let mut record = new_instance_record(
        TargetKind::ProjectLocal,
        &layout,
        2900,
        None,
        Some("pg18_demo"),
        StartedBy::Up,
        true,
        None,
        Some(root.to_path_buf()),
    );
    // Use an unreachable/non-existent PID
    record.pid = Some(1_999_999_999);
    record.exe = Some(PathBuf::from("/nonexistent/kalamdb-server"));
    save_instance(&layout, &record).unwrap();

    assert!(!instance_is_live(&record));
    assert!(live_instance(&layout).unwrap().is_none());

    let session_path = root.join(".kalam/dev-session.json");
    let stale_session = DevSession {
        pid:          1_999_999_999,
        started_at:   "2026-10-15T00:00:00Z".into(),
        project_root: root.to_path_buf(),
        log_path:     root.join(".kalam/logs/kalam.log"),
        exe:          PathBuf::from("/nonexistent/kalam"),
        url:          Some("http://127.0.0.1:2900".into()),
    };
    fs::create_dir_all(session_path.parent().unwrap()).unwrap();
    fs::write(&session_path, serde_json::to_vec_pretty(&stale_session).unwrap()).unwrap();

    assert!(!live_session_exists(&session_path));
    assert!(!session_path.exists(), "stale dev-session.json should be cleaned up on check");
}

#[test]
fn stopped_instance_with_port_reused_by_another_live_server_is_detected() {
    let stopped = InstanceSummary {
        name:     "old-project".into(),
        kind:     InstanceKind::Local,
        state:    InstanceState::Stopped,
        url:      Some("http://127.0.0.1:2900".into()),
        folder:   Some(PathBuf::from("/tmp/old-project")),
        global:   false,
        user:     None,
        auth:     None,
        aliases:  Vec::new(),
        identity: "/tmp/old-project/.kalam/run/instance.json".into(),
    };
    let live_other = InstanceSummary {
        name:     "active-project".into(),
        kind:     InstanceKind::Local,
        state:    InstanceState::Running,
        url:      Some("http://localhost:2900".into()),
        folder:   Some(PathBuf::from("/tmp/active-project")),
        global:   false,
        user:     None,
        auth:     None,
        aliases:  Vec::new(),
        identity: "/tmp/active-project/.kalam/run/instance.json".into(),
    };
    let catalog = vec![stopped.clone(), live_other];
    let owner = other_running_instance(&catalog, &stopped)
        .expect("must detect that another running instance now owns port 2900");
    assert_eq!(owner.name, "active-project");
}

#[tokio::test]
#[ntest::timeout(10000)]
async fn db_seed_refused_on_staging_and_production_targets() {
    for (env_name, purpose) in [
        ("staging", EnvironmentPurpose::Staging),
        ("prod", EnvironmentPurpose::Production),
    ] {
        let temp = TempDir::new().unwrap();
        let root = temp.path();
        fs::create_dir_all(root.join("kalam")).unwrap();
        fs::write(root.join("kalam/seed.sql"), "INSERT INTO app.items VALUES (1);\n").unwrap();

        let mut config = make_project_config(env_name, Some(purpose));
        config.connection.get_mut(env_name).unwrap().url = "https://db.example.com".into();
        let ctx = make_context(root, config);
        let output = WorkflowOutput::new(false, WorkflowLoggingPolicy::disabled());

        let err = seed_database(&ctx, &output, SeedMode::Force)
            .await
            .expect_err("seed_database must refuse staging/production targets");
        assert!(
            err.to_string().contains("refusing to load development fixtures"),
            "unexpected error for {env_name}: {err}"
        );
    }
}

#[test]
fn deploy_blocked_when_schema_diverges_from_committed_migrations() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    let mut config = make_project_config("prod", Some(EnvironmentPurpose::Production));
    config.connection.get_mut("prod").unwrap().url = "https://db.example.com".into();
    let output = WorkflowOutput::new(false, WorkflowLoggingPolicy::disabled());

    // Baseline has v1, schema.sql has uncommitted v2, and kalam/migrations has no covering
    // migration
    fs::create_dir_all(root.join("kalam/migrations")).unwrap();
    fs::write(
        config.schema_baseline_path(root),
        "CREATE SCHEMA app;\nCREATE TABLE app.items (id BIGINT PRIMARY KEY);\n",
    )
    .unwrap();
    fs::write(
        root.join("schema.sql"),
        "CREATE SCHEMA app;\nCREATE TABLE app.items (id BIGINT PRIMARY KEY, price \
         NUMERIC(10,2));\n",
    )
    .unwrap();

    let ctx = make_context(root, config);
    let err = validate_deploy_readiness(&ctx, "prod", &output)
        .expect_err("deploy must be blocked when schema.sql has uncommitted changes");
    assert!(
        err.to_string()
            .contains("deploy blocked: schema changes require a committed migration"),
        "{err}"
    );
}
