use std::{fs, process::Stdio, time::Duration};

use kalam_cli::workflow::project::config::{KalamProjectConfig, KALAM_TOML};
use kalamdb_commons::NamespaceId;
use tempfile::TempDir;

use crate::{
    common::*,
    test_project_workflow_dev::{start_migration_tracking_server, RecordedSqlRequest},
};

const SHARED_NAMESPACE: &str = "app";
const EVOLVED_SCHEMA: &str = r#"-- Shared schema for app
CREATE TABLE users (
  id INTEGER PRIMARY KEY,
  email TEXT NOT NULL,
  display_name TEXT,
  created_at TIMESTAMP
);
"#;
const EVOLVED_MIGRATION: &str = r#"-- Migration: init
-- Created: 2025-01-01T00:00:00Z

-- UP
CREATE TABLE users (
  id INTEGER PRIMARY KEY,
  email TEXT NOT NULL,
  display_name TEXT,
  created_at TIMESTAMP
);

-- DOWN
DROP TABLE users;
"#;

fn create_isolated_cli(
    home: &std::path::Path,
    credentials_path: &std::path::Path,
) -> std::process::Command {
    let mut cmd = std::process::Command::new(kalam_bin());
    cmd.env("NO_PROXY", "127.0.0.1,localhost,::1")
        .env("no_proxy", "127.0.0.1,localhost,::1")
        .env("HOME", home)
        .env("USERPROFILE", home)
        .env("KALAMDB_CREDENTIALS_PATH", credentials_path)
        .env("KALAM_TEST_SKIP_PACKAGE_INSTALL", "1");
    clear_workflow_url_env_overrides(&mut cmd);
    cmd.env_remove("HTTP_PROXY")
        .env_remove("http_proxy")
        .env_remove("HTTPS_PROXY")
        .env_remove("https_proxy")
        .env_remove("ALL_PROXY")
        .env_remove("all_proxy");
    cmd
}

