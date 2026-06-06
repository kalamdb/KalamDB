use std::fs;

use crate::common::*;
use tempfile::TempDir;

fn scaffold_sql_project(temp: &TempDir) -> std::path::PathBuf {
    let project_dir = temp.path().join("resolution-app");
    fs::create_dir_all(&project_dir).expect("create project dir");

    let mut cmd = create_cli_command();
    cmd.current_dir(&project_dir).args([
        "init",
        "--yes",
        "--name",
        "resolution-app",
        "--schema-mode",
        "sql",
        "--languages",
        "typescript",
    ]);
    assert!(cmd.output().expect("init").status.success());

    let kalam_toml = fs::read_to_string(project_dir.join("kalam.toml")).expect("read kalam.toml");
    let augmented = format!(
        "{kalam_toml}\n[connection.prod]\nurl = \"https://config.example.com\"\nnamespace = \"config-ns\"\n"
    );
    fs::write(project_dir.join("kalam.toml"), augmented).expect("write prod connection");

    project_dir
}

#[test]
fn test_project_workflow_environment_precedence_cli_over_env_and_config() {
    let temp = TempDir::new().expect("temp dir");
    let project_dir = scaffold_sql_project(&temp);

    let mut status_cmd = create_cli_command();
    status_cmd
        .current_dir(&project_dir)
        .env("KALAM_ENV", "prod")
        .env("KALAM_URL", "https://env.example.com")
        .env("KALAM_NAMESPACE", "env-ns")
        .args([
            "status",
            "--env",
            "prod",
            "--url",
            "https://cli.example.com",
            "--namespace",
            "cli-ns",
        ]);

    let output = status_cmd.output().expect("status");
    assert!(
        output.status.success(),
        "status failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("https://cli.example.com"), "CLI url flag should win\n{stderr}");
    assert!(stderr.contains("cli-ns"), "CLI namespace flag should win\n{stderr}");
    assert!(stderr.contains("cli flag"), "expected cli resolution source\n{stderr}");
}

#[test]
fn test_project_workflow_environment_precedence_env_over_config() {
    let temp = TempDir::new().expect("temp dir");
    let project_dir = scaffold_sql_project(&temp);

    let mut status_cmd = create_cli_command();
    status_cmd
        .current_dir(&project_dir)
        .env("KALAM_URL", "https://env.example.com")
        .env("KALAM_NAMESPACE", "env-ns")
        .args(["status", "--env", "prod"]);

    let output = status_cmd.output().expect("status");
    assert!(output.status.success());

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("https://env.example.com"),
        "env var should override kalam.toml\n{stderr}"
    );
    assert!(stderr.contains("env-ns"), "env namespace should override config\n{stderr}");
}
