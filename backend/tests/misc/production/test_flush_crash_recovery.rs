#![cfg(unix)]

use std::{
    fs,
    net::TcpListener,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    time::Duration,
};

use anyhow::{Context, Result};
use kalam_client::{models::ResponseStatus, AuthProvider, KalamLinkClient, KalamLinkTimeouts};
use kalamdb_commons::{Role, UserId};
use tokio::time::{sleep, Instant};

struct ProcessGuard {
    child: Option<Child>,
}

impl ProcessGuard {
    fn new(child: Child) -> Self {
        Self { child: Some(child) }
    }

    fn child_mut(&mut self) -> &mut Child {
        self.child.as_mut().expect("child process should be present")
    }

    fn kill_with_signal(&mut self, signal: i32) {
        if let Some(child) = self.child.as_mut() {
            if matches!(child.try_wait(), Ok(None)) {
                unsafe {
                    libc::kill(child.id() as i32, signal);
                }
            }
        }
    }
}

impl Drop for ProcessGuard {
    fn drop(&mut self) {
        let Some(child) = self.child.as_mut() else {
            return;
        };

        if matches!(child.try_wait(), Ok(None)) {
            unsafe {
                libc::kill(child.id() as i32, libc::SIGTERM);
            }

            for _ in 0..20 {
                if matches!(child.try_wait(), Ok(Some(_))) {
                    return;
                }
                std::thread::sleep(Duration::from_millis(100));
            }

            unsafe {
                libc::kill(child.id() as i32, libc::SIGKILL);
            }
            let _ = child.wait();
        }
    }
}

fn reserve_local_port() -> Result<u16> {
    let listener = TcpListener::bind(("127.0.0.1", 0))?;
    Ok(listener.local_addr()?.port())
}

fn write_config_file(base_dir: &Path, port: u16, root_password: &str, jwt_secret: &str) -> Result<PathBuf> {
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
root_password = "{root_password}"
jwt_secret = "{jwt_secret}"

[auth.local]
enabled = true

[shutdown.flush]
timeout = 5
"#,
        port = port,
        data_path = data_path.display(),
        logs_path = logs_path.display(),
        root_password = root_password,
        jwt_secret = jwt_secret,
    );

    fs::write(&config_path, config)?;
    Ok(config_path)
}

