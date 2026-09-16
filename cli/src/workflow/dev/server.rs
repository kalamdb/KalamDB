//! Local KalamDB server lifecycle helpers for `kalam dev`.

use std::{
    env, fs,
    io::IsTerminal,
    path::{Path, PathBuf},
    time::Duration,
};

use reqwest::StatusCode;
use serde_json::json;

pub use crate::workflow::target::DEFAULT_LOCAL_URL as DEFAULT_DEV_SERVER_URL;
use crate::{
    error::{CLIError, Result},
    history::get_kalam_config_dir,
    output::WorkflowOutput,
    process::{resolve_program_on_path, run_program},
    release_download::{
        archive_kind_for_platform, archive_name, copy_file_with_executable_bit, create_temp_dir,
        detect_platform, download_bytes, download_text, extract_archive, find_first_file_matching,
        release_base_url, verify_checksum,
    },
    terminal_ui,
    workflow::{
        dev::{logs::ServiceLogSource, processes::ProcessSupervisor},
        project::{
            config::KalamProjectConfig,
            guidance::{
                dev_kalamdb_server_bin_missing, dev_kalamdb_server_non_interactive_download,
                dev_kalamdb_server_not_found, dev_local_kalamdb_server_start_failed,
            },
            templates::render_scaffold_file,
        },
    },
};
pub const DEFAULT_LOCAL_DEV_ROOT_PASSWORD: &str = "kalamdb123";
const SERVER_ARTIFACT_PREFIX: &str = "kalamdb-server";
const SERVER_RELEASE_BASE_URL_ENV: &str = "KALAMDB_SERVER_RELEASE_BASE_URL";
const SCAFFOLD_SERVER_CONFIG_PATH: &str = "kalam/server/server.toml";

pub fn local_server_root_password(
    project_root: &Path,
    config: &KalamProjectConfig,
) -> Result<Option<String>> {
    let config_path = config.local_server_config_path(project_root);
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
    let config_path = config.local_server_config_path(project_root);
    std::fs::create_dir_all(config.local_server_dir(project_root))?;
    std::fs::create_dir_all(config.local_server_dir(project_root).join("data"))?;
    std::fs::create_dir_all(config.local_server_dir(project_root).join("logs"))?;
    write_server_toml_if_missing(
        &config_path,
        &config.relative_local_server_data_path(),
        &config.relative_local_server_logs_path(),
        port,
    )?;
    Ok(config_path)
}

pub fn write_managed_server_config(
    layout: &crate::workflow::instance::ManagedLayout,
    http_port: u16,
    postgres_port: Option<u16>,
) -> Result<PathBuf> {
    layout.ensure_dirs()?;
    let data_path =
        crate::workflow::instance::relative_layout_path(&layout.data_dir, &layout.working_dir);
    let logs_path =
        crate::workflow::instance::relative_layout_path(&layout.logs_dir, &layout.working_dir);
    write_server_toml_if_missing(&layout.config_path, &data_path, &logs_path, http_port)?;
    crate::workflow::instance::update_server_listen_ports(
        &layout.config_path,
        http_port,
        postgres_port,
    )?;
    Ok(layout.config_path.clone())
}

