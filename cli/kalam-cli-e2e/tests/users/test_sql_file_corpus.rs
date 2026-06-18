use std::{fs, path::Path, time::Duration};

use crate::common::*;

fn sql_fixture_dir() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/sql-files")
}

fn sql_fixture_paths() -> Vec<std::path::PathBuf> {
    let mut files = fs::read_dir(sql_fixture_dir())
        .unwrap_or_else(|err| panic!("failed to read sql fixture directory: {}", err))
        .filter_map(|entry| entry.ok().map(|value| value.path()))
        .filter(|path| path.extension().and_then(|ext| ext.to_str()) == Some("sql"))
        .collect::<Vec<_>>();
    files.sort();
    files
}

fn run_cli_file(path: &Path) -> Result<String, Box<dyn std::error::Error>> {
    let mut cmd = create_cli_command_with_auth(admin_username(), admin_password());
    let file_arg = format!("--file={}", path.display());
    cmd.arg("--no-spinner")
        .arg("--no-color")
        .arg(file_arg)
        .timeout(Duration::from_secs(120));

    let output = cmd.output()?;
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();

    if output.status.success() {
        Ok(stdout)
    } else {
        Err(format!("CLI --file failed. stderr: {}\nstdout: {}", stderr, stdout).into())
    }
}

#[test]
fn test_sql_file_corpus_runs_twice_via_cli_command() {
    if !is_server_running() {
        eprintln!("⚠️  Server not running. Skipping test.");
        return;
    }

    let files = sql_fixture_paths();
    assert!(
        !files.is_empty(),
        "expected at least one SQL fixture in {}",
        sql_fixture_dir().display()
    );

    for pass in 1..=2 {
        for file_path in &files {
            run_cli_file(file_path).unwrap_or_else(|err| {
                panic!(
                    "fixture execution failed on pass {} for {}: {}",
                    pass,
                    file_path.display(),
                    err
                )
            });
        }
    }
}
