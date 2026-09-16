//! Shared local-database prepare/start helpers for `up`, `dev`, reset, and `--exec`.

use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

use crate::{
    error::{CLIError, Result},
    output::WorkflowOutput,
    workflow::{
        instance::{
            self, allocate_http_port, default_http_url, live_instance, new_instance_record,
            parse_server_listen_ports, relative_layout_path, save_instance,
            stop_recorded_processes, InstanceRecord, ManagedLayout, StartedBy,
        },
        target::ResolvedTarget,
    },
};

pub struct PreparedServer {
    pub layout:          ManagedLayout,
    pub record:          InstanceRecord,
    pub program:         PathBuf,
    pub already_running: bool,
}

pub fn prepare_managed_server(
    target: &ResolvedTarget,
    explicit_port: Option<u16>,
    started_by: StartedBy,
    keep_on_dev_exit: bool,
    server_version: Option<String>,
) -> Result<PreparedServer> {
    if !target.is_managed_local() {
        return Err(CLIError::ConfigurationError(
            "`up` and `down` manage local databases only; remote environments need a separate \
             cloud or host operation"
                .into(),
        ));
    }
    let layout = target.layout.clone().ok_or_else(|| {
        CLIError::ConfigurationError("managed local database is missing a runtime layout".into())
    })?;
    layout.ensure_dirs()?;

    if let Some(record) = live_instance(&layout)? {
        if instance::instance_owns_url(&record, &target.url) || instance::instance_is_live(&record)
        {
            return Ok(PreparedServer {
                layout,
                record,
                program: PathBuf::new(),
                already_running: true,
            });
        }
    }

    if let Some(stale) = instance::load_instance(&layout)? {
        if !instance::instance_is_live(&stale) {
            let mut cleared = stale;
            instance::clear_live_pids(&mut cleared);
            save_instance(&layout, &cleared)?;
        }
    }

    let preferred = explicit_port
        .or_else(|| instance::load_instance(&layout).ok().flatten().map(|record| record.http_port))
        .unwrap_or(parse_preferred_http_port(target, &layout)?);
    let owned_port = instance::load_instance(&layout)
        .ok()
        .flatten()
        .and_then(|record| record.pid.is_some().then_some(record.http_port));
    let http_port = allocate_http_port(preferred, explicit_port.is_some(), owned_port)?;

    let (_, configured_postgres) = parse_server_listen_ports(&layout.config_path)?;
    let postgres_port = match configured_postgres {
        Some(port) => Some(allocate_http_port(port, false, None)?),
        None => None,
    };

    let program = PathBuf::new();

    let mut record = new_instance_record(
        target.kind,
        &layout,
        http_port,
        postgres_port,
        Some(target.namespace.as_str()),
        started_by,
        keep_on_dev_exit,
        server_version,
        target.project_root.clone(),
    );
    if let Some(existing) = instance::load_instance(&layout)? {
        record.seed_sql_hash = existing.seed_sql_hash;
    }
    save_instance(&layout, &record)?;

    Ok(PreparedServer {
        layout,
        record,
        program,
        already_running: false,
    })
}

