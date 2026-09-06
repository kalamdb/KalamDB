//! Background `kalam dev` session file, spawn, status, logs, and stop.

use std::{
    env,
    fs::{self, File, OpenOptions},
    io::{self, Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::Duration,
};

use serde::{Deserialize, Serialize};
use tokio::time::{self, Instant};

use crate::{
    agent_error::AgentError,
    error::{CLIError, Result},
    fs_atomic::{write_atomic, FileReadPolicy, FileWriteOptions},
    process::{kill_supervised_process_by_pid, SupervisedKillScope},
    workflow::WorkflowContext,
};

const START_TIMEOUT: Duration = Duration::from_secs(60);
const START_POLL: Duration = Duration::from_millis(50);
const STOP_WAIT: Duration = Duration::from_secs(8);
const STOP_POLL: Duration = Duration::from_millis(100);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DevSession {
    pub pid:          u32,
    pub started_at:   String,
    pub project_root: PathBuf,
    pub log_path:     PathBuf,
    pub exe:          PathBuf,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url:          Option<String>,
}

pub async fn start_background_session(ctx: &WorkflowContext, force: bool) -> Result<()> {
    let output = ctx.output();
    let session_path = ctx.config.dev_session_path(&ctx.project_root);
    let log_path = ctx.config.workflow_log_path(&ctx.project_root);

    if let Some(existing) = live_session(&session_path)? {
        emit_started(ctx, "KALAM_DEV_REUSED", &existing, "background dev session already running");
        return Ok(());
    }

    ctx.config.ensure_cli_log_dir(&ctx.project_root)?;
    let exe = env::current_exe().map_err(|error| {
        CLIError::FileError(format!("failed to resolve the kalam executable: {error}"))
    })?;
    let project_root = canonicalize_path(&ctx.project_root)?;
    let log_path = if log_path.is_absolute() {
        log_path
    } else {
        project_root.join(&log_path)
    };

    let mut child = spawn_foreground_dev(ctx, &exe, &project_root, &log_path, force)?;
    let pid = child.id();
    let session = DevSession {
        pid,
        started_at: chrono::Utc::now().to_rfc3339(),
        project_root: project_root.clone(),
        log_path: log_path.clone(),
        exe: exe.clone(),
        url: None,
    };
    write_session(&session_path, &session)?;

    match wait_for_ready(&mut child, &log_path).await {
        Ok(url) => {
            let mut ready = session;
            ready.url = url;
            write_session(&session_path, &ready)?;
            emit_started(ctx, "KALAM_DEV_STARTED", &ready, "background dev session started");
            Ok(())
        },
        Err(error) => {
            request_terminate(pid);
            let _ = time::timeout(Duration::from_secs(2), async {
                while pid_is_running(pid) {
                    time::sleep(STOP_POLL).await;
                }
            })
            .await;
            if pid_is_running(pid) {
                kill_supervised_process_by_pid(pid, SupervisedKillScope::Tree);
            }
            let _ = child.try_wait();
            let _ = fs::remove_file(&session_path);
            output.status("background dev session failed to start");
            Err(error)
        },
    }
}

pub async fn show_background_status(ctx: &WorkflowContext) -> Result<()> {
    let output = ctx.output();
    let session_path = ctx.config.dev_session_path(&ctx.project_root);
    match live_session(&session_path)? {
        Some(session) => {
            let pid = session.pid.to_string();
            let url = session.url.clone().unwrap_or_else(|| "-".to_string());
            let log = display_log_path(ctx, &session.log_path);
            output.agent_event(
                "KALAM_DEV_STATUS",
                &[
                    ("state", "running"),
                    ("pid", &pid),
                    ("url", &url),
                    ("log", &log),
                ],
            );
            output
                .status(format!("background dev session running (pid {pid}) url={url} log={log}"));
        },
        None => {
            output.agent_event("KALAM_DEV_STATUS", &[("state", "stopped")]);
            output.status("background dev session stopped");
        },
    }
    Ok(())
}

pub async fn print_background_logs(
    ctx: &WorkflowContext,
    follow: bool,
    lines: usize,
) -> Result<()> {
    let log_path = log_path_for_read(ctx)?;
    if !log_path.is_file() {
        ctx.output().status("no background dev logs yet");
        return Ok(());
    }

    let contents = fs::read_to_string(&log_path).map_err(|error| {
        CLIError::FileError(format!("failed to read '{}': {error}", log_path.display()))
    })?;
    print!("{}", trailing_lines(&contents, lines));
    io::stdout().flush().ok();

    if !follow {
        return Ok(());
    }

    let mut file = File::open(&log_path).map_err(|error| {
        CLIError::FileError(format!("failed to follow '{}': {error}", log_path.display()))
    })?;
    file.seek(SeekFrom::End(0)).map_err(|error| {
        CLIError::FileError(format!("failed to follow '{}': {error}", log_path.display()))
    })?;

    let shutdown = wait_for_dev_shutdown_signal();
    tokio::pin!(shutdown);
    let mut buf = String::new();
    loop {
        tokio::select! {
            _ = &mut shutdown => break,
            _ = time::sleep(Duration::from_millis(200)) => {
                buf.clear();
                if file.read_to_string(&mut buf).is_ok() && !buf.is_empty() {
                    print!("{buf}");
                    let _ = io::stdout().flush();
                }
            }
        }
    }
    Ok(())
}

pub async fn stop_background_session(ctx: &WorkflowContext) -> Result<()> {
    let output = ctx.output();
    let session_path = ctx.config.dev_session_path(&ctx.project_root);
    let Some(session) = load_session(&session_path)? else {
        output.agent_event("KALAM_DEV_STOPPED", &[("state", "stopped")]);
        output.status("background dev session already stopped");
        return Ok(());
    };

    if !process_matches_session(session.pid, &session.exe) {
        let _ = fs::remove_file(&session_path);
        output.agent_event("KALAM_DEV_STOPPED", &[("state", "stopped")]);
        output.status("background dev session already stopped");
        return Ok(());
    }

    let pid = session.pid;
    request_terminate(pid);
    let deadline = Instant::now() + STOP_WAIT;
    while pid_is_running(pid) && Instant::now() < deadline {
        time::sleep(STOP_POLL).await;
    }
    if pid_is_running(pid) {
        kill_supervised_process_by_pid(pid, SupervisedKillScope::Tree);
        let force_deadline = Instant::now() + Duration::from_secs(2);
        while pid_is_running(pid) && Instant::now() < force_deadline {
            time::sleep(STOP_POLL).await;
        }
    }

    let _ = fs::remove_file(&session_path);
    let pid_text = pid.to_string();
    output.agent_event("KALAM_DEV_STOPPED", &[("pid", &pid_text)]);
    output.status(format!("stopped background dev session (pid {pid})"));
    Ok(())
}

pub(crate) async fn wait_for_dev_shutdown_signal() -> Result<()> {
    #[cfg(unix)]
    {
        let mut sigterm = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .map_err(|error| {
                CLIError::ConfigurationError(format!("sigterm handler failed: {error}"))
            })?;
        tokio::select! {
            result = tokio::signal::ctrl_c() => {
                result.map_err(|error| {
                    CLIError::ConfigurationError(format!("ctrl-c handler failed: {error}"))
                })
            }
            _ = sigterm.recv() => Ok(()),
        }
    }
    #[cfg(not(unix))]
    {
        tokio::signal::ctrl_c().await.map_err(|error| {
            CLIError::ConfigurationError(format!("ctrl-c handler failed: {error}"))
        })
    }
}

fn emit_started(ctx: &WorkflowContext, event: &str, session: &DevSession, human: &str) {
    let output = ctx.output();
    let pid = session.pid.to_string();
    let url = session.url.clone().unwrap_or_else(|| "-".to_string());
    let log = display_log_path(ctx, &session.log_path);
    output.agent_event(event, &[("pid", &pid), ("url", &url), ("log", &log)]);
    output.status(format!("{human} (pid {pid}) url={url} log={log}"));
}

fn display_log_path(ctx: &WorkflowContext, log_path: &Path) -> String {
    log_path
        .strip_prefix(&ctx.project_root)
        .unwrap_or(log_path)
        .display()
        .to_string()
}

fn spawn_foreground_dev(
    ctx: &WorkflowContext,
    exe: &Path,
    project_root: &Path,
    log_path: &Path,
    force: bool,
) -> Result<std::process::Child> {
    let mut log_file =
        OpenOptions::new().create(true).append(true).open(log_path).map_err(|error| {
            CLIError::FileError(format!("failed to open '{}': {error}", log_path.display()))
        })?;
    writeln!(&mut log_file, "--- kalam dev start ---").ok();
    let log_err = log_file.try_clone().map_err(|error| {
        CLIError::FileError(format!("failed to duplicate '{}': {error}", log_path.display()))
    })?;

    let mut command = Command::new(exe);
    command
        .arg("dev")
        .arg("--agent")
        .arg("--no-color")
        .arg("--no-spinner")
        .arg("--project-dir")
        .arg(project_root)
        .stdin(Stdio::null())
        .stdout(log_file)
        .stderr(log_err)
        .current_dir(project_root);
    if force {
        command.arg("--force");
    }
    if let Some(env_name) = ctx.env_override.as_deref() {
        command.arg("--env").arg(env_name);
    }
    if let Some(namespace) = ctx.namespace_override.as_deref() {
        command.arg("--namespace").arg(namespace);
    }

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

    command.spawn().map_err(|error| {
        CLIError::from(AgentError::dev_start_failed(&format!(
            "failed to spawn background `kalam dev`: {error}"
        )))
    })
}

async fn wait_for_ready(
    child: &mut std::process::Child,
    log_path: &Path,
) -> Result<Option<String>> {
    let deadline = Instant::now() + START_TIMEOUT;
    loop {
        if let Some(status) = child.try_wait().map_err(|error| {
            CLIError::from(AgentError::dev_start_failed(&format!(
                "failed to poll background `kalam dev`: {error}"
            )))
        })? {
            let log = read_log(log_path);
            let session_log = current_session_log(&log);
            if let Some(error) = agent_error_from_log(session_log) {
                return Err(CLIError::from(error));
            }
            return Err(CLIError::from(AgentError::dev_start_failed(&format!(
                "background `kalam dev` exited before becoming ready ({status}).\n{}",
                trailing_lines(&log, 20)
            ))));
        }

        let log = read_log(log_path);
        let session_log = current_session_log(&log);
        if let Some(error) = agent_error_from_log(session_log) {
            return Err(CLIError::from(error));
        }
        if let Some(line) = last_matching_line(session_log, "KALAM_READY") {
            return Ok(event_field(line, "url"));
        }
        if Instant::now() >= deadline {
            return Err(CLIError::from(AgentError::dev_start_failed(&format!(
                "timed out waiting for background `kalam dev` to become ready.\n{}",
                trailing_lines(&log, 20)
            ))));
        }
        time::sleep(START_POLL).await;
    }
}

fn live_session(path: &Path) -> Result<Option<DevSession>> {
    let Some(session) = load_session(path)? else {
        return Ok(None);
    };
    if process_matches_session(session.pid, &session.exe) {
        Ok(Some(session))
    } else {
        let _ = fs::remove_file(path);
        Ok(None)
    }
}

fn load_session(path: &Path) -> Result<Option<DevSession>> {
    let Some(contents) =
        crate::fs_atomic::read_to_string_if_exists(path, FileReadPolicy::UserProvided).map_err(
            |error| CLIError::FileError(format!("failed to read '{}': {error}", path.display())),
        )?
    else {
        return Ok(None);
    };
    match serde_json::from_str(&contents) {
        Ok(session) => Ok(Some(session)),
        Err(_) => {
            let _ = fs::remove_file(path);
            Ok(None)
        },
    }
}

fn write_session(path: &Path, session: &DevSession) -> Result<()> {
    let payload = serde_json::to_vec_pretty(session).map_err(|error| {
        CLIError::FileError(format!("failed to serialize dev session: {error}"))
    })?;
    write_atomic(path, &payload, FileWriteOptions::DEFAULT).map_err(|error| {
        CLIError::FileError(format!("failed to write '{}': {error}", path.display()))
    })
}

fn log_path_for_read(ctx: &WorkflowContext) -> Result<PathBuf> {
    let session_path = ctx.config.dev_session_path(&ctx.project_root);
    if let Some(session) = load_session(&session_path)? {
        return Ok(session.log_path);
    }
    Ok(ctx.config.workflow_log_path(&ctx.project_root))
}

const SESSION_START_MARKER: &str = "--- kalam dev start ---";

fn read_log(path: &Path) -> String {
    fs::read_to_string(path).unwrap_or_default()
}

fn current_session_log(log: &str) -> &str {
    match log.rfind(SESSION_START_MARKER) {
        Some(idx) => &log[idx..],
        None => log,
    }
}

fn trailing_lines(text: &str, lines: usize) -> String {
    if lines == 0 {
        return text.to_string();
    }
    let collected: Vec<&str> = text.lines().collect();
    let start = collected.len().saturating_sub(lines);
    let mut out = collected[start..].join("\n");
    if text.ends_with('\n') && !out.ends_with('\n') {
        out.push('\n');
    } else if !out.is_empty() && !out.ends_with('\n') {
        out.push('\n');
    }
    out
}

fn last_matching_line<'a>(text: &'a str, needle: &str) -> Option<&'a str> {
    text.lines().rev().find(|line| line.contains(needle))
}

