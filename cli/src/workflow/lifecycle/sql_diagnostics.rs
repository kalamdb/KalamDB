//! Authenticated, read-only diagnostics shared by local and cloud instances.

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::Duration,
};

use kalam_client::{
    credentials::CredentialStore, AuthProvider, KalamCellValue, KalamLinkClient, QueryParam,
    QueryResponse, TimestampFormatter,
};

use super::{sql_log_cursor::SqlLogCursor, InstanceSummary, LogsOptions};
use crate::{
    error::{CLIError, Result},
    formatter::OutputFormatter,
    output::WorkflowOutput,
    workflow::sql::query_failure_message,
    CLISession, FileCredentialStore, OutputFormat,
};

pub fn local_diagnostic_instance(
    selector: &crate::workflow::target::TargetSelector,
    output: &WorkflowOutput,
) -> Result<Option<InstanceSummary>> {
    let Ok(target) = crate::workflow::target::resolve_lifecycle_target(selector) else {
        return Ok(None);
    };
    let Some(layout) = target.layout else {
        return Ok(None);
    };
    let folder = std::fs::canonicalize(layout.working_dir).ok();
    let store = FileCredentialStore::new()?;
    Ok(super::instances::instance_catalog(output, &store)?.into_iter().find(|row| {
        row.kind == super::InstanceKind::Local
            && row.state == super::InstanceState::Running
            && row.folder.as_ref().and_then(|path| std::fs::canonicalize(path).ok()) == folder
    }))
}

const LOG_COLUMNS: &str = "timestamp, level, thread, target, line, message";
const LOG_ORDER: &str = "timestamp, level, thread, target, line, message";
const PAGE_SIZE: usize = 1000;

/// Only reuse credentials bound to this endpoint. Never borrow unrelated auth.
pub fn diagnostic_client(instance: &InstanceSummary) -> Result<Option<KalamLinkClient>> {
    let store = FileCredentialStore::new()?;
    let Some(url) = instance.url.as_deref() else {
        return Ok(None);
    };
    for name in instance.aliases.iter().chain(std::iter::once(&instance.name)) {
        let Some(creds) = store.get_credentials(name)? else {
            continue;
        };
        let saved = creds.server_url.as_deref().unwrap_or(&creds.instance);
        if super::instances::endpoint_key(saved).is_none()
            || super::instances::endpoint_key(saved) != super::instances::endpoint_key(url)
        {
            continue;
        }
        if creds.is_expired() && !creds.can_refresh() {
            return Err(CLIError::ConfigurationError(format!(
                "Authentication for '{name}' expired. Run `kalam login --instance {name}`."
            )));
        }
        let mut builder = KalamLinkClient::builder()
            .base_url(url)
            .auth(AuthProvider::jwt_token(creds.jwt_token))
            .timeout(Duration::from_secs(10));
        if let Some(refresh) =
            CLISession::build_auth_refresher(url, Some(name), Some(Arc::new(Mutex::new(store))))
        {
            builder = builder.auth_refresher(refresh);
        }
        return builder.build().map(Some).map_err(Into::into);
    }
    Ok(None)
}

async fn query(
    client: &KalamLinkClient,
    sql: &str,
    params: Option<Vec<QueryParam>>,
) -> Result<QueryResponse> {
    let response = client.execute_query(sql, None, params, None).await?;
    if !response.success() {
        return Err(CLIError::ConfigurationError(format!(
            "Server diagnostics query failed: {}. System diagnostics require an authorized \
             administrative account.",
            query_failure_message(&response, "query failed"),
        )));
    }
    Ok(response)
}

pub async fn sql_instance_status(
    instance: &InstanceSummary,
    client: &KalamLinkClient,
    output: &WorkflowOutput,
) -> Result<()> {
    let spinner = output.status_spinner("Querying server status");
    let response = query(
        client,
        "SELECT node_id, status, api_addr, is_self, is_leader, hostname, uptime_human, \
         memory_usage_mb, cpu_usage_percent FROM system.cluster ORDER BY is_self DESC, node_id \
         LIMIT 1000",
        None,
    )
    .await?;
    drop(spinner);
    let record = if instance.kind == super::InstanceKind::Local {
        let layout = if instance.global {
            Some(crate::workflow::instance::shared_layout())
        } else {
            instance.folder.as_deref().map(crate::workflow::instance::project_layout)
        };
        layout
            .as_ref()
            .map(crate::workflow::instance::load_instance)
            .transpose()?
            .flatten()
    } else {
        None
    };
    if output.json {
        output.emit_json(&serde_json::json!({"ok": true, "server": "ready", "database": "running", "instance": instance.name, "url": instance.url, "folder": instance.folder, "config_path": record.as_ref().map(|r| &r.config_path), "data_path": record.as_ref().map(|r| &r.data_dir), "source": "system.cluster", "result": response}));
    } else {
        output.fields(&[
            ("Instance", instance.name.clone()),
            ("URL", instance.url.clone().unwrap_or_default()),
            ("Source", "system.cluster".into()),
        ]);
        if let Some(folder) = &instance.folder {
            output.fields(&[("Folder", folder.display().to_string())]);
        }
        if let Some(record) = record {
            output.fields(&[
                ("Config", record.config_path.display().to_string()),
                ("Data", record.data_dir.display().to_string()),
            ]);
        }
        let formatter = OutputFormatter::new(
            OutputFormat::Table,
            output.use_color,
            TimestampFormatter::default(),
        );
        println!("{}", formatter.format_response(&response)?);
    }
    Ok(())
}