fn init_remote_project(
    project_dir: &std::path::Path,
    home: &std::path::Path,
    credentials_path: &std::path::Path,
    name: &str,
    languages: &str,
    server_url: &str,
) {
    fs::create_dir_all(project_dir).expect("create project dir");
    let mut init = create_isolated_cli(home, credentials_path);
    init.current_dir(project_dir).args([
        "init",
        "--yes",
        "--name",
        name,
        "--schema-mode",
        "sql",
        "--languages",
        languages,
        "--server-mode",
        "remote",
        "--server-url",
        server_url,
    ]);
    let output = init.output().expect("init project");
    assert!(
        output.status.success(),
        "init {name} failed\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn configure_shared_namespace(project_dir: &std::path::Path, generate_types: bool, watch: bool) {
    let config_path = project_dir.join(KALAM_TOML);
    let mut config = KalamProjectConfig::load_from_path(&config_path).expect("load kalam.toml");
    let connection = config.connection.get_mut("dev").expect("dev connection");
    connection.namespace = NamespaceId::new(SHARED_NAMESPACE);
    config.dev.processes.clear();
    config.dev.apply_schema = true;
    config.dev.generate_types = generate_types;
    config.dev.watch = watch;
    config.schema.watch = watch;
    config.save_to_path(&config_path).expect("save kalam.toml");
}

fn copy_shared_evolution(from: &std::path::Path, to: &std::path::Path) {
    fs::copy(from.join("schema.sql"), to.join("schema.sql")).expect("copy schema.sql");
    let from_migrations = from.join("kalam/migrations");
    let to_migrations = to.join("kalam/migrations");
    fs::create_dir_all(&to_migrations).expect("create target migrations dir");
    for entry in fs::read_dir(&from_migrations).expect("read source migrations") {
        let entry = entry.expect("migration entry");
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.ends_with(".sql") {
            fs::copy(entry.path(), to_migrations.join(name.as_ref())).expect("copy migration");
        }
    }
}

fn sql_matches(request: &RecordedSqlRequest, needle: &str) -> bool {
    request.sql.to_ascii_uppercase().contains(&needle.to_ascii_uppercase())
}

fn count_sql(requests: &[RecordedSqlRequest], needle: &str) -> usize {
    requests.iter().filter(|request| sql_matches(request, needle)).count()
}

fn wait_for_log_contains(path: &std::path::Path, needle: &str, timeout: Duration) -> bool {
    let start = std::time::Instant::now();
    while start.elapsed() < timeout {
        if fs::read_to_string(path).unwrap_or_default().contains(needle) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    false
}

#[test]
fn test_second_project_reuses_shared_namespace_history_without_recreating_tables() {
    let temp = TempDir::new().expect("temp dir");
    let isolated_home = temp.path().join("home");
    fs::create_dir_all(isolated_home.join(".kalam")).expect("create isolated home");
    let credentials_path = isolated_home.join(".kalam/credentials.toml");
    store_test_kalam_dev_credentials(&credentials_path);

    let (server_url, requests, _server_handle) = start_migration_tracking_server();

    let ts_dir = temp.path().join("web-app");
    init_remote_project(
        &ts_dir,
        &isolated_home,
        &credentials_path,
        "web-app",
        "typescript",
        &server_url,
    );
    configure_shared_namespace(&ts_dir, false, false);
    fs::write(ts_dir.join("schema.sql"), EVOLVED_SCHEMA).expect("write evolved schema");
    fs::write(ts_dir.join("kalam/migrations/0001_init.sql"), EVOLVED_MIGRATION)
        .expect("write shared migration");

    let mut migrate_ts = create_isolated_cli(&isolated_home, &credentials_path);
    migrate_ts.current_dir(&ts_dir).args(["db", "migrate"]);
    let migrate_ts_output = migrate_ts.output().expect("migrate typescript project");
    assert!(
        migrate_ts_output.status.success(),
        "first project migrate failed\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&migrate_ts_output.stdout),
        String::from_utf8_lossy(&migrate_ts_output.stderr)
    );

    let after_first = requests.lock().expect("lock requests").clone();
    assert!(
        after_first.iter().any(|request| sql_matches(request, "CREATE TABLE users")
            && request.namespace_id.as_deref() == Some(SHARED_NAMESPACE)),
        "first project should apply CREATE TABLE users in namespace {SHARED_NAMESPACE}\nrequests: \
         {after_first:?}"
    );
    let create_count_after_first = count_sql(&after_first, "CREATE TABLE users");

    let dart_dir = temp.path().join("mobile-app");
    init_remote_project(
        &dart_dir,
        &isolated_home,
        &credentials_path,
        "mobile-app",
        "dart",
        &server_url,
    );
    configure_shared_namespace(&dart_dir, true, false);
    copy_shared_evolution(&ts_dir, &dart_dir);

    let mut migrate_dart = create_isolated_cli(&isolated_home, &credentials_path);
    migrate_dart.current_dir(&dart_dir).args(["db", "migrate"]);
    let migrate_dart_output = migrate_dart.output().expect("migrate dart project");
    let dart_migrate_stderr = String::from_utf8_lossy(&migrate_dart_output.stderr);
    assert!(
        migrate_dart_output.status.success(),
        "second project migrate failed\nstdout: {}\nstderr: {dart_migrate_stderr}",
        String::from_utf8_lossy(&migrate_dart_output.stdout)
    );
    assert!(
        dart_migrate_stderr.contains("all migrations already applied")
            || dart_migrate_stderr.contains("no migrations to apply"),
        "second project should skip already-applied history\nstderr: {dart_migrate_stderr}"
    );

    let mut gen_dart = create_isolated_cli(&isolated_home, &credentials_path);
    gen_dart.current_dir(&dart_dir).args(["schema", "gen"]);
    let gen_output = gen_dart.output().expect("schema gen dart");
    assert!(
        gen_output.status.success(),
        "dart schema gen failed\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&gen_output.stdout),
        String::from_utf8_lossy(&gen_output.stderr)
    );
    let dart_source =
        fs::read_to_string(dart_dir.join("lib/generated/kalam.dart")).expect("read dart types");
    assert!(dart_source.contains("tableId: 'users'"), "expected Users table in dart types");
    assert!(
        dart_source.contains("display_name") || dart_source.contains("displayName"),
        "dart types should come from the shared evolved schema, not the init \
         template\n{dart_source}"
    );

    let mut dev = create_isolated_cli(&isolated_home, &credentials_path);
    dev.current_dir(&dart_dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .arg("dev");
    let mut child = dev.spawn().expect("spawn dart kalam dev");
    let workflow_log = dart_dir.join("kalam/cli/logs/kalam.log");
    let reused = wait_for_log_contains(
        &workflow_log,
        "all migrations already applied",
        Duration::from_secs(8),
    ) || wait_for_log_contains(
        &workflow_log,
        "seeded schema baseline from schema.sql",
        Duration::from_secs(2),
    );
    let _ = child.kill();
    let dev_output = child.wait_with_output().expect("collect dart dev output");
    let dev_stderr = String::from_utf8_lossy(&dev_output.stderr);

    assert!(
        reused
            || workflow_log
                .is_file()
                .then(|| fs::read_to_string(&workflow_log).ok())
                .flatten()
                .is_some_and(|log| log.contains("all migrations already applied")
                    || log.contains("seeded schema baseline from schema.sql")),
        "second kalam dev should reuse applied history instead of recreating tables\nstderr: \
         {dev_stderr}\nlog: {}",
        fs::read_to_string(&workflow_log).unwrap_or_default()
    );
    assert!(
        !dart_dir.join("kalam/migrations/_draft.sql").is_file(),
        "copied history should not produce a recreate-everything draft\nstderr: {dev_stderr}"
    );

    let recorded = requests.lock().expect("lock requests").clone();
    assert_eq!(
        count_sql(&recorded, "CREATE TABLE users"),
        create_count_after_first,
        "second project must not re-apply CREATE TABLE users\nrequests: {recorded:?}"
    );
    assert!(
        recorded.iter().all(|request| !sql_matches(request, "DROP NAMESPACE")),
        "second project must not drop the shared namespace\nrequests: {recorded:?}"
    );
    assert!(
        recorded.iter().all(|request| !sql_matches(request, "DROP TABLE users")),
        "second project must not drop shared tables\nrequests: {recorded:?}"
    );
}

#[test]
fn test_fresh_init_on_same_namespace_does_not_drop_existing_schema() {
    let temp = TempDir::new().expect("temp dir");
    let isolated_home = temp.path().join("home");
    fs::create_dir_all(isolated_home.join(".kalam")).expect("create isolated home");
    let credentials_path = isolated_home.join(".kalam/credentials.toml");
    store_test_kalam_dev_credentials(&credentials_path);

    let (server_url, requests, _server_handle) = start_migration_tracking_server();

    let first_dir = temp.path().join("first-app");
    init_remote_project(
        &first_dir,
        &isolated_home,
        &credentials_path,
        "first-app",
        "typescript",
        &server_url,
    );
    configure_shared_namespace(&first_dir, false, false);
    fs::write(first_dir.join("schema.sql"), EVOLVED_SCHEMA).expect("write evolved schema");
    fs::write(first_dir.join("kalam/migrations/0001_init.sql"), EVOLVED_MIGRATION)
        .expect("write migration");

    let mut migrate_first = create_isolated_cli(&isolated_home, &credentials_path);
    migrate_first.current_dir(&first_dir).args(["db", "migrate"]);
    assert!(migrate_first.output().expect("migrate first").status.success());

    let second_dir = temp.path().join("blank-app");
    init_remote_project(
        &second_dir,
        &isolated_home,
        &credentials_path,
        "blank-app",
        "dart",
        &server_url,
    );
    configure_shared_namespace(&second_dir, false, false);

    let mut migrate_second = create_isolated_cli(&isolated_home, &credentials_path);
    migrate_second.current_dir(&second_dir).args(["db", "migrate"]);
    let second_output = migrate_second.output().expect("migrate blank project");
    assert!(
        second_output.status.success(),
        "blank second project migrate failed\nstderr: {}",
        String::from_utf8_lossy(&second_output.stderr)
    );

    let recorded = requests.lock().expect("lock requests").clone();
    assert!(
        recorded.iter().all(|request| !sql_matches(request, "DROP NAMESPACE")),
        "a fresh init on the same namespace must not drop existing schema\nrequests: {recorded:?}"
    );
    assert!(
        recorded.iter().all(|request| !sql_matches(request, "DROP TABLE")),
        "a fresh init on the same namespace must not drop existing tables\nrequests: {recorded:?}"
    );
}
