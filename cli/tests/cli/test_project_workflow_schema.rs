use std::fs;

use crate::common::*;
use tempfile::TempDir;

#[test]
fn test_project_workflow_schema_command_help_surface() {
    let cases = [
        ("schema", "Generate"),
        ("migration", "migration"),
        ("db", "database"),
    ];

    for (command, expected_snippet) in cases {
        let mut cmd = create_cli_command();
        cmd.args([command, "--help"]);

        let output = cmd.output().expect("run workflow schema help");
        assert!(
            output.status.success(),
            "{} help should succeed\nstdout: {}\nstderr: {}",
            command,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );

        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            stdout.to_lowercase().contains(&expected_snippet.to_lowercase()),
            "{} help should contain '{}'\nstdout: {}",
            command,
            expected_snippet,
            stdout
        );
    }
}

fn scaffold_sql_project(temp: &TempDir) -> std::path::PathBuf {
    let project_dir = temp.path().join("schema-app");
    fs::create_dir_all(&project_dir).expect("create project dir");

    let mut cmd = create_cli_command();
    cmd.current_dir(&project_dir).args([
        "init",
        "--yes",
        "--name",
        "schema-app",
        "--schema-mode",
        "sql",
        "--languages",
        "typescript,dart",
    ]);
    let output = cmd.output().expect("init project");
    assert!(
        output.status.success(),
        "init failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    project_dir
}

#[test]
fn test_project_workflow_schema_gen_from_sql() {
    let temp = TempDir::new().expect("temp dir");
    let project_dir = scaffold_sql_project(&temp);

    let mut cmd = create_cli_command();
    cmd.current_dir(&project_dir).args(["schema", "gen"]);

    let output = cmd.output().expect("schema gen");
    assert!(
        output.status.success(),
        "schema gen should succeed\nstderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let ts_output = project_dir.join("src/generated/kalam.ts");
    let dart_output = project_dir.join("lib/generated/kalam.dart");
    assert!(ts_output.is_file(), "typescript output missing");
    assert!(dart_output.is_file(), "dart output missing");

    let ts = fs::read_to_string(ts_output).expect("read ts");
    assert!(ts.contains("export interface Users"));
}

#[test]
fn test_project_workflow_migration_create_and_status() {
    let temp = TempDir::new().expect("temp dir");
    let project_dir = scaffold_sql_project(&temp);

    let mut create_cmd = create_cli_command();
    create_cmd
        .current_dir(&project_dir)
        .args(["migration", "create", "add_profile"]);

    let create_output = create_cmd.output().expect("migration create");
    assert!(
        create_output.status.success(),
        "migration create failed: {}",
        String::from_utf8_lossy(&create_output.stderr)
    );

    let migrations_dir = project_dir.join("kalam/migrations");
    let sql_files: Vec<_> = fs::read_dir(&migrations_dir)
        .expect("read migrations")
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "sql"))
        .collect();
    assert_eq!(sql_files.len(), 1, "expected one migration file");

    let migration_sql = fs::read_to_string(sql_files[0].path()).expect("read migration file");
    assert!(migration_sql.contains("-- UP"), "migration should include UP section");
    assert!(migration_sql.contains("-- DOWN"), "migration should include DOWN section");
    assert!(
        migration_sql.contains("sqlparser-backed"),
        "migration UP should come from kalam-schema-diff placeholder"
    );

    let mut status_cmd = create_cli_command();
    status_cmd.current_dir(&project_dir).args(["migration", "status"]);
    let status_output = status_cmd.output().expect("migration status");
    assert!(
        status_output.status.success(),
        "migration status failed: {}",
        String::from_utf8_lossy(&status_output.stderr)
    );

    let stderr = String::from_utf8_lossy(&status_output.stderr);
    assert!(stderr.contains("pending"), "expected pending migration in status");
}

#[test]
fn test_project_workflow_db_migrate_local_state() {
    let temp = TempDir::new().expect("temp dir");
    let project_dir = scaffold_sql_project(&temp);

    let mut create_cmd = create_cli_command();
    create_cmd.current_dir(&project_dir).args(["migration", "create", "initial"]);

    let create_output = create_cmd.output().expect("migration create");
    assert!(create_output.status.success());

    let mut migrate_cmd = create_cli_command();
    migrate_cmd.current_dir(&project_dir).args(["db", "migrate"]);
    let migrate_output = migrate_cmd.output().expect("db migrate");
    assert!(
        migrate_output.status.success(),
        "db migrate failed: {}",
        String::from_utf8_lossy(&migrate_output.stderr)
    );

    let state_path = project_dir.join("kalam/migrations/.kalam-state.json");
    assert!(state_path.is_file(), "migration state file missing");

    let mut status_cmd = create_cli_command();
    status_cmd.current_dir(&project_dir).args(["migration", "status"]);
    let status_output = status_cmd.output().expect("migration status");
    let stderr = String::from_utf8_lossy(&status_output.stderr);
    assert!(stderr.contains("applied"), "expected applied migration");
}

#[test]
fn test_project_workflow_schema_pull_requires_server() {
    let temp = TempDir::new().expect("temp dir");
    let project_dir = temp.path().join("remote-app");
    fs::create_dir_all(&project_dir).expect("create dir");

    let mut init_cmd = create_cli_command();
    init_cmd.current_dir(&project_dir).args([
        "init",
        "--yes",
        "--name",
        "remote-app",
        "--schema-mode",
        "remote",
        "--languages",
        "typescript",
    ]);
    assert!(init_cmd.output().expect("init").status.success());

    let mut pull_cmd = create_cli_command();
    pull_cmd.current_dir(&project_dir).args(["schema", "pull"]);
    let pull_output = pull_cmd.output().expect("schema pull");
    assert!(!pull_output.status.success(), "schema pull should fail without server");

    let stderr = String::from_utf8_lossy(&pull_output.stderr);
    assert!(
        stderr.to_lowercase().contains("server"),
        "expected clear server error, got: {stderr}"
    );
}
