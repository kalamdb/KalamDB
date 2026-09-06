use std::{
    fs, io,
    path::{Path, PathBuf},
};

use crate::{release_download::copy_file_with_executable_bit, CLIError, Result};

#[cfg(windows)]
const REPLACE_RETRY_ATTEMPTS: u32 = 10;

/// Replace `current_exe` with `new_binary`.
///
/// On Unix this is an atomic rename over the existing file. On Windows a running
/// executable cannot be overwritten, but it *can* be renamed, so the current file
/// is moved aside first and the new binary takes its original path before return.
/// That is the rustup / `self-replace` pattern: the next process sees the new
/// binary immediately, without waiting for this process to exit or for a reboot.
pub fn replace_installed_binary(current_exe: &Path, new_binary: &Path) -> Result<()> {
    cleanup_stale_update_artifacts(current_exe);

    #[cfg(unix)]
    {
        replace_by_rename_over(current_exe, new_binary)
    }

    #[cfg(windows)]
    {
        replace_by_renaming_aside(current_exe, new_binary)
    }
}

#[cfg(unix)]
fn replace_by_rename_over(current_exe: &Path, new_binary: &Path) -> Result<()> {
    let staging_path = sibling_with_suffix(current_exe, ".kalam-new");
    copy_file_with_executable_bit(new_binary, &staging_path)?;

    if let Err(error) = fs::rename(&staging_path, current_exe) {
        let _ = fs::remove_file(&staging_path);
        return Err(replace_error(current_exe, error));
    }

    Ok(())
}

#[cfg(windows)]
fn replace_by_renaming_aside(current_exe: &Path, new_binary: &Path) -> Result<()> {
    let old_path = sibling_with_suffix(current_exe, ".kalam-old");
    let staging_path = sibling_with_suffix(current_exe, ".kalam-new");
    let _ = fs::remove_file(&old_path);
    let _ = fs::remove_file(&staging_path);

    retry_io(|| fs::rename(current_exe, &old_path)).map_err(|error| {
        CLIError::FileError(format!(
            "Failed to move aside {}: {}. Close other kalam processes and retry",
            current_exe.display(),
            error
        ))
    })?;

    if let Err(error) = copy_file_with_executable_bit(new_binary, &staging_path) {
        let _ = retry_io(|| fs::rename(&old_path, current_exe));
        let _ = fs::remove_file(&staging_path);
        return Err(error);
    }

    if let Err(error) = retry_io(|| fs::rename(&staging_path, current_exe)) {
        let _ = fs::remove_file(&staging_path);
        let _ = retry_io(|| fs::rename(&old_path, current_exe));
        return Err(replace_error(current_exe, error));
    }

    if fs::remove_file(&old_path).is_err() {
        schedule_delete_after_exit(&old_path);
    }

    Ok(())
}

fn cleanup_stale_update_artifacts(current_exe: &Path) {
    let Some(parent) = current_exe.parent() else {
        return;
    };
    let Some(file_name) = current_exe.file_name().and_then(|name| name.to_str()) else {
        return;
    };
    let prefix = format!("{file_name}.");
    let Ok(entries) = fs::read_dir(parent) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        if name.starts_with(&prefix)
            && (name.ends_with(".kalam-old") || name.ends_with(".kalam-new"))
        {
            let _ = fs::remove_file(entry.path());
        }
    }
}

fn sibling_with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path
        .file_name()
        .map(|name| name.to_os_string())
        .unwrap_or_else(|| std::ffi::OsString::from("kalam"));
    name.push(".");
    name.push(std::process::id().to_string());
    name.push(suffix);
    path.with_file_name(name)
}

#[cfg(windows)]
fn retry_io<T>(mut op: impl FnMut() -> io::Result<T>) -> io::Result<T> {
    use std::{thread, time::Duration};

    let mut last_error = None;
    for attempt in 0..REPLACE_RETRY_ATTEMPTS {
        match op() {
            Ok(value) => return Ok(value),
            Err(error) => {
                last_error = Some(error);
                thread::sleep(Duration::from_millis(50 * u64::from(attempt + 1)));
            },
        }
    }
    Err(last_error.expect("replace retry attempts is greater than zero"))
}

