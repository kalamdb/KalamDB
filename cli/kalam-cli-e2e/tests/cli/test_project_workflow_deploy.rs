use std::fs;

use crate::common::*;
use tempfile::TempDir;

fn scaffold_sql_project(temp: &TempDir) -> std::path::PathBuf {
    let project_dir = temp.path().join("deploy-app");
    fs::create_dir_all(&project_dir).expect("create project dir");

    let mut cmd = create_cli_command();
    cmd.current_dir(&project_dir).args([
        "init",
        "--yes",
        "--name",
        "deploy-app",
        "--schema-mode",
        "sql",
        "--languages",
        "typescript",
    ]);
    assert!(cmd.output().expect("init").status.success());
    project_dir
}

#[test]
fn test_project_workflow_deploy_help_surface() {
    let mut cmd = create_cli_command();
    cmd.args(["deploy", "--help"]);

    let output = cmd.output().expect("run deploy help");
    assert!(
        output.status.success(),
        "deploy help should succeed\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("health") || stdout.contains("migration") || stdout.contains("deploy"),
        "deploy help should describe rollout and migration behavior\nstdout: {}",
        stdout
    );
}

#[test]
fn test_project_workflow_deploy_is_not_supported_yet() {
    let temp = TempDir::new().expect("temp dir");
    let project_dir = scaffold_sql_project(&temp);

    let mut deploy_cmd = create_cli_command();
    deploy_cmd.current_dir(&project_dir).args(["deploy", "--env", "dev"]);
    let deploy_output = deploy_cmd.output().expect("deploy");
    assert!(
        !deploy_output.status.success(),
        "deploy should fail until rollout support ships"
    );

    let stderr = String::from_utf8_lossy(&deploy_output.stderr);
    assert!(
        stderr.contains("not supported"),
        "expected deploy unsupported message\nstderr: {stderr}"
    );
}
