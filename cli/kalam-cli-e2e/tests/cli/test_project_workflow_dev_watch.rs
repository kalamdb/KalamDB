use std::{
    collections::HashMap,
    fs,
    io::{Read, Write},
    net::TcpListener,
    process::Stdio,
    time::{Duration, Instant},
};

use kalam_cli::{
    workflow::project::config::{KalamProjectConfig, KALAM_TOML},
    FileCredentialStore,
};
use kalam_client::credentials::Credentials;
use tempfile::TempDir;

use crate::common::*;

fn store_test_dev_credentials(credentials_path: &std::path::Path) {
    let mut credential_store = FileCredentialStore::with_path(credentials_path.to_path_buf())
        .expect("create credential store");
    credential_store
        .set_credentials(&Credentials::new("kalam-dev".to_string(), "test-dev-jwt".to_string()))
        .expect("store test credentials");
}

fn create_isolated_cli_std_command(
    home: &std::path::Path,
    credentials_path: &std::path::Path,
) -> std::process::Command {
    let mut cmd = std::process::Command::new(kalam_bin());
    cmd.env("NO_PROXY", "127.0.0.1,localhost,::1")
        .env("no_proxy", "127.0.0.1,localhost,::1")
        .env("HOME", home)
        .env("USERPROFILE", home)
        .env("KALAMDB_CREDENTIALS_PATH", credentials_path)
        .env("KALAM_TEST_SKIP_PACKAGE_INSTALL", "1");
    clear_workflow_url_env_overrides(&mut cmd);
    cmd.env_remove("HTTP_PROXY")
        .env_remove("http_proxy")
        .env_remove("HTTPS_PROXY")
        .env_remove("https_proxy")
        .env_remove("ALL_PROXY")
        .env_remove("all_proxy")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    cmd
}

fn start_recording_sql_server() -> (String, std::thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind recording server");
    let addr = listener.local_addr().expect("recording server addr");
    let url = format!("http://{}", addr);

    let handle = std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else {
                break;
            };

            let mut buffer = Vec::new();
            let mut chunk = [0_u8; 4096];
            let mut header_end = None;
            loop {
                let read = stream.read(&mut chunk).expect("read request chunk");
                if read == 0 {
                    break;
                }
                buffer.extend_from_slice(&chunk[..read]);
                if let Some(pos) = buffer.windows(4).position(|window| window == b"\r\n\r\n") {
                    header_end = Some(pos + 4);
                    break;
                }
            }
            let Some(header_end) = header_end else {
                continue;
            };

            let headers = String::from_utf8_lossy(&buffer[..header_end]).to_string();
            let content_length = headers
                .lines()
                .find_map(|line| {
                    let (name, value) = line.split_once(':')?;
                    if name.eq_ignore_ascii_case("content-length") {
                        value.trim().parse::<usize>().ok()
                    } else {
                        None
                    }
                })
                .unwrap_or(0);

            while buffer.len() < header_end + content_length {
                let read = stream.read(&mut chunk).expect("read request body");
                if read == 0 {
                    break;
                }
                buffer.extend_from_slice(&chunk[..read]);
            }

            let request_line = headers.lines().next().unwrap_or_default().to_string();

            let response = if request_line.starts_with("GET /health") {
                b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 52\r\nConnection: close\r\n\r\n{\"status\":\"ok\",\"version\":\"test\",\"api_version\":\"v1\"}".to_vec()
            } else if request_line.starts_with("GET /v1/api/auth/me") {
                b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 30\r\nConnection: close\r\n\r\n{\"user\":\"test\",\"role\":\"admin\"}".to_vec()
            } else if request_line.starts_with("POST /v1/api/sql") {
                b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 33\r\nConnection: close\r\n\r\n{\"status\":\"success\",\"results\":[]}".to_vec()
            } else {
                b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_vec()
            };

            let _ = stream.write_all(&response);
            let _ = stream.flush();
        }
    });

    (url, handle)
}

fn workflow_log_path(project_dir: &std::path::Path) -> std::path::PathBuf {
    project_dir.join("kalam/cli/logs/kalam.log")
}

