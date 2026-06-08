use std::{
    collections::HashMap,
    fs,
    io::{Read, Write},
    net::TcpListener,
    sync::{Arc, Mutex},
    time::Duration,
};

use crate::common::*;
use kalam_cli::workflow::project::config::{KalamProjectConfig, KALAM_TOML};
use tempfile::TempDir;
use wait_timeout::ChildExt;

#[derive(Debug, Clone, PartialEq, Eq)]
struct RecordedSqlRequest {
    sql: String,
    namespace_id: Option<String>,
}

fn add_dev_processes(project_dir: &std::path::Path, processes: HashMap<String, String>) {
    let config_path = project_dir.join(KALAM_TOML);
    let mut config = KalamProjectConfig::load_from_path(&config_path).expect("load kalam.toml");
    config.dev.processes = processes;
    config.save_to_path(&config_path).expect("save kalam.toml");
}

fn update_dev_project(project_dir: &std::path::Path, mutate: impl FnOnce(&mut KalamProjectConfig)) {
    let config_path = project_dir.join(KALAM_TOML);
    let mut config = KalamProjectConfig::load_from_path(&config_path).expect("load kalam.toml");
    mutate(&mut config);
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

fn start_fake_kalam_server() -> (String, std::thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind fake server");
    let addr = listener.local_addr().expect("fake server addr");
    let url = format!("http://{}", addr);

    let handle = std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else {
                break;
            };
            let mut buffer = [0_u8; 1024];
            let _ = stream.read(&mut buffer);
            let response =
                b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
            let _ = stream.write_all(response);
            let _ = stream.flush();
        }
    });

    (url, handle)
}

fn start_recording_sql_server(
) -> (String, Arc<Mutex<Vec<RecordedSqlRequest>>>, std::thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind recording server");
    let addr = listener.local_addr().expect("recording server addr");
    let url = format!("http://{}", addr);
    let requests = Arc::new(Mutex::new(Vec::new()));
    let requests_handle = Arc::clone(&requests);

    let handle = std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else {
                break;
            };

            let mut buffer = Vec::new();
            let mut chunk = [0_u8; 4096];
            let header_end;
            loop {
                let read = stream.read(&mut chunk).expect("read request chunk");
                if read == 0 {
                    return;
                }
                buffer.extend_from_slice(&chunk[..read]);
                if let Some(pos) = buffer.windows(4).position(|window| window == b"\r\n\r\n") {
                    header_end = pos + 4;
                    break;
                }
            }

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
            let body =
                &buffer[header_end..header_end + content_length.min(buffer.len() - header_end)];

            let response = if request_line.starts_with("GET /health") {
                b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 52\r\nConnection: close\r\n\r\n{\"status\":\"ok\",\"version\":\"test\",\"api_version\":\"v1\"}".to_vec()
            } else if request_line.starts_with("POST /v1/api/sql") {
                let payload: serde_json::Value =
                    serde_json::from_slice(body).expect("parse sql request body");
                requests_handle.lock().expect("lock requests").push(RecordedSqlRequest {
                    sql: payload
                        .get("sql")
                        .and_then(|value| value.as_str())
                        .unwrap_or_default()
                        .to_string(),
                    namespace_id: payload
                        .get("namespace_id")
                        .and_then(|value| value.as_str())
                        .map(ToString::to_string),
                });
                b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 33\r\nConnection: close\r\n\r\n{\"status\":\"success\",\"results\":[]}".to_vec()
            } else {
                b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_vec()
            };

            let _ = stream.write_all(&response);
            let _ = stream.flush();
        }
    });

    (url, requests, handle)
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
    assert!(stdout.contains("--progress"));
}

#[test]
fn test_project_workflow_dev_progress_hides_prefixed_process_logs() {
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
        stderr.contains("✓ Environment ready"),
        "expected compact progress output\nstderr: {stderr}"
    );
    assert!(
        !stderr.contains("[frontend]") && !stderr.contains("frontend tick"),
        "expected progress mode to hide raw process logs\nstderr: {stderr}"
    );
}

