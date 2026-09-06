use std::{env, fmt::Write as _, path::PathBuf, time::Duration};

use colored::Colorize;
use kalam_cli::{
    config::expand_config_path, CLIConfiguration, CLIError, FileCredentialStore, Result,
};
use kalam_client::{credentials::CredentialStore, KalamLinkClient};
use serde::Serialize;
use serde_json::json;

use crate::{
    args::Cli,
    commands::auth::{fetch_current_user, resolve_auth_context},
    connect::resolve_server_url,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum CheckStatus {
    Ok,
    Warn,
    Fail,
}

#[derive(Debug, Serialize)]
struct DoctorCheck {
    name:   &'static str,
    status: CheckStatus,
    detail: String,
}

impl DoctorCheck {
    fn ok(name: &'static str, detail: impl Into<String>) -> Self {
        Self {
            name,
            status: CheckStatus::Ok,
            detail: detail.into(),
        }
    }

    fn warn(name: &'static str, detail: impl Into<String>) -> Self {
        Self {
            name,
            status: CheckStatus::Warn,
            detail: detail.into(),
        }
    }

    fn fail(name: &'static str, detail: impl Into<String>) -> Self {
        Self {
            name,
            status: CheckStatus::Fail,
            detail: detail.into(),
        }
    }
}

pub async fn handle_doctor(
    cli: &Cli,
    credential_store_result: std::result::Result<FileCredentialStore, String>,
    strict: bool,
) -> Result<bool> {
    let mut checks = Vec::new();
    checks.push(DoctorCheck::ok("version", crate::args::version_report()));
    checks.push(binary_check());
    checks.push(path_check());

    let config_path = expand_config_path(&cli.config);
    let config_result = CLIConfiguration::load_existing(&cli.config);
    let (config, config_loaded) = match config_result {
        Ok(Some(config)) => {
            checks.push(DoctorCheck::ok(
                "config",
                format!("{} parsed successfully", config_path.display()),
            ));
            (config, true)
        },
        Ok(None) => {
            checks.push(DoctorCheck::warn(
                "config",
                format!("{} not found; defaults will be used", config_path.display()),
            ));
            (CLIConfiguration::default(), false)
        },
        Err(error) => {
            checks.push(DoctorCheck::fail("config", error.to_string()));
            (CLIConfiguration::default(), false)
        },
    };

    let mut credential_store = match credential_store_result {
        Ok(store) => {
            checks.extend(credentials_checks(cli, &store));
            Some(store)
        },
        Err(error) => {
            checks.push(DoctorCheck::fail(
                "credentials",
                format!("{}: {}", FileCredentialStore::default_path().display(), error),
            ));
            None
        },
    };

    let server_url = if let Some(store) = credential_store.as_ref() {
        match resolve_server_url(cli, store) {
            Ok(server_url) => {
                checks.push(DoctorCheck::ok("server_url", server_url.clone()));
                Some(server_url)
            },
            Err(error) => {
                checks.push(DoctorCheck::fail("server_url", error.to_string()));
                None
            },
        }
    } else {
        let server_url = fallback_server_url(cli);
        checks.push(DoctorCheck::warn(
            "server_url",
            format!("{} (credential store unavailable)", server_url),
        ));
        Some(server_url)
    };

    if let Some(server_url) = &server_url {
        checks.push(health_check(cli, server_url).await);
    }

    if let Some(store) = credential_store.as_mut() {
        let auth_check = auth_check(cli, store, &config).await;
        checks.push(auth_check);
    }

    let has_failures = checks.iter().any(|check| check.status == CheckStatus::Fail);
    let has_warnings = checks.iter().any(|check| check.status == CheckStatus::Warn);

    if cli.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "ok": !has_failures,
                "strict_ok": !has_failures && !has_warnings,
                "instance": cli.instance,
                "config_loaded": config_loaded,
                "checks": checks,
            }))
            .map_err(|error| CLIError::FormatError(error.to_string()))?
        );
    } else {
        print!("{}", render_doctor_report(&cli.instance, &checks, cli.verbose, !cli.no_color));
    }

    if strict && (has_failures || has_warnings) {
        return Err(CLIError::ConfigurationError(
            "doctor --strict found warnings or failures".to_string(),
        ));
    }

    Ok(true)
}

