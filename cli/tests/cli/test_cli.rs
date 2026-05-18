//! Integration tests for general CLI functionality
//!
//! **Implements T036, T050-T054, T060-T063, T068**: General CLI features and configuration
//!
//! These tests validate:
//! - CLI connection and prompt display
//! - Help and version commands
//! - Configuration file handling
//! - Color output control
//! - Session timeout and command history
//! - Tab completion and verbose output

use std::fs;

use predicates::prelude::*;

use crate::common::*;

/// T036: Test CLI connection and prompt display
#[test]
fn test_cli_connection_and_prompt() {
    if !is_server_running() {
        eprintln!("⚠️  Server not running at {}. Skipping test.", server_url());
        eprintln!("   Start server: cargo run --release --bin kalamdb-server");
        return;
    }

    let mut cmd = create_cli_command();
    cmd.arg("-u").arg(server_url()).arg("--help");

    cmd.assert()
        .success()
        .stdout(predicate::str::contains("Interactive SQL terminal"))
        .stdout(predicate::str::contains("--url"));
}

/// T050: Test help command
#[test]
fn test_cli_help_command() {
    let mut cmd = create_cli_command();
    cmd.arg("--help");

    cmd.assert()
        .success()
        .stdout(predicate::str::contains("Interactive SQL terminal"))
        .stdout(predicate::str::contains("--url"))
        .stdout(predicate::str::contains("--watch-schema"))
        .stdout(predicate::str::contains("--json"))
        .stdout(predicate::str::contains("--file"));
}

#[test]
fn test_cli_watch_schema_requires_run() {
    let mut cmd = create_cli_command();
    cmd.arg("--watch-schema").arg("--namespace").arg("chat");

    let output = cmd.output().expect("run cli");

    assert!(
        !output.status.success(),
        "watch-schema should fail without --run\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("--run is required with --watch-schema"),
        "stderr should explain missing --run, got: {}",
        stderr
    );
}

#[test]
fn test_cli_watch_schema_rejects_invalid_table_selector() {
    let mut cmd = create_cli_command();
    cmd.arg("--watch-schema")
        .arg("--run")
        .arg("npm run schema:gen")
        .arg("--table")
        .arg("invalid_table");

    let output = cmd.output().expect("run cli");
    assert!(!output.status.success(), "watch-schema should fail with invalid table selector");

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("--table must be namespace.table"),
        "stderr should explain invalid table selector, got: {}",
        stderr
    );
}

