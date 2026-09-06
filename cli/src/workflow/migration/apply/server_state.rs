use kalam_client::{KalamCellValue, KalamLinkClient};
use serde::Deserialize;

use crate::{
    error::{CLIError, Result},
    workflow::migration::{MigrationRecord, MigrationState, MigrationStatus},
};

#[derive(Debug, Deserialize)]
struct ServerMigrationRecord {
    migration_key: String,
    migration_id:  String,
    namespace:     String,
    name:          String,
    checksum:      String,
    status:        MigrationStatus,
    started_at:    Option<i64>,
    finished_at:   Option<i64>,
    error_message: Option<String>,
    source:        Option<String>,
    kalam_version: Option<String>,
}

pub(crate) async fn load_server_migration_state(
    client: &KalamLinkClient,
    namespace: &kalamdb_commons::NamespaceId,
) -> Result<MigrationState> {
    let sql = format!(
        "SELECT migration_key, migration_id, namespace, name, checksum, status, started_at, \
         finished_at, error_message, source, kalam_version FROM system.migrations WHERE namespace \
         = {}",
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
    if record_exists {
        let update_sql = build_update_sql(record, &migration_key)?;
        let response = client
            .execute_query(&update_sql, None, None, None)
            .await
            .map_err(CLIError::from)?;
        if !response.success() {
            return Err(save_failed_error(record, &response));
        }
        if response.row_count() > 0 {
            return Ok(());
        }
    }

    upsert_server_migration_record(client, record, &migration_key).await
}

pub(crate) async fn save_server_migration_records(
    client: &KalamLinkClient,
    records: &[&MigrationRecord],
) -> Result<()> {
    for record in records {
        save_server_migration_record(client, record, true).await?;
    }
    Ok(())
}

pub(crate) async fn upsert_server_migration_record(
    client: &KalamLinkClient,
    record: &MigrationRecord,
    migration_key: &str,
) -> Result<()> {
    let insert_sql = build_insert_sql(record, migration_key)?;
    let response = client
        .execute_query(&insert_sql, None, None, None)
        .await
        .map_err(CLIError::from)?;
    if response.success() && response.row_count() > 0 {
        return Ok(());
    }
    Err(save_failed_error(record, &response))
}

fn build_update_sql(record: &MigrationRecord, migration_key: &str) -> Result<String> {
    Ok(format!(
        "UPDATE system.migrations SET namespace = {}, name = {}, checksum = {}, status = {}, \
         started_at = {}, finished_at = {}, error_message = {}, source = {}, kalam_version = {} \
         WHERE migration_key = {}",
        sql_string(record.namespace.as_str()),
        sql_string(&record.name),
        sql_string(&record.checksum),
        sql_string(&record.status.to_string()),
        sql_timestamp(record.started_at.as_deref())?,
        sql_timestamp(record.finished_at.as_deref())?,
        sql_nullable_string(record.error_message.as_deref()),
        sql_nullable_string(record.source.as_deref()),
        sql_nullable_string(record.kalam_version.as_deref()),
        sql_string(migration_key),
    ))
}

fn build_insert_sql(record: &MigrationRecord, migration_key: &str) -> Result<String> {
    Ok(format!(
        "INSERT INTO system.migrations (migration_key, migration_id, namespace, name, checksum, \
         status, started_at, finished_at, error_message, source, kalam_version) VALUES ({}, {}, \
         {}, {}, {}, {}, {}, {}, {}, {}, {})",
        sql_string(migration_key),
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
    ))
}

fn save_failed_error(record: &MigrationRecord, response: &kalam_client::QueryResponse) -> CLIError {
    CLIError::ConfigurationError(format!(
        "failed to save server migration state for {}: {}",
        record.migration_id,
        crate::workflow::sql::query_failure_message(response, "query failed")
    ))
}

pub(crate) fn server_migration_key(record: &MigrationRecord) -> String {
    record
        .migration_key
        .clone()
        .unwrap_or_else(|| format!("{}:{}", record.namespace.as_str(), record.migration_id))
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
        migration_key: required_string(&row, "migration_key")?,
        migration_id:  required_string(&row, "migration_id")?,
        namespace:     required_string(&row, "namespace")?,
        name:          required_string(&row, "name")?,
        checksum:      required_string(&row, "checksum")?,
        status:        parse_status(&required_string(&row, "status")?)?,
        started_at:    optional_timestamp_millis(&row, "started_at"),
        finished_at:   optional_timestamp_millis(&row, "finished_at"),
        error_message: optional_string(&row, "error_message"),
        source:        optional_string(&row, "source"),
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

fn optional_timestamp_millis(
    row: &std::collections::HashMap<String, KalamCellValue>,
    column: &str,
) -> Option<i64> {
    row.get(column)?.as_timestamp().map(timestamp_micros_to_millis)
}

fn timestamp_micros_to_millis(value: i64) -> i64 {
    value / 1_000
}

impl From<ServerMigrationRecord> for MigrationRecord {
    fn from(record: ServerMigrationRecord) -> Self {
        Self {
            migration_id:  record.migration_id,
            namespace:     kalamdb_commons::NamespaceId::new(record.namespace),
            migration_key: Some(record.migration_key),
            name:          record.name,
            checksum:      record.checksum,
            status:        record.status,
            started_at:    record.started_at.map(timestamp_millis_to_rfc3339),
            finished_at:   record.finished_at.map(timestamp_millis_to_rfc3339),
            error_message: record.error_message,
            sql:           None,
            source:        record.source,
            kalam_version: record.kalam_version,
        }
    }
}

fn timestamp_millis_to_rfc3339(value: i64) -> String {
    chrono::DateTime::<chrono::Utc>::from_timestamp_millis(value)
        .unwrap_or_else(chrono::Utc::now)
        .to_rfc3339()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn server_migration_key_prefers_loaded_primary_key() {
        let record = MigrationRecord {
            migration_id:  "0001_init.sql".into(),
            namespace:     kalamdb_commons::NamespaceId::new("okf_sync"),
            migration_key: Some("legacy-key:0001_init.sql".into()),
            name:          "init".into(),
            checksum:      "abc".into(),
            status:        MigrationStatus::Failed,
            started_at:    None,
            finished_at:   None,
            error_message: None,
            sql:           None,
            source:        None,
            kalam_version: None,
        };

        assert_eq!(server_migration_key(&record), "legacy-key:0001_init.sql");
    }

    #[test]
    fn server_migration_record_converts_timestamp_wire_micros_to_model_millis() {
        let row = std::collections::HashMap::from([
            ("migration_key".to_string(), KalamCellValue::text("app:0001_init.sql")),
            ("migration_id".to_string(), KalamCellValue::text("0001_init.sql")),
            ("namespace".to_string(), KalamCellValue::text("app")),
            ("name".to_string(), KalamCellValue::text("init")),
            ("checksum".to_string(), KalamCellValue::text("abc")),
            ("status".to_string(), KalamCellValue::text("failed")),
            ("started_at".to_string(), KalamCellValue::int(1_700_000_000_000_000)),
            ("finished_at".to_string(), KalamCellValue::int(1_700_000_000_100_000)),
            ("error_message".to_string(), KalamCellValue::text("table already exists")),
            ("source".to_string(), KalamCellValue::text("0001_init.sql")),
            ("kalam_version".to_string(), KalamCellValue::text("test")),
        ]);

        let record = server_migration_record_from_row(row).unwrap();
        let migration = MigrationRecord::from(record);

        assert_eq!(migration.started_at.as_deref(), Some("2023-11-14T22:13:20+00:00"));
        assert_eq!(migration.finished_at.as_deref(), Some("2023-11-14T22:13:20.100+00:00"));
    }
}