fn write_server_toml_if_missing(
    config_path: &Path,
    data_path: &str,
    logs_path: &str,
    http_port: u16,
) -> Result<()> {
    if config_path.is_file() {
        return Ok(());
    }
    if let Some(parent) = config_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let contents = render_scaffold_file(
        SCAFFOLD_SERVER_CONFIG_PATH,
        &json!({
            "port": http_port,
            "data_path": data_path,
            "logs_path": logs_path,
            "root_password": DEFAULT_LOCAL_DEV_ROOT_PASSWORD,
        }),
    )?;
    std::fs::write(config_path, contents)?;
    Ok(())
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

pub fn resolve_kalamdb_server_bin_for_version(version: &str) -> Result<PathBuf> {
    resolve_kalamdb_server_bin_from_version(std::env::current_exe().ok(), version)
}

fn resolve_kalamdb_server_bin_from(current_exe: Option<PathBuf>) -> Result<PathBuf> {
    resolve_kalamdb_server_bin_from_version(current_exe, env!("CARGO_PKG_VERSION"))
}

fn resolve_kalamdb_server_bin_from_version(
    current_exe: Option<PathBuf>,
    version: &str,
) -> Result<PathBuf> {
    if let Ok(path) = env::var("KALAMDB_SERVER_BIN") {
        let path = PathBuf::from(path);
        if path.is_file() {
            return Ok(path);
        }
        return Err(CLIError::ConfigurationError(dev_kalamdb_server_bin_missing(&path)));
    }

    if let Some(path) = current_exe.as_deref().and_then(colocated_server_binary_path) {
        return Ok(path);
    }

    let versioned = crate::workflow::instance::versioned_server_binary_path(version);
    if versioned.is_file() {
        return Ok(versioned);
    }

    let managed_path = managed_server_binary_path();
    if managed_path.is_file() {
        match read_server_binary_version(&managed_path) {
            Ok(Some(found)) if found != version => {},
            Ok(_) | Err(_) => return Ok(managed_path),
        }
    }

    if let Some(path) = resolve_program_on_path("kalamdb-server") {
        return Ok(path);
    }

    Err(CLIError::ConfigurationError(dev_kalamdb_server_not_found()))
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

pub async fn ensure_local_server_binary_version(
    use_color: bool,
    auto_install: bool,
    output: &WorkflowOutput,
    server_source: &ServiceLogSource,
    version: &str,
) -> Result<PathBuf> {
    match resolve_kalamdb_server_bin_for_version(version) {
        Ok(path) => Ok(path),
        Err(error) => {
            output.status(format!("precheck: {error}"));
            if auto_install {
                output.status(format!("precheck: downloading kalamdb-server {version}"));
                return download_and_install_managed_server_version(output, server_source, version)
                    .await
                    .map_err(|download_error| map_server_download_error(output, download_error));
            }
            if !std::io::stdin().is_terminal() || !std::io::stdout().is_terminal() {
                return Err(CLIError::ConfigurationError(
                    dev_kalamdb_server_non_interactive_download(&error.to_string()),
                ));
            }

            let install_dir = crate::workflow::instance::versioned_server_install_dir(version);
            let confirmed = terminal_ui::prompt_confirm(
                &format!("Download KalamDB server {version} into {}", install_dir.display()),
                true,
                use_color,
            )
            .map_err(|prompt_error| {
                CLIError::FileError(format!("failed to read download confirmation: {prompt_error}"))
            })?;

            if !confirmed {
                return Err(CLIError::ConfigurationError(format!("{error}; download declined")));
            }

            output.status(format!("precheck: downloading kalamdb-server {version}"));
            download_and_install_managed_server_version(output, server_source, version).await
        },
    }
}

fn read_server_binary_version(path: &Path) -> Result<Option<String>> {
    let output = run_program(path, &["--version"], None).map_err(|error| {
        CLIError::ConfigurationError(format!(
            "failed to read version from kalamdb-server '{}': {error}",
            path.display()
        ))
    })?;

    if !output.status.success() {
        return Ok(None);
    }

    Ok(parse_server_version_output(&String::from_utf8_lossy(&output.stdout)))
}

fn parse_server_version_output(output: &str) -> Option<String> {
    let rest = output.trim().strip_prefix("KalamDB Server v")?;
    let version = rest.split(" |").next()?.trim();
    if version.is_empty() {
        return None;
    }
    Some(version.to_string())
}

/// Download and install the managed `kalamdb-server` release matching `version`.
pub async fn install_managed_server_version(version: &str, show_progress: bool) -> Result<PathBuf> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(60))
        .user_agent(format!("kalam-cli/{}", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|error| {
            CLIError::ConfigurationError(format!("failed to create HTTP client: {error}"))
        })?;

    let platform = detect_platform()?;
    let archive_kind = archive_kind_for_platform(&platform);
    let archive_name = archive_name(SERVER_ARTIFACT_PREFIX, version, &platform, archive_kind);
    let base_url = release_base_url(version, SERVER_RELEASE_BASE_URL_ENV)?;
    let archive_url = format!("{base_url}/{archive_name}");
    let checksums_url = format!("{base_url}/SHA256SUMS");

    let archive_bytes = download_bytes(&client, &archive_url, &archive_name, show_progress).await?;
    let checksums = download_text(&client, &checksums_url, "checksum file").await?;
    verify_checksum(&archive_name, &archive_bytes, &checksums)?;

    let temp_dir = create_temp_dir("kalamdb-server-install")?;
    let cleanup_dir = temp_dir.clone();
    let install_result = (|| -> Result<PathBuf> {
        extract_archive(&archive_bytes, archive_kind, &temp_dir)?;
        install_server_payload_for_version(&temp_dir, version)
    })();
    let _ = fs::remove_dir_all(cleanup_dir);
    install_result
}

async fn download_and_install_managed_server_version(
    output: &WorkflowOutput,
    _server_source: &ServiceLogSource,
    version: &str,
) -> Result<PathBuf> {
    let show_progress = !output.is_agent() && !output.json;
    let path = install_managed_server_version(version, show_progress).await?;
    output.status(format!("precheck: downloaded and verified kalamdb-server {version}"));
    output.agent_event("KALAM_SERVER_INSTALLED", &[("version", version)]);
    Ok(path)
}

fn map_server_download_error(output: &WorkflowOutput, error: CLIError) -> CLIError {
    if output.is_agent() || output.json {
        crate::agent_error::AgentError::server_download_failed(
            env!("CARGO_PKG_VERSION"),
            &error.to_string(),
        )
        .into()
    } else {
        error
    }
}

fn install_server_payload_for_version(extracted_root: &Path, version: &str) -> Result<PathBuf> {
    let install_dir = crate::workflow::instance::versioned_server_install_dir(version);
    let binary_path = crate::workflow::instance::versioned_server_binary_path(version);
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
            binary_path.clone()
        } else {
            install_dir.join(file_name)
        };
        copy_file_with_executable_bit(&file, &target)?;
    }

    Ok(binary_path)
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

