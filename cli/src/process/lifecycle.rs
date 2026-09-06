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

/// Backward-compatible helper that always tears down the full supervised tree.
pub async fn kill_child_process_tree(child: &mut Child) {
    kill_supervised_child(child, SupervisedKillScope::Tree).await;
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