#[test]
fn test_project_workflow_dev_verbose_streams_prefixed_process_logs() {
    let temp = TempDir::new().expect("temp dir");
    let project_dir = scaffold_dev_project(&temp);

    let mut cmd = create_cli_std_command();
    cmd.current_dir(&project_dir).args(["--verbose", "dev"]);

    let mut child = cmd.spawn().expect("spawn kalam dev --verbose");
    let timeout = Duration::from_secs(4);
    let exit_status = child.wait_timeout(timeout).expect("wait with timeout");

    if exit_status.is_some() {
        let output = child.wait_with_output().expect("collect output");
        panic!(
            "kalam dev --verbose exited early\nstdout: {}\nstderr: {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    let _ = child.kill();
    let output = child.wait_with_output().expect("collect output");
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(
        stderr.contains("[frontend]") || stderr.contains("frontend tick"),
        "expected frontend prefixed logs in verbose mode\nstderr: {stderr}"
    );
    assert!(
        stderr.contains("[agent]") || stderr.contains("agent tick"),
        "expected agent prefixed logs in verbose mode\nstderr: {stderr}"
    );
}

#[test]
fn test_project_workflow_dev_stays_running_when_reusing_existing_local_server() {
    let temp = TempDir::new().expect("temp dir");
    let project_dir = temp.path().join("dev-reuse-app");
    fs::create_dir_all(&project_dir).expect("create project dir");

    let mut init = create_cli_command();
    init.current_dir(&project_dir).args([
        "init",
        "--yes",
        "--name",
        "dev-reuse-app",
        "--schema-mode",
        "sql",
        "--languages",
        "typescript",
        "--server-mode",
        "local",
    ]);
    assert!(init.output().expect("init").status.success());

    let (url, _server_handle) = start_fake_kalam_server();
    update_dev_project(&project_dir, |config| {
        config.connection.get_mut("dev").expect("dev env").url = url.clone();
        config.dev.processes.clear();
        config.dev.watch = true;
        config.schema.watch = true;
    });

    let mut cmd = create_cli_std_command();
    cmd.current_dir(&project_dir).arg("dev");

    let mut child = cmd.spawn().expect("spawn kalam dev");
    let timeout = Duration::from_secs(3);
    let exit_status = child.wait_timeout(timeout).expect("wait with timeout");

    if exit_status.is_some() {
        let output = child.wait_with_output().expect("collect output");
        panic!(
            "kalam dev exited early when reusing local server\nstdout: {}\nstderr: {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    let _ = child.kill();
    let output = child.wait_with_output().expect("collect output");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("✓ Local server ready"),
        "expected local server progress\nstderr: {stderr}"
    );
}

#[test]
fn test_project_workflow_dev_prechecks_missing_local_server_binary() {
    let temp = TempDir::new().expect("temp dir");
    let project_dir = temp.path().join("dev-precheck-app");
    fs::create_dir_all(&project_dir).expect("create project dir");

    let mut init = create_cli_command();
    init.current_dir(&project_dir).args([
        "init",
        "--yes",
        "--name",
        "dev-precheck-app",
        "--schema-mode",
        "sql",
        "--languages",
        "typescript",
        "--server-mode",
        "local",
    ]);
    assert!(init.output().expect("init").status.success());

    update_dev_project(&project_dir, |config| {
        config.connection.get_mut("dev").expect("dev env").url = "http://127.0.0.1:39991".into();
        config.dev.processes.clear();
    });

    let mut cmd = create_cli_std_command();
    cmd.current_dir(&project_dir)
        .arg("dev")
        .env("KALAMDB_SERVER_BIN", project_dir.join("missing-kalamdb-server"));

    let output = cmd.output().expect("run kalam dev");
    assert!(!output.status.success(), "dev should fail when local server binary is missing");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("precheck"),
        "expected precheck messaging for missing local binary\nstderr: {stderr}"
    );
}

#[test]
fn test_project_workflow_dev_bootstraps_namespace_and_schema_sql() {
    let temp = TempDir::new().expect("temp dir");
    let project_dir = temp.path().join("dev-bootstrap-app");
    fs::create_dir_all(&project_dir).expect("create project dir");

    let (server_url, requests, _server_handle) = start_recording_sql_server();

    let mut init = create_cli_command();
    init.current_dir(&project_dir).args([
        "init",
        "--yes",
        "--name",
        "dev-bootstrap-app",
        "--schema-mode",
        "sql",
        "--server-mode",
        "remote",
        "--server-url",
        &server_url,
    ]);
    let init_output = init.output().expect("init project");
    assert!(
        init_output.status.success(),
        "init failed\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&init_output.stdout),
        String::from_utf8_lossy(&init_output.stderr)
    );

    update_dev_project(&project_dir, |config| {
        config.dev.processes.clear();
        config.dev.watch = true;
    });

    let mut cmd = create_cli_std_command();
    cmd.current_dir(&project_dir).arg("dev");

    let mut child = cmd.spawn().expect("spawn kalam dev");
    let start = std::time::Instant::now();
    while start.elapsed() < Duration::from_secs(5) {
        if requests.lock().expect("lock requests").len() >= 2 {
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }

    let recorded = requests.lock().expect("lock requests").clone();
    let _ = child.kill();
    let output = child.wait_with_output().expect("collect output");
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(
        recorded.len() >= 2,
        "expected namespace bootstrap and schema apply SQL requests\nrequests: {recorded:?}\nstderr: {stderr}"
    );
    assert!(
        recorded
            .iter()
            .any(|request| request.sql.contains("CREATE NAMESPACE IF NOT EXISTS")),
        "expected create namespace request\nrequests: {recorded:?}"
    );
    let namespace_request = recorded
        .iter()
        .find(|request| request.sql.contains("CREATE NAMESPACE IF NOT EXISTS"))
        .expect("namespace bootstrap request");
    assert_eq!(
        namespace_request.sql, "CREATE NAMESPACE IF NOT EXISTS dev-bootstrap-app",
        "expected namespace bootstrap SQL without quoted namespace\nrequests: {recorded:?}"
    );
    assert!(
        recorded.iter().any(|request| request.sql.contains("CREATE TABLE users")
            && request.namespace_id.as_deref() == Some("dev-bootstrap-app")),
        "expected schema.sql request in project namespace\nrequests: {recorded:?}"
    );
    assert!(
        stderr.contains("⠋ Applying schema...") && stderr.contains("✓ Schema applied"),
        "expected schema progress output\nstderr: {stderr}"
    );
}