fn spawn_server(config_path: &Path) -> Result<ProcessGuard> {
    let child = Command::new(env!("CARGO_BIN_EXE_kalamdb-server"))
        .arg(config_path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .context("failed to spawn kalamdb-server")?;

    Ok(ProcessGuard::new(child))
}

fn build_client(base_url: &str, jwt_secret: &str) -> Result<KalamLinkClient> {
    let (token, _claims) = kalamdb_auth::providers::jwt_auth::create_and_sign_token(
        &UserId::root(),
        &Role::System,
        None,
        Some(1),
        jwt_secret,
    )
    .context("failed to create root JWT token")?;

    let client = KalamLinkClient::builder()
        .base_url(base_url)
        .auth(AuthProvider::jwt_token(token))
        .timeouts(KalamLinkTimeouts::fast())
        .build()
        .context("failed to build test client")?;
    Ok(client)
}

async fn wait_until_sql_ready(base_url: &str, jwt_secret: &str, timeout: Duration) -> Result<()> {
    let deadline = Instant::now() + timeout;
    loop {
        let client = build_client(base_url, jwt_secret)?;
        if let Ok(resp) = client.execute_query("SELECT 1 AS ok", None, None, None).await {
            if resp.status == ResponseStatus::Success {
                return Ok(());
            }
        }

        if Instant::now() >= deadline {
            anyhow::bail!("timed out waiting for SQL readiness at {}", base_url);
        }

        sleep(Duration::from_millis(100)).await;
    }
}

async fn wait_for_child_exit(child: &mut Child, timeout: Duration) -> Result<()> {
    let deadline = Instant::now() + timeout;
    loop {
        if child.try_wait()?.is_some() {
            return Ok(());
        }

        if Instant::now() >= deadline {
            anyhow::bail!("timed out waiting for child process exit");
        }

        sleep(Duration::from_millis(100)).await;
    }
}

#[tokio::test(flavor = "multi_thread")]
#[ntest::timeout(120000)]
async fn crash_during_flush_restarts_and_recovers_visible_rows() -> Result<()> {
    let temp_dir = tempfile::tempdir()?;
    let port = reserve_local_port()?;
    let root_password = "kalamdb123";
    let jwt_secret = "flush-crash-recovery-jwt-secret-at-least-32-characters";
    let config_path = write_config_file(temp_dir.path(), port, root_password, jwt_secret)?;
    let base_url = format!("http://127.0.0.1:{}", port);

    let mut first_server = spawn_server(&config_path)?;
    wait_until_sql_ready(&base_url, jwt_secret, Duration::from_secs(20)).await?;
    let client = build_client(&base_url, jwt_secret)?;

    let unique_suffix = format!(
        "{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system clock before unix epoch")
            .as_millis()
    );
    let namespace = format!("flush_crash_{}", unique_suffix);
    let table = "events";
    let total_rows: usize = 12_000;

    let create_ns = client
        .execute_query(&format!("CREATE NAMESPACE {}", namespace), None, None, None)
        .await?;
    anyhow::ensure!(create_ns.status == ResponseStatus::Success, "CREATE NAMESPACE failed");

    let create_table = client
        .execute_query(
            &format!(
                "CREATE TABLE {}.{} (id BIGINT PRIMARY KEY, payload TEXT) WITH (TYPE='SHARED', \
                 FLUSH_POLICY='rows:500')",
                namespace, table
            ),
            None,
            None,
            None,
        )
        .await?;
    anyhow::ensure!(create_table.status == ResponseStatus::Success, "CREATE TABLE failed");

    let payload = "x".repeat(2048);
    let batch_size = 400;
    for chunk_start in (1..=total_rows).step_by(batch_size) {
        let chunk_end = usize::min(chunk_start + batch_size - 1, total_rows);
        let mut values = Vec::with_capacity(chunk_end - chunk_start + 1);
        for id in chunk_start..=chunk_end {
            values.push(format!("({}, '{}')", id, payload));
        }

        let insert_sql = format!(
            "INSERT INTO {}.{} (id, payload) VALUES {}",
            namespace,
            table,
            values.join(",")
        );
        let insert_resp = client.execute_query(&insert_sql, None, None, None).await?;
        anyhow::ensure!(insert_resp.status == ResponseStatus::Success, "INSERT batch failed");
    }

    let flush_resp = client
        .execute_query(
            &format!("STORAGE FLUSH TABLE {}.{}", namespace, table),
            None,
            None,
            None,
        )
        .await?;
    anyhow::ensure!(flush_resp.status == ResponseStatus::Success, "flush command failed");

    sleep(Duration::from_millis(20)).await;
    first_server.kill_with_signal(libc::SIGKILL);
    wait_for_child_exit(first_server.child_mut(), Duration::from_secs(10)).await?;

    let mut second_server = spawn_server(&config_path)?;
    wait_until_sql_ready(&base_url, jwt_secret, Duration::from_secs(30)).await?;
    let recovery_client = build_client(&base_url, jwt_secret)?;

    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let count_resp = recovery_client
            .execute_query(
                &format!("SELECT COUNT(*) AS cnt FROM {}.{}", namespace, table),
                None,
                None,
                None,
            )
            .await?;

        if count_resp.status == ResponseStatus::Success {
            let observed = count_resp.get_i64("cnt").unwrap_or(-1);
            if observed == total_rows as i64 {
                break;
            }
        }

        if Instant::now() >= deadline {
            anyhow::bail!(
                "count after restart did not converge to expected row count ({})",
                total_rows
            );
        }
        sleep(Duration::from_millis(200)).await;
    }

    let health_query = recovery_client.execute_query("SELECT 1 AS ok", None, None, None).await?;
    anyhow::ensure!(health_query.status == ResponseStatus::Success, "post-restart query failed");

    second_server.kill_with_signal(libc::SIGTERM);
    let _ = wait_for_child_exit(second_server.child_mut(), Duration::from_secs(10)).await;
    Ok(())
}