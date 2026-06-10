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

    let mut login = create_cli_command_with_root_auth();
    login
        .current_dir(&project_dir)
        .args(["login", "--instance", "kalam-dev"]);
    assert!(
        login.output().expect("login").status.success(),
        "failed to store kalam-dev credentials for status"
    );

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
    let cli_url = server_url();
    let env_url = format!("{cli_url}/env-override");

    let mut status_cmd = create_cli_command_with_root_auth();
    status_cmd
        .current_dir(&project_dir)
        .env("KALAM_ENV", "prod")
        .env("KALAM_URL", &env_url)
        .env("KALAM_NAMESPACE", "env-ns")
        .args([
            "status",
            "--env",
            "prod",
            "--url",
            &cli_url,
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
    assert!(stderr.contains(&cli_url), "CLI url flag should win\n{stderr}");
    assert!(stderr.contains("cli-ns"), "CLI namespace flag should win\n{stderr}");
    assert!(stderr.contains("cli flag"), "expected cli resolution source\n{stderr}");
}

#[test]
fn test_project_workflow_environment_precedence_env_over_config() {
    let temp = TempDir::new().expect("temp dir");
    let project_dir = scaffold_sql_project(&temp);
    let env_url = server_url();

    let mut status_cmd = create_cli_command_with_root_auth();
    status_cmd
        .current_dir(&project_dir)
        .env("KALAM_URL", &env_url)
        .env("KALAM_NAMESPACE", "env-ns")
        .args(["status", "--env", "prod"]);

    let output = status_cmd.output().expect("status");
    assert!(output.status.success());

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains(&env_url),
        "env var should override kalam.toml\n{stderr}"
    );
    assert!(stderr.contains("env-ns"), "env namespace should override config\n{stderr}");
}