pub async fn attach_or_start_managed_server(
    target: &ResolvedTarget,
    explicit_port: Option<u16>,
    started_by: StartedBy,
    keep_on_dev_exit: bool,
    server_version: Option<String>,
    output: &WorkflowOutput,
) -> Result<PreparedServer> {
    let version = server_version.unwrap_or_else(|| crate::CLI_VERSION.to_string());
    let mut prepared = prepare_managed_server(
        target,
        explicit_port,
        started_by,
        keep_on_dev_exit,
        Some(version.clone()),
    )?;
    if prepared.already_running {
        output.status(format!("database already running at {}", prepared.record.url));
        output.agent_event("KALAM_SERVER_REUSED", &[("url", &prepared.record.url)]);
        return Ok(prepared);
    }

    let dummy_source = crate::workflow::dev::logs::ServiceLogSource::new(
        "server",
        crate::workflow::dev::logs::ServiceColor::Cyan,
    );
    let program = crate::workflow::dev::server::ensure_local_server_binary_version(
        output.use_color,
        output.is_agent() || output.json,
        output,
        &dummy_source,
        &version,
    )
    .await?;
    prepared.program = program.clone();

    crate::workflow::dev::server::write_managed_server_config(
        &prepared.layout,
        prepared.record.http_port,
        prepared.record.postgres_port,
    )?;

    let pid = spawn_detached_server(
        &program,
        &prepared.layout.config_path,
        &prepared.layout.working_dir,
        &prepared.layout.log_file,
    )?;
    prepared.record.pid = Some(pid);
    prepared.record.exe = Some(program.clone());
    prepared.record.url = default_http_url(prepared.record.http_port);
    save_instance(&prepared.layout, &prepared.record)?;

    crate::workflow::dev::server::wait_for_http_ready(&prepared.record.url, Some(pid))
        .await
        .map_err(|error| {
            stop_recorded_processes(&prepared.record);
            error
        })?;
    output.status(format!("database started at {}", prepared.record.url));
    output.agent_event("KALAM_SERVER_STARTED", &[("url", &prepared.record.url)]);
    Ok(prepared)
}

pub fn spawn_detached_server(
    program: &Path,
    config_path: &Path,
    working_dir: &Path,
    log_file: &Path,
) -> Result<u32> {
    if let Some(parent) = log_file.parent() {
        fs::create_dir_all(parent).map_err(|error| {
            CLIError::FileError(format!("failed to create '{}': {error}", parent.display()))
        })?;
    }
    let mut log = OpenOptions::new().create(true).append(true).open(log_file).map_err(|error| {
        CLIError::FileError(format!("failed to open '{}': {error}", log_file.display()))
    })?;
    writeln!(&mut log, "--- kalam up ---").ok();
    let log_err = log.try_clone().map_err(|error| {
        CLIError::FileError(format!("failed to duplicate '{}': {error}", log_file.display()))
    })?;

    let mut command = Command::new(program);
    command
        .arg(config_path)
        .stdin(Stdio::null())
        .stdout(log)
        .stderr(log_err)
        .current_dir(working_dir);

    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;

        unsafe {
            command.pre_exec(|| {
                libc::setsid();
                Ok(())
            });
        }
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;

        const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
        const DETACHED_PROCESS: u32 = 0x0000_0008;
        command.creation_flags(CREATE_NEW_PROCESS_GROUP | DETACHED_PROCESS);
    }

    let child = command.spawn().map_err(|error| {
        CLIError::from(crate::agent_error::AgentError::server_start_failed(&format!(
            "failed to start kalamdb-server: {error}"
        )))
    })?;
    Ok(child.id())
}

pub fn clear_managed_data(layout: &ManagedLayout, output: &WorkflowOutput) -> Result<usize> {
    let mut removed = 0usize;
    if layout.data_dir.exists() {
        fs::remove_dir_all(&layout.data_dir).map_err(|error| {
            CLIError::FileError(format!(
                "failed to remove '{}': {error}",
                layout.data_dir.display()
            ))
        })?;
        removed += 1;
        output.status(format!(
            "removed {}",
            relative_layout_path(&layout.data_dir, &layout.working_dir)
        ));
    }
    fs::create_dir_all(&layout.data_dir).map_err(|error| {
        CLIError::FileError(format!("failed to recreate '{}': {error}", layout.data_dir.display()))
    })?;
    Ok(removed)
}

fn parse_preferred_http_port(target: &ResolvedTarget, layout: &ManagedLayout) -> Result<u16> {
    if let Ok(port) = crate::workflow::project::connection_url::parse_server_port(&target.url) {
        if port != 0 {
            return Ok(port);
        }
    }
    Ok(parse_server_listen_ports(&layout.config_path)?.0)
}
