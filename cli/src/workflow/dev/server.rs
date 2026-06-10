//! Local KalamDB server lifecycle helpers for `kalam dev`.

use std::{
    env, fs,
    io::IsTerminal,
    path::{Path, PathBuf},
    time::Duration,
};

use reqwest::StatusCode;
use url::Url;

use crate::{
    error::{CLIError, Result},
    history::get_kalam_config_dir,
    output::WorkflowOutput,
    release_download::{
        archive_kind_for_platform, archive_name, copy_file_with_executable_bit, create_temp_dir,
        detect_platform, download_bytes, download_text, extract_archive, find_first_file_matching,
        release_base_url, verify_checksum,
    },
    terminal_ui,
    workflow::{dev::logs::ServiceLogSource, project::config::KalamProjectConfig},
};

pub const DEFAULT_DEV_SERVER_URL: &str = "http://localhost:2900";
const SERVER_ARTIFACT_PREFIX: &str = "kalamdb-server";
const SERVER_RELEASE_BASE_URL_ENV: &str = "KALAMDB_SERVER_RELEASE_BASE_URL";

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

pub fn local_server_config_path(project_root: &Path, config: &KalamProjectConfig) -> PathBuf {
    config.local_server_config_path(project_root)
}

pub fn local_server_root_password(
    project_root: &Path,
    config: &KalamProjectConfig,
) -> Result<Option<String>> {
    let config_path = local_server_config_path(project_root, config);
    if !config_path.is_file() {
        return Ok(None);
    }

    let contents = fs::read_to_string(&config_path).map_err(|error| {
        CLIError::FileError(format!(
            "failed to read local server config '{}': {error}",
            config_path.display()
        ))
    })?;
    let value: toml::Value = toml::from_str(&contents).map_err(|error| {
        CLIError::ConfigurationError(format!(
            "failed to parse local server config '{}': {error}",
            config_path.display()
        ))
    })?;

    Ok(value
        .get("auth")
        .and_then(|auth| auth.get("root_password"))
        .and_then(|password| password.as_str())
        .map(ToString::to_string))
}

pub fn write_local_server_config(
    project_root: &Path,
    config: &KalamProjectConfig,
    port: u16,
) -> Result<PathBuf> {
    let config_path = local_server_config_path(project_root, config);
    std::fs::create_dir_all(config.local_server_dir(project_root))?;
    std::fs::create_dir_all(config.local_server_dir(project_root).join("data"))?;
    std::fs::create_dir_all(config.local_server_dir(project_root).join("logs"))?;
    if config_path.is_file() {
        return Ok(config_path);
    }
    if let Some(parent) = config_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let data_path = config.relative_local_server_data_path();
    let logs_path = config.relative_local_server_logs_path();
    let contents = format!(
        r#"# Local KalamDB server config (managed by kalam dev)
[server]
host = "127.0.0.1"
port = {port}
public_origin = "http://localhost:{port}"

[storage]
data_path = "{data_path}"

[limits]

[logging]
logs_path = "{logs_path}"
log_to_console = true

[performance]

[auth]
root_password = "mypass"
jwt_secret = "dev-local-jwt-secret-minimum-32-chars"
"#
    );
    std::fs::write(&config_path, contents)?;
    Ok(config_path)
}

pub fn managed_server_install_dir() -> PathBuf {
    get_kalam_config_dir().join("bin")
}

pub fn managed_server_binary_path() -> PathBuf {
    managed_server_install_dir().join(server_binary_name())
}

pub fn resolve_kalamdb_server_bin() -> Result<PathBuf> {
    resolve_kalamdb_server_bin_from(std::env::current_exe().ok())
}

fn resolve_kalamdb_server_bin_from(current_exe: Option<PathBuf>) -> Result<PathBuf> {
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

    if let Some(path) = current_exe.as_deref().and_then(colocated_server_binary_path) {
        return Ok(path);
    }

    let managed_path = managed_server_binary_path();
    if managed_path.is_file() {
        return Ok(managed_path);
    }

    if let Some(path) = find_on_path("kalamdb-server") {
        return Ok(path);
    }

    Err(CLIError::ConfigurationError(
        "kalamdb-server not found in KALAMDB_SERVER_BIN, ~/.kalam/bin, or PATH".into(),
    ))
}

fn colocated_server_binary_path(current_exe: &Path) -> Option<PathBuf> {
    let candidate = current_exe.parent()?.join(server_binary_name());
    if candidate == current_exe || !candidate.is_file() {
        return None;
    }
    Some(candidate)
}

