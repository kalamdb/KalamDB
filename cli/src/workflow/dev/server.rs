//! Local KalamDB server lifecycle helpers for `kalam dev`.

use std::{
    env, fs,
    io::IsTerminal,
    path::{Path, PathBuf},
    time::Duration,
};

use reqwest::StatusCode;
use serde_json::json;
use url::Url;

use crate::{
    error::{CLIError, Result},
    history::get_kalam_config_dir,
    output::WorkflowOutput,
    process::{resolve_program_on_path, run_program, shell_working_directory},
    release_download::{
        archive_kind_for_platform, archive_name, copy_file_with_executable_bit, create_temp_dir,
        detect_platform, download_bytes, download_text, extract_archive, find_first_file_matching,
        release_base_url, verify_checksum,
    },
    terminal_ui,
    workflow::{
        dev::{
            logs::ServiceLogSource,
            processes::ProcessSupervisor,
        },
        project::{
            config::KalamProjectConfig,
            guidance::{
                dev_kalamdb_server_bin_missing, dev_kalamdb_server_non_interactive_download,
                dev_kalamdb_server_not_found, dev_local_kalamdb_server_start_failed,
            },
            templates::{find_template_file, render_template, resolve_scaffold_template},
        },
    },
};

pub const DEFAULT_DEV_SERVER_URL: &str = "http://localhost:2900";
const SERVER_ARTIFACT_PREFIX: &str = "kalamdb-server";
const SERVER_RELEASE_BASE_URL_ENV: &str = "KALAMDB_SERVER_RELEASE_BASE_URL";
const SCAFFOLD_SERVER_CONFIG_PATH: &str = "kalam/server/server.toml";

fn scaffold_template_file(project_path: &str) -> Result<&'static str> {
    let template = resolve_scaffold_template()?;
    find_template_file(template, project_path).ok_or_else(|| {
        CLIError::ConfigurationError(format!("missing scaffold template file '{project_path}'"))
    })
}

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
    let template = scaffold_template_file(SCAFFOLD_SERVER_CONFIG_PATH)?;
    let contents = render_template(
        template,
        &json!({
            "port": port,
            "data_path": data_path,
            "logs_path": logs_path,
        }),
    )?;
    std::fs::write(&config_path, contents)?;
    Ok(config_path)
}

pub fn managed_server_install_dir() -> PathBuf {
    get_kalam_config_dir().join("bin")
}

pub fn managed_server_binary_path() -> PathBuf {
    managed_server_install_dir().join(server_binary_name())
}

#[cfg(windows)]
const MANAGED_SERVER_RUNTIME_DLLS: &[&str] = &[
    "msvcp140.dll",
    "vcruntime140.dll",
    "vcruntime140_1.dll",
];

#[cfg(windows)]
fn managed_server_runtime_is_complete(install_dir: &Path) -> bool {
    MANAGED_SERVER_RUNTIME_DLLS
        .iter()
        .all(|name| install_dir.join(name).is_file())
}

