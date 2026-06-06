use std::fs;

use crate::common::*;
use tempfile::TempDir;

fn scaffold_sql_project(temp: &TempDir) -> std::path::PathBuf {
    let project_dir = temp.path().join("status-app");
    fs::create_dir_all(&project_dir).expect("create project dir");

    let mut cmd = create_cli_command();
    cmd.current_dir(&project_dir).args([
        "init",
        "--yes",
        "--name",
        "status-app",
        "--schema-mode",
        "sql",
        "--languages",
        "typescript",
    ]);
    assert!(cmd.output().expect("init").status.success());
    project_dir
}

#[test]
fn test_project_workflow_status_and_link_help_surface() {
    let cases = [("status", "project"), ("link", "namespace")];

    for (command, expected_snippet) in cases {
        let mut cmd = create_cli_command();
        cmd.args([command, "--help"]);

        let output = cmd.output().expect("run workflow status help");
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

#[test]
fn test_project_workflow_link_and_status_report_environment() {
    let temp = TempDir::new().expect("temp dir");
    let project_dir = scaffold_sql_project(&temp);

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
    let link_output = link_cmd.output().expect("link");
    assert!(
        link_output.status.success(),
        "link failed: {}",
        String::from_utf8_lossy(&link_output.stderr)
    );

    let mut status_cmd = create_cli_command();
    status_cmd.current_dir(&project_dir).args(["status", "--env", "prod"]);
    let status_output = status_cmd.output().expect("status");
    assert!(
        status_output.status.success(),
        "status failed: {}",
        String::from_utf8_lossy(&status_output.stderr)
    );

    let stderr = String::from_utf8_lossy(&status_output.stderr);
    assert!(stderr.contains("https://db.example.com"), "expected url in status\n{stderr}");
    assert!(stderr.contains("prod-app"), "expected namespace in status\n{stderr}");
    assert!(stderr.contains("project: status-app"), "expected project name\n{stderr}");
}
