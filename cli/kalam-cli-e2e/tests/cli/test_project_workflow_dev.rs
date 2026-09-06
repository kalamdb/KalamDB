use std::{
    collections::HashMap,
    fs,
    io::{Read, Write},
    net::TcpListener,
    process::Stdio,
    sync::{Arc, Mutex},
    time::Duration,
};

use kalam_cli::workflow::{
    dev::draft_prompt::DRAFT_PROMPT_MESSAGE,
    project::config::{KalamProjectConfig, KALAM_TOML},
};
use tempfile::TempDir;
use wait_timeout::ChildExt;

use crate::common::*;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RecordedSqlRequest {
    pub(crate) sql:          String,
    pub(crate) namespace_id: Option<String>,
}

pub(crate) fn add_dev_processes(project_dir: &std::path::Path, processes: HashMap<String, String>) {
    let config_path = project_dir.join(KALAM_TOML);
    let mut config = KalamProjectConfig::load_from_path(&config_path).expect("load kalam.toml");
    config.dev.processes = processes;
    config.save_to_path(&config_path).expect("save kalam.toml");
}

pub(crate) fn update_dev_project(
    project_dir: &std::path::Path,
    mutate: impl FnOnce(&mut KalamProjectConfig),
) {
    let config_path = project_dir.join(KALAM_TOML);
    let mut config = KalamProjectConfig::load_from_path(&config_path).expect("load kalam.toml");
    mutate(&mut config);
    config.save_to_path(&config_path).expect("save kalam.toml");
}

pub(crate) fn store_test_dev_credentials(credentials_path: &std::path::Path) {
    let mut credential_store = FileCredentialStore::with_path(credentials_path.to_path_buf())
        .expect("create credential store");
    credential_store
        .set_credentials(&Credentials::new("kalam-dev".to_string(), "test-dev-jwt".to_string()))
        .expect("store test credentials");
}