fn server_binary_name() -> &'static str {
    if cfg!(windows) {
        "kalamdb-server.exe"
    } else {
        "kalamdb-server"
    }
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

pub async fn ensure_local_server_binary(
    use_color: bool,
    output: &WorkflowOutput,
    server_source: &ServiceLogSource,
) -> Result<PathBuf> {
    match resolve_kalamdb_server_bin() {
        Ok(path) => Ok(path),
        Err(error) => {
            output.service_log(server_source, format!("precheck: {error}"));
            if !std::io::stdin().is_terminal() || !std::io::stdout().is_terminal() {
                return Err(CLIError::ConfigurationError(format!(
                    "{error}; rerun interactively to download it automatically or set KALAMDB_SERVER_BIN"
                )));
            }

            let install_dir = managed_server_install_dir();
            let confirmed = terminal_ui::prompt_confirm(
                &format!(
                    "Download KalamDB server {} into {}",
                    env!("CARGO_PKG_VERSION"),
                    install_dir.display()
                ),
                true,
                use_color,
            )
            .map_err(|prompt_error| {
                CLIError::FileError(format!("failed to read download confirmation: {prompt_error}"))
            })?;

            if !confirmed {
                return Err(CLIError::ConfigurationError(format!("{error}; download declined")));
            }

            output.service_log(server_source, "precheck: downloading kalamdb-server");
            download_and_install_managed_server(output, server_source).await
        },
    }
}

pub fn build_local_server_command(
    project_root: &Path,
    config: &KalamProjectConfig,
    server_url: &str,
) -> Result<String> {
    let port = parse_server_port(server_url)?;
    let config_path = local_server_config_path(project_root, config);
    if !config_path.is_file() {
        write_local_server_config(project_root, config, port)?;
    }

    let server_bin = resolve_kalamdb_server_bin()?;
    let root = shell_escape(&project_root.display().to_string());
    let bin = shell_escape(&server_bin.display().to_string());
    let cfg = shell_escape(&config_path.display().to_string());
    Ok(format!("cd {root} && exec {bin} {cfg}"))
}

async fn download_and_install_managed_server(
    output: &WorkflowOutput,
    server_source: &ServiceLogSource,
) -> Result<PathBuf> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(60))
        .user_agent(format!("kalam-cli/{}", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|error| {
            CLIError::ConfigurationError(format!("failed to create HTTP client: {error}"))
        })?;

    let version = env!("CARGO_PKG_VERSION");
    let platform = detect_platform()?;
    let archive_kind = archive_kind_for_platform(&platform);
    let archive_name = archive_name(SERVER_ARTIFACT_PREFIX, version, &platform, archive_kind);
    let base_url = release_base_url(version, SERVER_RELEASE_BASE_URL_ENV);
    let archive_url = format!("{base_url}/{archive_name}");
    let checksums_url = format!("{base_url}/SHA256SUMS");

    let archive_bytes = download_bytes(&client, &archive_url, &archive_name, true).await?;
    let checksums = download_text(&client, &checksums_url, "checksum file").await?;
    verify_checksum(&archive_name, &archive_bytes, &checksums)?;
    output.service_log(server_source, "precheck: downloaded and verified kalamdb-server");

    let temp_dir = create_temp_dir("kalamdb-server-install")?;
    let cleanup_dir = temp_dir.clone();
    let install_result = (|| -> Result<PathBuf> {
        extract_archive(&archive_bytes, archive_kind, &temp_dir)?;
        install_server_payload(&temp_dir)
    })();
    let _ = fs::remove_dir_all(cleanup_dir);
    install_result
}

fn install_server_payload(extracted_root: &Path) -> Result<PathBuf> {
    let install_dir = managed_server_install_dir();
    fs::create_dir_all(&install_dir).map_err(|error| {
        CLIError::FileError(format!(
            "failed to create managed server install dir '{}': {error}",
            install_dir.display()
        ))
    })?;

    let primary_binary = find_first_file_matching(extracted_root, is_server_binary_candidate)
        .ok_or_else(|| {
            CLIError::ConfigurationError("downloaded archive did not contain kalamdb-server".into())
        })?;

    for file in collect_files_recursively(extracted_root)? {
        let file_name = file.file_name().and_then(|name| name.to_str()).ok_or_else(|| {
            CLIError::FileError(format!("invalid extracted filename '{}'", file.display()))
        })?;
        let target = if file == primary_binary {
            managed_server_binary_path()
        } else {
            install_dir.join(file_name)
        };
        copy_file_with_executable_bit(&file, &target)?;
    }

    Ok(managed_server_binary_path())
}

