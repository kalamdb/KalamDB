use std::{
    io::{Read, Write},
    net::TcpListener,
    thread,
    time::Duration,
};

#[test]
fn cli_local_login_explains_disabled_policy_without_password_prompt() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let server_url = format!("http://{}", listener.local_addr().unwrap());
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut buffer = [0; 1024];
        let read = stream.read(&mut buffer).unwrap();
        let request = String::from_utf8_lossy(&buffer[..read]);
        assert!(request.starts_with("GET /v1/api/auth/login-options HTTP/1.1"));

        let body = r#"{"local":{"enabled":false},"oidc":null}"#;
        write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
            body.len(),
            body
        )
        .unwrap();
    });

    let temp_home = tempfile::tempdir().unwrap();
    let credentials_path = temp_home.path().join("credentials.toml");
    let mut command = assert_cmd::Command::new(env!("CARGO_BIN_EXE_kalam"));
    command
        .arg("--url")
        .arg(server_url)
        .arg("login")
        .env("HOME", temp_home.path())
        .env("USERPROFILE", temp_home.path())
        .env("KALAMDB_CREDENTIALS_PATH", credentials_path)
        .env("NO_PROXY", "127.0.0.1,localhost,::1")
        .env("no_proxy", "127.0.0.1,localhost,::1")
        .env_remove("HTTP_PROXY")
        .env_remove("http_proxy")
        .env_remove("HTTPS_PROXY")
        .env_remove("https_proxy")
        .env_remove("ALL_PROXY")
        .env_remove("all_proxy")
        .timeout(Duration::from_secs(5));

    command.assert().failure().stderr(predicates::str::contains(
        "local username/password login is disabled; use `kalam login --oidc`",
    ));

    server.join().unwrap();
}