fn scaffold_dev_project(
    temp: &TempDir,
    isolated_home: &std::path::Path,
    credentials_path: &std::path::Path,
    server_url: &str,
) -> std::path::PathBuf {
    let project_dir = temp.path().join("dev-app");
    fs::create_dir_all(&project_dir).expect("create project dir");

    let mut cmd = create_isolated_cli_std_command(isolated_home, credentials_path);
    cmd.current_dir(&project_dir).args([
        "init",
        "--yes",
        "--name",
        "dev-app",
        "--schema-mode",
        "sql",
        "--languages",
        "typescript",
        "--server-mode",
        "remote",
        "--server-url",
        server_url,
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
    update_dev_project(&project_dir, |config| {
        config.dev.apply_schema = false;
        config.dev.generate_types = false;
        config.dev.watch = false;
        config.schema.watch = false;
    });

    project_dir
}

pub(crate) fn start_recording_sql_server(
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
            } else if request_line.starts_with("GET /v1/api/auth/me") {
                b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 30\r\nConnection: close\r\n\r\n{\"user\":\"test\",\"role\":\"admin\"}".to_vec()
            } else if request_line.starts_with("POST /v1/api/sql") {
                let payload: serde_json::Value =
                    serde_json::from_slice(body).expect("parse sql request body");
                requests_handle.lock().expect("lock requests").push(RecordedSqlRequest {
                    sql:          payload
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

/// Fake server that tracks migration state across INSERT/UPDATE/SELECT queries.
///
/// Unlike `start_recording_sql_server` (which always returns an empty SELECT
/// result), this server maintains an in-memory migration table so that
/// `has_pending_numbered_migrations` sees migrations as "applied" after the
/// CLI has called `save_server_migration_record`.
pub(crate) fn start_migration_tracking_server(
) -> (String, Arc<Mutex<Vec<RecordedSqlRequest>>>, std::thread::JoinHandle<()>) {
    use std::collections::HashMap as StdHashMap;

    #[derive(Clone, Default)]
    struct FakeMig {
        migration_key: String,
        migration_id:  String,
        namespace:     String,
        name:          String,
        checksum:      String,
        status:        String,
        source:        String,
    }

    // Extract the nth single-quoted SQL string value.  Handles '' escapes.
    fn nth_quoted(sql: &str, n: usize) -> Option<String> {
        let bytes = sql.as_bytes();
        let mut count = 0usize;
        let mut i = 0usize;
        while i < bytes.len() {
            if bytes[i] != b'\'' {
                i += 1;
                continue;
            }
            // Found an opening quote.
            i += 1;
            let mut value = String::new();
            while i < bytes.len() {
                if bytes[i] == b'\'' {
                    if i + 1 < bytes.len() && bytes[i + 1] == b'\'' {
                        value.push('\'');
                        i += 2;
                    } else {
                        i += 1; // closing quote
                        break;
                    }
                } else {
                    value.push(bytes[i] as char);
                    i += 1;
                }
            }
            if count == n {
                return Some(value);
            }
            count += 1;
        }
        None
    }

    fn build_migrations_response(records: &[FakeMig]) -> String {
        let schema = r#"[{"name":"migration_key","data_type":"Text","index":0},{"name":"migration_id","data_type":"Text","index":1},{"name":"namespace","data_type":"Text","index":2},{"name":"name","data_type":"Text","index":3},{"name":"checksum","data_type":"Text","index":4},{"name":"status","data_type":"Text","index":5},{"name":"started_at","data_type":"Text","index":6},{"name":"finished_at","data_type":"Text","index":7},{"name":"error_message","data_type":"Text","index":8},{"name":"source","data_type":"Text","index":9},{"name":"kalam_version","data_type":"Text","index":10}]"#;
        let rows: Vec<String> = records
            .iter()
            .map(|r| {
                format!(
                    r#"["{}","{}","{}","{}","{}","{}",null,null,null,"{}",null]"#,
                    r.migration_key.replace('"', "\\\""),
                    r.migration_id.replace('"', "\\\""),
                    r.namespace.replace('"', "\\\""),
                    r.name.replace('"', "\\\""),
                    r.checksum.replace('"', "\\\""),
                    r.status.replace('"', "\\\""),
                    r.source.replace('"', "\\\""),
                )
            })
            .collect();
        let body = format!(
            r#"{{"status":"success","results":[{{"schema":{},"rows":[{}],"row_count":{}}}]}}"#,
            schema,
            rows.join(","),
            records.len()
        );
        body
    }

    fn make_response(body: &str) -> Vec<u8> {
        format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: \
             {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        )
        .into_bytes()
    }

    let listener = TcpListener::bind("127.0.0.1:0").expect("bind tracking server");
    let addr = listener.local_addr().expect("tracking server addr");
    let url = format!("http://{}", addr);
    let requests = Arc::new(Mutex::new(Vec::<RecordedSqlRequest>::new()));
    let requests_clone = Arc::clone(&requests);
    let migrations: Arc<Mutex<StdHashMap<String, FakeMig>>> =
        Arc::new(Mutex::new(StdHashMap::new()));

    let handle = std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { break };
            let mut buffer = Vec::new();
            let mut chunk = [0u8; 4096];
            let header_end;
            loop {
                let read = stream.read(&mut chunk).expect("read chunk");
                if read == 0 {
                    return;
                }
                buffer.extend_from_slice(&chunk[..read]);
                if let Some(pos) = buffer.windows(4).position(|w| w == b"\r\n\r\n") {
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
                let read = stream.read(&mut chunk).expect("read body");
                if read == 0 {
                    break;
                }
                buffer.extend_from_slice(&chunk[..read]);
            }
            let request_line = headers.lines().next().unwrap_or_default().to_string();
            let body =
                &buffer[header_end..header_end + content_length.min(buffer.len() - header_end)];

            let response = if request_line.starts_with("GET /health") {
                make_response(r#"{"status":"ok","version":"test","api_version":"v1"}"#)
            } else if request_line.starts_with("GET /v1/api/auth/me") {
                make_response(r#"{"user":"test","role":"admin"}"#)
            } else if request_line.starts_with("POST /v1/api/sql") {
                let payload: serde_json::Value = serde_json::from_slice(body).unwrap_or_default();
                let sql =
                    payload.get("sql").and_then(|v| v.as_str()).unwrap_or_default().to_string();
                let ns =
                    payload.get("namespace_id").and_then(|v| v.as_str()).map(ToString::to_string);
                requests_clone.lock().expect("lock requests").push(RecordedSqlRequest {
                    sql:          sql.clone(),
                    namespace_id: ns,
                });
                let sql_up = sql.to_uppercase();
                if sql_up.contains("FROM SYSTEM.MIGRATIONS") {
                    // SELECT: return tracked migration state
                    let migs = migrations.lock().expect("lock migs");
                    let records: Vec<FakeMig> = migs.values().cloned().collect();
                    let body = build_migrations_response(&records);
                    make_response(&body)
                } else if sql_up.starts_with("DROP NAMESPACE") {
                    migrations.lock().expect("lock migs").clear();
                    make_response(r#"{"status":"success","results":[]}"#)
                } else if sql_up.starts_with("DELETE FROM SYSTEM.MIGRATIONS") {
                    migrations.lock().expect("lock migs").clear();
                    make_response(r#"{"status":"success","results":[]}"#)
                } else if sql_up.starts_with("INSERT INTO SYSTEM.MIGRATIONS") {
                    // Parse and record: nth_quoted order is
                    // 0=key 1=id 2=ns 3=name 4=checksum 5=status 6=source
                    if let (Some(key), Some(mid), Some(mns), Some(name), Some(ck), Some(st)) = (
                        nth_quoted(&sql, 0),
                        nth_quoted(&sql, 1),
                        nth_quoted(&sql, 2),
                        nth_quoted(&sql, 3),
                        nth_quoted(&sql, 4),
                        nth_quoted(&sql, 5),
                    ) {
                        let src = nth_quoted(&sql, 6).unwrap_or_else(|| mid.clone());
                        migrations.lock().expect("lock migs").insert(
                            key.clone(),
                            FakeMig {
                                migration_key: key,
                                migration_id: mid,
                                namespace: mns,
                                name,
                                checksum: ck,
                                status: st,
                                source: src,
                            },
                        );
                    }
                    make_response(
                        r#"{"status":"success","results":[{"schema":[],"rows":[],"row_count":1}]}"#,
                    )
                } else if sql_up.starts_with("UPDATE SYSTEM.MIGRATIONS") {
                    // Extract new status (always 4th quoted string: ns, name,
                    // checksum, status) and migration_key from the WHERE clause
                    // (robust against kalam_version being non-NULL at position 5).
                    let new_status = nth_quoted(&sql, 3);
                    let key = sql
                        .find("WHERE migration_key = '")
                        .and_then(|pos| nth_quoted(&sql[pos..], 0));
                    if let (Some(new_status), Some(key)) = (new_status, key) {
                        if let Some(rec) = migrations.lock().expect("lock migs").get_mut(&key) {
                            rec.status = new_status;
                        }
                    }
                    make_response(
                        r#"{"status":"success","results":[{"schema":[],"rows":[],"row_count":1}]}"#,
                    )
                } else {
                    make_response(r#"{"status":"success","results":[]}"#)
                }
            } else {
                b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_vec()
            };
            let _ = stream.write_all(&response);
            let _ = stream.flush();
        }
    });

    (url, requests, handle)
}

fn wait_for_file_contains(path: &std::path::Path, needle: &str, timeout: Duration) -> bool {
    let start = std::time::Instant::now();
    while start.elapsed() < timeout {
        if path
            .is_file()
            .then(|| fs::read_to_string(path).ok())
            .flatten()
            .is_some_and(|contents| contents.contains(needle))
        {
            return true;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    false
}

fn wait_for_file_count(
    path: &std::path::Path,
    needle: &str,
    expected_count: usize,
    timeout: Duration,
) -> bool {
    let start = std::time::Instant::now();
    while start.elapsed() < timeout {
        if fs::read_to_string(path)
            .ok()
            .is_some_and(|contents| contents.matches(needle).count() >= expected_count)
        {
            return true;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    false
}

fn wait_for_file_absent(path: &std::path::Path, timeout: Duration) -> bool {
    let start = std::time::Instant::now();
    while start.elapsed() < timeout {
        if !path.exists() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    false
}

fn wait_for_recorded_requests<F>(
    requests: &Arc<Mutex<Vec<RecordedSqlRequest>>>,
    mut predicate: F,
    timeout: Duration,
) -> bool
where
    F: FnMut(&[RecordedSqlRequest]) -> bool,
{
    let start = std::time::Instant::now();
    while start.elapsed() < timeout {
        let recorded = requests.lock().expect("lock requests");
        if predicate(&recorded) {
            return true;
        }
        drop(recorded);
        std::thread::sleep(Duration::from_millis(50));
    }
    false
}

pub(crate) fn wait_for_recorded_sql_contains(
    requests: &Arc<Mutex<Vec<RecordedSqlRequest>>>,
    needle: &str,
    timeout: Duration,
) -> bool {
    let upper = needle.to_ascii_uppercase();
    wait_for_recorded_requests(
        requests,
        |recorded| recorded.iter().any(|request| request.sql.to_ascii_uppercase().contains(&upper)),
        timeout,
    )
}

fn wait_for_numbered_migration(
    migrations_dir: &std::path::Path,
    needle: &str,
    timeout: Duration,
) -> Option<std::path::PathBuf> {
    let start = std::time::Instant::now();
    while start.elapsed() < timeout {
        let entries = fs::read_dir(migrations_dir).expect("read migrations dir");
        for entry in entries.filter_map(|entry| entry.ok()) {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().to_string();
            if name.chars().next().is_some_and(|ch| ch.is_ascii_digit())
                && path
                    .is_file()
                    .then(|| fs::read_to_string(&path).ok())
                    .flatten()
                    .is_some_and(|contents| contents.contains(needle))
            {
                return Some(path);
            }
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    None
}

pub(crate) fn create_isolated_cli_std_command(
    home: &std::path::Path,
    credentials_path: &std::path::Path,
) -> std::process::Command {
    let mut cmd = std::process::Command::new(crate::common::kalam_bin());
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

#[test]
fn test_project_workflow_dev_migration_recovery_abort_stops_dev_session() {
    let temp = TempDir::new().expect("temp dir");
    let project_dir = temp.path().join("dev-abort-migration-app");
    fs::create_dir_all(&project_dir).expect("create project dir");
    let isolated_home = temp.path().join("home");
    fs::create_dir_all(isolated_home.join(".kalam")).expect("create isolated home");
    let credentials_path = isolated_home.join(".kalam/credentials.toml");
    store_test_dev_credentials(&credentials_path);

    let (server_url, _requests, _server_handle) = start_migration_tracking_server();

    let mut init = create_isolated_cli_std_command(&isolated_home, &credentials_path);
    init.current_dir(&project_dir).args([
        "init",
        "--yes",
        "--name",
        "dev-abort-migration-app",
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
        config.dev.watch = false;
        config.schema.watch = false;
        config.dev.generate_types = false;
    });

    let migration_path = project_dir.join("kalam/migrations/20250101120000_fail.sql");
    fs::write(&migration_path, "-- UP\nSELECT __KALAM_TEST_SCHEMA_FAIL__;\n\n-- DOWN\n")
        .expect("write failing migration");

    let mut first_run = create_isolated_cli_std_command(&isolated_home, &credentials_path);
    first_run.current_dir(&project_dir).arg("dev");
    let first_output = first_run.output().expect("run first kalam dev");
    assert!(
        first_output.status.success(),
        "first dev run should pause schema and complete without processes\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&first_output.stdout),
        String::from_utf8_lossy(&first_output.stderr)
    );
    let first_stderr = String::from_utf8_lossy(&first_output.stderr);
    assert!(
        first_stderr.contains("schema pipeline paused"),
        "expected first run to record a failed migration\nstderr: {first_stderr}"
    );

    let mut second_run = create_isolated_cli_std_command(&isolated_home, &credentials_path);
    second_run.current_dir(&project_dir).stdin(Stdio::piped()).arg("dev");
    let mut child = second_run.spawn().expect("spawn second kalam dev");
    child
        .stdin
        .take()
        .expect("capture second run stdin")
        .write_all(b"a")
        .expect("send abort decision");
    let second_output = child.wait_with_output().expect("collect second dev output");
    assert!(
        !second_output.status.success(),
        "abort should fail kalam dev\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&second_output.stdout),
        String::from_utf8_lossy(&second_output.stderr)
    );
    let second_stderr = String::from_utf8_lossy(&second_output.stderr);
    assert!(
        second_stderr.contains("migration 20250101120000_fail.sql recovery aborted"),
        "expected migration recovery abort error\nstderr: {second_stderr}"
    );
    assert!(
        second_stderr.contains("schema pipeline aborted; stopping kalam dev"),
        "expected abort to stop dev instead of pausing\nstderr: {second_stderr}"
    );
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
    assert!(stdout.contains("--agent"));
    assert!(stdout.contains("AI coding agents"));
    assert!(stdout.contains("--progress"));
    assert!(stdout.contains("start"));
    assert!(stdout.contains("status"));
    assert!(stdout.contains("logs"));
    assert!(stdout.contains("stop"));
}

#[test]
fn test_project_workflow_dev_streams_prefixed_process_logs_by_default() {
    let temp = TempDir::new().expect("temp dir");
    let isolated_home = temp.path().join("home");
    fs::create_dir_all(isolated_home.join(".kalam")).expect("create isolated home");
    let credentials_path = isolated_home.join(".kalam/credentials.toml");
    store_test_dev_credentials(&credentials_path);
    let (server_url, _requests, _server_handle) = start_recording_sql_server();
    let project_dir = scaffold_dev_project(&temp, &isolated_home, &credentials_path, &server_url);

    let mut cmd = create_isolated_cli_std_command(&isolated_home, &credentials_path);
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
        stderr.contains("[cli] starting kalam dev session"),
        "expected append-only CLI logs\nstderr: {stderr}"
    );
    assert!(
        stderr.contains("[frontend]") || stderr.contains("frontend tick"),
        "expected append-only mode to stream raw process logs\nstderr: {stderr}"
    );
}

#[test]
fn test_project_workflow_dev_verbose_streams_prefixed_process_logs() {
    let temp = TempDir::new().expect("temp dir");
    let isolated_home = temp.path().join("home");
    fs::create_dir_all(isolated_home.join(".kalam")).expect("create isolated home");
    let credentials_path = isolated_home.join(".kalam/credentials.toml");
    store_test_dev_credentials(&credentials_path);
    let (server_url, _requests, _server_handle) = start_recording_sql_server();
    let project_dir = scaffold_dev_project(&temp, &isolated_home, &credentials_path, &server_url);

    let mut cmd = create_isolated_cli_std_command(&isolated_home, &credentials_path);
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
    let isolated_home = temp.path().join("home");
    fs::create_dir_all(isolated_home.join(".kalam")).expect("create isolated home");
    let credentials_path = isolated_home.join(".kalam/credentials.toml");
    store_test_dev_credentials(&credentials_path);

    let mut init = create_isolated_cli_std_command(&isolated_home, &credentials_path);
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

    let (url, _requests, _server_handle) = start_recording_sql_server();
    update_dev_project(&project_dir, |config| {
        config.connection.get_mut("dev").expect("dev env").url = url.clone();
        config.dev.processes.clear();
        config.dev.watch = true;
        config.schema.watch = true;
        config.dev.apply_schema = false;
        config.dev.generate_types = false;
    });

    let mut cmd = create_isolated_cli_std_command(&isolated_home, &credentials_path);
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
        stderr.contains("using existing KalamDB server"),
        "expected reused local server status in dev output\nstderr: {stderr}"
    );
    assert!(
        stderr.contains("already running") || stderr.contains("WARN:"),
        "expected reused local server to be a warning\nstderr: {stderr}"
    );
}

#[test]
fn test_project_workflow_dev_injects_dotenv_into_app_process() {
    let temp = TempDir::new().expect("temp dir");
    let isolated_home = temp.path().join("home");
    fs::create_dir_all(isolated_home.join(".kalam")).expect("create isolated home");
    let credentials_path = isolated_home.join(".kalam/credentials.toml");
    store_test_dev_credentials(&credentials_path);
    let (server_url, _requests, _server_handle) = start_recording_sql_server();
    let project_dir = scaffold_dev_project(&temp, &isolated_home, &credentials_path, &server_url);

    let password = "dotenv-regression-secret";
    fs::write(
        project_dir.join(".env"),
        format!("KALAM_PROFILE=kalam-dev\nKALAM_PASSWORD={password}\n"),
    )
    .expect("write project .env");
    add_dev_processes(
        &project_dir,
        HashMap::from([(
            "app".into(),
            format!(
                "printf 'captured-password=%s\\n' \"$KALAM_PASSWORD\"; while true; do sleep 0.5; \
                 done"
            ),
        )]),
    );

    let mut cmd = create_isolated_cli_std_command(&isolated_home, &credentials_path);
    cmd.current_dir(&project_dir).env_remove("KALAM_PASSWORD").arg("dev");

    let mut child = cmd.spawn().expect("spawn kalam dev");
    let timeout = Duration::from_secs(4);
    let exit_status = child.wait_timeout(timeout).expect("wait with timeout");

    if exit_status.is_some() {
        let output = child.wait_with_output().expect("collect output");
        panic!(
            "kalam dev exited early while injecting .env\nstdout: {}\nstderr: {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    let _ = child.kill();
    let output = child.wait_with_output().expect("collect output");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains(&format!("captured-password={password}")),
        "expected kalam dev to inject project .env into [dev.processes]\nstderr: {stderr}"
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
    clear_workflow_url_env_overrides(&mut cmd);
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
fn test_project_workflow_dev_writes_schema_draft_without_applying_schema_sql() {
    let temp = TempDir::new().expect("temp dir");
    let project_dir = temp.path().join("dev-bootstrap-app");
    fs::create_dir_all(&project_dir).expect("create project dir");

    let (server_url, requests, _server_handle) = start_migration_tracking_server();

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
        config.dev.generate_types = false;
    });

    let (mut credential_store, credentials_dir) = create_temp_store();
    credential_store
        .set_credentials(&Credentials::new("kalam-dev".to_string(), "test-dev-jwt".to_string()))
        .expect("store test credentials");

    let mut cmd = create_cli_std_command();
    clear_workflow_url_env_overrides(&mut cmd);
    cmd.current_dir(&project_dir)
        .env("KALAMDB_CREDENTIALS_PATH", credentials_dir.path().join("credentials.toml"))
        .arg("dev");

    let mut child = cmd.spawn().expect("spawn kalam dev");
    let draft_path = project_dir.join("kalam/migrations/_draft.sql");
    let start = std::time::Instant::now();
    while start.elapsed() < Duration::from_secs(5) {
        if draft_path.is_file() {
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }

    let recorded = requests.lock().expect("lock requests").clone();
    let _ = child.kill();
    let output = child.wait_with_output().expect("collect output");
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(
        draft_path.is_file(),
        "expected schema.sql to be written to draft migration\nstderr: {stderr}"
    );
    let draft_sql = fs::read_to_string(&draft_path).expect("read draft migration");
    assert!(
        draft_sql.contains("-- Generated KalamDB schema evolution"),
        "expected semantic schema diff in draft\nsql: {draft_sql}"
    );
    assert!(
        draft_sql.contains("CREATE TABLE users"),
        "expected schema.sql table in draft\nsql: {draft_sql}"
    );
    assert!(
        recorded.iter().all(|request| !request.sql.contains("CREATE TABLE users")),
        "expected kalam dev to defer schema SQL until user applies the draft\nrequests: \
         {recorded:?}"
    );
}

#[test]
fn test_project_workflow_dev_force_on_watch_refreshes_draft_until_user_applies() {
    let temp = TempDir::new().expect("temp dir");
    let project_dir = temp.path().join("dev-watch-force-app");
    fs::create_dir_all(&project_dir).expect("create project dir");

    let (server_url, requests, _server_handle) = start_migration_tracking_server();

    let mut init = create_cli_command();
    init.current_dir(&project_dir).args([
        "init",
        "--yes",
        "--name",
        "dev-watch-force-app",
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
        config.dev.generate_types = false;
    });

    let (mut credential_store, credentials_dir) = create_temp_store();
    credential_store
        .set_credentials(&Credentials::new("kalam-dev".to_string(), "test-dev-jwt".to_string()))
        .expect("store test credentials");

    let schema_path = project_dir.join("schema.sql");
    let mut cmd = create_cli_std_command();
    clear_workflow_url_env_overrides(&mut cmd);
    cmd.current_dir(&project_dir)
        .env_remove("KALAM_PROFILE")
        .env("KALAMDB_CREDENTIALS_PATH", credentials_dir.path().join("credentials.toml"))
        .args(["dev", "--force"]);

    let mut child = cmd.spawn().expect("spawn kalam dev --force");
    let draft_path = project_dir.join("kalam/migrations/_draft.sql");
    let start = std::time::Instant::now();
    loop {
        if draft_path
            .is_file()
            .then(|| fs::read_to_string(&draft_path).ok())
            .flatten()
            .is_some_and(|sql| sql.contains("-- Generated KalamDB schema evolution"))
            || start.elapsed() > Duration::from_secs(5)
        {
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    std::thread::sleep(Duration::from_millis(1_100));

    fs::write(
        &schema_path,
        "-- Example schema for dev-watch-force-app\nCREATE TABLE users (\n  id INTEGER PRIMARY \
         KEY,\n  email TEXT NOT NULL,\n  first_name TEXT,\n  created_at TIMESTAMP\n);\n",
    )
    .expect("update schema first time");

    let start = std::time::Instant::now();
    loop {
        if draft_path
            .is_file()
            .then(|| fs::read_to_string(&draft_path).ok())
            .flatten()
            .is_some_and(|sql| sql.contains("first_name TEXT"))
            || start.elapsed() > Duration::from_secs(5)
        {
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    std::thread::sleep(Duration::from_millis(2_200));

    fs::write(
        &schema_path,
        "-- Example schema for dev-watch-force-app\nCREATE TABLE users (\n  id INTEGER PRIMARY \
         KEY,\n  email TEXT NOT NULL,\n  fi\n  created_at TIMESTAMP\n);\n",
    )
    .expect("write transient invalid schema");

    let start = std::time::Instant::now();
    loop {
        let workflow_log =
            fs::read_to_string(project_dir.join("kalam/cli/logs/kalam.log")).unwrap_or_default();
        if workflow_log.contains("failed to parse statement")
            || start.elapsed() > Duration::from_secs(5)
        {
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    std::thread::sleep(Duration::from_millis(2_200));

    fs::write(
        &schema_path,
        "-- Example schema for dev-watch-force-app\nCREATE TABLE users (\n  id INTEGER PRIMARY \
         KEY,\n  email TEXT NOT NULL,\n  last_name TEXT,\n  created_at TIMESTAMP\n);\n",
    )
    .expect("update schema second time");

    let start = std::time::Instant::now();
    loop {
        if draft_path
            .is_file()
            .then(|| fs::read_to_string(&draft_path).ok())
            .flatten()
            .is_some_and(|sql| sql.contains("last_name TEXT") && !sql.contains("first_name TEXT"))
            || start.elapsed() > Duration::from_secs(5)
        {
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }

    let _ = child.kill();
    let output = child.wait_with_output().expect("collect output");
    let stderr = String::from_utf8_lossy(&output.stderr);
    let workflow_log = fs::read_to_string(project_dir.join("kalam/cli/logs/kalam.log"))
        .expect("read workflow log");

    let migrations_dir = project_dir.join("kalam/migrations");
    let numbered_files: Vec<_> = fs::read_dir(&migrations_dir)
        .expect("read migrations dir")
        .filter_map(|entry| entry.ok())
        .filter(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .chars()
                .next()
                .is_some_and(|ch| ch.is_ascii_digit())
        })
        .collect();

    assert!(
        numbered_files.is_empty(),
        "expected dev watch to leave draft unsealed until the user applies it\nstderr: {stderr}"
    );
    assert!(
        migrations_dir.join("_draft.sql").exists(),
        "expected latest schema diff to remain in draft migration\nstderr: {stderr}"
    );
    let draft_sql =
        fs::read_to_string(migrations_dir.join("_draft.sql")).expect("read draft migration");
    assert!(
        draft_sql.contains("-- Generated KalamDB schema evolution"),
        "expected draft to come from semantic schema diff\nsql: {draft_sql}"
    );
    assert!(
        draft_sql.contains("last_name TEXT") && !draft_sql.contains("first_name TEXT"),
        "expected draft to be overwritten with the latest schema edit\nsql: {draft_sql}\nlog: \
         {workflow_log}"
    );
    assert!(
        workflow_log.contains("schema.sql") && workflow_log.contains(".schema-baseline.sql"),
        "expected schema diff logging in workflow log\nlog: {workflow_log}"
    );
    assert!(
        !stderr.contains("pending migrations require confirmation"),
        "expected --force watch flow to skip interactive confirmation\nstderr: {stderr}"
    );

    let recorded = requests.lock().expect("lock requests").clone();
    assert!(
        recorded.iter().all(|request| !request.sql.contains("last_name TEXT")),
        "expected dev watch to defer draft SQL until user applies it\nrequests: \
         {recorded:?}\nstderr: {stderr}"
    );
}

#[test]
fn test_project_workflow_dev_prompt_refreshes_and_applies_latest_schema_draft() {
    let temp = TempDir::new().expect("temp dir");
    let project_dir = temp.path().join("dev-prompt-refresh-app");
    fs::create_dir_all(&project_dir).expect("create project dir");
    let isolated_home = temp.path().join("home");
    fs::create_dir_all(isolated_home.join(".kalam")).expect("create isolated home");
    let credentials_path = isolated_home.join(".kalam/credentials.toml");

    let (server_url, requests, _server_handle) = start_migration_tracking_server();

    let mut init = create_isolated_cli_std_command(&isolated_home, &credentials_path);
    init.current_dir(&project_dir).args([
        "init",
        "--yes",
        "--name",
        "dev-prompt-refresh-app",
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
        config.dev.generate_types = false;
    });

    let mut credential_store =
        FileCredentialStore::with_path(credentials_path.clone()).expect("create credential store");
    credential_store
        .set_credentials(&Credentials::new("kalam-dev".to_string(), "test-dev-jwt".to_string()))
        .expect("store test credentials");

    let schema_path = project_dir.join("schema.sql");
    let migrations_dir = project_dir.join("kalam/migrations");
    let draft_path = migrations_dir.join("_draft.sql");
    let workflow_log_path = project_dir.join("kalam/cli/logs/kalam.log");

    let captured_stderr = Arc::new(Mutex::new(String::new()));
    let mut cmd = create_isolated_cli_std_command(&isolated_home, &credentials_path);
    cmd.current_dir(&project_dir)
        .env_remove("KALAM_PROFILE")
        .stdin(Stdio::piped())
        .arg("dev");

    let mut child = cmd.spawn().expect("spawn kalam dev");
    let mut child_stdin = child.stdin.take().expect("capture child stdin");
    let mut child_stderr = child.stderr.take().expect("capture child stderr");
    let reader_output = Arc::clone(&captured_stderr);
    let reader = std::thread::spawn(move || {
        let mut buffer = [0_u8; 1024];
        loop {
            let read = child_stderr.read(&mut buffer).expect("read child stderr");
            if read == 0 {
                break;
            }
            let chunk = String::from_utf8_lossy(&buffer[..read]);
            reader_output.lock().expect("lock output").push_str(&chunk);
        }
    });

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        assert!(
            wait_for_file_contains(&draft_path, "CREATE TABLE users", Duration::from_secs(8)),
            "expected initial schema draft to be written\nstderr: {}",
            captured_stderr.lock().expect("lock output")
        );
        assert!(
            wait_for_file_count(
                &workflow_log_path,
                DRAFT_PROMPT_MESSAGE,
                1,
                Duration::from_secs(8),
            ),
            "expected initial apply prompt\nstderr: {}",
            captured_stderr.lock().expect("lock output")
        );

        child_stdin.write_all(b"y").expect("apply initial draft");
        child_stdin.flush().expect("flush initial confirmation");

        let initial_sealed = wait_for_numbered_migration(
            &migrations_dir,
            "CREATE TABLE users",
            Duration::from_secs(8),
        )
        .unwrap_or_else(|| {
            panic!(
                "expected initial draft to be sealed\nstderr: {}\nrequests: {:?}",
                captured_stderr.lock().expect("lock output"),
                requests.lock().expect("lock requests").clone()
            )
        });
        assert!(
            !draft_path.exists(),
            "expected initial _draft.sql to be removed after sealing into {}",
            initial_sealed.display()
        );

        std::thread::sleep(Duration::from_millis(1_200));
        fs::write(
            &schema_path,
            "-- Example schema for dev-prompt-refresh-app\nCREATE TABLE users (\n  id INTEGER \
             PRIMARY KEY,\n  email TEXT NOT NULL,\n  first_name TEXT,\n  created_at \
             TIMESTAMP\n);\n",
        )
        .expect("write first schema edit");
        fs::write(
            &schema_path,
            "-- Example schema for dev-prompt-refresh-app\nCREATE TABLE users (\n  id INTEGER \
             PRIMARY KEY,\n  email TEXT NOT NULL,\n  first_name TEXT,\n  last_name TEXT,\n  \
             created_at TIMESTAMP\n);\n",
        )
        .expect("write second schema edit");
        assert!(
            wait_for_file_contains(&draft_path, "last_name TEXT", Duration::from_secs(8)),
            "expected draft to include latest schema edit\nstderr: {}",
            captured_stderr.lock().expect("lock output")
        );
        assert!(
            wait_for_file_count(
                &workflow_log_path,
                DRAFT_PROMPT_MESSAGE,
                2,
                Duration::from_secs(10),
            ),
            "expected one prompt to be displayed after rapid schema edits\nstderr: {}",
            captured_stderr.lock().expect("lock output")
        );
        assert!(
            fs::read_to_string(&workflow_log_path)
                .expect("read workflow log")
                .contains("last_name TEXT"),
            "expected refreshed prompt summary to show the latest schema column\nlog: {}\nstderr: \
             {}",
            fs::read_to_string(&workflow_log_path).expect("read workflow log"),
            captured_stderr.lock().expect("lock output")
        );

        child_stdin.write_all(b"y").expect("confirm apply");
        child_stdin.flush().expect("flush confirmation");

        let sealed =
            wait_for_numbered_migration(&migrations_dir, "last_name TEXT", Duration::from_secs(8))
                .unwrap_or_else(|| {
                    panic!(
                        "expected latest draft to be sealed into a numbered migration\nstderr: \
                         {}\nrequests: {:?}",
                        captured_stderr.lock().expect("lock output"),
                        requests.lock().expect("lock requests").clone()
                    )
                });
        assert!(
            !draft_path.exists(),
            "expected _draft.sql to be removed after sealing into {}",
            sealed.display()
        );

        let sealed_sql = fs::read_to_string(&sealed).expect("read sealed migration");
        assert!(
            sealed_sql.contains("first_name TEXT") && sealed_sql.contains("last_name TEXT"),
            "expected sealed migration to contain the latest schema writes\nmigration: \
             {sealed_sql}"
        );

        assert!(
            wait_for_recorded_requests(
                &requests,
                |recorded| {
                    recorded.iter().any(|request| request.sql.contains("first_name TEXT"))
                        && recorded.iter().any(|request| request.sql.contains("last_name TEXT"))
                },
                Duration::from_secs(8),
            ),
            "expected apply to execute the latest schema diff SQL\nrequests: {:?}",
            requests.lock().expect("lock requests").clone()
        );
    }));

    let _ = child.kill();
    let _ = child.wait();
    let _ = reader.join();

    if let Err(payload) = result {
        std::panic::resume_unwind(payload);
    }
}

#[test]
fn test_project_workflow_dev_prompt_shows_again_after_apply_and_new_change() {
    let temp = TempDir::new().expect("temp dir");
    let project_dir = temp.path().join("dev-apply-cycle-app");
    fs::create_dir_all(&project_dir).expect("create project dir");
    let isolated_home = temp.path().join("home");
    fs::create_dir_all(isolated_home.join(".kalam")).expect("create isolated home");
    let credentials_path = isolated_home.join(".kalam/credentials.toml");

    // Use the migration-tracking server so that SELECT system.migrations returns
    // the correct applied state after the CLI calls save_server_migration_record.
    // Without this, has_pending_numbered_migrations always returns true and
    // blocks draft generation for the second schema change.
    let (server_url, requests, _server_handle) = start_migration_tracking_server();

    let mut init = create_isolated_cli_std_command(&isolated_home, &credentials_path);
    init.current_dir(&project_dir).args([
        "init",
        "--yes",
        "--name",
        "dev-apply-cycle-app",
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
        config.dev.generate_types = false;
    });

    let mut credential_store =
        FileCredentialStore::with_path(credentials_path.clone()).expect("create credential store");
    credential_store
        .set_credentials(&Credentials::new("kalam-dev".to_string(), "test-dev-jwt".to_string()))
        .expect("store test credentials");

    let schema_path = project_dir.join("schema.sql");
    let migrations_dir = project_dir.join("kalam/migrations");
    let draft_path = migrations_dir.join("_draft.sql");
    let workflow_log_path = project_dir.join("kalam/cli/logs/kalam.log");

    let captured_stderr = Arc::new(Mutex::new(String::new()));
    let mut cmd = create_isolated_cli_std_command(&isolated_home, &credentials_path);
    cmd.current_dir(&project_dir)
        .env_remove("KALAM_PROFILE")
        .stdin(Stdio::piped())
        .arg("dev");

    let mut child = cmd.spawn().expect("spawn kalam dev");
    let mut child_stdin = child.stdin.take().expect("capture child stdin");
    let mut child_stderr = child.stderr.take().expect("capture child stderr");
    let reader_output = Arc::clone(&captured_stderr);
    let reader = std::thread::spawn(move || {
        let mut buffer = [0_u8; 1024];
        loop {
            let read = child_stderr.read(&mut buffer).expect("read child stderr");
            if read == 0 {
                break;
            }
            let chunk = String::from_utf8_lossy(&buffer[..read]);
            reader_output.lock().expect("lock output").push_str(&chunk);
        }
    });

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        // ── Phase 1: initial draft + prompt ───────────────────────────────────
        assert!(
            wait_for_file_contains(&draft_path, "CREATE TABLE users", Duration::from_secs(8)),
            "expected initial draft to be written\nstderr: {}",
            captured_stderr.lock().expect("lock output")
        );
        assert!(
            wait_for_file_count(
                &workflow_log_path,
                DRAFT_PROMPT_MESSAGE,
                1,
                Duration::from_secs(8)
            ),
            "expected first apply prompt\nstderr: {}",
            captured_stderr.lock().expect("lock output")
        );

        // ── Phase 2: user confirms → migration applied ────────────────────────
        child_stdin.write_all(b"y").expect("send y to apply first draft");
        child_stdin.flush().expect("flush confirmation");

        let first_sealed = wait_for_numbered_migration(
            &migrations_dir,
            "CREATE TABLE users",
            Duration::from_secs(8),
        )
        .unwrap_or_else(|| {
            panic!(
                "expected first draft to be sealed after applying\nstderr: {}\nrequests: {:?}",
                captured_stderr.lock().expect("lock output"),
                requests.lock().expect("lock requests").clone()
            )
        });
        assert!(
            !draft_path.exists(),
            "expected _draft.sql to be removed after first apply\nsealed: {}",
            first_sealed.display()
        );

        // ── Phase 3: second schema change should trigger a new prompt ─────────
        // Add a new column that wasn't in the initial template (which already
        // has email and created_at), so that the diff produces a non-empty draft.
        std::thread::sleep(Duration::from_millis(1_200));
        fs::write(
            &schema_path,
            "-- Example schema for dev-apply-cycle-app\nCREATE TABLE users (\n  id INTEGER \
             PRIMARY KEY,\n  email TEXT NOT NULL,\n  username TEXT,\n  created_at TIMESTAMP\n);\n",
        )
        .expect("write second schema change");

        assert!(
            wait_for_file_contains(&draft_path, "username TEXT", Duration::from_secs(10)),
            "expected second draft to contain the new column\nstderr: {}\nlog: {}\nrequests: {:?}",
            captured_stderr.lock().expect("lock output"),
            fs::read_to_string(&workflow_log_path).unwrap_or_default(),
            requests.lock().expect("lock requests").clone(),
        );
        assert!(
            wait_for_file_count(
                &workflow_log_path,
                DRAFT_PROMPT_MESSAGE,
                2,
                Duration::from_secs(10)
            ),
            "expected apply prompt to reappear after second schema change\nstderr: {}\nlog: {}",
            captured_stderr.lock().expect("lock output"),
            fs::read_to_string(&workflow_log_path).unwrap_or_default()
        );

        // ── Phase 4: user applies the second draft ────────────────────────────
        child_stdin.write_all(b"y").expect("send y to apply second draft");
        child_stdin.flush().expect("flush second confirmation");

        let second_sealed =
            wait_for_numbered_migration(&migrations_dir, "username TEXT", Duration::from_secs(10))
                .unwrap_or_else(|| {
                    panic!(
                        "expected second draft to be sealed\nstderr: {}\nrequests: {:?}",
                        captured_stderr.lock().expect("lock output"),
                        requests.lock().expect("lock requests").clone()
                    )
                });

        let second_sql = fs::read_to_string(&second_sealed).expect("read second migration");
        assert!(
            second_sql.contains("username TEXT"),
            "sealed migration should contain the username column\nmigration: {second_sql}"
        );

        assert!(
            wait_for_file_absent(&draft_path, Duration::from_secs(8)),
            "expected _draft.sql to be removed after second apply\nstderr: {}\nlog: {}",
            captured_stderr.lock().expect("lock output"),
            fs::read_to_string(&workflow_log_path).unwrap_or_default()
        );

        assert!(
            wait_for_recorded_sql_contains(&requests, "USERNAME TEXT", Duration::from_secs(8)),
            "expected second apply to send username column SQL to server\nrequests: {:?}",
            requests.lock().expect("lock requests").clone()
        );

        // ── Phase 5: third schema change must still generate and prompt ───────
        fs::write(
            &schema_path,
            "-- Example schema for dev-apply-cycle-app\nCREATE TABLE users (\n  id INTEGER \
             PRIMARY KEY,\n  email TEXT NOT NULL,\n  username TEXT,\n  display_name TEXT,\n  \
             created_at TIMESTAMP\n);\n",
        )
        .expect("write third schema change");

        assert!(
            wait_for_file_contains(&draft_path, "display_name TEXT", Duration::from_secs(10)),
            "expected third draft to contain the new column\nstderr: {}\nlog: {}\nrequests: {:?}",
            captured_stderr.lock().expect("lock output"),
            fs::read_to_string(&workflow_log_path).unwrap_or_default(),
            requests.lock().expect("lock requests").clone(),
        );
        assert!(
            wait_for_file_count(
                &workflow_log_path,
                DRAFT_PROMPT_MESSAGE,
                3,
                Duration::from_secs(10)
            ),
            "expected apply prompt to reappear after third schema change\nstderr: {}\nlog: {}",
            captured_stderr.lock().expect("lock output"),
            fs::read_to_string(&workflow_log_path).unwrap_or_default()
        );

        child_stdin.write_all(b"y").expect("send y to apply third draft");
        child_stdin.flush().expect("flush third confirmation");

        let third_sealed = wait_for_numbered_migration(
            &migrations_dir,
            "display_name TEXT",
            Duration::from_secs(10),
        )
        .unwrap_or_else(|| {
            panic!(
                "expected third draft to be sealed\nstderr: {}\nrequests: {:?}",
                captured_stderr.lock().expect("lock output"),
                requests.lock().expect("lock requests").clone()
            )
        });

        let third_sql = fs::read_to_string(&third_sealed).expect("read third migration");
        assert!(
            third_sql.contains("display_name TEXT"),
            "sealed migration should contain the display_name column\nmigration: {third_sql}"
        );

        assert!(
            wait_for_recorded_sql_contains(&requests, "DISPLAY_NAME TEXT", Duration::from_secs(8)),
            "expected third apply to send display_name column SQL to server\nrequests: {:?}",
            requests.lock().expect("lock requests").clone()
        );
    }));

    let _ = child.kill();
    let _ = child.wait();
    let _ = reader.join();

    if let Err(payload) = result {
        std::panic::resume_unwind(payload);
    }
}

#[test]
fn test_project_workflow_dev_prompt_cancel_stops_dev_session() {
    let temp = TempDir::new().expect("temp dir");
    let project_dir = temp.path().join("dev-cancel-draft-app");
    fs::create_dir_all(&project_dir).expect("create project dir");
    let isolated_home = temp.path().join("home");
    fs::create_dir_all(isolated_home.join(".kalam")).expect("create isolated home");
    let credentials_path = isolated_home.join(".kalam/credentials.toml");

    let (server_url, _requests, _server_handle) = start_recording_sql_server();

    let mut init = create_isolated_cli_std_command(&isolated_home, &credentials_path);
    init.current_dir(&project_dir).args([
        "init",
        "--yes",
        "--name",
        "dev-cancel-draft-app",
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
        config.dev.generate_types = false;
    });

    let mut credential_store =
        FileCredentialStore::with_path(credentials_path.clone()).expect("create credential store");
    credential_store
        .set_credentials(&Credentials::new("kalam-dev".to_string(), "test-dev-jwt".to_string()))
        .expect("store test credentials");

    let draft_path = project_dir.join("kalam/migrations/_draft.sql");
    let workflow_log_path = project_dir.join("kalam/cli/logs/kalam.log");

    let mut cmd = create_isolated_cli_std_command(&isolated_home, &credentials_path);
    cmd.current_dir(&project_dir)
        .env_remove("KALAM_PROFILE")
        .stdin(Stdio::piped())
        .arg("dev");

    let mut child = cmd.spawn().expect("spawn kalam dev");
    let mut child_stdin = child.stdin.take().expect("capture child stdin");

    assert!(
        wait_for_file_contains(&draft_path, "CREATE TABLE users", Duration::from_secs(8)),
        "expected initial draft to be written"
    );
    assert!(
        wait_for_file_count(&workflow_log_path, DRAFT_PROMPT_MESSAGE, 1, Duration::from_secs(8)),
        "expected draft prompt to be shown\nlog: {}",
        fs::read_to_string(&workflow_log_path).unwrap_or_default()
    );

    child_stdin.write_all(b"n").expect("cancel prompt");
    child_stdin.flush().expect("flush cancellation");
    drop(child_stdin);

    let status = child
        .wait_timeout(Duration::from_secs(8))
        .expect("wait for child")
        .unwrap_or_else(|| {
            let _ = child.kill();
            panic!("expected kalam dev to exit after cancelling schema draft prompt");
        });
    assert!(status.success(), "cancel should stop dev cleanly, got status {status}");
    assert!(draft_path.exists(), "cancel should leave the draft pending for later");
}

#[test]
fn test_project_workflow_dev_prompt_reset_restarts_schema_from_clean_namespace() {
    let temp = TempDir::new().expect("temp dir");
    let project_dir = temp.path().join("dev-reset-draft-app");
    fs::create_dir_all(&project_dir).expect("create project dir");
    let isolated_home = temp.path().join("home");
    fs::create_dir_all(isolated_home.join(".kalam")).expect("create isolated home");
    let credentials_path = isolated_home.join(".kalam/credentials.toml");

    let (server_url, requests, _server_handle) = start_migration_tracking_server();

    let mut init = create_isolated_cli_std_command(&isolated_home, &credentials_path);
    init.current_dir(&project_dir).args([
        "init",
        "--yes",
        "--name",
        "dev-reset-draft-app",
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
        config.dev.generate_types = false;
    });

    store_test_dev_credentials(&credentials_path);

    let schema_path = project_dir.join("schema.sql");
    let migrations_dir = project_dir.join("kalam/migrations");
    let draft_path = migrations_dir.join("_draft.sql");
    let workflow_log_path = project_dir.join("kalam/cli/logs/kalam.log");

    let captured_stderr = Arc::new(Mutex::new(String::new()));
    let mut cmd = create_isolated_cli_std_command(&isolated_home, &credentials_path);
    cmd.current_dir(&project_dir)
        .env_remove("KALAM_PROFILE")
        .stdin(Stdio::piped())
        .arg("dev");

    let mut child = cmd.spawn().expect("spawn kalam dev");
    let mut child_stdin = child.stdin.take().expect("capture child stdin");
    let mut child_stderr = child.stderr.take().expect("capture child stderr");
    let reader_output = Arc::clone(&captured_stderr);
    let reader = std::thread::spawn(move || {
        let mut buffer = [0_u8; 1024];
        loop {
            let read = child_stderr.read(&mut buffer).expect("read child stderr");
            if read == 0 {
                break;
            }
            let chunk = String::from_utf8_lossy(&buffer[..read]);
            reader_output.lock().expect("lock output").push_str(&chunk);
        }
    });

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        assert!(
            wait_for_file_contains(&draft_path, "CREATE TABLE users", Duration::from_secs(8)),
            "expected initial schema draft\nstderr: {}",
            captured_stderr.lock().expect("lock output")
        );
        child_stdin.write_all(b"y").expect("apply initial draft");
        child_stdin.flush().expect("flush first apply");

        let first_sealed = wait_for_numbered_migration(
            &migrations_dir,
            "CREATE TABLE users",
            Duration::from_secs(8),
        )
        .unwrap_or_else(|| {
            panic!(
                "expected initial migration to be sealed\nstderr: {}\nrequests: {:?}",
                captured_stderr.lock().expect("lock output"),
                requests.lock().expect("lock requests").clone()
            )
        });
        assert!(first_sealed.exists(), "expected first migration file to exist before reset");

        std::thread::sleep(Duration::from_millis(1_200));
        fs::write(
            &schema_path,
            "-- Example schema for dev-reset-draft-app\nCREATE TABLE users (\n  id INTEGER \
             PRIMARY KEY,\n  email TEXT NOT NULL,\n  reset_column TEXT,\n  created_at \
             TIMESTAMP\n);\n",
        )
        .expect("write schema change before reset");
        assert!(
            wait_for_file_contains(&draft_path, "reset_column TEXT", Duration::from_secs(10)),
            "expected reset prompt draft\nstderr: {}\nlog: {}",
            captured_stderr.lock().expect("lock output"),
            fs::read_to_string(&workflow_log_path).unwrap_or_default()
        );

        child_stdin.write_all(b"r").expect("select reset");
        child_stdin.flush().expect("flush reset selection");

        let reset_sealed = wait_for_numbered_migration(
            &migrations_dir,
            "reset_column TEXT",
            Duration::from_secs(10),
        )
        .unwrap_or_else(|| {
            panic!(
                "expected reset to rebuild and apply schema from scratch\nstderr: {}\nlog: \
                 {}\nrequests: {:?}",
                captured_stderr.lock().expect("lock output"),
                fs::read_to_string(&workflow_log_path).unwrap_or_default(),
                requests.lock().expect("lock requests").clone()
            )
        });

        let numbered_migrations = fs::read_dir(&migrations_dir)
            .expect("read migrations after reset")
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.path())
            .filter(|path| {
                path.file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.chars().next().is_some_and(|ch| ch.is_ascii_digit()))
            })
            .collect::<Vec<_>>();
        assert_eq!(
            numbered_migrations.len(),
            1,
            "reset should leave only the fresh migration\nmigrations: {numbered_migrations:?}"
        );
        assert!(
            reset_sealed
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("0001_")),
            "reset should restart migration numbering from 0001, got {}",
            reset_sealed.display()
        );
        let reset_sql = fs::read_to_string(&reset_sealed).expect("read reset migration");
        assert!(
            reset_sql.contains("CREATE TABLE users") && reset_sql.contains("reset_column TEXT"),
            "reset migration should recreate the current schema\nmigration: {reset_sql}"
        );

        assert!(
            wait_for_recorded_sql_contains(&requests, "DROP NAMESPACE", Duration::from_secs(8)),
            "reset should drop the namespace before rebuilding\nrequests: {:?}",
            requests.lock().expect("lock requests").clone()
        );
        assert!(
            wait_for_recorded_requests(
                &requests,
                |recorded| {
                    recorded.iter().any(|request| {
                        request.sql.contains("CREATE TABLE users")
                            && request.sql.contains("reset_column TEXT")
                    })
                },
                Duration::from_secs(8),
            ),
            "reset should apply the current schema as a fresh create\nrequests: {:?}",
            requests.lock().expect("lock requests").clone()
        );
        let recorded = requests.lock().expect("lock requests").clone();
        assert!(
            recorded.iter().all(|request| {
                let sql = request.sql.to_ascii_uppercase();
                !(sql.contains("DELETE") && sql.contains("SYSTEM.MIGRATIONS"))
            }),
            "reset should not delete system.migrations directly\nrequests: {recorded:?}"
        );
    }));

    let _ = child.kill();
    let _ = child.wait();
    let _ = reader.join();

    if let Err(payload) = result {
        std::panic::resume_unwind(payload);
    }
}
