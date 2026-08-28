use std::{fs, process::Stdio, time::Duration};

use tempfile::TempDir;
use wait_timeout::ChildExt;

use crate::{
    common::*,
    test_project_workflow_dev::{
        create_isolated_cli_std_command, start_recording_sql_server, store_test_dev_credentials,
        update_dev_project,
    },
};

fn combined_output(stdout: &[u8], stderr: &[u8]) -> String {
    format!("{}\n{}", String::from_utf8_lossy(stdout), String::from_utf8_lossy(stderr))
}

fn scaffold_lifecycle_project(
    temp: &TempDir,
    isolated_home: &std::path::Path,
    credentials_path: &std::path::Path,
    server_url: &str,
) -> std::path::PathBuf {
    let project_dir = temp.path().join("dev-lifecycle-app");
    fs::create_dir_all(&project_dir).expect("create project dir");

    let mut init = create_isolated_cli_std_command(isolated_home, credentials_path);
    init.current_dir(&project_dir).args([
        "init",
        "--yes",
        "--name",
        "dev-lifecycle-app",
        "--schema-mode",
        "sql",
        "--languages",
        "typescript",
        "--server-mode",
        "local",
    ]);
    let output = init.output().expect("init project");
    assert!(
        output.status.success(),
        "init failed\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    update_dev_project(&project_dir, |config| {
        config.dev.processes.clear();
        config.dev.watch = true;
        config.schema.watch = true;
        config.dev.apply_schema = false;
        config.dev.generate_types = false;
        config.connection.get_mut("dev").expect("dev env").url = server_url.to_string();
    });
    project_dir
}

fn run_dev_lifecycle(
    isolated_home: &std::path::Path,
    credentials_path: &std::path::Path,
    project_dir: &std::path::Path,
    args: &[&str],
) -> std::process::Output {
    let mut cmd = create_isolated_cli_std_command(isolated_home, credentials_path);
    cmd.current_dir(project_dir)
        .stdin(Stdio::null())
        .args(args)
        .env("KALAMDB_SERVER_BIN", project_dir.join("missing-kalamdb-server"));
    cmd.output().expect("run kalam dev lifecycle command")
}

struct StopOnDrop<'a> {
    isolated_home:    &'a std::path::Path,
    credentials_path: &'a std::path::Path,
    project_dir:      &'a std::path::Path,
}

impl Drop for StopOnDrop<'_> {
    fn drop(&mut self) {
        let _ = run_dev_lifecycle(
            self.isolated_home,
            self.credentials_path,
            self.project_dir,
            &["dev", "stop"],
        );
    }
}