fn wait_for_dev_log(project_dir: &std::path::Path, needles: &[&str], timeout: Duration) -> String {
    let log_path = workflow_log_path(project_dir);
    let started = Instant::now();
    loop {
        let log = fs::read_to_string(&log_path).unwrap_or_default();
        if needles.iter().any(|needle| log.contains(needle)) {
            return log;
        }
        assert!(
            started.elapsed() < timeout,
            "timed out waiting for {needles:?} in {}\nlog: {log}",
            log_path.display()
        );
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn collect_dev_stderr(mut child: std::process::Child) -> String {
    let _ = child.kill();
    let output = child.wait_with_output().expect("collect kalam dev output");
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn add_dev_processes(project_dir: &std::path::Path, processes: HashMap<String, String>) {
    let config_path = project_dir.join(KALAM_TOML);
    let mut config = KalamProjectConfig::load_from_path(&config_path).expect("load kalam.toml");
    config.dev.processes = processes;
    config.dev.generate_types = false;
    config.save_to_path(&config_path).expect("save kalam.toml");
}

fn scaffold_watch_project(
    temp: &TempDir,
    isolated_home: &std::path::Path,
    credentials_path: &std::path::Path,
    server_url: &str,
    failing_migration: bool,
) -> std::path::PathBuf {
    let project_dir = temp.path().join("watch-app");
    fs::create_dir_all(&project_dir).expect("create project dir");

    let mut cmd = create_isolated_cli_std_command(isolated_home, credentials_path);
    cmd.current_dir(&project_dir).args([
        "init",
        "--yes",
        "--name",
        "watch_app",
        "--schema-mode",
        "sql",
        "--languages",
        "typescript",
        "--server-mode",
        "remote",
        "--server-url",
        server_url,
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
    let isolated_home = temp.path().join("home");
    fs::create_dir_all(isolated_home.join(".kalam")).expect("create isolated home");
    let credentials_path = isolated_home.join(".kalam/credentials.toml");
    store_test_dev_credentials(&credentials_path);
    let (server_url, _server_handle) = start_recording_sql_server();
    let project_dir =
        scaffold_watch_project(&temp, &isolated_home, &credentials_path, &server_url, true);

    let mut cmd = create_isolated_cli_std_command(&isolated_home, &credentials_path);
    cmd.current_dir(&project_dir).arg("dev");

    let child = cmd.spawn().expect("spawn kalam dev");
    let log = wait_for_dev_log(
        &project_dir,
        &[
            "schema pipeline failed",
            "schema pipeline paused",
            "schema apply failed (test hook)",
        ],
        Duration::from_secs(8),
    );
    let stderr = collect_dev_stderr(child);

    assert!(
        stderr.contains("schema pipeline failed")
            || stderr.contains("schema pipeline paused")
            || log.contains("schema pipeline failed")
            || log.contains("schema pipeline paused")
            || log.contains("schema apply failed (test hook)"),
        "expected schema pipeline to pause on migration failure\nstderr: {stderr}\nlog: {log}"
    );
}

#[test]
fn test_project_workflow_dev_force_recovers_schema_pipeline() {
    let temp = TempDir::new().expect("temp dir");
    let isolated_home = temp.path().join("home");
    fs::create_dir_all(isolated_home.join(".kalam")).expect("create isolated home");
    let credentials_path = isolated_home.join(".kalam/credentials.toml");
    store_test_dev_credentials(&credentials_path);
    let (server_url, _server_handle) = start_recording_sql_server();
    let project_dir =
        scaffold_watch_project(&temp, &isolated_home, &credentials_path, &server_url, true);

    // First run fails and pauses.
    let mut fail_cmd = create_isolated_cli_std_command(&isolated_home, &credentials_path);
    fail_cmd.current_dir(&project_dir).arg("dev");
    let child = fail_cmd.spawn().expect("spawn dev");
    wait_for_dev_log(
        &project_dir,
        &[
            "schema pipeline failed",
            "schema pipeline paused",
            "schema apply failed (test hook)",
        ],
        Duration::from_secs(8),
    );
    let _ = collect_dev_stderr(child);

    // Remove failing migration and retry with --force.
    fs::remove_file(project_dir.join("kalam/migrations/20250101120000_fail.sql")).unwrap();

    let mut retry_cmd = create_isolated_cli_std_command(&isolated_home, &credentials_path);
    retry_cmd.current_dir(&project_dir).args(["dev", "--force"]);
    let child = retry_cmd.spawn().expect("spawn dev --force");
    let log = wait_for_dev_log(
        &project_dir,
        &[
            "Schema applied",
            "Schema recovered",
            "schema pipeline completed",
            "schema pipeline recovered",
        ],
        Duration::from_secs(10),
    );
    let stderr = collect_dev_stderr(child);
    assert!(
        stderr.contains("✓ Schema applied")
            || stderr.contains("✓ Schema recovered")
            || stderr.contains("schema pipeline completed")
            || stderr.contains("schema pipeline recovered")
            || log.contains("Schema applied")
            || log.contains("Schema recovered")
            || log.contains("schema pipeline completed")
            || log.contains("schema pipeline recovered"),
        "expected schema recovery with --force\nstderr: {stderr}\nlog: {log}"
    );
}
