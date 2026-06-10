use std::fs;

use crate::common::*;
use tempfile::TempDir;

#[test]
fn test_project_workflow_deploy_is_not_supported_on_prod() {
    let temp = TempDir::new().expect("temp dir");
    let project_dir = temp.path().join("guardrails-app");
    fs::create_dir_all(&project_dir).expect("create project dir");

    let mut init_cmd = create_cli_command();
    init_cmd.current_dir(&project_dir).args([
        "init",
        "--yes",
        "--name",
        "guardrails-app",
        "--schema-mode",
        "sql",
        "--languages",
        "typescript",
    ]);
    assert!(init_cmd.output().expect("init").status.success());

    let mut link_cmd = create_cli_command();
    link_cmd.current_dir(&project_dir).args([
        "link",
        "--env",
        "prod",
        "--url",
        "https://db.example.com",
        "--namespace",
        "prod-app",
    ]);
    assert!(link_cmd.output().expect("link").status.success());

    // Establish baseline then modify schema without creating a migration.
    fs::copy(project_dir.join("schema.sql"), project_dir.join("kalam/.schema-baseline.sql"))
        .expect("copy baseline");
    fs::write(
        project_dir.join("schema.sql"),
        "CREATE TABLE users (id INT);\nCREATE TABLE audit_log (id INT);\n",
    )
    .expect("modify schema");

    let mut deploy_cmd = create_cli_command();
    deploy_cmd.current_dir(&project_dir).args(["deploy", "--env", "prod"]);
    let deploy_output = deploy_cmd.output().expect("deploy");
    assert!(
        !deploy_output.status.success(),
        "prod deploy should fail until rollout support ships"
    );

    let stderr = String::from_utf8_lossy(&deploy_output.stderr);
    assert!(
        stderr.contains("not supported"),
        "expected deploy unsupported message\nstderr: {stderr}"
    );
}