#[test]
fn test_project_workflow_dev_start_status_logs_stop() {
    let temp = TempDir::new().expect("temp dir");
    let isolated_home = temp.path().join("home");
    fs::create_dir_all(isolated_home.join(".kalam")).expect("create isolated home");
    let credentials_path = isolated_home.join(".kalam/credentials.toml");
    store_test_dev_credentials(&credentials_path);
    let (server_url, _requests, _server_handle) = start_recording_sql_server();
    let project_dir =
        scaffold_lifecycle_project(&temp, &isolated_home, &credentials_path, &server_url);
    let _guard = StopOnDrop {
        isolated_home:    &isolated_home,
        credentials_path: &credentials_path,
        project_dir:      &project_dir,
    };

    let stopped = run_dev_lifecycle(
        &isolated_home,
        &credentials_path,
        &project_dir,
        &["dev", "status", "--agent"],
    );
    assert!(
        stopped.status.success(),
        "status while stopped should succeed\n{}",
        combined_output(&stopped.stdout, &stopped.stderr)
    );
    let stopped_out = combined_output(&stopped.stdout, &stopped.stderr);
    assert!(
        stopped_out.contains("KALAM_DEV_STATUS") && stopped_out.contains("state=stopped"),
        "expected stopped status\n{stopped_out}"
    );

    let stop_idle = run_dev_lifecycle(
        &isolated_home,
        &credentials_path,
        &project_dir,
        &["dev", "stop", "--agent"],
    );
    assert!(
        stop_idle.status.success(),
        "stop while idle should succeed\n{}",
        combined_output(&stop_idle.stdout, &stop_idle.stderr)
    );

    let start = run_dev_lifecycle(
        &isolated_home,
        &credentials_path,
        &project_dir,
        &["dev", "start", "--agent"],
    );
    assert!(
        start.status.success(),
        "dev start --agent should succeed\n{}",
        combined_output(&start.stdout, &start.stderr)
    );
    let start_out = combined_output(&start.stdout, &start.stderr);
    assert!(start_out.contains("KALAM_DEV_STARTED"), "expected start event\n{start_out}");
    assert!(
        project_dir.join("kalam/cli/dev.session.json").is_file(),
        "session file should exist after start"
    );

    let status = run_dev_lifecycle(
        &isolated_home,
        &credentials_path,
        &project_dir,
        &["dev", "status", "--agent"],
    );
    assert!(
        status.status.success(),
        "status while running should succeed\n{}",
        combined_output(&status.stdout, &status.stderr)
    );
    let status_out = combined_output(&status.stdout, &status.stderr);
    assert!(status_out.contains("state=running"), "expected running status\n{status_out}");

    let reuse = run_dev_lifecycle(
        &isolated_home,
        &credentials_path,
        &project_dir,
        &["dev", "start", "--agent"],
    );
    assert!(
        reuse.status.success(),
        "second start should reuse\n{}",
        combined_output(&reuse.stdout, &reuse.stderr)
    );
    let reuse_out = combined_output(&reuse.stdout, &reuse.stderr);
    assert!(reuse_out.contains("KALAM_DEV_REUSED"), "expected reuse event\n{reuse_out}");

    let logs = run_dev_lifecycle(
        &isolated_home,
        &credentials_path,
        &project_dir,
        &["dev", "logs", "-n", "50"],
    );
    assert!(
        logs.status.success(),
        "logs should succeed\n{}",
        combined_output(&logs.stdout, &logs.stderr)
    );
    let log_text = String::from_utf8_lossy(&logs.stdout);
    assert!(log_text.contains("KALAM_READY"), "logs should contain KALAM_READY\n{log_text}");

    let stop = run_dev_lifecycle(
        &isolated_home,
        &credentials_path,
        &project_dir,
        &["dev", "stop", "--agent"],
    );
    assert!(
        stop.status.success(),
        "stop should succeed\n{}",
        combined_output(&stop.stdout, &stop.stderr)
    );
    let stop_out = combined_output(&stop.stdout, &stop.stderr);
    assert!(stop_out.contains("KALAM_DEV_STOPPED"), "expected stop event\n{stop_out}");

    let after = run_dev_lifecycle(
        &isolated_home,
        &credentials_path,
        &project_dir,
        &["dev", "status", "--agent"],
    );
    let after_out = combined_output(&after.stdout, &after.stderr);
    assert!(
        after.status.success() && after_out.contains("state=stopped"),
        "expected stopped after stop\n{after_out}"
    );
}

#[test]
fn test_project_workflow_dev_lifecycle_help_lists_commands() {
    let mut cmd = create_cli_command();
    cmd.args(["dev", "start", "--help"]);
    let output = cmd.output().expect("dev start help");
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.to_ascii_lowercase().contains("background"),
        "start help should mention background\n{stdout}"
    );
}

#[test]
fn test_project_workflow_dev_start_does_not_hang_waiting_for_child() {
    let temp = TempDir::new().expect("temp dir");
    let isolated_home = temp.path().join("home");
    fs::create_dir_all(isolated_home.join(".kalam")).expect("create isolated home");
    let credentials_path = isolated_home.join(".kalam/credentials.toml");
    store_test_dev_credentials(&credentials_path);
    let (server_url, _requests, _server_handle) = start_recording_sql_server();
    let project_dir =
        scaffold_lifecycle_project(&temp, &isolated_home, &credentials_path, &server_url);
    let _guard = StopOnDrop {
        isolated_home:    &isolated_home,
        credentials_path: &credentials_path,
        project_dir:      &project_dir,
    };

    let mut cmd = create_isolated_cli_std_command(&isolated_home, &credentials_path);
    cmd.current_dir(&project_dir)
        .stdin(Stdio::null())
        .args(["dev", "start", "--agent"])
        .env("KALAMDB_SERVER_BIN", project_dir.join("missing-kalamdb-server"));
    let mut child = cmd.spawn().expect("spawn start");
    let status = child.wait_timeout(Duration::from_secs(15)).expect("wait start");
    assert!(
        status.is_some(),
        "kalam dev start --agent must return after the session is ready"
    );
    let output = child.wait_with_output().expect("collect start output");
    assert!(
        output.status.success(),
        "start should succeed\n{}",
        combined_output(&output.stdout, &output.stderr)
    );
}
