use kalam_client::{KalamCellValue, KalamLinkClient};
use serde::Deserialize;

use crate::{
    error::{CLIError, Result},
    workflow::migration::{MigrationRecord, MigrationState, MigrationStatus},
};

#[derive(Debug, Deserialize)]
struct ServerMigrationRecord {
    migration_id: String,
    namespace: String,
    name: String,
    checksum: String,
    status: MigrationStatus,
    started_at: Option<i64>,
    finished_at: Option<i64>,
    error_message: Option<String>,
    source: Option<String>,
    kalam_version: Option<String>,
}

pub(crate) async fn load_server_migration_state(
    client: &KalamLinkClient,
    namespace: &kalamdb_commons::NamespaceId,
) -> Result<MigrationState> {
    let sql = format!(
        "SELECT migration_id, namespace, name, checksum, status, started_at, finished_at, error_message, source, kalam_version FROM system.migrations WHERE namespace = {}",
        sql_string(namespace.as_str())
    );
    let response = client.execute_query(&sql, None, None, None).await.map_err(CLIError::from)?;
    if !response.success() {
        return Err(CLIError::ConfigurationError(format!(
            "failed to load server migration state: {}",
            crate::workflow::sql::query_failure_message(&response, "query failed")
        )));
    }
    let records = response
        .rows_as_maps()
        .into_iter()
        .map(server_migration_record_from_row)
        .collect::<Result<Vec<_>>>()?;
    Ok(MigrationState {
        applied: records
            .iter()
            .filter(|record| record.status == MigrationStatus::Applied)
            .map(|record| record.migration_id.clone())
            .collect(),
        records: records.into_iter().map(MigrationRecord::from).collect(),
    })
}

pub(crate) async fn save_server_migration_record(
    client: &KalamLinkClient,
    record: &MigrationRecord,
    record_exists: bool,
) -> Result<()> {
    let migration_key = server_migration_key(record);
    let sql = if record_exists {
        format!(
            "UPDATE system.migrations SET namespace = {}, name = {}, checksum = {}, status = {}, started_at = {}, finished_at = {}, error_message = {}, source = {}, kalam_version = {} WHERE migration_key = {}",
            sql_string(record.namespace.as_str()),
            sql_string(&record.name),
            sql_string(&record.checksum),
            sql_string(&record.status.to_string()),
            sql_timestamp(record.started_at.as_deref())?,
            sql_timestamp(record.finished_at.as_deref())?,
            sql_nullable_string(record.error_message.as_deref()),
            sql_nullable_string(record.source.as_deref()),
            sql_nullable_string(record.kalam_version.as_deref()),
            sql_string(&migration_key),
        )
    } else {
        format!(
            "INSERT INTO system.migrations (migration_key, migration_id, namespace, name, checksum, status, started_at, finished_at, error_message, source, kalam_version) VALUES ({}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {})",
            sql_string(&migration_key),
            sql_string(&record.migration_id),
            sql_string(record.namespace.as_str()),
            sql_string(&record.name),
            sql_string(&record.checksum),
            sql_string(&record.status.to_string()),
            sql_timestamp(record.started_at.as_deref())?,
            sql_timestamp(record.finished_at.as_deref())?,
            sql_nullable_string(record.error_message.as_deref()),
            sql_nullable_string(record.source.as_deref()),
            sql_nullable_string(record.kalam_version.as_deref()),
        )
    };
    let response = client.execute_query(&sql, None, None, None).await.map_err(CLIError::from)?;
    if response.success() {
        return Ok(());
    }
    Err(CLIError::ConfigurationError(format!(
        "failed to save server migration state for {}: {}",
        record.migration_id,
        crate::workflow::sql::query_failure_message(&response, "query failed")
    )))
}

fn server_migration_key(record: &MigrationRecord) -> String {
    format!("{}:{}", record.namespace.as_str(), record.migration_id)
}

fn sql_timestamp(value: Option<&str>) -> Result<String> {
    Ok(match parse_rfc3339_millis(value)? {
        Some(value) => value.to_string(),
        None => "NULL".to_string(),
    })
}

fn sql_nullable_string(value: Option<&str>) -> String {
    value.map(sql_string).unwrap_or_else(|| "NULL".to_string())
}

fn sql_string(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

fn parse_rfc3339_millis(value: Option<&str>) -> Result<Option<i64>> {
    value
        .map(|value| {
            chrono::DateTime::parse_from_rfc3339(value)
                .map(|dt| dt.timestamp_millis())
                .map_err(|error| {
                    CLIError::ConfigurationError(format!(
                        "invalid migration timestamp '{value}': {error}"
                    ))
                })
        })
        .transpose()
}

fn server_migration_record_from_row(
    row: std::collections::HashMap<String, KalamCellValue>,
) -> Result<ServerMigrationRecord> {
    Ok(ServerMigrationRecord {
        migration_id: required_string(&row, "migration_id")?,
        namespace: required_string(&row, "namespace")?,
        name: required_string(&row, "name")?,
        checksum: required_string(&row, "checksum")?,
        status: parse_status(&required_string(&row, "status")?)?,
        started_at: optional_i64(&row, "started_at"),
        finished_at: optional_i64(&row, "finished_at"),
        error_message: optional_string(&row, "error_message"),
        source: optional_string(&row, "source"),
        kalam_version: optional_string(&row, "kalam_version"),
    })
}

fn parse_status(value: &str) -> Result<MigrationStatus> {
    match value {
        "draft" => Ok(MigrationStatus::Draft),
        "applying" => Ok(MigrationStatus::Applying),
        "applied" => Ok(MigrationStatus::Applied),
        "failed" => Ok(MigrationStatus::Failed),
        other => Err(CLIError::ConfigurationError(format!(
            "invalid migration status from system.migrations: {other}"
        ))),
    }
}

fn required_string(
    row: &std::collections::HashMap<String, KalamCellValue>,
    column: &str,
) -> Result<String> {
    optional_string(row, column).ok_or_else(|| {
        CLIError::ConfigurationError(format!("server migration row missing {column}"))
    })
}

fn optional_string(
    row: &std::collections::HashMap<String, KalamCellValue>,
    column: &str,
) -> Option<String> {
    row.get(column)?.as_str().map(ToString::to_string)
}

fn optional_i64(
    row: &std::collections::HashMap<String, KalamCellValue>,
    column: &str,
) -> Option<i64> {
    row.get(column).and_then(|value| match value.inner() {
        serde_json::Value::Number(number) => number.as_i64(),
        serde_json::Value::String(value) => value.parse().ok(),
        _ => None,
    })
}

impl From<ServerMigrationRecord> for MigrationRecord {
    fn from(record: ServerMigrationRecord) -> Self {
        Self {
            migration_id: record.migration_id,
            namespace: kalamdb_commons::NamespaceId::new(record.namespace),
            name: record.name,
            checksum: record.checksum,
            status: record.status,
            started_at: record.started_at.map(timestamp_millis_to_rfc3339),
            finished_at: record.finished_at.map(timestamp_millis_to_rfc3339),
            error_message: record.error_message,
            sql: None,
            source: record.source,
            kalam_version: record.kalam_version,
        }
    }
}

fn timestamp_millis_to_rfc3339(value: i64) -> String {
    chrono::DateTime::<chrono::Utc>::from_timestamp_millis(value)
        .unwrap_or_else(chrono::Utc::now)
        .to_rfc3339()
}