#[cfg(not(windows))]
fn managed_server_runtime_is_complete(_install_dir: &Path) -> bool {
    true
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
        return Err(CLIError::ConfigurationError(dev_kalamdb_server_bin_missing(&path)));
    }

    if let Some(path) = current_exe.as_deref().and_then(colocated_server_binary_path) {
        return Ok(path);
    }

    let managed_path = managed_server_binary_path();
    if managed_path.is_file() {
        return Ok(managed_path);
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

pub async fn ensure_local_server_binary(
    use_color: bool,
    output: &WorkflowOutput,
    server_source: &ServiceLogSource,
) -> Result<PathBuf> {
    match resolve_kalamdb_server_bin() {
        Ok(path) => {
            if is_managed_server_binary(&path)
                && !managed_server_runtime_is_complete(&managed_server_install_dir())
            {
                output.status(
                    "precheck: managed kalamdb-server is missing Windows runtime DLLs; redownloading",
                );
                return download_and_install_managed_server(output, server_source).await;
            }
            if let Some(installed_version) = managed_server_version_if_stale(&path)? {
                return refresh_managed_server_binary(
                    use_color,
                    output,
                    server_source,
                    &installed_version,
                )
                .await;
            }
            Ok(path)
        },
        Err(error) => {
            output.status(format!("precheck: {error}"));
            if !std::io::stdin().is_terminal() || !std::io::stdout().is_terminal() {
                return Err(CLIError::ConfigurationError(
                    dev_kalamdb_server_non_interactive_download(&error.to_string()),
                ));
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

            output.status("precheck: downloading kalamdb-server");
            download_and_install_managed_server(output, server_source).await
        },
    }
}

async fn refresh_managed_server_binary(
    use_color: bool,
    output: &WorkflowOutput,
    server_source: &ServiceLogSource,
    installed_version: &str,
) -> Result<PathBuf> {
    let target_version = env!("CARGO_PKG_VERSION");
    output.status(format!(
        "precheck: managed kalamdb-server is {installed_version}, updating to {target_version}"
    ));

    if !std::io::stdin().is_terminal() || !std::io::stdout().is_terminal() {
        output.status("precheck: downloading kalamdb-server");
        return download_and_install_managed_server(output, server_source).await;
    }

    let install_dir = managed_server_install_dir();
    let confirmed = terminal_ui::prompt_confirm(
        &format!(
            "Update managed KalamDB server from {installed_version} to {target_version} in {}",
            install_dir.display()
        ),
        true,
        use_color,
    )
    .map_err(|prompt_error| {
        CLIError::FileError(format!("failed to read update confirmation: {prompt_error}"))
    })?;

    if !confirmed {
        return Err(CLIError::ConfigurationError(format!(
            "managed kalamdb-server is {installed_version}, but CLI expects {target_version}; update declined"
        )));
    }

    output.status("precheck: downloading kalamdb-server");
    download_and_install_managed_server(output, server_source).await
}

fn managed_server_version_if_stale(path: &Path) -> Result<Option<String>> {
    if !is_managed_server_binary(path) {
        return Ok(None);
    }

    let expected_version = env!("CARGO_PKG_VERSION");
    let installed_version = read_server_binary_version(path)?;
    if installed_version.as_deref() == Some(expected_version) {
        return Ok(None);
    }

    Ok(Some(installed_version.unwrap_or_else(|| "unknown".to_string())))
}

fn is_managed_server_binary(path: &Path) -> bool {
    let managed_path = managed_server_binary_path();
    if path == managed_path {
        return true;
    }

    match (path.canonicalize(), managed_path.canonicalize()) {
        (Ok(path), Ok(managed_path)) => path == managed_path,
        _ => false,
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

pub struct LocalServerLaunch {
    pub program: PathBuf,
    pub config_path: PathBuf,
    pub working_dir: PathBuf,
}

pub fn prepare_local_server_launch(
    project_root: &Path,
    config: &KalamProjectConfig,
    server_url: &str,
) -> Result<LocalServerLaunch> {
    let port = parse_server_port(server_url)?;
    let config_path = local_server_config_path(project_root, config);
    if !config_path.is_file() {
        write_local_server_config(project_root, config, port)?;
    }

    Ok(LocalServerLaunch {
        program: resolve_kalamdb_server_bin()?,
        config_path: shell_working_directory(&config_path),
        working_dir: shell_working_directory(project_root),
    })
}

async fn download_and_install_managed_server(
    output: &WorkflowOutput,
    _server_source: &ServiceLogSource,
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
    let base_url = release_base_url(version, SERVER_RELEASE_BASE_URL_ENV)?;
    let archive_url = format!("{base_url}/{archive_name}");
    let checksums_url = format!("{base_url}/SHA256SUMS");

    let archive_bytes = download_bytes(&client, &archive_url, &archive_name, true).await?;
    let checksums = download_text(&client, &checksums_url, "checksum file").await?;
    verify_checksum(&archive_name, &archive_bytes, &checksums)?;
    output.status("precheck: downloaded and verified kalamdb-server");

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

    output.detail(format!(
        "waiting for local KalamDB server (timeout {SERVER_READY_TIMEOUT_SECS}s)..."
    ));

    for attempt in 1..=SERVER_READY_TIMEOUT_SECS {
        if check_server_health(&client, server_url).await {
            output.detail(format!("local KalamDB server ready ({attempt}s)"));
            return Ok(());
        }

        for (name, code) in supervisor.reap_finished().await {
            if name == "server" {
                return Err(CLIError::ConfigurationError(
                    dev_local_kalamdb_server_start_failed(
                        server_program,
                        &format!("server exited with code {code} before becoming ready"),
                    ),
                ));
            }
        }

        if attempt > 1 && attempt % SERVER_READY_PROGRESS_INTERVAL_SECS == 0 {
            output.detail(format!(
                "still waiting for local KalamDB server ({attempt}/{SERVER_READY_TIMEOUT_SECS}s)..."
            ));
        }

        tokio::time::sleep(Duration::from_secs(1)).await;
    }

    Err(CLIError::ConfigurationError(dev_local_kalamdb_server_start_failed(
        server_program,
        &format!("timed out after {SERVER_READY_TIMEOUT_SECS}s waiting for {server_url}"),
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
        assert!(contents.contains("[rate_limit]"));
        assert!(contents.contains("max_queries_per_sec = 100000"));
        assert!(contents.contains("data_path = \"kalam/server/data\""));
        assert!(contents.contains("logs_path = \"kalam/server/logs\""));
    }

    #[test]
    fn prepare_local_server_launch_preserves_existing_server_config() {
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

        let _ = prepare_local_server_launch(temp.path(), &config, "http://localhost:2900").unwrap();
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
    fn parse_server_version_output_reads_release_version() {
        assert_eq!(
            parse_server_version_output(
                "KalamDB Server v0.5.2-rc.1 | Build: 2026-06-06 10:26:45 UTC\n"
            ),
            Some("0.5.2-rc.1".to_string())
        );
    }

    #[test]
    #[cfg(windows)]
    fn managed_server_runtime_is_complete_requires_runtime_dlls() {
        let temp = tempfile::TempDir::new().unwrap();
        assert!(!managed_server_runtime_is_complete(temp.path()));
        for (index, name) in MANAGED_SERVER_RUNTIME_DLLS.iter().enumerate() {
            std::fs::write(temp.path().join(name), b"x").unwrap();
            let complete = managed_server_runtime_is_complete(temp.path());
            if index + 1 == MANAGED_SERVER_RUNTIME_DLLS.len() {
                assert!(complete);
            } else {
                assert!(!complete);
            }
        }
    }

    #[test]
    fn managed_server_version_if_stale_ignores_explicit_override() {
        let temp = tempfile::TempDir::new().unwrap();
        let custom_bin = temp.path().join("custom-kalamdb-server");
        std::fs::write(&custom_bin, "#!/bin/sh\necho 'KalamDB Server v0.1.0 | Build: old'\n")
            .unwrap();

        let original_server_bin = std::env::var_os("KALAMDB_SERVER_BIN");
        std::env::set_var("KALAMDB_SERVER_BIN", &custom_bin);

        let stale = managed_server_version_if_stale(&custom_bin).expect("check custom binary");
        assert!(stale.is_none());

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
