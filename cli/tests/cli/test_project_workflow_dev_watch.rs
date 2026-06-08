use std::{collections::HashMap, fs, time::Duration};

use crate::common::*;
use kalam_cli::workflow::project::config::{KalamProjectConfig, KALAM_TOML};
use tempfile::TempDir;
use wait_timeout::ChildExt;

fn add_dev_processes(project_dir: &std::path::Path, processes: HashMap<String, String>) {
    let config_path = project_dir.join(KALAM_TOML);
    let mut config = KalamProjectConfig::load_from_path(&config_path).expect("load kalam.toml");
    config.dev.processes = processes;
    config.dev.generate_types = false;
    config.save_to_path(&config_path).expect("save kalam.toml");
}

fn scaffold_watch_project(temp: &TempDir, failing_migration: bool) -> std::path::PathBuf {
    let project_dir = temp.path().join("watch-app");
    fs::create_dir_all(&project_dir).expect("create project dir");

    let mut cmd = create_cli_command();
    cmd.current_dir(&project_dir).args([
        "init",
        "--yes",
        "--name",
        "watch_app",
        "--schema-mode",
        "sql",
        "--languages",
        "typescript",
    ]);
    assert!(cmd.output().expect("init").status.success());
    fs::write(
        project_dir.join("schema.sql"),
        "-- Watch test schema\nCREATE TABLE IF NOT EXISTS users (id INTEGER PRIMARY KEY);\n",
    )
    .expect("rewrite schema for idempotent retries");

    add_dev_processes(
        &project_dir,
        HashMap::from([(
            "worker".into(),
            "while true; do echo worker alive; sleep 0.3; done".into(),
        )]),
    );

    if failing_migration {
        let migrations_dir = project_dir.join("kalam/migrations");
        fs::write(
            migrations_dir.join("20250101120000_fail.sql"),
            "-- UP\nSELECT __KALAM_TEST_SCHEMA_FAIL__;\n\n-- DOWN\n",
        )
        .expect("write failing migration");
    }

    project_dir
}

#[test]
fn test_project_workflow_dev_schema_failure_pauses_pipeline() {
    let temp = TempDir::new().expect("temp dir");
    let project_dir = scaffold_watch_project(&temp, true);

    let mut cmd = create_cli_std_command();
    cmd.current_dir(&project_dir).arg("dev");

    let mut child = cmd.spawn().expect("spawn kalam dev");
    let timeout = Duration::from_secs(4);
    let exit_status = child.wait_timeout(timeout).expect("wait with timeout");

    if exit_status.is_some() {
        let output = child.wait_with_output().expect("collect output");
        panic!("dev exited early\nstderr: {}", String::from_utf8_lossy(&output.stderr));
    }

    let _ = child.kill();
    let output = child.wait_with_output().expect("collect output");
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(
        stderr.contains("✗ Schema failed"),
        "expected schema failure messaging\nstderr: {stderr}"
    );
    assert!(
        !stderr.contains("[worker]") && !stderr.contains("worker alive"),
        "expected progress mode to hide worker logs\nstderr: {stderr}"
    );
}

#[test]
fn test_project_workflow_dev_force_recovers_schema_pipeline() {
    let temp = TempDir::new().expect("temp dir");
    let project_dir = scaffold_watch_project(&temp, true);

    // First run fails and pauses.
    let mut fail_cmd = create_cli_std_command();
    fail_cmd.current_dir(&project_dir).arg("dev");
    let mut child = fail_cmd.spawn().expect("spawn dev");
    std::thread::sleep(Duration::from_millis(800));
    let _ = child.kill();
    let _ = child.wait_with_output();

    // Remove failing migration and retry with --force.
    fs::remove_file(project_dir.join("kalam/migrations/20250101120000_fail.sql")).unwrap();

    let mut retry_cmd = create_cli_std_command();
    retry_cmd.current_dir(&project_dir).args(["dev", "--force"]);
    let mut child = retry_cmd.spawn().expect("spawn dev --force");
    std::thread::sleep(Duration::from_millis(800));
    let _ = child.kill();
    let output = child.wait_with_output().expect("collect output");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("✓ Schema applied") || stderr.contains("✓ Schema recovered"),
        "expected schema recovery with --force\nstderr: {stderr}"
    );
}
