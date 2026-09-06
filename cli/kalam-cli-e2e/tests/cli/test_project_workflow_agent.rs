use std::{fs, process::Stdio, time::Duration};

use kalam_cli::workflow::dev::draft_prompt::DRAFT_PROMPT_MESSAGE;
use serde_json::Value;
use tempfile::TempDir;
use wait_timeout::ChildExt;

use crate::{
    common::*,
    test_project_workflow_dev::{
        create_isolated_cli_std_command, start_migration_tracking_server,
        start_recording_sql_server, store_test_dev_credentials, update_dev_project,
        wait_for_recorded_sql_contains,
    },
};

fn combined_output(stdout: &[u8], stderr: &[u8]) -> String {
    format!("{}\n{}", String::from_utf8_lossy(stdout), String::from_utf8_lossy(stderr))
}

fn assert_agent_output_is_compact(combined: &str) {
    assert!(
        !combined.contains('\u{1b}'),
        "agent output must not contain ANSI escapes\n{combined}"
    );
    assert!(
        !combined.contains("⠋") && !combined.contains("⠙") && !combined.contains("⠹"),
        "agent output must not contain spinner frames\n{combined}"
    );
    assert!(
        !combined.contains(DRAFT_PROMPT_MESSAGE),
        "agent output must not contain modal draft prompts\n{combined}"
    );
    assert!(
        !combined.contains("\u{1b}[2J") && !combined.contains("\u{1b}[H"),
        "agent output must not clear the terminal\n{combined}"
    );
}

fn scaffold_agent_project(
    temp: &TempDir,
    isolated_home: &std::path::Path,
    credentials_path: &std::path::Path,
    server_url: &str,
    name: &str,
    server_mode: &str,
) -> std::path::PathBuf {
    let project_dir = temp.path().join(name);
    fs::create_dir_all(&project_dir).expect("create project dir");

    let mut init = create_isolated_cli_std_command(isolated_home, credentials_path);
    let mut args = vec![
        "init",
        "--yes",
        "--name",
        name,
        "--schema-mode",
        "sql",
        "--languages",
        "typescript",
        "--server-mode",
        server_mode,
    ];
    if server_mode == "remote" {
        args.push("--server-url");
        args.push(server_url);
    }
    init.current_dir(&project_dir).args(&args);
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
        config.dev.apply_schema = true;
        config.dev.generate_types = false;
        if server_mode == "local" && !server_url.is_empty() {
            config.connection.get_mut("dev").expect("dev env").url = server_url.to_string();
        }
    });
    project_dir
}

fn spawn_agent_dev(
    isolated_home: &std::path::Path,
    credentials_path: &std::path::Path,
    project_dir: &std::path::Path,
    extra_env: &[(&str, &str)],
) -> std::process::Child {
    let mut cmd = create_isolated_cli_std_command(isolated_home, credentials_path);
    cmd.current_dir(project_dir).stdin(Stdio::null()).args(["dev", "--agent"]);
    for (key, value) in extra_env {
        cmd.env(key, value);
    }
    cmd.spawn().expect("spawn kalam dev --agent")
}

#[test]
fn test_project_workflow_init_yes_is_deterministic_and_json() {
    let temp = TempDir::new().expect("temp dir");
    let project_dir = temp.path().join("agent-init-app");
    fs::create_dir_all(&project_dir).expect("create project dir");

    let mut yes_cmd = create_cli_command();
    yes_cmd
        .current_dir(&project_dir)
        .args(["init", "--yes", "--name", "agent-init-app"]);
    let yes_output = yes_cmd.output().expect("init --yes");
    assert!(
        yes_output.status.success(),
        "init --yes should succeed\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&yes_output.stdout),
        String::from_utf8_lossy(&yes_output.stderr)
    );
    let yes_stderr = String::from_utf8_lossy(&yes_output.stderr);
    assert!(
        yes_stderr.contains("next: kalam dev --agent"),
        "init --yes should tell the agent the next command\nstderr: {yes_stderr}"
    );

    let json_dir = temp.path().join("agent-init-json");
    fs::create_dir_all(&json_dir).expect("create json project dir");
    let mut json_cmd = create_cli_command();
    json_cmd
        .current_dir(&json_dir)
        .args(["init", "--yes", "--json", "--name", "agent-init-json"]);
    let json_output = json_cmd.output().expect("init --yes --json");
    assert!(
        json_output.status.success(),
        "init --json should succeed\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&json_output.stdout),
        String::from_utf8_lossy(&json_output.stderr)
    );
    let stdout = String::from_utf8_lossy(&json_output.stdout);
    let parsed: Value = serde_json::from_str(stdout.trim()).unwrap_or_else(|error| {
        panic!("init --json stdout must be JSON: {error}\nstdout: {stdout}")
    });
    assert_eq!(parsed["ok"], true);
    assert_eq!(parsed["next"], "kalam dev --agent");
    assert!(parsed["created"].as_array().is_some_and(|files| !files.is_empty()));
}