fn event_field(line: &str, key: &str) -> Option<String> {
    let prefix = format!("{key}=");
    line.split_whitespace().find_map(|token| {
        token.strip_prefix(&prefix).map(|value| {
            let trimmed = value.trim_matches('"');
            trimmed.replace("\\\"", "\"")
        })
    })
}

fn agent_error_from_log(log: &str) -> Option<AgentError> {
    let idx = log.rfind("KALAM_ERROR code=")?;
    let block = log[idx..].trim();
    let first = block.lines().next()?;
    let code = first.strip_prefix("KALAM_ERROR code=")?.split_whitespace().next()?;
    AgentError::from_log_code(code, block)
}

fn canonicalize_path(path: &Path) -> Result<PathBuf> {
    if let Ok(canonical) = fs::canonicalize(path) {
        return Ok(canonical);
    }
    if path.is_absolute() {
        Ok(path.to_path_buf())
    } else {
        Ok(env::current_dir().unwrap_or_else(|_| PathBuf::from(".")).join(path))
    }
}

fn process_matches_session(pid: u32, exe: &Path) -> bool {
    if !pid_is_running(pid) {
        return false;
    }
    let Some(cmdline) = process_command_line(pid) else {
        return true;
    };
    let exe_str = exe.to_string_lossy();
    let exe_name = exe.file_name().and_then(|name| name.to_str()).unwrap_or("kalam");
    cmdline.contains(exe_str.as_ref()) || cmdline.contains(exe_name) || cmdline.contains("kalam")
}

