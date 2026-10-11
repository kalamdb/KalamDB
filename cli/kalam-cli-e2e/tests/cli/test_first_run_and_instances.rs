//! Regression coverage for a first project, a cloud OIDC login, and several
//! local servers at once.
//!
//! These tests talk to loopback mocks through an isolated `HOME` and
//! credential file. They never use the developer's `~/.kalam` directory.

use std::{
    fs,
    io::{Read, Write},
    net::TcpListener,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{Arc, Mutex},
    time::Duration,
};

use kalam_cli::workflow::project::config::KalamProjectConfig;
use serde_json::Value;

use super::test_project_workflow_dev::{
    create_isolated_cli_std_command, update_dev_project, write_owned_instance,
};
use crate::common::*;

const FAR_FUTURE: &str = "2099-01-01T00:00:00Z";
const LONG_AGO: &str = "2000-01-01T00:00:00Z";

#[derive(Clone)]
struct ServerMode {
    local_enabled: bool,
    password:      String,
}

struct Hit {
    request:       String,
    body:          String,
    authorization: String,
}

fn start_mock_server(mode: ServerMode) -> (String, Arc<Mutex<ServerMode>>, Arc<Mutex<Vec<Hit>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock server");
    let url = format!("http://{}", listener.local_addr().expect("mock addr"));
    let mode = Arc::new(Mutex::new(mode));
    let hits = Arc::new(Mutex::new(Vec::new()));
    let mode_thread = Arc::clone(&mode);
    let hits_thread = Arc::clone(&hits);
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else {
                continue;
            };
            let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
            let Some((request, headers, body)) = read_http(&mut stream) else {
                continue;
            };
            let authorization = header_value(&headers, "authorization").unwrap_or_default();
            hits_thread.lock().expect("hits").push(Hit {
                request: request.clone(),
                body: body.clone(),
                authorization,
            });
            let response = respond(&request, &body, &mode_thread.lock().expect("mode"));
            let _ = stream.write_all(&response);
            let _ = stream.flush();
        }
    });
    (url, mode, hits)
}

fn read_http(stream: &mut std::net::TcpStream) -> Option<(String, String, String)> {
    let mut buffer = Vec::new();
    let mut chunk = [0_u8; 4096];
    let header_end = loop {
        let read = stream.read(&mut chunk).ok()?;
        if read == 0 {
            return None;
        }
        buffer.extend_from_slice(&chunk[..read]);
        if let Some(pos) = buffer.windows(4).position(|window| window == b"\r\n\r\n") {
            break pos + 4;
        }
    };
    let headers = String::from_utf8_lossy(&buffer[..header_end]).to_string();
    let content_length = header_value(&headers, "content-length")
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(0);
    while buffer.len() < header_end + content_length {
        let read = stream.read(&mut chunk).ok()?;
        if read == 0 {
            break;
        }
        buffer.extend_from_slice(&chunk[..read]);
    }
    let body = String::from_utf8_lossy(
        &buffer[header_end..header_end + content_length.min(buffer.len() - header_end)],
    )
    .to_string();
    let request = headers.lines().next().unwrap_or_default().to_string();
    Some((request, headers, body))
}

fn header_value(headers: &str, name: &str) -> Option<String> {
    headers.lines().find_map(|line| {
        let (key, value) = line.split_once(':')?;
        key.eq_ignore_ascii_case(name).then(|| value.trim().to_string())
    })
}