fn collect_files_recursively(root: &Path) -> Result<Vec<PathBuf>> {
    let mut stack = vec![root.to_path_buf()];
    let mut files = Vec::new();

    while let Some(path) = stack.pop() {
        for entry in fs::read_dir(&path).map_err(|error| {
            CLIError::FileError(format!(
                "failed to read extracted directory '{}': {error}",
                path.display()
            ))
        })? {
            let entry = entry.map_err(|error| {
                CLIError::FileError(format!("failed to read extracted entry: {error}"))
            })?;
            let entry_path = entry.path();
            if entry_path.is_dir() {
                stack.push(entry_path);
            } else {
                files.push(entry_path);
            }
        }
    }

    Ok(files)
}

fn is_server_binary_candidate(path: &Path) -> bool {
    let Some(file_name) = path.file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    file_name == "kalamdb-server"
        || file_name == "kalamdb-server.exe"
        || file_name.starts_with("kalamdb-server-")
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

    fn test_project_config() -> KalamProjectConfig {
        KalamProjectConfig::parse(
            r#"
[project]
name = "demo"

[schema]
mode = "sql"
path = "schema.sql"

[schema.targets.typescript]
output = "src/generated/kalam.ts"
"#,
        )
        .unwrap()
    }

    #[test]
    fn parse_server_port_reads_explicit_port() {
        assert_eq!(parse_server_port("http://localhost:2900").unwrap(), 2900);
    }

    #[test]
    fn write_local_server_config_uses_requested_port() {
        let temp = tempfile::TempDir::new().unwrap();
        let config = test_project_config();
        let path = write_local_server_config(temp.path(), &config, 3001).unwrap();
        let contents = std::fs::read_to_string(path).unwrap();
        assert!(contents.contains("port = 3001"));
        assert!(temp.path().join("kalam/server/data").is_dir());
        assert!(temp.path().join("kalam/server/logs").is_dir());
    }

    #[test]
    fn write_local_server_config_uses_project_server_directory() {
        let temp = tempfile::TempDir::new().unwrap();
        let config = test_project_config();
        let path = write_local_server_config(temp.path(), &config, 2900).unwrap();
        assert_eq!(path, temp.path().join("kalam/server/server.toml"));
    }

    #[test]
    fn write_local_server_config_includes_required_server_sections() {
        let temp = tempfile::TempDir::new().unwrap();
        let config = test_project_config();
        let path = write_local_server_config(temp.path(), &config, 2900).unwrap();
        let contents = std::fs::read_to_string(path).unwrap();
        assert!(contents.contains("[limits]"));
        assert!(contents.contains("[logging]"));
        assert!(contents.contains("[performance]"));
        assert!(contents.contains("data_path = \"kalam/server/data\""));
        assert!(contents.contains("logs_path = \"kalam/server/logs\""));
    }

    #[test]
    fn build_local_server_command_preserves_existing_server_config() {
        let temp = tempfile::TempDir::new().unwrap();
        let config = test_project_config();
        let config_path = local_server_config_path(temp.path(), &config);
        std::fs::create_dir_all(config_path.parent().unwrap()).unwrap();
        std::fs::write(&config_path, "# manual config\n[server]\nport = 2900\n").unwrap();

        let original_home = std::env::var_os("HOME");
        let original_userprofile = std::env::var_os("USERPROFILE");
        let original_path = std::env::var_os("PATH");
        let original_server_bin = std::env::var_os("KALAMDB_SERVER_BIN");

        let fake_home = temp.path().join("home");
        let fake_bin = temp.path().join("kalamdb-server");
        std::fs::create_dir_all(&fake_home).unwrap();
        std::fs::write(&fake_bin, "#!/bin/sh\n").unwrap();
        std::env::set_var("HOME", &fake_home);
        std::env::set_var("USERPROFILE", &fake_home);
        std::env::remove_var("PATH");
        std::env::set_var("KALAMDB_SERVER_BIN", &fake_bin);

        let _ = build_local_server_command(temp.path(), &config, "http://localhost:2900").unwrap();
        let contents = std::fs::read_to_string(&config_path).unwrap();
        assert_eq!(contents, "# manual config\n[server]\nport = 2900\n");

        match original_home {
            Some(value) => std::env::set_var("HOME", value),
            None => std::env::remove_var("HOME"),
        }
        match original_userprofile {
            Some(value) => std::env::set_var("USERPROFILE", value),
            None => std::env::remove_var("USERPROFILE"),
        }
        match original_path {
            Some(value) => std::env::set_var("PATH", value),
            None => std::env::remove_var("PATH"),
        }
        match original_server_bin {
            Some(value) => std::env::set_var("KALAMDB_SERVER_BIN", value),
            None => std::env::remove_var("KALAMDB_SERVER_BIN"),
        }
    }

    #[test]
    fn resolve_kalamdb_server_bin_prefers_managed_install_path() {
        let temp = tempfile::TempDir::new().unwrap();
        let home = temp.path().join("home");
        let managed_bin = home.join(".kalam/bin/kalamdb-server");
        std::fs::create_dir_all(managed_bin.parent().unwrap()).unwrap();
        std::fs::write(&managed_bin, "#!/bin/sh\n").unwrap();

        let original_home = std::env::var_os("HOME");
        let original_userprofile = std::env::var_os("USERPROFILE");
        let original_path = std::env::var_os("PATH");
        let original_server_bin = std::env::var_os("KALAMDB_SERVER_BIN");

        std::env::set_var("HOME", &home);
        std::env::set_var("USERPROFILE", &home);
        std::env::set_var("PATH", temp.path().join("empty-bin"));
        std::env::remove_var("KALAMDB_SERVER_BIN");

        let resolved = resolve_kalamdb_server_bin_from(None).expect("resolve managed install");
        assert_eq!(resolved, managed_bin);

        match original_home {
            Some(value) => std::env::set_var("HOME", value),
            None => std::env::remove_var("HOME"),
        }
        match original_userprofile {
            Some(value) => std::env::set_var("USERPROFILE", value),
            None => std::env::remove_var("USERPROFILE"),
        }
        match original_path {
            Some(value) => std::env::set_var("PATH", value),
            None => std::env::remove_var("PATH"),
        }
        match original_server_bin {
            Some(value) => std::env::set_var("KALAMDB_SERVER_BIN", value),
            None => std::env::remove_var("KALAMDB_SERVER_BIN"),
        }
    }

    #[test]
    fn resolve_kalamdb_server_bin_prefers_colocated_binary_over_managed_install() {
        let temp = tempfile::TempDir::new().unwrap();
        let home = temp.path().join("home");
        let managed_bin = home.join(".kalam/bin/kalamdb-server");
        let release_dir = temp.path().join("target/release");
        let cli_bin = release_dir.join("kalam");
        let colocated_bin = release_dir.join("kalamdb-server");
        std::fs::create_dir_all(managed_bin.parent().unwrap()).unwrap();
        std::fs::create_dir_all(&release_dir).unwrap();
        std::fs::write(&managed_bin, "#!/bin/sh\n").unwrap();
        std::fs::write(&cli_bin, "#!/bin/sh\n").unwrap();
        std::fs::write(&colocated_bin, "#!/bin/sh\n").unwrap();

        let original_home = std::env::var_os("HOME");
        let original_userprofile = std::env::var_os("USERPROFILE");
        let original_path = std::env::var_os("PATH");
        let original_server_bin = std::env::var_os("KALAMDB_SERVER_BIN");

        std::env::set_var("HOME", &home);
        std::env::set_var("USERPROFILE", &home);
        std::env::set_var("PATH", temp.path().join("empty-bin"));
        std::env::remove_var("KALAMDB_SERVER_BIN");

        let resolved =
            resolve_kalamdb_server_bin_from(Some(cli_bin)).expect("resolve colocated install");
        assert_eq!(resolved, colocated_bin);

        match original_home {
            Some(value) => std::env::set_var("HOME", value),
            None => std::env::remove_var("HOME"),
        }
        match original_userprofile {
            Some(value) => std::env::set_var("USERPROFILE", value),
            None => std::env::remove_var("USERPROFILE"),
        }
        match original_path {
            Some(value) => std::env::set_var("PATH", value),
            None => std::env::remove_var("PATH"),
        }
        match original_server_bin {
            Some(value) => std::env::set_var("KALAMDB_SERVER_BIN", value),
            None => std::env::remove_var("KALAMDB_SERVER_BIN"),
        }
    }
}
