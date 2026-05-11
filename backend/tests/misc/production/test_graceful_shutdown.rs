#![cfg(unix)]

use std::{
    fs,
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    path::{Path, PathBuf},
    process::{Child, Command, ExitStatus, Stdio},
    thread,
    time::{Duration, Instant},
};

struct ChildGuard {
    child: Option<Child>,
}

impl ChildGuard {
    fn new(child: Child) -> Self {
        Self { child: Some(child) }
    }

    fn child(&self) -> &Child {
        self.child.as_ref().expect("child process should be present")
    }

    fn child_mut(&mut self) -> &mut Child {
        self.child.as_mut().expect("child process should be present")
    }
}

impl Drop for ChildGuard {
    fn drop(&mut self) {
        let Some(child) = self.child.as_mut() else {
            return;
        };

        if matches!(child.try_wait(), Ok(None)) {
            unsafe {
                libc::kill(child.id() as i32, libc::SIGKILL);
            }
            let _ = child.wait();
        }
    }
}

fn reserve_local_port() -> std::io::Result<u16> {
    let listener = TcpListener::bind(("127.0.0.1", 0))?;
    let port = listener.local_addr()?.port();
    drop(listener);
    Ok(port)
}

fn write_config_file(base_dir: &Path, port: u16) -> std::io::Result<PathBuf> {
    let data_path = base_dir.join("data");
    let logs_path = base_dir.join("logs");
    let config_path = base_dir.join("server.toml");
    let config = format!(
        r#"[server]
host = "127.0.0.1"
port = {port}

[storage]
data_path = "{data_path}"

[limits]

[logging]
logs_path = "{logs_path}"
log_to_console = false

[performance]

[auth]
jwt_secret = "kalamdb-test-jwt-secret-please-change-32chars"

[retention]
enable_dba_stats = false

[shutdown.flush]
timeout = 5
"#,
        port = port,
        data_path = data_path.display(),
        logs_path = logs_path.display(),
    );

    fs::write(&config_path, config)?;
    Ok(config_path)
}

fn wait_for_healthcheck(port: u16, timeout: Duration) -> std::io::Result<()> {
    let deadline = Instant::now() + timeout;
    let request = b"GET /health HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n";

    loop {
        match TcpStream::connect(("127.0.0.1", port)) {
            Ok(mut stream) => {
                stream.set_read_timeout(Some(Duration::from_secs(1)))?;
                stream.write_all(request)?;

                let mut response = String::new();
                let _ = stream.read_to_string(&mut response);
                if response.starts_with("HTTP/1.1 200") || response.starts_with("HTTP/1.0 200") {
                    return Ok(());
                }
            }
            Err(error) if Instant::now() < deadline => {
                if error.kind() != std::io::ErrorKind::ConnectionRefused {
                    return Err(error);
                }
            }
            Err(error) => return Err(error),
        }

        if Instant::now() >= deadline {
            return Err(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                format!("timed out waiting for KalamDB healthcheck on port {}", port),
            ));
        }

        thread::sleep(Duration::from_millis(100));
    }
}

fn wait_for_exit(child: &mut Child, timeout: Duration) -> std::io::Result<ExitStatus> {
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(status) = child.try_wait()? {
            return Ok(status);
        }

        if Instant::now() >= deadline {
            return Err(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "timed out waiting for KalamDB process to exit",
            ));
        }

        thread::sleep(Duration::from_millis(100));
    }
}

#[test]
fn kalamdb_server_gracefully_shuts_down_on_sigterm() -> Result<(), Box<dyn std::error::Error>> {
    let temp_dir = tempfile::tempdir()?;
    let port = reserve_local_port()?;
    let config_path = write_config_file(temp_dir.path(), port)?;
    let log_path = temp_dir.path().join("logs").join("server.log");

    let child = Command::new(env!("CARGO_BIN_EXE_kalamdb-server"))
        .arg(&config_path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    let mut child = ChildGuard::new(child);

    wait_for_healthcheck(port, Duration::from_secs(20))?;

    let signal_result = unsafe { libc::kill(child.child().id() as i32, libc::SIGTERM) };
    if signal_result != 0 {
        return Err(Box::new(std::io::Error::last_os_error()));
    }

    let status = wait_for_exit(child.child_mut(), Duration::from_secs(15))?;
    assert!(status.success(), "expected graceful SIGTERM exit, got {}", status);

    let log_contents = fs::read_to_string(&log_path)?;
    assert!(
        log_contents.contains("Received SIGTERM, initiating graceful shutdown"),
        "expected SIGTERM shutdown log in {}:\n{}",
        log_path.display(),
        log_contents
    );
    assert!(
        log_contents.contains("Server shutdown complete"),
        "expected final shutdown completion log in {}:\n{}",
        log_path.display(),
        log_contents
    );

    Ok(())
}