fn pid_is_running(pid: u32) -> bool {
    #[cfg(unix)]
    {
        unsafe {
            if libc::kill(pid as i32, 0) == 0 {
                return true;
            }
            std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
        }
    }
    #[cfg(windows)]
    {
        process_command_line(pid).is_some()
    }
}

fn process_command_line(pid: u32) -> Option<String> {
    #[cfg(unix)]
    {
        let output = Command::new("ps")
            .args(["-p", &pid.to_string(), "-o", "args="])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .output()
            .ok()?;
        if !output.status.success() {
            return None;
        }
        let cmdline = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if cmdline.is_empty() {
            None
        } else {
            Some(cmdline)
        }
    }
    #[cfg(windows)]
    {
        let output = Command::new("tasklist")
            .args(["/FI", &format!("PID eq {pid}"), "/FO", "CSV", "/NH"])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .output()
            .ok()?;
        let text = String::from_utf8_lossy(&output.stdout);
        if text.to_ascii_lowercase().contains("no tasks") || text.trim().is_empty() {
            None
        } else {
            Some(text.trim().to_string())
        }
    }
}

fn request_terminate(pid: u32) {
    #[cfg(unix)]
    unsafe {
        let _ = libc::kill(pid as i32, libc::SIGTERM);
    }
    #[cfg(windows)]
    {
        kill_supervised_process_by_pid(pid, SupervisedKillScope::Tree);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trailing_lines_returns_last_n() {
        let text = "a\nb\nc\nd\n";
        assert_eq!(trailing_lines(text, 2), "c\nd\n");
        assert_eq!(trailing_lines(text, 0), text);
    }

    #[test]
    fn event_field_reads_url() {
        let line = "KALAM_READY url=http://127.0.0.1:2900 namespace=task_app schema=applied \
                    types=typescript";
        assert_eq!(event_field(line, "url").as_deref(), Some("http://127.0.0.1:2900"));
        assert_eq!(event_field(line, "namespace").as_deref(), Some("task_app"));
    }

    #[test]
    fn agent_error_from_log_prefers_structured_code() {
        let log = "starting\nKALAM_ERROR code=DESTRUCTIVE_SCHEMA_CHANGE object=\"table \
                   `messages`\"\n\nschema.sql requires rebuilding\n";
        let error = agent_error_from_log(log).expect("parse error");
        assert_eq!(error.code, crate::agent_error::AgentErrorCode::DestructiveSchemaChange);
    }

    #[test]
    fn agent_error_ignores_stale_error_before_session_marker() {
        let log = "KALAM_ERROR code=SCHEMA_FAILED object=\"old\"\n--- kalam dev start ---\nstarting\n\
                   KALAM_READY url=http://127.0.0.1:2900\n";
        assert!(agent_error_from_log(current_session_log(log)).is_none());
        assert!(last_matching_line(current_session_log(log), "KALAM_READY").is_some());
    }

    #[test]
    fn ready_ignores_stale_ready_before_session_marker() {
        let log = "KALAM_READY url=http://127.0.0.1:2800\n--- kalam dev start ---\nstarting\n";
        assert!(last_matching_line(current_session_log(log), "KALAM_READY").is_none());
    }

    #[test]
    fn session_json_roundtrip() {
        let session = DevSession {
            pid:          42,
            started_at:   "2026-08-28T00:00:00Z".into(),
            project_root: PathBuf::from("/tmp/app"),
            log_path:     PathBuf::from("/tmp/app/kalam/cli/logs/kalam.log"),
            exe:          PathBuf::from("/usr/bin/kalam"),
            url:          Some("http://127.0.0.1:2900".into()),
        };
        let encoded = serde_json::to_string(&session).unwrap();
        let decoded: DevSession = serde_json::from_str(&encoded).unwrap();
        assert_eq!(decoded, session);
    }

    #[test]
    fn current_pid_is_running() {
        assert!(pid_is_running(std::process::id()));
        assert!(!pid_is_running(2_000_000_000));
    }
}
