use std::{collections::HashMap, fs, time::Duration};

use crate::common::*;
use kalam_cli::workflow::project::config::{KalamProjectConfig, KALAM_TOML};
use tempfile::TempDir;
use wait_timeout::ChildExt;

fn add_dev_processes(project_dir: &std::path::Path, processes: HashMap<String, String>) {
    let config_path = project_dir.join(KALAM_TOML);
    let mut config = KalamProjectConfig::load_from_path(&config_path).expect("load kalam.toml");
    config.dev.processes = processes;
    config.save_to_path(&config_path).expect("save kalam.toml");
}

fn scaffold_dev_project(temp: &TempDir) -> std::path::PathBuf {
    let project_dir = temp.path().join("dev-app");
    fs::create_dir_all(&project_dir).expect("create project dir");

    let mut cmd = create_cli_command();
    cmd.current_dir(&project_dir).args([
        "init",
        "--yes",
        "--name",
        "dev-app",
        "--schema-mode",
        "sql",
        "--languages",
        "typescript",
    ]);
    let output = cmd.output().expect("init project");
    assert!(
        output.status.success(),
        "init failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    add_dev_processes(
        &project_dir,
        HashMap::from([
            ("frontend".into(), "while true; do echo frontend tick; sleep 0.3; done".into()),
            ("agent".into(), "while true; do echo agent tick; sleep 0.3; done".into()),
        ]),
    );

    project_dir
}

#[test]
fn test_project_workflow_dev_help_surface() {
    let mut cmd = create_cli_command();
    cmd.args(["dev", "--help"]);

    let output = cmd.output().expect("run dev help");
    assert!(
        output.status.success(),
        "dev help should succeed\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("local")
            || stdout.contains("development")
            || stdout.contains("orchestration"),
        "dev help should describe local orchestration\nstdout: {}",
        stdout
    );
    assert!(stdout.contains("--force"));
}

#[test]
fn test_project_workflow_dev_streams_prefixed_process_logs() {
    let temp = TempDir::new().expect("temp dir");
    let project_dir = scaffold_dev_project(&temp);

    let mut cmd = create_cli_std_command();
    cmd.current_dir(&project_dir).arg("dev");

    let mut child = cmd.spawn().expect("spawn kalam dev");
    let timeout = Duration::from_secs(4);
    let exit_status = child.wait_timeout(timeout).expect("wait with timeout");

    if exit_status.is_some() {
        let output = child.wait_with_output().expect("collect output");
        panic!(
            "kalam dev exited early\nstdout: {}\nstderr: {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    let _ = child.kill();
    let output = child.wait_with_output().expect("collect output");
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(
        stderr.contains("[frontend]") || stderr.contains("frontend tick"),
        "expected frontend prefixed logs\nstderr: {stderr}"
    );
    assert!(
        stderr.contains("[agent]") || stderr.contains("agent tick"),
        "expected agent prefixed logs\nstderr: {stderr}"
    );
}
