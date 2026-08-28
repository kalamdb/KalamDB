#[cfg(windows)]
use std::process::{Command, Stdio};
use std::{fs, path::Path};

use crate::{release_download::copy_file_with_executable_bit, CLIError, Result};

/// Result of attempting to install a downloaded binary over the running executable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReplaceMode {
    /// The on-disk binary was replaced before returning.
    Completed,
    /// A helper process will replace the binary after the current process exits.
    ScheduledExit,
}

/// Replace `current_exe` with `new_binary`.
///
/// On Unix this is an atomic rename. On Windows the running executable is locked,
/// so a detached helper waits for this process to exit before replacing the file.
pub fn replace_installed_binary(
    current_exe: &Path,
    new_binary: &Path,
    #[cfg_attr(not(windows), allow(unused_variables))] temp_dir: &Path,
) -> Result<ReplaceMode> {
    #[cfg(unix)]
    {
        replace_immediately(current_exe, new_binary)?;
        Ok(ReplaceMode::Completed)
    }

    #[cfg(windows)]
    {
        schedule_deferred_replace(current_exe, new_binary, temp_dir)?;
        Ok(ReplaceMode::ScheduledExit)
    }
}

#[cfg(unix)]
fn replace_immediately(current_exe: &Path, new_binary: &Path) -> Result<()> {
    let temp_path = current_exe.with_extension("kalam-update-tmp");
    copy_file_with_executable_bit(new_binary, &temp_path)?;

    if let Err(error) = fs::rename(&temp_path, current_exe) {
        let _ = fs::remove_file(&temp_path);
        return Err(CLIError::FileError(format!(
            "Failed to replace {}: {}",
            current_exe.display(),
            error
        )));
    }

    Ok(())
}

#[cfg(windows)]
fn schedule_deferred_replace(current_exe: &Path, new_binary: &Path, temp_dir: &Path) -> Result<()> {
    use std::os::windows::process::CommandExt;

    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    const DETACHED_PROCESS: u32 = 0x0000_0008;

    let staging_path = temp_dir.join("kalam-update-staged.exe");
    copy_file_with_executable_bit(new_binary, &staging_path)?;

    let script_path = temp_dir.join("kalam-update-helper.ps1");
    let leftover_tmp = current_exe.with_extension("kalam-update-tmp");
    let script = build_powershell_helper_script(
        std::process::id(),
        &staging_path,
        current_exe,
        temp_dir,
        &script_path,
        &leftover_tmp,
    );
    fs::write(&script_path, script).map_err(|error| {
        CLIError::FileError(format!("Failed to write Windows update helper script: {}", error))
    })?;

    Command::new("powershell.exe")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-WindowStyle",
            "Hidden",
            "-ExecutionPolicy",
            "Bypass",
            "-File",
            &script_path.to_string_lossy(),
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .creation_flags(CREATE_NO_WINDOW | DETACHED_PROCESS)
        .spawn()
        .map_err(|error| {
            CLIError::FileError(format!("Failed to launch Windows update helper: {}", error))
        })?;

    Ok(())
}

#[cfg(windows)]
fn build_powershell_helper_script(
    parent_pid: u32,
    staging_path: &Path,
    target_path: &Path,
    temp_dir: &Path,
    script_path: &Path,
    leftover_tmp: &Path,
) -> String {
    format!(
        r#"$ErrorActionPreference = 'Stop'
Wait-Process -Id {parent_pid} -ErrorAction SilentlyContinue
Start-Sleep -Milliseconds 300
if (Test-Path -LiteralPath '{leftover_tmp}') {{
    Remove-Item -LiteralPath '{leftover_tmp}' -Force -ErrorAction SilentlyContinue
}}
$attempts = 12
for ($i = 0; $i -lt $attempts; $i++) {{
    try {{
        Move-Item -LiteralPath '{staging_path}' -Destination '{target_path}' -Force
        break
    }} catch {{
        if ($i -eq ($attempts - 1)) {{
            throw
        }}
        Start-Sleep -Milliseconds 500
    }}
}}
Remove-Item -LiteralPath '{temp_dir}' -Recurse -Force -ErrorAction SilentlyContinue
Remove-Item -LiteralPath '{script_path}' -Force -ErrorAction SilentlyContinue
"#,
        parent_pid = parent_pid,
        leftover_tmp = escape_powershell_single_quoted(leftover_tmp),
        staging_path = escape_powershell_single_quoted(staging_path),
        target_path = escape_powershell_single_quoted(target_path),
        temp_dir = escape_powershell_single_quoted(temp_dir),
        script_path = escape_powershell_single_quoted(script_path),
    )
}

#[cfg(windows)]
fn escape_powershell_single_quoted(value: &Path) -> String {
    value.display().to_string().replace('\'', "''")
}

#[cfg(test)]
mod tests {
    #[cfg(windows)]
    use std::path::Path;

    #[cfg(windows)]
    use super::{build_powershell_helper_script, escape_powershell_single_quoted};

    #[cfg(windows)]
    #[test]
    fn powershell_path_escape_doubles_single_quotes() {
        assert_eq!(
            escape_powershell_single_quoted(Path::new(r"C:\Users\O'Brien\kalam.exe")),
            r"C:\Users\O'Brien\kalam.exe".replace('\'', "''")
        );
    }

    #[cfg(windows)]
    #[test]
    fn helper_script_waits_for_parent_and_moves_staged_binary() {
        let script = build_powershell_helper_script(
            4242,
            Path::new(r"C:\Temp\kalam-update\kalam-update-staged.exe"),
            Path::new(r"C:\npm\dist\kalam.exe"),
            Path::new(r"C:\Temp\kalam-update"),
            Path::new(r"C:\Temp\kalam-update\kalam-update-helper.ps1"),
            Path::new(r"C:\npm\dist\kalam.kalam-update-tmp"),
        );

        assert!(script.contains("Wait-Process -Id 4242"));
        assert!(script.contains("kalam-update-staged.exe"));
        assert!(script.contains("Move-Item -LiteralPath"));
    }
}