fn binary_check() -> DoctorCheck {
    match env::current_exe() {
        Ok(path) => DoctorCheck::ok("binary", path.display().to_string()),
        Err(error) => {
            DoctorCheck::fail("binary", format!("failed to locate current binary: {}", error))
        },
    }
}

fn path_check() -> DoctorCheck {
    let Ok(current_exe) = env::current_exe() else {
        return DoctorCheck::warn("path", "unable to compare PATH entry with current binary");
    };
    let Some(found) = find_binary_on_path() else {
        return DoctorCheck::warn("path", "kalam was not found on PATH");
    };

    if same_path(&found, &current_exe) {
        DoctorCheck::ok("path", format!("PATH resolves to {}", found.display()))
    } else {
        DoctorCheck::warn(
            "path",
            format!(
                "PATH resolves to {}, current process is {}",
                found.display(),
                current_exe.display()
            ),
        )
    }
}

fn credentials_checks(cli: &Cli, credential_store: &FileCredentialStore) -> Vec<DoctorCheck> {
    let mut checks = Vec::new();
    let path = credential_store.path().display().to_string();
    let instances = credential_store.list_instances().unwrap_or_default();

    if instances.is_empty() {
        checks.push(DoctorCheck::warn(
            "credentials",
            format!("{} contains no saved instances", path),
        ));
        return checks;
    }

    checks.push(DoctorCheck::ok(
        "credentials",
        format!("{} contains {} saved instance(s)", path, instances.len()),
    ));

    match credential_store.get_credentials(&cli.instance) {
        Ok(Some(creds)) => {
            if creds.is_expired() {
                checks.push(DoctorCheck::fail(
                    "instance_credentials",
                    format!("credentials for '{}' are expired", cli.instance),
                ));
            } else {
                let user = creds
                    .user
                    .as_ref()
                    .map(ToString::to_string)
                    .unwrap_or_else(|| "unknown user".to_string());
                checks.push(DoctorCheck::ok(
                    "instance_credentials",
                    format!("credentials for '{}' are available for {}", cli.instance, user),
                ));
            }
        },
        Ok(None) => checks.push(DoctorCheck::warn(
            "instance_credentials",
            format!("no credentials saved for '{}'", cli.instance),
        )),
        Err(error) => checks.push(DoctorCheck::fail(
            "instance_credentials",
            format!("failed to inspect credentials: {}", error),
        )),
    }

    checks
}

async fn health_check(cli: &Cli, server_url: &str) -> DoctorCheck {
    let client = match KalamLinkClient::builder()
        .base_url(server_url)
        .timeout(Duration::from_secs(cli.timeout))
        .build()
    {
        Ok(client) => client,
        Err(error) => return DoctorCheck::fail("health", error.to_string()),
    };

    match client.health_check().await {
        Ok(health) => DoctorCheck::ok(
            "health",
            format!("{} (server {}, api {})", health.status, health.version, health.api_version),
        ),
        Err(error) => DoctorCheck::fail("health", error.to_string()),
    }
}

async fn auth_check(
    cli: &Cli,
    credential_store: &mut FileCredentialStore,
    config: &CLIConfiguration,
) -> DoctorCheck {
    let auth_context = match resolve_auth_context(cli, credential_store, config, true).await {
        Ok(Some(auth_context)) => auth_context,
        Ok(None) => {
            return DoctorCheck::warn("auth", "no credentials available; run `kalam login`")
        },
        Err(error) => return DoctorCheck::fail("auth", error.to_string()),
    };

    match fetch_current_user(cli, &auth_context).await {
        Ok(user) => DoctorCheck::ok(
            "auth",
            format!(
                "authenticated as {} ({}) via {:?}",
                user.user.id, user.user.role, auth_context.source
            ),
        ),
        Err(error) => DoctorCheck::fail("auth", error.to_string()),
    }
}

fn fallback_server_url(cli: &Cli) -> String {
    if let Some(url) = &cli.url {
        return url.clone();
    }
    if let Some(host) = &cli.host {
        return format!("http://{}:{}", host, cli.port);
    }
    "http://localhost:2900".to_string()
}