fn respond(request: &str, body: &str, mode: &ServerMode) -> Vec<u8> {
    if request.starts_with("GET /health") || request.starts_with("GET /v1/api/healthcheck") {
        return http_ok(r#"{"status":"ok","version":"test","api_version":"v1"}"#);
    }
    if request.starts_with("GET /v1/api/auth/login-options") {
        return http_ok(&format!(r#"{{"local":{{"enabled":{}}}}}"#, mode.local_enabled));
    }
    if request.starts_with("GET /v1/api/auth/me") {
        return http_ok(
            r#"{"user":{"id":"root","role":"dba","name":"root","email":null,"created_at":"2026-01-01T00:00:00Z","updated_at":"2026-01-01T00:00:00Z"},"admin_ui_access":true}"#,
        );
    }
    if request.starts_with("POST /v1/api/auth/login") {
        let password = serde_json::from_str::<Value>(body)
            .ok()
            .and_then(|value| value.get("password").and_then(Value::as_str).map(str::to_string))
            .unwrap_or_default();
        if mode.password.is_empty() || password != mode.password {
            return http("401 Unauthorized", r#"{"error":"invalid credentials"}"#);
        }
        return http_ok(
            r#"{"user":{"id":"root","role":"dba","name":"root","email":null,"created_at":"2026-01-01T00:00:00Z","updated_at":"2026-01-01T00:00:00Z"},"admin_ui_access":true,"expires_at":"2099-01-01T00:00:00Z","access_token":"access-root","refresh_token":"refresh-root","refresh_expires_at":"2099-01-02T00:00:00Z"}"#,
        );
    }
    if request.starts_with("POST /v1/api/sql") {
        return http_ok(r#"{"status":"success","results":[]}"#);
    }
    http("404 Not Found", r#"{"error":"not found"}"#)
}

fn http_ok(body: &str) -> Vec<u8> {
    http("200 OK", body)
}

fn http(status: &str, body: &str) -> Vec<u8> {
    format!(
        "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: \
         close\r\n\r\n{body}",
        body.len()
    )
    .into_bytes()
}

struct IsolatedProject {
    dir:              PathBuf,
    home:             PathBuf,
    credentials_path: PathBuf,
    namespace:        String,
}

fn init_project(home: &Path, root: &Path, name: &str, template: Option<&str>) -> IsolatedProject {
    let dir = root.join(name);
    fs::create_dir_all(&dir).expect("project dir");
    fs::create_dir_all(home.join(".kalam")).expect("home");
    let credentials_path = home.join(".kalam/credentials.toml");
    let mut init = cli(&home, &credentials_path);
    init.current_dir(&dir).args([
        "init",
        "--yes",
        "--name",
        name,
        "--schema-mode",
        "sql",
        "--languages",
        "typescript,dart",
        "--server-mode",
        "local",
    ]);
    if let Some(template) = template {
        init.args(["--template", template]);
    }
    let output = init.output().expect("init");
    assert!(output.status.success(), "init {name} failed\n{}", output_text(&output));
    let config = KalamProjectConfig::load_from_path(&dir.join("kalam.toml")).expect("kalam.toml");
    let namespace = config.connection.get("dev").expect("dev connection").namespace.to_string();
    IsolatedProject {
        dir,
        home: home.to_path_buf(),
        credentials_path,
        namespace,
    }
}

fn point_project_at(project: &IsolatedProject, server_url: &str, password: &str) {
    update_dev_project(&project.dir, |config| {
        config.connection.get_mut("dev").expect("dev connection").url = server_url.to_string();
    });
    let server_toml = project.dir.join("kalam/server/server.toml");
    fs::write(&server_toml, format!("[auth]\nroot_password = \"{password}\"\n"))
        .expect("server.toml");
}

fn cli(home: &Path, credentials_path: &Path) -> Command {
    let mut command = create_isolated_cli_std_command(home, credentials_path);
    command.stdin(Stdio::null());
    command
}

fn run(project: &IsolatedProject, args: &[&str]) -> (bool, String) {
    let mut command = cli(&project.home, &project.credentials_path);
    command.current_dir(&project.dir).args(args);
    let output = command.output().expect("kalam");
    (output.status.success(), output_text(&output))
}

fn output_text(output: &std::process::Output) -> String {
    format!(
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

fn delete_login(path: &Path, instance: &str) {
    let mut store = FileCredentialStore::with_path(path.to_path_buf()).expect("credential store");
    store.delete_credentials(instance).expect("delete credentials");
}

fn save_login(path: &Path, instance: &str, token: &str, server_url: &str, expires_at: &str) {
    let mut store = FileCredentialStore::with_path(path.to_path_buf()).expect("credential store");
    store
        .set_credentials(&Credentials::with_details(
            instance.to_string(),
            token.to_string(),
            "root",
            expires_at.to_string(),
            Some(server_url.to_string()),
        ))
        .expect("save credentials");
}

fn clear_hits(hits: &Mutex<Vec<Hit>>) {
    hits.lock().expect("hits").clear();
}

fn sql_hits(hits: &Mutex<Vec<Hit>>) -> Vec<Value> {
    hits.lock()
        .expect("hits")
        .iter()
        .filter(|hit| hit.request.starts_with("POST /v1/api/sql"))
        .map(|hit| serde_json::from_str(&hit.body).unwrap_or(Value::Null))
        .collect()
}

fn user_sql(hits: &Mutex<Vec<Hit>>) -> Vec<Value> {
    sql_hits(hits)
        .into_iter()
        .filter(|query| query["sql"] != "SELECT 1 AS auth_check")
        .collect()
}

fn assert_sql_namespace(hits: &Mutex<Vec<Hit>>, namespace: &str) {
    let queries = user_sql(hits);
    assert!(!queries.is_empty(), "expected SQL");
    assert!(
        queries.iter().all(|query| query["namespace_id"] == namespace),
        "namespace mismatch\n{queries:?}"
    );
}

fn assert_sql_without_namespace(hits: &Mutex<Vec<Hit>>) {
    let queries = user_sql(hits);
    assert!(!queries.is_empty(), "expected SQL");
    assert!(
        queries.iter().all(|query| query.get("namespace_id").is_none()),
        "cloud SQL inherited a namespace\n{queries:?}"
    );
}

fn saw_path(hits: &Mutex<Vec<Hit>>, prefix: &str) -> bool {
    hits.lock().expect("hits").iter().any(|hit| hit.request.starts_with(prefix))
}

fn authorizations(hits: &Mutex<Vec<Hit>>) -> String {
    hits.lock()
        .expect("hits")
        .iter()
        .map(|hit| hit.authorization.clone())
        .collect::<Vec<_>>()
        .join("\n")
}

fn login_passwords(hits: &Mutex<Vec<Hit>>) -> Vec<String> {
    hits.lock()
        .expect("hits")
        .iter()
        .filter(|hit| hit.request.starts_with("POST /v1/api/auth/login"))
        .filter_map(|hit| {
            serde_json::from_str::<Value>(&hit.body)
                .ok()
                .and_then(|value| value.get("password").and_then(Value::as_str).map(str::to_string))
        })
        .collect()
}

fn assert_success(ok: bool, text: &str, what: &str) {
    assert!(ok, "{what} failed\n{text}");
}

fn assert_failure(ok: bool, text: &str, needle: &str, what: &str) {
    assert!(!ok, "{what} succeeded unexpectedly\n{text}");
    assert!(text.contains(needle), "{what} missing `{needle}`\n{text}");
}

fn sql_args(sql: &str) -> Vec<String> {
    vec![
        "--no-color".into(),
        "--connection-timeout".into(),
        "2".into(),
        "--timeout".into(),
        "2".into(),
        "-c".into(),
        sql.into(),
    ]
}

#[test]
fn first_run_sql_uses_the_project_namespace_password_and_generated_names() {
    let root = TempDir::new().expect("temp");
    let home = root.path().join("home");
    let project = init_project(&home, root.path(), "shop-live", None);
    let (server_url, _mode, hits) = start_mock_server(ServerMode {
        local_enabled: true,
        password:      "project-root-secret".into(),
    });
    point_project_at(&project, &server_url, "project-root-secret");
    write_owned_instance(&project.dir, &server_url);
    let (ok, text) = run(&project, &["instances"]);
    assert_success(ok, &text, "register the project server");

    fs::write(
        project.dir.join("schema.sql"),
        "CREATE TABLE users (\n  id INTEGER PRIMARY KEY,\n  email TEXT NOT NULL\n);\n",
    )
    .expect("schema.sql");
    let (generated_ok, generated_text) = run(&project, &["schema", "gen"]);
    assert_success(generated_ok, &generated_text, "schema gen");
    let schema =
        fs::read_to_string(project.dir.join("src/generated/schema.ts")).expect("schema.ts");
    let dart = fs::read_to_string(project.dir.join("lib/generated/kalam.dart")).expect("dart");
    assert!(
        schema.contains(&format!("kTable(\"{}.users\"", project.namespace)),
        "generated table should use the project namespace\n{schema}"
    );
    assert!(!schema.contains("public.users"), "public.users leaked into schema.ts\n{schema}");
    assert!(!schema.contains("pgSchema"), "pgSchema leaked into schema.ts\n{schema}");
    assert!(
        dart.contains(&format!("{}.users", project.namespace)),
        "generated dart should use the project namespace\n{dart}"
    );
    assert!(!dart.contains("public.users"), "public.users leaked into dart\n{dart}");

    clear_hits(&hits);
    let sql = sql_args("SELECT * FROM users");
    let sql_refs: Vec<&str> = sql.iter().map(String::as_str).collect();
    let (ok, text) = run(&project, &sql_refs);
    assert_success(ok, &text, "project SQL");
    assert!(text.contains(&project.namespace), "session should print the namespace\n{text}");
    assert_sql_namespace(&hits, &project.namespace);
    assert_eq!(login_passwords(&hits), vec!["project-root-secret".to_string()]);
    assert!(
        authorizations(&hits).contains("access-root"),
        "SQL should use the token from the server.toml login\n{}",
        authorizations(&hits)
    );

    clear_hits(&hits);
    let mut override_ns = cli(&project.home, &project.credentials_path);
    override_ns
        .current_dir(&project.dir)
        .env("KALAM_NAMESPACE", "tenant_override")
        .args(&sql_refs);
    let output = override_ns.output().expect("namespace override");
    let text = output_text(&output);
    assert_success(output.status.success(), &text, "KALAM_NAMESPACE");
    assert_sql_namespace(&hits, "tenant_override");

    save_login(
        &project.credentials_path,
        "local",
        "jwt-foreign-should-not-leak",
        "https://db.example",
        FAR_FUTURE,
    );
    clear_hits(&hits);
    let (ok, text) = run(&project, &sql_refs);
    assert_success(ok, &text, "foreign local credential");
    assert!(
        text.contains("points at https://db.example"),
        "warn about the other server\n{text}"
    );
    assert!(!authorizations(&hits).contains("jwt-foreign-should-not-leak"));
    assert_eq!(login_passwords(&hits), vec!["project-root-secret".to_string()]);

    let localhost_url = server_url.replace("127.0.0.1", "localhost");
    save_login(
        &project.credentials_path,
        "local",
        "jwt-loopback-token",
        &localhost_url,
        FAR_FUTURE,
    );
    clear_hits(&hits);
    let (ok, text) = run(&project, &sql_refs);
    assert_success(ok, &text, "loopback credential alias");
    assert!(authorizations(&hits).contains("jwt-loopback-token"), "{text}");
    assert!(
        !saw_path(&hits, "POST /v1/api/auth/login"),
        "matching loopback token should not log in again"
    );

    save_login(
        &project.credentials_path,
        "local",
        "jwt-whoami-other-server",
        "https://db.example",
        FAR_FUTURE,
    );
    clear_hits(&hits);
    let (ok, text) = run(&project, &["whoami", "--no-color"]);
    assert_failure(ok, &text, "No authentication credentials", "whoami for a different server");
    assert!(!saw_path(&hits, "GET /v1/api/auth/me"));
    assert!(!saw_path(&hits, "POST /v1/api/auth/refresh"));

    save_login(&project.credentials_path, "local", "jwt-expired", &server_url, LONG_AGO);
    clear_hits(&hits);
    let (ok, text) = run(&project, &["whoami", "--no-color"]);
    assert_failure(ok, &text, "--oidc", "expired credential");
    assert!(!saw_path(&hits, "GET /v1/api/auth/me"));
}

#[test]
fn cloud_oidc_login_stays_separate_from_the_project_database() {
    let root = TempDir::new().expect("temp");
    let home = root.path().join("home");
    let project = init_project(&home, root.path(), "shop-cloud", None);
    let (project_url, project_mode, project_hits) = start_mock_server(ServerMode {
        local_enabled: true,
        password:      "project-root-secret".into(),
    });
    let (cloud_url, cloud_mode, cloud_hits) = start_mock_server(ServerMode {
        local_enabled: true,
        password:      "cloud-password".into(),
    });
    point_project_at(&project, &project_url, "project-root-secret");
    write_owned_instance(&project.dir, &project_url);
    assert!(run(&project, &["instances"]).0, "register project");

    let (ok, text) = run(
        &project,
        &[
            "login",
            "--no-color",
            "--url",
            "https://cloud.example",
            "--user",
            "root",
            "--password",
            "secret",
        ],
    );
    assert_failure(ok, &text, "Name it before signing in", "unnamed remote login");
    assert!(!saw_path(&project_hits, "POST /v1/api/auth/login"));

    let (ok, text) = run(&project, &["login", "--oidc", "--no-color"]);
    assert_failure(ok, &text, "OIDC login is not enabled", "oidc on a password server");
    assert!(text.contains(&project_url), "the message should name this server\n{text}");

    let (ok, text) = run(
        &project,
        &[
            "login",
            "--no-color",
            "--instance",
            "prod",
            "--url",
            &cloud_url,
            "--user",
            "root",
            "--password",
            "cloud-password",
        ],
    );
    assert_success(ok, &text, "named cloud login");
    assert!(text.contains(&cloud_url), "login should announce the cloud server\n{text}");
    assert!(saw_path(&cloud_hits, "POST /v1/api/auth/login"));
    assert!(!saw_path(&project_hits, "POST /v1/api/auth/login"));

    clear_hits(&project_hits);
    clear_hits(&cloud_hits);
    let project_sql = sql_args("SELECT current_user");
    let project_sql: Vec<&str> = project_sql.iter().map(String::as_str).collect();
    let (ok, text) = run(&project, &project_sql);
    assert_success(ok, &text, "project SQL after cloud login");
    assert_sql_namespace(&project_hits, &project.namespace);
    assert!(sql_hits(&cloud_hits).is_empty(), "project SQL reached the cloud");

    clear_hits(&project_hits);
    clear_hits(&cloud_hits);
    let mut prod_sql = sql_args("SELECT current_user");
    prod_sql.insert(0, "prod".into());
    prod_sql.insert(0, "--instance".into());
    let prod_sql: Vec<&str> = prod_sql.iter().map(String::as_str).collect();
    let mut command = cli(&project.home, &project.credentials_path);
    command
        .current_dir(&project.dir)
        .env("KALAM_NAMESPACE", "should_not_apply")
        .args(&prod_sql);
    let output = command.output().expect("prod sql");
    let text = output_text(&output);
    assert_success(output.status.success(), &text, "named cloud SQL");
    assert_sql_without_namespace(&cloud_hits);
    assert!(
        authorizations(&cloud_hits).contains("access-root"),
        "cloud SQL should use the prod login token\n{}",
        authorizations(&cloud_hits)
    );
    assert!(sql_hits(&project_hits).is_empty());

    clear_hits(&project_hits);
    clear_hits(&cloud_hits);
    let (ok, text) = run(
        &project,
        &[
            "--instance",
            "prod",
            "--url",
            &project_url,
            "--no-color",
            "--connection-timeout",
            "2",
            "--timeout",
            "2",
            "-c",
            "SELECT 1",
        ],
    );
    assert_success(ok, &text, "explicit url overrides the named instance");
    assert!(text.contains("overrides `--instance prod`"), "warn that --url won\n{text}");
    assert_sql_namespace(&project_hits, &project.namespace);
    assert!(sql_hits(&cloud_hits).is_empty(), "overridden URL still hit the cloud");

    let port = project_url.rsplit(':').next().expect("port");
    clear_hits(&project_hits);
    let (ok, text) = run(
        &project,
        &[
            "--instance",
            "prod",
            "--host",
            "127.0.0.1",
            "--port",
            port,
            "--no-color",
            "--connection-timeout",
            "2",
            "--timeout",
            "2",
            "-c",
            "SELECT 1",
        ],
    );
    assert_success(ok, &text, "host and port override the named instance");
    assert!(text.contains("overrides `--instance prod`"), "{text}");
    assert_sql_namespace(&project_hits, &project.namespace);

    for (command, label) in [
        (&["db", "migration", "status", "--instance", "prod"][..], "db"),
        (&["functions", "status", "--instance", "prod"][..], "functions"),
        (&["deploy", "--instance", "prod"][..], "deploy"),
        (&["link", "--instance", "prod"][..], "link"),
    ] {
        clear_hits(&cloud_hits);
        let (ok, text) = run(&project, command);
        assert_failure(ok, &text, "aimed at", label);
        assert!(sql_hits(&cloud_hits).is_empty(), "{label} sent SQL to the cloud\n{text}");
    }

    clear_hits(&cloud_hits);
    let (ok, text) = run(&project, &["dev", "status", "--instance", "prod", "--no-color"]);
    assert!(text.contains("kalam dev` still uses this project's database"), "{text}");
    assert!(text.contains(&project_url), "{text}");
    assert!(sql_hits(&cloud_hits).is_empty(), "dev status used the cloud\n{text}");
    let _ = ok;

    let (ok, text) = run(&project, &["up", "--instance", "prod", "--no-color"]);
    assert_failure(ok, &text, "cloud connection", "up of a cloud name");

    save_login(
        &project.credentials_path,
        "local",
        "jwt-project-token",
        &project_url,
        FAR_FUTURE,
    );
    clear_hits(&cloud_hits);
    let (ok, text) = run(
        &project,
        &[
            "login",
            "--no-color",
            "--url",
            &cloud_url,
            "--user",
            "root",
            "--password",
            "cloud-password",
        ],
    );
    assert_failure(ok, &text, "already signed in", "overwrite the default login");
    assert!(!saw_path(&cloud_hits, "POST /v1/api/auth/login"));
    let stored = fs::read_to_string(&project.credentials_path).expect("credentials");
    assert!(stored.contains("jwt-project-token"), "the project login was replaced\n{stored}");
    assert!(stored.contains(&project_url));

    project_mode.lock().expect("mode").local_enabled = false;
    project_mode.lock().expect("mode").password = String::new();
    delete_login(&project.credentials_path, "local");
    clear_hits(&project_hits);
    let (ok, text) = run(&project, &project_sql);
    assert_failure(ok, &text, "does not accept a username and password", "oidc-only SQL");
    assert!(text.contains("--oidc"), "{text}");
    assert!(
        sql_hits(&project_hits).is_empty(),
        "password SQL was sent to an OIDC-only server"
    );

    clear_hits(&project_hits);
    let (ok, text) = run(
        &project,
        &[
            "--user",
            "root",
            "--password",
            "project-root-secret",
            "--no-color",
            "--connection-timeout",
            "2",
            "--timeout",
            "2",
            "-c",
            "SELECT 1",
        ],
    );
    assert_failure(ok, &text, "--oidc", "explicit password on an OIDC-only server");
    assert!(!saw_path(&project_hits, "POST /v1/api/auth/login"));
    assert!(sql_hits(&project_hits).is_empty());

    save_login(&project.credentials_path, "local", "jwt-oidc-session", &project_url, FAR_FUTURE);
    clear_hits(&project_hits);
    let (ok, text) = run(&project, &project_sql);
    assert_success(ok, &text, "stored OIDC token");
    assert!(authorizations(&project_hits).contains("jwt-oidc-session"));
    assert!(!saw_path(&project_hits, "POST /v1/api/auth/login"));

    cloud_mode.lock().expect("mode").local_enabled = false;
    let (instances_ok, instances) = run(&project, &["instances", "--no-color"]);
    assert_success(instances_ok, &instances, "instances");
    assert!(instances.contains("prod"), "{instances}");
    assert!(!instances.contains("jwt-oidc-session"));
    assert!(!instances.contains("cloud-password"));
    assert!(!instances.contains("access-root"));
}

#[test]
fn several_local_servers_do_not_share_a_port_namespace_or_password() {
    let root = TempDir::new().expect("temp");
    let home = root.path().join("home");
    let live = init_project(&home, root.path(), "shop-live", None);
    let paused = init_project(&home, root.path(), "shop-paused", None);
    let east = init_project(&home, root.path(), "shop-east", None);
    let (live_url, _, live_hits) = start_mock_server(ServerMode {
        local_enabled: true,
        password:      "live-root-secret".into(),
    });
    let (east_url, _, east_hits) = start_mock_server(ServerMode {
        local_enabled: true,
        password:      "east-root-secret".into(),
    });
    point_project_at(&live, &live_url, "live-root-secret");
    point_project_at(&paused, &live_url, "paused-root-secret");
    point_project_at(&east, &east_url, "east-root-secret");
    write_owned_instance(&live.dir, &live_url);
    write_owned_instance(&east.dir, &east_url);
    write_stopped_instance(&paused.dir, &live_url);
    assert!(run(&live, &["instances"]).0);
    assert!(run(&paused, &["instances"]).0);
    assert!(run(&east, &["instances"]).0);

    clear_hits(&live_hits);
    let (ok, text) = run(
        &paused,
        &[
            "--no-color",
            "--connection-timeout",
            "2",
            "--timeout",
            "2",
            "-c",
            "SELECT 1",
        ],
    );
    assert_failure(ok, &text, "running server `shop-live`", "paused project URL");
    assert!(
        sql_hits(&live_hits).is_empty(),
        "paused project sent SQL to the live server\n{text}"
    );

    let (ok, text) = run(
        &live,
        &[
            "--instance",
            "shop-paused",
            "--no-color",
            "--connection-timeout",
            "2",
            "--timeout",
            "2",
            "-c",
            "SELECT 1",
        ],
    );
    assert_failure(ok, &text, "running server `shop-live`", "stopped instance sharing a port");
    assert!(sql_hits(&live_hits).is_empty());

    clear_hits(&live_hits);
    clear_hits(&east_hits);
    let (ok, text) = run(
        &live,
        &[
            "--instance",
            "shop-east",
            "--no-color",
            "--connection-timeout",
            "2",
            "--timeout",
            "2",
            "-c",
            "SELECT * FROM users",
        ],
    );
    assert_success(ok, &text, "other running project");
    assert_sql_namespace(&east_hits, &east.namespace);
    assert_eq!(login_passwords(&east_hits), vec!["east-root-secret".to_string()]);
    assert!(sql_hits(&live_hits).is_empty(), "east SQL hit the live server");

    clear_hits(&live_hits);
    let (ok, text) = run(
        &east,
        &[
            "--instance",
            "shop-live",
            "--no-color",
            "--connection-timeout",
            "2",
            "--timeout",
            "2",
            "-c",
            "SELECT * FROM users",
        ],
    );
    assert_success(ok, &text, "named live project from another directory");
    assert_sql_namespace(&live_hits, &live.namespace);
    assert_eq!(login_passwords(&live_hits), vec!["live-root-secret".to_string()]);

    let (ok, text) = run(&paused, &["instances", "--no-color"]);
    assert_success(ok, &text, "instances");
    assert!(
        text.contains("now running as shop-live"),
        "port conflict should be visible\n{text}"
    );
    assert!(!text.contains("live-root-secret"));
    assert!(!text.contains("east-root-secret"));
}

#[test]
fn simple_live_template_keeps_running_after_a_duplicate_primary_key() {
    let root = TempDir::new().expect("temp");
    let home = root.path().join("home");
    let project = init_project(&home, root.path(), "shop-template", Some("simple-live"));
    let source = fs::read_to_string(project.dir.join("src/index.ts")).expect("starter");
    assert!(
        source.contains("already exists|Primary key violation"),
        "the starter should keep serving after the demo row already exists\n{source}"
    );
    assert!(
        source.contains("KALAM_NAMESPACE"),
        "the starter should use the project namespace"
    );
}

fn write_stopped_instance(project_dir: &Path, server_url: &str) {
    write_owned_instance(project_dir, server_url);
    let path = project_dir.join(".kalam/run/instance.json");
    let mut record: Value =
        serde_json::from_str(&fs::read_to_string(&path).expect("instance")).unwrap();
    record["pid"] = Value::from(4_000_000_000_u64);
    record.as_object_mut().expect("object").remove("exe");
    fs::write(path, serde_json::to_vec_pretty(&record).unwrap()).expect("stopped instance");
}
