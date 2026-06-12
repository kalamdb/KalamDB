//! Remote schema watch command (legacy server polling).
//!
//! File-based schema watch during local development lives in
//! `workflow::dev::watch` and is orchestrated by `kalam dev`.

use std::{process::Stdio, time::Duration};

use chrono::{SecondsFormat, Utc};
use kalam_cli::{
    workflow::project::identifiers::{parse_namespace_id, parse_table_ref},
    CLIConfiguration, CLIError, CLISession, FileCredentialStore, Result,
};
use kalamdb_commons::{NamespaceId, TableId};
use tokio::time::sleep;

use crate::{args::Cli, connect::create_session};

#[derive(Debug, Clone, PartialEq, Eq)]
struct TableSelector {
    table_id: TableId,
}

#[derive(Debug, Clone)]
struct WatchSchemaConfig {
    namespaces: Vec<NamespaceId>,
    tables: Vec<TableSelector>,
    run_command: String,
    run_on_start: bool,
    interval: Duration,
}

pub async fn handle_watch_schema(
    cli: &Cli,
    credential_store: &mut FileCredentialStore,
) -> Result<bool> {
    if !cli.watch_schema {
        return Ok(false);
    }

    let config = WatchSchemaConfig::from_cli(cli)?;
    let runtime_config = CLIConfiguration::load(&cli.config)?;
    let config_path = kalam_cli::config::expand_config_path(&cli.config);
    let mut session = create_session(cli, credential_store, &runtime_config, config_path).await?;

    watch_schema_loop(&mut session, &config).await?;
    Ok(true)
}

impl WatchSchemaConfig {
    fn from_cli(cli: &Cli) -> Result<Self> {
        let run_command = cli.watch_run.clone().ok_or_else(|| {
            CLIError::ConfigurationError("--run is required with --watch-schema".into())
        })?;

        let tables = cli
            .watch_table
            .iter()
            .map(|value| parse_table_selector(value))
            .collect::<Result<Vec<_>>>()?;

        let namespaces = cli
            .watch_namespace
            .iter()
            .map(|value| parse_namespace_id(value))
            .collect::<Result<Vec<_>>>()?;

        Ok(Self {
            namespaces,
            tables,
            run_command,
            run_on_start: cli.watch_run_on_start,
            interval: cli.watch_interval,
        })
    }
}

async fn watch_schema_loop(session: &mut CLISession, config: &WatchSchemaConfig) -> Result<()> {
    let mut last_seen = utc_timestamp_now();

    println!(
        "Watching schema changes every {}s{}",
        config.interval.as_secs_f64(),
        describe_scope(config)
    );

    if config.run_on_start {
        println!("Running startup command: {}", config.run_command);
        run_shell_command(&config.run_command).await?;
    }

    loop {
        let poll_started_at = utc_timestamp_now();
        let sql = build_schema_change_query(&config.namespaces, &config.tables, &last_seen);
        let response = session.execute_query_response(&sql).await?;
        let changed_count = extract_changed_count(&response)?;

        if changed_count > 0 {
            let label = if changed_count == 1 {
                "change"
            } else {
                "changes"
            };
            println!("Detected {} schema {} since {}", changed_count, label, last_seen);
            println!("Running: {}", config.run_command);
            run_shell_command(&config.run_command).await?;
        }

        last_seen = poll_started_at;
        sleep(config.interval).await;
    }
}

fn describe_scope(config: &WatchSchemaConfig) -> String {
    let mut parts = Vec::new();

    if !config.namespaces.is_empty() {
        let namespaces = config
            .namespaces
            .iter()
            .map(NamespaceId::as_str)
            .collect::<Vec<_>>()
            .join(", ");
        parts.push(format!(" for namespaces {namespaces}"));
    }

    if !config.tables.is_empty() {
        let tables = config
            .tables
            .iter()
            .map(|table| table.table_id.to_string())
            .collect::<Vec<_>>()
            .join(", ");
        parts.push(format!(" for tables {tables}"));
    }

    if parts.is_empty() {
        " across user-visible namespaces".to_string()
    } else {
        parts.join("")
    }
}

async fn run_shell_command(command: &str) -> Result<()> {
    let command = command.to_string();
    let spawned_command = command.clone();
    let status = tokio::task::spawn_blocking(move || {
        let mut child = if cfg!(windows) {
            let mut cmd = std::process::Command::new("cmd");
            cmd.arg("/C").arg(&spawned_command);
            cmd
        } else {
            let mut cmd = std::process::Command::new("sh");
            cmd.arg("-c").arg(&spawned_command);
            cmd
        };

        child
            .stdin(Stdio::inherit())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .status()
            .map_err(|error| {
                CLIError::FileError(format!(
                    "Failed to start watch command '{}': {}",
                    spawned_command, error
                ))
            })
    })
    .await
    .map_err(|error| CLIError::FileError(format!("Watch command task failed: {error}")))??;

    if status.success() {
        return Ok(());
    }

    let code = status
        .code()
        .map(|value| value.to_string())
        .unwrap_or_else(|| "terminated by signal".into());
    Err(CLIError::ConfigurationError(format!(
        "Watch command failed ({code}): {command}"
    )))
}

