//! Local KalamDB server lifecycle helpers for `kalam dev`.

use std::{
    env,
    path::{Path, PathBuf},
    time::Duration,
};

use reqwest::StatusCode;
use url::Url;

use crate::{
    error::{CLIError, Result},
    output::WorkflowOutput,
    workflow::project::config::KalamProjectConfig,
};

pub const DEFAULT_DEV_SERVER_URL: &str = "http://localhost:2900";
pub const LOCAL_SERVER_CONFIG: &str = ".kalam/server.toml";

pub fn parse_server_port(server_url: &str) -> Result<u16> {
    let url = Url::parse(server_url).map_err(|e| {
        CLIError::ConfigurationError(format!("invalid server url '{server_url}': {e}"))
    })?;
    url.port_or_known_default().ok_or_else(|| {
        CLIError::ConfigurationError(format!(
            "server url '{server_url}' must include an explicit port"
        ))
    })
}

pub fn local_server_config_path(project_root: &Path) -> PathBuf {
    project_root.join(LOCAL_SERVER_CONFIG)
}

pub fn write_local_server_config(project_root: &Path, port: u16) -> Result<PathBuf> {
    let kalam_dir = project_root.join(".kalam");
    std::fs::create_dir_all(kalam_dir.join("data"))?;
    std::fs::create_dir_all(kalam_dir.join("logs"))?;

    let config_path = local_server_config_path(project_root);
    let contents = format!(
        r#"# Local KalamDB server config (managed by kalam dev)
[server]
host = "127.0.0.1"
port = {port}

[storage]
data_path = ".kalam/data"

[auth]
jwt_secret = "dev-local-jwt-secret-minimum-32-chars"
root_password = "mypass"
"#
    );
    std::fs::write(&config_path, contents)?;
    Ok(config_path)
}

pub fn resolve_kalamdb_server_bin() -> Result<PathBuf> {
    if let Ok(path) = env::var("KALAMDB_SERVER_BIN") {
        let path = PathBuf::from(path);
        if path.is_file() {
            return Ok(path);
        }
        return Err(CLIError::ConfigurationError(format!(
            "KALAMDB_SERVER_BIN points to missing file '{}'",
            path.display()
        )));
    }

    if let Some(path) = find_on_path("kalamdb-server") {
        return Ok(path);
    }

    Err(CLIError::ConfigurationError(
        "kalamdb-server not found on PATH; install KalamDB server or set KALAMDB_SERVER_BIN".into(),
    ))
}

fn find_on_path(binary: &str) -> Option<PathBuf> {
    let path_var = env::var_os("PATH")?;
    for dir in env::split_paths(&path_var) {
        let candidate = dir.join(binary);
        if candidate.is_file() {
            return Some(candidate);
        }
        #[cfg(windows)]
        {
            let exe_candidate = dir.join(format!("{binary}.exe"));
            if exe_candidate.is_file() {
                return Some(exe_candidate);
            }
        }
    }
    None
}

pub fn build_local_server_command(
    project_root: &Path,
    config: &KalamProjectConfig,
) -> Result<String> {
    let env = config.connection.get(&config.project.default_env).ok_or_else(|| {
        CLIError::ConfigurationError(format!(
            "connection.{} is not configured",
            config.project.default_env
        ))
    })?;

    let port = parse_server_port(&env.url)?;
    let config_path = local_server_config_path(project_root);
    if !config_path.is_file() {
        write_local_server_config(project_root, port)?;
    }

    let server_bin = resolve_kalamdb_server_bin()?;
    let root = shell_escape(&project_root.display().to_string());
    let bin = shell_escape(&server_bin.display().to_string());
    let cfg = shell_escape(&config_path.display().to_string());
    Ok(format!("cd {root} && exec {bin} {cfg}"))
}

pub async fn wait_for_server_ready(server_url: &str, output: &WorkflowOutput) -> Result<()> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(2))
        .build()
        .map_err(|e| CLIError::ConfigurationError(format!("failed to build http client: {e}")))?;

    for attempt in 1..=60 {
        if check_server_health(&client, server_url).await {
            output.detail(format!("local KalamDB server ready ({attempt}s)"));
            return Ok(());
        }
        if attempt == 1 {
            output.detail("waiting for local KalamDB server...");
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }

    Err(CLIError::ConfigurationError(format!(
        "timed out waiting for local KalamDB server at {server_url}"
    )))
}

pub async fn server_already_ready(server_url: &str) -> bool {
    let Ok(client) = reqwest::Client::builder().timeout(Duration::from_secs(2)).build() else {
        return false;
    };

    check_server_health(&client, server_url).await
}

async fn check_server_health(client: &reqwest::Client, server_url: &str) -> bool {
    let health_urls = [
        format!("{server_url}/health"),
        format!("{server_url}/v1/api/healthcheck"),
        format!("{server_url}/ui"),
    ];

    for url in &health_urls {
        if let Ok(response) = client.get(url).send().await {
            if response.status().is_success() || response.status() == StatusCode::NOT_FOUND {
                return true;
            }
        }
    }

    false
}

fn shell_escape(value: &str) -> String {
    if value.chars().all(|c| c.is_ascii_alphanumeric() || "/._-".contains(c)) {
        value.to_string()
    } else {
        format!("'{}'", value.replace('\'', "'\\''"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_server_port_reads_explicit_port() {
        assert_eq!(parse_server_port("http://localhost:2900").unwrap(), 2900);
    }

    #[test]
    fn write_local_server_config_uses_requested_port() {
        let temp = tempfile::TempDir::new().unwrap();
        let path = write_local_server_config(temp.path(), 3001).unwrap();
        let contents = std::fs::read_to_string(path).unwrap();
        assert!(contents.contains("port = 3001"));
        assert!(temp.path().join(".kalam/data").is_dir());
    }
}