fn replace_error(current_exe: &Path, error: io::Error) -> CLIError {
    CLIError::FileError(format!("Failed to replace {}: {}", current_exe.display(), error))
}

#[cfg(windows)]
fn schedule_delete_after_exit(path: &Path) {
    use std::{
        os::windows::process::CommandExt,
        process::{Command, Stdio},
    };

    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    const DETACHED_PROCESS: u32 = 0x0000_0008;
    const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
    const CREATE_BREAKAWAY_FROM_JOB: u32 = 0x0100_0000;

    let command = format!("ping -n 3 127.0.0.1 >nul & del /F /Q {}", quote_cmd_path(path));
    let spawn_cleanup = |breakaway: bool| {
        let mut flags = CREATE_NO_WINDOW | DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP;
        if breakaway {
            flags |= CREATE_BREAKAWAY_FROM_JOB;
        }
        Command::new("cmd.exe")
            .args(["/C", &command])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .creation_flags(flags)
            .spawn()
            .map(|_| ())
    };

    if spawn_cleanup(true).is_err() {
        if let Err(error) = spawn_cleanup(false) {
            eprintln!(
                "Updated kalam, but left leftover {} (could not schedule cleanup: {})",
                path.display(),
                error
            );
        }
    }
}

#[cfg(windows)]
fn quote_cmd_path(path: &Path) -> String {
    format!("\"{}\"", path.to_string_lossy().replace('"', "\"\""))
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::TempDir;

    use super::{cleanup_stale_update_artifacts, replace_installed_binary, sibling_with_suffix};

    #[test]
    fn sibling_suffix_keeps_original_file_name() {
        let path = std::path::Path::new("dist").join("kalam.exe");
        let sibling = sibling_with_suffix(&path, ".kalam-old");
        let name = sibling.file_name().unwrap().to_string_lossy();
        assert!(name.starts_with("kalam.exe."));
        assert!(name.ends_with(".kalam-old"));
        assert!(!name.ends_with("kalam.kalam-old"));
    }

    #[test]
    fn replace_installed_binary_overwrites_destination_contents() {
        let temp = TempDir::new().expect("temp dir");
        let current = temp.path().join("kalam-current");
        let incoming = temp.path().join("kalam-new");
        fs::write(&current, b"old-binary").expect("write current");
        fs::write(&incoming, b"new-binary").expect("write incoming");

        replace_installed_binary(&current, &incoming).expect("replace");

        assert_eq!(fs::read(&current).expect("read replaced"), b"new-binary");
        assert!(fs::read_dir(temp.path()).expect("read dir").flatten().all(|entry| {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            !name.contains(".kalam-new")
        }));
    }

    #[test]
    fn cleanup_removes_stale_update_artifacts() {
        let temp = TempDir::new().expect("temp dir");
        let current = temp.path().join("kalam.exe");
        fs::write(&current, b"current").expect("write current");
        fs::write(temp.path().join("kalam.exe.1.kalam-old"), b"old").expect("write old");
        fs::write(temp.path().join("kalam.exe.2.kalam-new"), b"new").expect("write new");
        fs::write(temp.path().join("unrelated.exe.kalam-old"), b"keep").expect("write unrelated");

        cleanup_stale_update_artifacts(&current);

        assert!(current.is_file());
        assert!(!temp.path().join("kalam.exe.1.kalam-old").exists());
        assert!(!temp.path().join("kalam.exe.2.kalam-new").exists());
        assert!(temp.path().join("unrelated.exe.kalam-old").exists());
    }
}

#[cfg(all(test, windows))]
mod windows_tests {
    use std::path::Path;

    use super::quote_cmd_path;

    #[test]
    fn cmd_path_quote_wraps_and_escapes_quotes() {
        assert_eq!(
            quote_cmd_path(Path::new(r#"C:\Users\O"Brien\kalam.exe.old"#)),
            r#""C:\Users\O""Brien\kalam.exe.old""#
        );
    }
}