#[test]
fn test_project_workflow_status_json_is_valid() {
    let temp = TempDir::new().expect("temp dir");
    let isolated_home = temp.path().join("home");
    fs::create_dir_all(isolated_home.join(".kalam")).expect("create isolated home");
    let credentials_path = isolated_home.join(".kalam/credentials.toml");
    store_test_dev_credentials(&credentials_path);
    let (server_url, _requests, _server_handle) = start_recording_sql_server();
    let project_dir = scaffold_agent_project(
        &temp,
        &isolated_home,
        &credentials_path,
        &server_url,
        "agent-status-app",
        "remote",
    );

    let mut cmd = create_isolated_cli_std_command(&isolated_home, &credentials_path);
    cmd.current_dir(&project_dir).args(["status", "--json"]);
    let output = cmd.output().expect("status --json");
    assert!(
        output.status.success(),
        "status --json should succeed\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let parsed: Value = serde_json::from_str(stdout.trim()).unwrap_or_else(|error| {
        panic!("status --json stdout must be JSON: {error}\nstdout: {stdout}")
    });
    assert_eq!(parsed["ok"], true);
    assert_eq!(parsed["server"], "ready");
    assert!(parsed["schema"].is_string());
    assert!(parsed["types"].is_string());
}

#[test]
fn test_project_workflow_agent_reuses_healthy_server_without_local_binary() {
    let temp = TempDir::new().expect("temp dir");
    let isolated_home = temp.path().join("home");
    fs::create_dir_all(isolated_home.join(".kalam")).expect("create isolated home");
    let credentials_path = isolated_home.join(".kalam/credentials.toml");
    store_test_dev_credentials(&credentials_path);
    let (server_url, _requests, _server_handle) = start_recording_sql_server();
    let project_dir = scaffold_agent_project(
        &temp,
        &isolated_home,
        &credentials_path,
        &server_url,
        "agent-reuse-app",
        "local",
    );
    update_dev_project(&project_dir, |config| {
        config.dev.apply_schema = false;
        config.dev.generate_types = false;
    });

    let missing_bin = temp.path().join("missing-kalamdb-server");
    let mut child = spawn_agent_dev(
        &isolated_home,
        &credentials_path,
        &project_dir,
        &[("KALAMDB_SERVER_BIN", missing_bin.to_str().unwrap())],
    );

    let exit_status = child.wait_timeout(Duration::from_secs(3)).expect("wait with timeout");
    if exit_status.is_some() {
        let output = child.wait_with_output().expect("collect output");
        panic!(
            "kalam dev --agent exited while reusing a healthy server\nstdout: {}\nstderr: {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    let _ = child.kill();
    let output = child.wait_with_output().expect("collect output");
    let combined = combined_output(&output.stdout, &output.stderr);
    assert!(
        combined.contains("KALAM_SERVER_REUSED"),
        "expected compact reuse event\n{combined}"
    );
    assert!(combined.contains("KALAM_READY"), "expected compact ready event\n{combined}");
    assert!(
        !combined.contains("SERVER_DOWNLOAD_FAILED"),
        "healthy server reuse must not require a local download\n{combined}"
    );
    assert_agent_output_is_compact(&combined);
}

#[test]
fn test_project_workflow_agent_applies_ordinary_schema_without_stdin() {
    let temp = TempDir::new().expect("temp dir");
    let isolated_home = temp.path().join("home");
    fs::create_dir_all(isolated_home.join(".kalam")).expect("create isolated home");
    let credentials_path = isolated_home.join(".kalam/credentials.toml");
    store_test_dev_credentials(&credentials_path);
    let (server_url, requests, _server_handle) = start_migration_tracking_server();
    let project_dir = scaffold_agent_project(
        &temp,
        &isolated_home,
        &credentials_path,
        &server_url,
        "agent-apply-app",
        "remote",
    );

    let mut child = spawn_agent_dev(&isolated_home, &credentials_path, &project_dir, &[]);
    let applied =
        wait_for_recorded_sql_contains(&requests, "CREATE TABLE users", Duration::from_secs(8));
    let _ = child.kill();
    let output = child.wait_with_output().expect("collect output");
    let combined = combined_output(&output.stdout, &output.stderr);

    assert!(applied, "agent mode must auto-apply ordinary schema changes\n{combined}");
    assert!(
        combined.contains("KALAM_SCHEMA_APPLIED") || combined.contains("KALAM_READY"),
        "expected compact schema/ready events\n{combined}"
    );
    assert!(
        !project_dir.join("kalam/migrations/_draft.sql").exists(),
        "ordinary schema drafts should be sealed after agent apply"
    );
    assert_agent_output_is_compact(&combined);
}

#[test]
fn test_project_workflow_agent_rejects_destructive_schema_without_force() {
    let temp = TempDir::new().expect("temp dir");
    let isolated_home = temp.path().join("home");
    fs::create_dir_all(isolated_home.join(".kalam")).expect("create isolated home");
    let credentials_path = isolated_home.join(".kalam/credentials.toml");
    store_test_dev_credentials(&credentials_path);
    let (server_url, requests, _server_handle) = start_migration_tracking_server();
    let project_dir = scaffold_agent_project(
        &temp,
        &isolated_home,
        &credentials_path,
        &server_url,
        "agent-destructive-app",
        "remote",
    );

    let mut child = spawn_agent_dev(&isolated_home, &credentials_path, &project_dir, &[]);
    assert!(
        wait_for_recorded_sql_contains(&requests, "CREATE TABLE users", Duration::from_secs(8)),
        "expected the initial users table to apply before the destructive edit"
    );

    fs::write(
        project_dir.join("schema.sql"),
        "-- Example schema for agent-destructive-app\nCREATE TABLE tasks (\n  id INTEGER PRIMARY \
         KEY,\n  title TEXT NOT NULL\n);\n",
    )
    .expect("write destructive schema");

    let exit_status = child
        .wait_timeout(Duration::from_secs(10))
        .expect("wait for destructive schema error");
    let output = if exit_status.is_some() {
        child.wait_with_output().expect("collect output")
    } else {
        let _ = child.kill();
        child.wait_with_output().expect("collect output")
    };
    let combined = combined_output(&output.stdout, &output.stderr);

    assert!(
        !output.status.success(),
        "destructive schema changes must fail in agent mode without --force\n{combined}"
    );
    assert!(
        combined.contains("KALAM_ERROR code=DESTRUCTIVE_SCHEMA_CHANGE"),
        "expected a stable destructive schema error\n{combined}"
    );
    assert!(
        combined.contains("kalam dev --force"),
        "destructive errors should include a recommended action\n{combined}"
    );
    assert_agent_output_is_compact(&combined);
}

#[test]
fn test_project_workflow_agent_downloads_missing_server_without_prompt() {
    let temp = TempDir::new().expect("temp dir");
    let isolated_home = temp.path().join("home");
    fs::create_dir_all(isolated_home.join(".kalam")).expect("create isolated home");
    let credentials_path = isolated_home.join(".kalam/credentials.toml");
    store_test_dev_credentials(&credentials_path);
    let project_dir = scaffold_agent_project(
        &temp,
        &isolated_home,
        &credentials_path,
        "http://127.0.0.1:1",
        "agent-download-app",
        "local",
    );

    let missing_bin = temp.path().join("missing-kalamdb-server");
    let mut child = spawn_agent_dev(
        &isolated_home,
        &credentials_path,
        &project_dir,
        &[
            ("KALAMDB_SERVER_BIN", missing_bin.to_str().unwrap()),
            ("KALAMDB_SERVER_RELEASE_BASE_URL", "http://127.0.0.1:1"),
        ],
    );
    let exit_status =
        child.wait_timeout(Duration::from_secs(15)).expect("wait for download failure");
    let output = if exit_status.is_some() {
        child.wait_with_output().expect("collect output")
    } else {
        let _ = child.kill();
        child.wait_with_output().expect("collect output")
    };
    let combined = combined_output(&output.stdout, &output.stderr);

    assert!(
        !output.status.success(),
        "missing-server download failure must be non-zero\n{combined}"
    );
    assert!(
        combined.contains("KALAM_ERROR code=SERVER_DOWNLOAD_FAILED")
            || combined.contains("KALAM_ERROR code=SERVER_UNAVAILABLE")
            || combined.contains("KALAM_ERROR code=SERVER_START_FAILED"),
        "expected a stable server setup error code\n{combined}"
    );
    assert!(
        !combined.contains("Download KalamDB server"),
        "agent mode must not prompt to download the server\n{combined}"
    );
    assert_agent_output_is_compact(&combined);
}