fn render_doctor_report(
    instance: &str,
    checks: &[DoctorCheck],
    verbose: bool,
    use_color: bool,
) -> String {
    let mut output = String::new();
    let header = if verbose {
        "Doctor summary:"
    } else {
        "Doctor summary (to see all details, run kalam doctor -v):"
    };
    let _ = writeln!(output, "{}", style_header(header, use_color));
    let _ = writeln!(output, "{}", style_dim(&format!("Instance: {}", instance), use_color));

    for check in checks {
        let title = doctor_check_title(check.name);
        let summary = if verbose || check.status != CheckStatus::Ok {
            String::new()
        } else {
            first_detail_line(&check.detail)
                .map(|detail| format!(" ({})", style_dim(detail, use_color)))
                .unwrap_or_default()
        };
        let _ = writeln!(
            output,
            "{} {}{}",
            status_marker(check.status, use_color),
            style_title(title, check.status, use_color),
            summary
        );

        if verbose || check.status != CheckStatus::Ok {
            append_detail_lines(&mut output, check.status, &check.detail, use_color);
        }
    }

    let issue_count = checks.iter().filter(|check| check.status != CheckStatus::Ok).count();
    if issue_count == 0 {
        let _ = writeln!(
            output,
            "\n{} Doctor found no issues.",
            status_symbol(CheckStatus::Ok, use_color)
        );
    } else {
        let worst_status = if checks.iter().any(|check| check.status == CheckStatus::Fail) {
            CheckStatus::Fail
        } else {
            CheckStatus::Warn
        };
        let category = if issue_count == 1 {
            "category"
        } else {
            "categories"
        };
        let _ = writeln!(
            output,
            "\n{} Doctor found issues in {} {}.",
            status_symbol(worst_status, use_color),
            issue_count,
            category
        );
    }

    output
}

fn append_detail_lines(output: &mut String, status: CheckStatus, detail: &str, use_color: bool) {
    for (index, line) in detail.lines().enumerate() {
        if index == 0 {
            let _ =
                writeln!(output, "    {} {}", status_symbol(status, use_color), line.trim_end());
        } else {
            let _ = writeln!(output, "      {}", line.trim_end());
        }
    }
}

fn doctor_check_title(name: &'static str) -> &'static str {
    match name {
        "version" => "Kalam CLI",
        "binary" => "Binary",
        "path" => "PATH",
        "config" => "Configuration",
        "credentials" => "Credentials",
        "instance_credentials" => "Instance credentials",
        "server_url" => "Server URL",
        "health" => "KalamDB server",
        "auth" => "Authentication",
        other => other,
    }
}

fn first_detail_line(detail: &str) -> Option<&str> {
    detail.lines().find_map(|line| {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed)
        }
    })
}

fn status_marker(status: CheckStatus, use_color: bool) -> String {
    format!("[{}]", status_symbol(status, use_color))
}

fn status_symbol(status: CheckStatus, use_color: bool) -> String {
    let symbol = match status {
        CheckStatus::Ok => "✓",
        CheckStatus::Warn => "!",
        CheckStatus::Fail => "✗",
    };

    if !use_color {
        return symbol.to_string();
    }

    match status {
        CheckStatus::Ok => symbol.green().bold().to_string(),
        CheckStatus::Warn => symbol.yellow().bold().to_string(),
        CheckStatus::Fail => symbol.red().bold().to_string(),
    }
}

fn style_header(value: &str, use_color: bool) -> String {
    if use_color {
        value.bold().to_string()
    } else {
        value.to_string()
    }
}

fn style_title(value: &str, status: CheckStatus, use_color: bool) -> String {
    if !use_color {
        return value.to_string();
    }

    match status {
        CheckStatus::Ok => value.green().bold().to_string(),
        CheckStatus::Warn => value.yellow().bold().to_string(),
        CheckStatus::Fail => value.red().bold().to_string(),
    }
}

fn style_dim(value: &str, use_color: bool) -> String {
    if use_color {
        value.dimmed().to_string()
    } else {
        value.to_string()
    }
}