fn rows(response: &QueryResponse) -> Vec<HashMap<String, KalamCellValue>> {
    response
        .results
        .first()
        .map(|result| result.named_rows.clone().unwrap_or_else(|| result.rows_as_maps()))
        .unwrap_or_default()
}

fn row_key(row: &HashMap<String, KalamCellValue>) -> String {
    serde_json::to_string(
        &["timestamp", "level", "thread", "target", "line", "message"]
            .map(|column| row.get(column)),
    )
    .expect("log cells serialize")
}

fn timestamp(row: &HashMap<String, KalamCellValue>) -> &str {
    row.get("timestamp").and_then(|value| value.as_str()).unwrap_or("")
}

fn print_log(row: &HashMap<String, KalamCellValue>, output: &WorkflowOutput) {
    if output.json {
        output.emit_json(
            &serde_json::json!({"type": "log", "source": "system.server_logs", "entry": row}),
        );
    } else {
        let text = |name| row.get(name).and_then(|v| v.as_str()).unwrap_or("");
        println!(
            "{} {:<5} {}: {}",
            text("timestamp"),
            text("level"),
            text("target"),
            text("message")
        );
    }
}

pub async fn sql_instance_logs(
    instance: &InstanceSummary,
    client: &KalamLinkClient,
    options: LogsOptions,
    output: &WorkflowOutput,
) -> Result<()> {
    output.fields(&[
        ("Instance", instance.name.clone()),
        ("URL", instance.url.clone().unwrap_or_default()),
        ("Source", "system.server_logs (SQL)".into()),
    ]);
    // Include timestamp ties in the initial snapshot so following does not
    // replay rows omitted by the requested tail size.
    let filter = if options.lines == 0 {
        String::new()
    } else {
        format!(
            " WHERE timestamp >= (SELECT MIN(timestamp) FROM (SELECT timestamp FROM \
             system.server_logs ORDER BY timestamp DESC LIMIT {}) AS tail)",
            options.lines
        )
    };
    let initial = query(
        client,
        &format!(
            "SELECT {LOG_COLUMNS} FROM system.server_logs{filter} ORDER BY timestamp DESC, level \
             DESC, thread DESC, target DESC, line DESC, message DESC LIMIT 9223372036854775807"
        ),
        None,
    )
    .await?;
    let mut initial_rows = rows(&initial);
    initial_rows.reverse();
    let mut cursor = SqlLogCursor::default();
    let skip = if options.lines == 0 {
        0
    } else {
        initial_rows.len().saturating_sub(options.lines)
    };
    for (index, row) in initial_rows.iter().enumerate() {
        cursor.observe(timestamp(row), row_key(row));
        if index >= skip {
            print_log(row, output);
        }
    }
    if initial_rows.is_empty() {
        output.detail(
            "No SQL log entries. This view requires [logging] format = \"json\" on the server. \
             For local capture logs, use --local-file.",
        );
        if output.json {
            output.emit_json(&serde_json::json!({"type": "logs", "source": "system.server_logs", "entries": [], "hint": "SQL logs require logging.format = json"}));
        }
    }
    if !options.follow {
        return Ok(());
    }
    output.detail("Following SQL logs on the connected server; press Ctrl+C to stop");
    let shutdown = crate::workflow::dev::session::wait_for_dev_shutdown_signal();
    tokio::pin!(shutdown);
    loop {
        tokio::select! {
            _ = &mut shutdown => break,
            _ = tokio::time::sleep(Duration::from_secs(1)) => {},
        }
        let mut next = SqlLogCursor::default();
        let mut offset = 0usize;
        loop {
            let sql = format!(
                "SELECT {LOG_COLUMNS} FROM system.server_logs WHERE timestamp >= $1 ORDER BY \
                 {LOG_ORDER} LIMIT {PAGE_SIZE} OFFSET {offset}"
            );
            let response = tokio::select! {
                _ = &mut shutdown => return Ok(()),
                result = query(client, &sql, Some(vec![QueryParam::Text(cursor.timestamp.clone())])) => result?,
            };
            let page = rows(&response);
            for row in &page {
                let key = row_key(row);
                let occurrence = next.observe(timestamp(row), key.clone());
                if !cursor.contains(timestamp(row), &key, occurrence) {
                    print_log(row, output);
                }
            }
            if page.len() < PAGE_SIZE {
                break;
            }
            offset += page.len();
        }
        if !next.timestamp.is_empty() {
            cursor = next;
        }
    }
    Ok(())
}
