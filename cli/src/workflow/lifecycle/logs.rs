//! `kalam logs` — print or follow managed database logs.

use std::{
    fs::{self, File},
    io::{self, Read, Seek, SeekFrom, Write},
};

use tokio::time::{self, Duration};

use crate::{
    error::{CLIError, Result},
    workflow::{
        instance,
        target::{resolve_lifecycle_target, TargetSelector},
    },
};

#[derive(Debug, Clone, Copy)]
pub struct LogsOptions {
    pub follow: bool,
    pub lines:  usize,
}

pub async fn print_database_logs(selector: &TargetSelector, options: LogsOptions) -> Result<()> {
    let target = resolve_lifecycle_target(selector)?;
    if !target.is_managed_local() {
        return Err(CLIError::ConfigurationError(
            "remote log streaming is not supported yet; run `kalam logs` against a local or \
             `--global` database"
                .into(),
        ));
    }
    let layout = target.layout.ok_or_else(|| {
        CLIError::ConfigurationError("no managed database log file for this target".into())
    })?;
    let log_path = instance::load_instance(&layout)?
        .map(|record| record.log_path)
        .unwrap_or(layout.log_file);
    if !log_path.is_file() {
        println!("no database logs yet");
        return Ok(());
    }

    let contents = fs::read_to_string(&log_path).map_err(|error| {
        CLIError::FileError(format!("failed to read '{}': {error}", log_path.display()))
    })?;
    print!("{}", trailing_lines(&contents, options.lines));
    io::stdout().flush().ok();
    if !options.follow {
        return Ok(());
    }

    let mut file = File::open(&log_path).map_err(|error| {
        CLIError::FileError(format!("failed to follow '{}': {error}", log_path.display()))
    })?;
    file.seek(SeekFrom::End(0)).map_err(|error| {
        CLIError::FileError(format!("failed to follow '{}': {error}", log_path.display()))
    })?;
    let shutdown = crate::workflow::dev::session::wait_for_dev_shutdown_signal();
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

fn trailing_lines(contents: &str, lines: usize) -> String {
    if lines == 0 {
        return contents.to_string();
    }
    let collected: Vec<&str> = contents.lines().rev().take(lines).collect();
    let mut out = collected.into_iter().rev().collect::<Vec<_>>().join("\n");
    if contents.ends_with('\n') && !out.ends_with('\n') {
        out.push('\n');
    }
    out
}
