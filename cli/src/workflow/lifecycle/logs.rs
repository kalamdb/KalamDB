//! `kalam logs` — print or follow managed database logs.

use std::{
    fs::File,
    io::{self, Read, Seek, SeekFrom, Write},
};

use tokio::time::{self, Duration};

use crate::{
    error::{CLIError, Result},
    output::WorkflowOutput,
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

pub async fn print_database_logs(
    selector: &TargetSelector,
    options: LogsOptions,
    output: &WorkflowOutput,
) -> Result<()> {
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
    output.fields(&[("Log file", log_path.display().to_string())]);
    if options.follow {
        output.detail("Following logs; press Ctrl+C to stop");
    }
    if !log_path.is_file() {
        output.status("No server logs yet");
        return Ok(());
    }

    // Keep the same file cursor for the initial tail and subsequent reads.
    // Reopening and seeking to EOF here can skip concurrently appended lines.
    let mut file = File::open(&log_path).map_err(|error| {
        CLIError::FileError(format!("failed to open '{}': {error}", log_path.display()))
    })?;
    let mut contents = Vec::new();
    file.read_to_end(&mut contents)?;
    io::stdout()
        .write_all(trailing_lines(&String::from_utf8_lossy(&contents), options.lines).as_bytes())?;
    io::stdout().flush()?;
    if !options.follow {
        return Ok(());
    }

    let shutdown = crate::workflow::dev::session::wait_for_dev_shutdown_signal();
    tokio::pin!(shutdown);
    let mut buf = Vec::new();
    loop {
        tokio::select! {
            _ = &mut shutdown => break,
            _ = time::sleep(Duration::from_millis(200)) => {
                buf.clear();
                if file.metadata()?.len() < file.stream_position()? {
                    file.seek(SeekFrom::Start(0))?;
                }
                file.read_to_end(&mut buf)?;
                if !buf.is_empty() {
                    io::stdout().write_all(&buf)?;
                    io::stdout().flush()?;
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