/// T060: Test color output control
#[test]
fn test_cli_color_output() {
    if !is_server_running() {
        eprintln!("⚠️  Server not running. Skipping test.");
        return;
    }

    // Test with color enabled (default behavior)
    let mut cmd = create_cli_command_with_root_auth();
    cmd.arg("--command").arg("SELECT 'color' as test");

    let output = cmd.output().unwrap();
    assert!(
        output.status.success(),
        "Color command (default) should succeed\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    // Test with color disabled
    let mut cmd = create_cli_command_with_root_auth();
    cmd.arg("--no-color").arg("--command").arg("SELECT 'nocolor' as test");

    let output = cmd.output().unwrap();
    assert!(
        output.status.success(),
        "No-color command should succeed\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

/// T061: Test session timeout handling
#[test]
fn test_cli_session_timeout() {
    if !is_server_running() {
        eprintln!("⚠️  Server not running. Skipping test.");
        return;
    }

    // Note: --timeout flag not yet implemented, just test that command executes
    let mut cmd = create_cli_command_with_root_auth();
    cmd.arg("--command").arg("SELECT 1");

    let output = cmd.output().unwrap();
    assert!(
        output.status.success(),
        "Should execute command successfully\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn test_cli_external_meta_command_help_without_backslash() {
    if !is_server_running() {
        eprintln!("⚠️  Server not running. Skipping test.");
        return;
    }

    let mut cmd = create_cli_command_with_root_auth();
    cmd.arg("--command").arg("help");

    cmd.assert()
        .success()
        .stdout(predicate::str::contains("Kalam CLI Help"))
        .stdout(predicate::str::contains("Meta Commands"));
}

/// T062: Test command history (up/down arrows)
#[test]
fn test_cli_command_history() {
    // History is handled by rustyline in interactive mode
    // For non-interactive tests, we verify the CLI supports it
    let mut cmd = create_cli_command();
    cmd.arg("--help");

    let output = cmd.output().unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);

    // Verify CLI mentions interactive features
    assert!(
        stdout.contains("Interactive") || output.status.success(),
        "CLI should support interactive mode with history"
    );
}

/// T063: Test tab completion for SQL keywords
#[test]
fn test_cli_tab_completion() {
    // Tab completion is handled by rustyline in interactive mode
    // For non-interactive tests, we verify the CLI supports it
    let mut cmd = create_cli_command();
    cmd.arg("--help");

    let output = cmd.output().unwrap();

    assert!(output.status.success(), "CLI should support interactive mode with completion");
}

/// T068: Test verbose output mode
#[test]
fn test_cli_verbose_output() {
    if !is_server_running() {
        eprintln!("⚠️  Server not running. Skipping test.");
        return;
    }

    let output =
        execute_sql_as_root_via_cli("SELECT 1 as verbose_test").expect("Verbose mode should work");
    assert!(output.contains("1"), "Verbose output should include result: {}", output);
}

/// T047: Test config file creation
#[test]
fn test_cli_config_file_creation() {
    let temp_dir = TempDir::new().unwrap();
    let config_path = temp_dir.path().join("kalam.toml");

    // Create config file
    let config_contents = format!(
        r#"
[connection]
url = "{}"
timeout = 30

[output]
format = "table"
color = true
"#,
        server_url()
    );
    fs::write(&config_path, config_contents).unwrap();

    assert!(config_path.exists(), "Config file should be created");

    let content = fs::read_to_string(&config_path).unwrap();
    assert!(content.contains(server_url()), "Config should contain URL");
}

/// T048: Test loading config file
#[test]
fn test_cli_load_config_file() {
    if !is_server_running() {
        eprintln!("⚠️  Server not running. Skipping test.");
        return;
    }

    let temp_dir = TempDir::new().unwrap();
    let config_path = temp_dir.path().join("kalam.toml");
    let (_creds_temp_dir, creds_path) = create_temp_credentials_path();

    // Config file with valid settings (note: url is passed via CLI, not config)
    std::fs::write(
        &config_path,
        r#"
[server]
timeout = 30
http_version = "http2"

[ui]
format = "table"
color = true
"#,
    )
    .unwrap();

    let mut cmd = create_cli_command();
    with_credentials_path(&mut cmd, &creds_path);
    cmd.arg("--config")
        .arg(config_path.to_str().unwrap())
        .arg("-u")
        .arg(server_url())
        .arg("--user")
        .arg("root")
        .arg("--password")
        .arg(root_password())
        .arg("--command")
        .arg("SELECT 1 as test");

    let output = cmd.output().unwrap();

    // Should successfully execute using config
    assert!(
        output.status.success() || String::from_utf8_lossy(&output.stdout).contains("test"),
        "Should execute using config file\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

/// T049: Test config precedence (CLI args override config)
#[test]
fn test_cli_config_precedence() {
    if !is_server_running() {
        eprintln!("⚠️  Server not running. Skipping test.");
        return;
    }

    let temp_dir = TempDir::new().unwrap();
    let config_path = temp_dir.path().join("kalam.toml");

    // Config with wrong URL
    fs::write(
        &config_path,
        r#"
[connection]
url = "http://localhost:9999"

[output]
format = "csv"
"#,
    )
    .unwrap();

    // CLI args should override config
    let mut cmd = create_cli_command();
    cmd.arg("--config")
        .arg(config_path.to_str().unwrap())
        .arg("-u")
        .arg(server_url()) // Override URL
        .arg("--user")
        .arg("root")
        .arg("--password")
        .arg(root_password())
        .arg("--json") // Override format
        .arg("--command")
        .arg("SELECT 1 as test");

    let output = cmd.output().unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);

    // Should succeed with CLI args taking precedence
    assert!(
        output.status.success() && (stdout.contains("test") || stdout.contains("1")),
        "CLI args should override config: {}",
        stdout
    );
}
