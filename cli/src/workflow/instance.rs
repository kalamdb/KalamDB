//! Managed local database identity, layout, and port allocation.
//!
//! Runtime identity lives in `run/instance.json`. A healthy HTTP port is not
//! enough to claim a database: the live PID must match this record.

use std::{
    fs,
    net::TcpListener,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};

use crate::{
    error::{CLIError, Result},
    fs_atomic::{write_atomic, FileReadPolicy, FileWriteOptions},
    history::get_kalam_config_dir,
    process::{pid_is_running, process_matches_executable, request_terminate, SupervisedKillScope},
};

pub const INSTANCE_SCHEMA_VERSION: u32 = 1;
pub const DEFAULT_HTTP_PORT: u16 = 2900;
pub const SHARED_SERVER_NAME: &str = "default";
const PORT_SEARCH_SPAN: u16 = 100;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TargetKind {
    ProjectLocal,
    SharedLocal,
    Remote,
}

pub fn default_http_url(port: u16) -> String {
    format!("http://127.0.0.1:{port}")
}

pub fn shared_server_root() -> PathBuf {
    get_kalam_config_dir().join("servers").join(SHARED_SERVER_NAME)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManagedLayout {
    pub root:          PathBuf,
    pub working_dir:   PathBuf,
    pub data_dir:      PathBuf,
    pub logs_dir:      PathBuf,
    pub run_dir:       PathBuf,
    pub config_path:   PathBuf,
    pub log_file:      PathBuf,
    pub instance_path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstanceRecord {
    pub schema_version:   u32,
    pub kind:             TargetKind,
    pub url:              String,
    pub http_port:        u16,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub postgres_port:    Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub namespace:        Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pid:              Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dev_pid:          Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exe:              Option<PathBuf>,
    pub data_dir:         PathBuf,
    pub log_path:         PathBuf,
    pub config_path:      PathBuf,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_root:     Option<PathBuf>,
    #[serde(default)]
    pub started_by:       StartedBy,
    #[serde(default)]
    pub keep_on_dev_exit: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub server_version:   Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seed_sql_hash:    Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started_at:       Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum StartedBy {
    #[default]
    Up,
    Dev,
    Exec,
}

impl ManagedLayout {
    pub fn ensure_dirs(&self) -> Result<()> {
        for dir in [&self.data_dir, &self.logs_dir, &self.run_dir] {
            fs::create_dir_all(dir).map_err(|error| {
                CLIError::FileError(format!("failed to create '{}': {error}", dir.display()))
            })?;
        }
        Ok(())
    }
}

/// Layout for a project-local database.
///
/// Existing `kalam/server` directories are honored. New empty-folder databases
/// use `.kalam/`.
pub fn project_layout(project_root: &Path) -> ManagedLayout {
    let runtime = project_root.join(".kalam");
    let run_dir = runtime.join("run");
    let legacy = project_root.join("kalam").join("server");
    if legacy.exists() {
        return ManagedLayout {
            root:          legacy.clone(),
            working_dir:   project_root.to_path_buf(),
            data_dir:      legacy.join("data"),
            logs_dir:      legacy.join("logs"),
            run_dir:       run_dir.clone(),
            config_path:   legacy.join("server.toml"),
            log_file:      legacy.join("logs").join("console.log"),
            instance_path: run_dir.join("instance.json"),
        };
    }

    let tracked_config = project_root.join("kalam").join("server.toml");
    ManagedLayout {
        root:          runtime.clone(),
        working_dir:   project_root.to_path_buf(),
        data_dir:      runtime.join("data"),
        logs_dir:      runtime.join("logs"),
        run_dir:       run_dir.clone(),
        config_path:   if tracked_config.exists() {
            tracked_config
        } else {
            runtime.join("server.toml")
        },
        log_file:      runtime.join("logs").join("console.log"),
        instance_path: run_dir.join("instance.json"),
    }
}

pub fn shared_layout() -> ManagedLayout {
    let root = shared_server_root();
    let run_dir = root.join("run");
    ManagedLayout {
        root:          root.clone(),
        working_dir:   root.clone(),
        data_dir:      root.join("data"),
        logs_dir:      root.join("logs"),
        run_dir:       run_dir.clone(),
        config_path:   root.join("server.toml"),
        log_file:      root.join("logs").join("console.log"),
        instance_path: run_dir.join("instance.json"),
    }
}

pub fn load_instance(layout: &ManagedLayout) -> Result<Option<InstanceRecord>> {
    let Some(contents) = crate::fs_atomic::read_to_string_if_exists(
        &layout.instance_path,
        FileReadPolicy::UserProvided,
    )
    .map_err(|error| {
        CLIError::FileError(format!("failed to read '{}': {error}", layout.instance_path.display()))
    })?
    else {
        return Ok(None);
    };
    match serde_json::from_str(&contents) {
        Ok(record) => Ok(Some(record)),
        Err(_) => Ok(None),
    }
}

pub fn save_instance(layout: &ManagedLayout, record: &InstanceRecord) -> Result<()> {
    layout.ensure_dirs()?;
    let payload = serde_json::to_vec_pretty(record).map_err(|error| {
        CLIError::FileError(format!("failed to serialize instance identity: {error}"))
    })?;
    write_atomic(&layout.instance_path, &payload, FileWriteOptions::DEFAULT).map_err(|error| {
        CLIError::FileError(format!(
            "failed to write '{}': {error}",
            layout.instance_path.display()
        ))
    })
}

pub fn live_instance(layout: &ManagedLayout) -> Result<Option<InstanceRecord>> {
    let Some(record) = load_instance(layout)? else {
        return Ok(None);
    };
    if instance_is_live(&record) {
        Ok(Some(record))
    } else {
        Ok(None)
    }
}

pub fn instance_is_live(record: &InstanceRecord) -> bool {
    let Some(pid) = record.pid else {
        return false;
    };
    match &record.exe {
        Some(exe) => process_matches_executable(pid, exe),
        None => pid_is_running(pid),
    }
}

pub fn instance_owns_url(record: &InstanceRecord, url: &str) -> bool {
    instance_is_live(record) && normalize_url(&record.url) == normalize_url(url)
}

pub fn allocate_http_port(preferred: u16, explicit: bool, owned_port: Option<u16>) -> Result<u16> {
    if owned_port == Some(preferred) && !port_is_available(preferred) {
        // Occupied, but we already claim it via instance identity. Caller verifies ownership.
        return Ok(preferred);
    }
    if port_is_available(preferred) {
        return Ok(preferred);
    }
    if explicit {
        return Err(CLIError::from(
            crate::agent_error::AgentError::new(
                crate::agent_error::AgentErrorCode::PortInUse,
                format!("port {preferred} is already in use"),
            )
            .with_field("port", preferred.to_string())
            .with_action("stop the other process or omit --port to allocate a free port"),
        ));
    }
    for port in preferred.saturating_add(1)..=preferred.saturating_add(PORT_SEARCH_SPAN) {
        if port_is_available(port) {
            return Ok(port);
        }
    }
    Err(CLIError::ConfigurationError(format!(
        "could not find a free HTTP port near {preferred}"
    )))
}

pub fn port_is_available(port: u16) -> bool {
    TcpListener::bind(("127.0.0.1", port)).is_ok()
}

pub fn parse_server_listen_ports(config_path: &Path) -> Result<(u16, Option<u16>)> {
    if !config_path.is_file() {
        return Ok((DEFAULT_HTTP_PORT, None));
    }
    let contents = fs::read_to_string(config_path).map_err(|error| {
        CLIError::FileError(format!("failed to read '{}': {error}", config_path.display()))
    })?;
    let value: toml::Value = toml::from_str(&contents).map_err(|error| {
        CLIError::ConfigurationError(format!(
            "failed to parse '{}': {error}",
            config_path.display()
        ))
    })?;
    let http = value
        .get("server")
        .and_then(|server| server.get("port"))
        .and_then(toml::Value::as_integer)
        .unwrap_or(DEFAULT_HTTP_PORT as i64) as u16;
    let postgres_enabled = value
        .get("postgres_wire")
        .and_then(|wire| wire.get("enabled"))
        .and_then(toml::Value::as_bool)
        .unwrap_or(false);
    let postgres = if postgres_enabled {
        value
            .get("postgres_wire")
            .and_then(|wire| wire.get("port"))
            .and_then(toml::Value::as_integer)
            .map(|port| port as u16)
    } else {
        None
    };
    Ok((http, postgres))
}

pub fn update_server_listen_ports(
    config_path: &Path,
    http_port: u16,
    postgres_port: Option<u16>,
) -> Result<()> {
    if !config_path.is_file() {
        return Ok(());
    }
    let contents = fs::read_to_string(config_path).map_err(|error| {
        CLIError::FileError(format!("failed to read '{}': {error}", config_path.display()))
    })?;
    let mut value: toml::Value = toml::from_str(&contents).map_err(|error| {
        CLIError::ConfigurationError(format!(
            "failed to parse '{}': {error}",
            config_path.display()
        ))
    })?;
    if let Some(server) = value.get_mut("server").and_then(toml::Value::as_table_mut) {
        server.insert("port".into(), toml::Value::Integer(http_port as i64));
    }
    if let Some(port) = postgres_port {
        if let Some(wire) = value.get_mut("postgres_wire").and_then(toml::Value::as_table_mut) {
            wire.insert("port".into(), toml::Value::Integer(port as i64));
        }
    }
    let serialized = toml::to_string_pretty(&value).map_err(|error| {
        CLIError::ConfigurationError(format!("failed to serialize server.toml: {error}"))
    })?;
    fs::write(config_path, serialized).map_err(|error| {
        CLIError::FileError(format!("failed to write '{}': {error}", config_path.display()))
    })
}

pub fn clear_live_pids(record: &mut InstanceRecord) {
    record.pid = None;
    record.dev_pid = None;
}

pub fn stop_recorded_processes(record: &InstanceRecord) {
    if let Some(dev_pid) = record.dev_pid {
        if pid_is_running(dev_pid) {
            wait_for_cooperative_exit(dev_pid, SupervisedKillScope::Tree);
        }
    }
    if let Some(pid) = record.pid {
        if pid_is_running(pid) {
            wait_for_cooperative_exit(pid, SupervisedKillScope::Process);
        }
    }
}

fn wait_for_cooperative_exit(pid: u32, scope: SupervisedKillScope) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    request_terminate(pid);
    while pid_is_running(pid) && std::time::Instant::now() < deadline {
        request_terminate(pid);
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    if pid_is_running(pid) {
        crate::process::kill_supervised_process_by_pid(pid, scope);
    }
}

pub fn versioned_server_install_dir(version: &str) -> PathBuf {
    get_kalam_config_dir().join("bin").join(version)
}

pub fn versioned_server_binary_path(version: &str) -> PathBuf {
    let name = if cfg!(windows) {
        "kalamdb-server.exe"
    } else {
        "kalamdb-server"
    };
    versioned_server_install_dir(version).join(name)
}

pub fn new_instance_record(
    kind: TargetKind,
    layout: &ManagedLayout,
    http_port: u16,
    postgres_port: Option<u16>,
    namespace: Option<&str>,
    started_by: StartedBy,
    keep_on_dev_exit: bool,
    server_version: Option<String>,
    project_root: Option<PathBuf>,
) -> InstanceRecord {
    InstanceRecord {
        schema_version: INSTANCE_SCHEMA_VERSION,
        kind,
        url: default_http_url(http_port),
        http_port,
        postgres_port,
        namespace: namespace.map(ToString::to_string),
        pid: None,
        dev_pid: None,
        exe: None,
        data_dir: layout.data_dir.clone(),
        log_path: layout.log_file.clone(),
        config_path: layout.config_path.clone(),
        project_root,
        started_by,
        keep_on_dev_exit,
        server_version,
        seed_sql_hash: None,
        started_at: Some(chrono::Utc::now().to_rfc3339()),
    }
}

pub fn relative_layout_path(path: &Path, working_dir: &Path) -> String {
    path.strip_prefix(working_dir)
        .map(|relative| relative.to_string_lossy().replace('\\', "/"))
        .unwrap_or_else(|_| path.display().to_string())
}

pub fn register_dev_pid(layout: &ManagedLayout, dev_pid: u32) -> Result<()> {
    let Some(mut record) = load_instance(layout)? else {
        return Ok(());
    };
    record.dev_pid = Some(dev_pid);
    save_instance(layout, &record)
}

pub fn clear_dev_pid(layout: &ManagedLayout) -> Result<()> {
    let Some(mut record) = load_instance(layout)? else {
        return Ok(());
    };
    record.dev_pid = None;
    save_instance(layout, &record)
}

fn normalize_url(url: &str) -> String {
    url.trim().trim_end_matches('/').to_ascii_lowercase()
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::*;

    #[test]
    fn honors_existing_kalam_server_directory() {
        let temp = TempDir::new().unwrap();
        let legacy = temp.path().join("kalam/server");
        fs::create_dir_all(&legacy).unwrap();
        let layout = project_layout(temp.path());
        assert_eq!(layout.root, legacy);
        assert_eq!(layout.data_dir, legacy.join("data"));
        assert_eq!(layout.instance_path, temp.path().join(".kalam/run/instance.json"));
    }

    #[test]
    fn empty_folder_uses_dot_kalam() {
        let temp = TempDir::new().unwrap();
        let layout = project_layout(temp.path());
        assert_eq!(layout.root, temp.path().join(".kalam"));
        assert_ne!(layout.log_file, layout.logs_dir.join("server.log"));
        assert_eq!(layout.data_dir, temp.path().join(".kalam/data"));
        assert_eq!(layout.config_path, temp.path().join(".kalam/server.toml"));
    }

    #[test]
    fn allocate_http_port_reuses_free_preferred() {
        for _ in 0..16 {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let preferred = listener.local_addr().unwrap().port();
            drop(listener);
            if !port_is_available(preferred) {
                continue;
            }
            let allocated = allocate_http_port(preferred, false, None).unwrap();
            assert_eq!(allocated, preferred);
            return;
        }
        panic!("could not find a free ephemeral port to reuse");
    }

    #[test]
    fn allocate_http_port_errors_when_explicit_port_is_busy() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let occupied = listener.local_addr().unwrap().port();
        let error = allocate_http_port(occupied, true, None).unwrap_err().to_string();
        assert!(
            error.contains("already in use")
                || error.contains("PORT_IN_USE")
                || error.contains(&occupied.to_string())
        );
    }

    #[test]
    fn allocate_http_port_skips_busy_preferred_when_not_explicit() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let occupied = listener.local_addr().unwrap().port();
        let allocated = allocate_http_port(occupied, false, None).unwrap();
        assert_ne!(allocated, occupied);
        assert!(port_is_available(allocated));
    }
}
