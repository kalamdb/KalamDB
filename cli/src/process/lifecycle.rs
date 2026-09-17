//! Process-group setup and teardown for supervised CLI child processes.

#[cfg(windows)]
use std::process::{Command, Stdio};
use std::time::Duration;

use tokio::process::Child;

/// How aggressively to terminate a supervised child.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SupervisedKillScope {
    /// Kill only the root child PID (directly spawned binaries such as kalamdb-server).
    Process,
    /// Kill the root PID and its descendants (shell-managed dev processes).
    Tree,
}

/// Configure a child command so its process tree can be stopped together.
pub fn configure_supervised_child(command: &mut tokio::process::Command) {
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;

        unsafe {
            command.as_std_mut().pre_exec(|| {
                libc::setpgid(0, 0);
                Ok(())
            });
        }
    }
}

/// Terminate a supervised child using the configured kill scope.
pub async fn kill_supervised_child(child: &mut Child, scope: SupervisedKillScope) {
    if let Some(pid) = child.id() {
        kill_supervised_process_by_pid(pid, scope);
    }
    let _ = child.start_kill();
    let _ = child.wait().await;
}

/// Terminate a process previously spawned by this CLI session.
pub fn kill_supervised_process_by_pid(pid: u32, scope: SupervisedKillScope) {
    match scope {
        SupervisedKillScope::Process => kill_single_process(pid),
        SupervisedKillScope::Tree => kill_process_tree_by_pid(pid),
    }
}

fn kill_single_process(pid: u32) {
    #[cfg(windows)]
    {
        let _ = Command::new("taskkill")
            .args(["/PID", &pid.to_string(), "/F"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }

    #[cfg(unix)]
    {
        signal_process(pid, libc::SIGTERM);
        std::thread::sleep(Duration::from_millis(100));
        signal_process(pid, libc::SIGKILL);
    }
}

pub fn kill_process_tree_by_pid(pid: u32) {
    #[cfg(windows)]
    {
        let _ = Command::new("taskkill")
            .args(["/PID", &pid.to_string(), "/T", "/F"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }

    #[cfg(unix)]
    {
        if let Some(pgid) = supervised_process_group_id(pid) {
            signal_process_group(pgid, libc::SIGTERM);
        } else {
            signal_process(pid, libc::SIGTERM);
        }
        std::thread::sleep(Duration::from_millis(100));
        if let Some(pgid) = supervised_process_group_id(pid) {
            signal_process_group(pgid, libc::SIGKILL);
        }
        signal_process(pid, libc::SIGKILL);
    }
}

#[cfg(unix)]
fn supervised_process_group_id(pid: u32) -> Option<i32> {
    unsafe {
        let pid_i = pid as i32;
        let pgid = libc::getpgid(pid_i);
        if pgid < 0 || pgid != pid_i {
            return None;
        }
        Some(pgid)
    }
}

#[cfg(unix)]
fn signal_process(pid: u32, signal: i32) {
    unsafe {
        let _ = libc::kill(pid as i32, signal);
    }
}

#[cfg(unix)]
fn signal_process_group(pgid: i32, signal: i32) {
    unsafe {
        let _ = libc::kill(-pgid, signal);
    }
}

/// Returns true when `pid` is still alive.
pub fn pid_is_running(pid: u32) -> bool {
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

/// Best-effort command line for `pid`, used to confirm process identity.
pub fn process_command_line(pid: u32) -> Option<String> {
    #[cfg(unix)]
    {
        use std::process::{Command, Stdio};

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
        use std::process::{Command, Stdio};

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

/// Returns true when `pid` is running and its command line looks like `exe`.
pub fn process_matches_executable(pid: u32, exe: &std::path::Path) -> bool {
    if !pid_is_running(pid) {
        return false;
    }
    let Some(cmdline) = process_command_line(pid) else {
        return true;
    };
    let exe_str = exe.to_string_lossy();
    let exe_name = exe.file_name().and_then(|name| name.to_str()).unwrap_or_default();
    cmdline.contains(exe_str.as_ref()) || (!exe_name.is_empty() && cmdline.contains(exe_name))
}

/// Send SIGTERM (Unix) or equivalent without waiting.
pub fn request_terminate(pid: u32) {
    #[cfg(unix)]
    unsafe {
        let _ = libc::kill(pid as i32, libc::SIGTERM);
    }
    #[cfg(windows)]
    {
        kill_single_process(pid);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn supervised_kill_scope_variants_are_distinct() {
        assert_ne!(SupervisedKillScope::Process, SupervisedKillScope::Tree);
    }

    #[cfg(unix)]
    #[test]
    fn supervised_process_group_id_requires_isolated_group_leader() {
        assert!(supervised_process_group_id(u32::MAX).is_none());
    }
}