fn build_schema_change_query(
    namespaces: &[NamespaceId],
    tables: &[TableSelector],
    last_seen: &str,
) -> String {
    let scope = build_scope_predicate(namespaces, tables);
    format!(
        "SELECT COUNT(*) AS changed_count FROM system.tables WHERE {scope} AND updated_at > '{}'",
        escape_sql_literal(last_seen)
    )
}

fn build_scope_predicate(namespaces: &[NamespaceId], tables: &[TableSelector]) -> String {
    let namespace_predicates = namespaces
        .iter()
        .map(|namespace| {
            format!(
                "namespace_id = '{}'",
                escape_sql_literal(namespace.as_str())
            )
        })
        .collect::<Vec<_>>();
    let table_predicates = tables
        .iter()
        .map(|table| {
            format!(
                "(namespace_id = '{}' AND table_name = '{}')",
                escape_sql_literal(table.table_id.namespace_id().as_str()),
                escape_sql_literal(table.table_id.table_name().as_str())
            )
        })
        .collect::<Vec<_>>();

    let mut predicates = Vec::new();
    if !namespace_predicates.is_empty() {
        if namespace_predicates.len() == 1 {
            predicates.push(namespace_predicates[0].clone());
        } else {
            predicates.push(format!("({})", namespace_predicates.join(" OR ")));
        }
    }

    if !table_predicates.is_empty() {
        if table_predicates.len() == 1 {
            predicates.push(table_predicates[0].clone());
        } else {
            predicates.push(format!("({})", table_predicates.join(" OR ")));
        }
    }

    if predicates.is_empty() {
        "namespace_id != 'system'".to_string()
    } else if predicates.len() == 1 {
        predicates.remove(0)
    } else {
        format!("({})", predicates.join(" OR "))
    }
}

fn parse_table_selector(value: &str) -> Result<TableSelector> {
    parse_table_ref(value)
        .map(|table_id| TableSelector { table_id })
        .map_err(|error| {
            CLIError::ConfigurationError(format!(
                "--table must be namespace.table, got '{value}': {error}"
            ))
        })
}

fn extract_changed_count(response: &kalam_client::QueryResponse) -> Result<i64> {
    if !response.success() {
        let message = response
            .error
            .as_ref()
            .map(|error| error.message.clone())
            .unwrap_or_else(|| "schema watch query failed".to_string());
        return Err(CLIError::ConfigurationError(message));
    }

    response.get_i64("changed_count").ok_or_else(|| {
        CLIError::ParseError(
            "Schema watch query did not return a numeric changed_count column".into(),
        )
    })
}

fn escape_sql_literal(value: &str) -> String {
    value.replace('\'', "''")
}

fn utc_timestamp_now() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true)
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    #[test]
    fn build_schema_change_query_filters_one_namespace() {
        let query = build_schema_change_query(
            &[NamespaceId::new("chat")],
            &[],
            "2026-05-05T11:05:27.741Z",
        );

        assert_eq!(
            query,
            "SELECT COUNT(*) AS changed_count FROM system.tables WHERE namespace_id = 'chat' AND updated_at > '2026-05-05T11:05:27.741Z'"
        );
    }

    #[test]
    fn build_schema_change_query_or_filters_multiple_namespaces() {
        let namespaces = vec![NamespaceId::new("chat"), NamespaceId::new("billing")];
        let query = build_schema_change_query(&namespaces, &[], "2026-05-05T11:05:27.741Z");

        assert_eq!(
            query,
            "SELECT COUNT(*) AS changed_count FROM system.tables WHERE (namespace_id = 'chat' OR namespace_id = 'billing') AND updated_at > '2026-05-05T11:05:27.741Z'"
        );
    }

    #[test]
    fn escape_sql_literal_doubles_single_quotes() {
        assert_eq!(escape_sql_literal("team's"), "team''s");
    }

    #[test]
    fn build_schema_change_query_includes_table_filters() {
        let query = build_schema_change_query(
            &[],
            &[TableSelector {
                table_id: kalamdb_commons::TableId::from_strings("chat", "messages"),
            }],
            "2026-05-05T11:05:27.741Z",
        );

        assert_eq!(
            query,
            "SELECT COUNT(*) AS changed_count FROM system.tables WHERE (namespace_id = 'chat' AND table_name = 'messages') AND updated_at > '2026-05-05T11:05:27.741Z'"
        );
    }

    #[test]
    fn build_schema_change_query_defaults_to_non_system_namespaces() {
        let query = build_schema_change_query(&[], &[], "2026-05-05T11:05:27.741Z");

        assert_eq!(
            query,
            "SELECT COUNT(*) AS changed_count FROM system.tables WHERE namespace_id != 'system' AND updated_at > '2026-05-05T11:05:27.741Z'"
        );
    }

    #[test]
    fn parse_table_selector_requires_namespace_table() {
        let error = parse_table_selector("chat").unwrap_err();

        assert!(error.to_string().contains("--table must be namespace.table"));
    }

    #[test]
    fn describe_scope_defaults_to_user_visible_namespaces() {
        let config = WatchSchemaConfig {
            namespaces: Vec::new(),
            tables: Vec::new(),
            run_command: "npm run schema:gen".into(),
            run_on_start: false,
            interval: Duration::from_secs(5),
        };

        assert_eq!(describe_scope(&config), " across user-visible namespaces");
    }
}