pub const SERVER_READY_TIMEOUT_SECS: u64 = 60;
const SERVER_READY_PROGRESS_INTERVAL_SECS: u64 = 10;

pub async fn wait_for_server_ready(
    server_url: &str,
    server_program: &Path,
    output: &WorkflowOutput,
    supervisor: &mut ProcessSupervisor,
) -> Result<()> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(2))
        .build()
        .map_err(|e| CLIError::ConfigurationError(format!("failed to build http client: {e}")))?;

    let spinner = output.status_spinner(format!(
        "waiting for local KalamDB server (timeout {SERVER_READY_TIMEOUT_SECS}s)..."
    ));

    for attempt in 1..=SERVER_READY_TIMEOUT_SECS {
        if check_server_health(&client, server_url).await {
            drop(spinner);
            output.detail(format!("local KalamDB server ready ({attempt}s)"));
            return Ok(());
        }

        for (name, code) in supervisor.reap_finished().await {
            if name == "server" {
                return Err(CLIError::ConfigurationError(dev_local_kalamdb_server_start_failed(
                    server_program,
                    &format!("server exited with code {code} before becoming ready"),
                )));
            }
        }

        if !spinner.is_animating()
            && attempt > 1
            && attempt % SERVER_READY_PROGRESS_INTERVAL_SECS == 0
        {
            output.detail(format!(
                "still waiting for local KalamDB server \
                 ({attempt}/{SERVER_READY_TIMEOUT_SECS}s)..."
            ));
        }

        tokio::time::sleep(Duration::from_secs(1)).await;
    }

    Err(CLIError::ConfigurationError(dev_local_kalamdb_server_start_failed(
        server_program,
        &format!("timed out after {SERVER_READY_TIMEOUT_SECS}s waiting for {server_url}"),
    )))
}

pub async fn wait_for_http_ready(server_url: &str, pid: Option<u32>) -> Result<()> {
    for _ in 1..=SERVER_READY_TIMEOUT_SECS {
        if let Some(pid) = pid {
            if !crate::process::pid_is_running(pid) {
                return Err(CLIError::from(crate::agent_error::AgentError::server_start_failed(
                    &format!("kalamdb-server exited before becoming ready at {server_url}"),
                )));
            }
        }
        if server_already_ready(server_url).await {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
    Err(CLIError::from(crate::agent_error::AgentError::server_start_failed(&format!(
        "timed out after {SERVER_READY_TIMEOUT_SECS}s waiting for {server_url}"
    ))))
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workflow::test_support::parse_minimal_project_config;

    #[test]
    fn write_local_server_config_uses_requested_port() {
        let temp = tempfile::TempDir::new().unwrap();
        let config = parse_minimal_project_config();
        let path = write_local_server_config(temp.path(), &config, 3001).unwrap();
        let contents = std::fs::read_to_string(path).unwrap();
        assert!(contents.contains("port = 3001"));
        assert!(temp.path().join("kalam/server/data").is_dir());
        assert!(temp.path().join("kalam/server/logs").is_dir());
    }

    #[test]
    fn write_local_server_config_uses_project_server_directory() {
        let temp = tempfile::TempDir::new().unwrap();
        let config = parse_minimal_project_config();
        let path = write_local_server_config(temp.path(), &config, 2900).unwrap();
        assert_eq!(path, temp.path().join("kalam/server/server.toml"));
    }

    #[test]
    fn write_local_server_config_includes_required_server_sections() {
        let temp = tempfile::TempDir::new().unwrap();
        let config = parse_minimal_project_config();
        let path = write_local_server_config(temp.path(), &config, 2900).unwrap();
        let contents = std::fs::read_to_string(path).unwrap();
        assert!(contents.contains("[limits]"));
        assert!(contents.contains("[logging]"));
        assert!(contents.contains("[performance]"));
        assert!(contents.contains("[rate_limit]"));
        assert!(contents.contains("max_queries_per_sec = 100000"));
        assert!(contents.contains("[postgres_wire]"));
        assert!(contents.contains("enabled = false"));
        assert!(!contents.contains("pg_catalog_enabled"));
        assert!(contents.contains("data_path = \"kalam/server/data\""));
        assert!(contents.contains("logs_path = \"kalam/server/logs\""));
        assert!(
            contents.contains(&format!("root_password = \"{DEFAULT_LOCAL_DEV_ROOT_PASSWORD}\""))
        );
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
    fn parse_server_version_output_reads_release_version() {
        assert_eq!(
            parse_server_version_output(
                "KalamDB Server v0.5.2-rc.1 | Build: 2026-06-06 10:26:45 UTC\n"
            ),
            Some("0.5.2-rc.1".to_string())
        );
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