fn find_binary_on_path() -> Option<PathBuf> {
    let path_var = env::var_os("PATH")?;
    let binary_names: &[&str] = if cfg!(windows) {
        &["kalam.exe", "kalam"]
    } else {
        &["kalam"]
    };

    for dir in env::split_paths(&path_var) {
        for binary_name in binary_names {
            let candidate = dir.join(binary_name);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }

    None
}

fn same_path(left: &PathBuf, right: &PathBuf) -> bool {
    match (left.canonicalize(), right.canonicalize()) {
        (Ok(left), Ok(right)) => left == right,
        _ => left == right,
    }
}

#[cfg(test)]
mod tests {
    use clap::Parser;
    use tempfile::TempDir;

    use super::*;

    fn sample_checks() -> Vec<DoctorCheck> {
        vec![
            DoctorCheck::ok("version", "0.5.1-beta.2\nCommit: 9e1d2d6c\nBuilt: 2026-05-24 11:32:35 UTC"),
            DoctorCheck::ok("config", "/Users/jamal/.kalam/config.toml parsed successfully"),
            DoctorCheck::warn(
                "path",
                "PATH resolves to /Users/jamal/.kalam/bin/kalam, current process is /Users/jamal/git/KalamDB/target/release/kalam",
            ),
            DoctorCheck::fail(
                "health",
                "Network error: Connection failed: error sending request for url (http://localhost:2900/v1/api/healthcheck)",
            ),
        ]
    }

    #[test]
    fn doctor_report_compact_mode_matches_flutter_style() {
        let report = render_doctor_report("local", &sample_checks(), false, false);

        assert!(report.contains("Doctor summary (to see all details, run kalam doctor -v):"));
        assert!(report.contains("Instance: local"));
        assert!(report.contains("[✓] Kalam CLI"));
        assert!(report.contains("[✓] Configuration"));
        assert!(report.contains("[!] PATH"));
        assert!(report.contains("[✗] KalamDB server"));
        assert!(report.contains("Doctor found issues in 2 categories."));
        assert!(!report.contains("Commit: 9e1d2d6c"));
    }

    #[test]
    fn doctor_report_verbose_mode_includes_detail_lines() {
        let report = render_doctor_report("local", &sample_checks(), true, false);

        assert!(report.contains("Doctor summary:"));
        assert!(report.contains("    ✓ 0.5.1-beta.2"));
        assert!(report.contains("      Commit: 9e1d2d6c"));
        assert!(report.contains("    ! PATH resolves to /Users/jamal/.kalam/bin/kalam"));
        assert!(report.contains("    ✗ Network error: Connection failed"));
    }

    #[tokio::test]
    async fn doctor_command_reports_without_creating_config() {
        let temp_dir = TempDir::new().expect("temp dir");
        let config_path = temp_dir.path().join("missing-config.toml");
        let credentials_path = temp_dir.path().join("credentials.toml");
        let store = FileCredentialStore::with_path(credentials_path).expect("credential store");
        let cli = Cli::try_parse_from([
            "kalam",
            "doctor",
            "--url",
            "http://127.0.0.1:9",
            "--timeout",
            "1",
            "--no-spinner",
            "--config",
            config_path.to_str().expect("config path should be utf8"),
        ])
        .expect("doctor args parse");

        handle_doctor(&cli, Ok(store), false)
            .await
            .expect("doctor should report failures without failing unless strict");

        assert!(!config_path.exists(), "doctor should not create a config file");
    }

    #[tokio::test]
    async fn doctor_strict_fails_on_warnings_or_failures() {
        let temp_dir = TempDir::new().expect("temp dir");
        let config_path = temp_dir.path().join("missing-config.toml");
        let credentials_path = temp_dir.path().join("credentials.toml");
        let store = FileCredentialStore::with_path(credentials_path).expect("credential store");
        let cli = Cli::try_parse_from([
            "kalam",
            "doctor",
            "--strict",
            "--url",
            "http://127.0.0.1:9",
            "--timeout",
            "1",
            "--config",
            config_path.to_str().expect("config path should be utf8"),
        ])
        .expect("doctor args parse");

        assert!(handle_doctor(&cli, Ok(store), true).await.is_err());
    }
}
