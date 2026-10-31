//! Ordinal-field insert helpers shared by user and shared tables.

use std::collections::HashSet;

use datafusion::{arrow::datatypes::SchemaRef, scalar::ScalarValue};
use kalamdb_commons::{
    ids::VersionId,
    models::{datatypes::KalamDataType, rows::Row, schemas::TableDefinition, TopicOp},
    websocket::ChangeNotification,
    TableId, UserId,
};

use crate::{error::KalamDbError, utils::core::TableProviderCore};

pub(crate) fn coerce_row_batch(
    rows: Vec<Row>,
    schema: &SchemaRef,
) -> Result<Vec<Row>, KalamDbError> {
    kalamdb_commons::conversions::arrow_json_conversion::coerce_rows(rows, schema)
        .map_err(|error| KalamDbError::InvalidOperation(format!("Schema coercion failed: {error}")))
}

pub(crate) fn require_version_count(versions: usize, rows: usize) -> Result<(), KalamDbError> {
    if versions != rows {
        return Err(KalamDbError::InvalidOperation(format!(
            "version count {versions} does not match row count {rows}"
        )));
    }
    Ok(())
}

pub(crate) fn coerce_update_row(updates: Row, schema: &SchemaRef) -> Result<Row, KalamDbError> {
    kalamdb_commons::conversions::arrow_json_conversion::coerce_updates(updates, schema)
        .map_err(|error| KalamDbError::InvalidOperation(format!("Schema coercion failed: {error}")))
}

pub(crate) fn merge_row_updates(latest: &Row, updates: Row) -> Row {
    let mut merged = latest.values.clone();
    for (key, value) in updates.values {
        merged.insert(key, value);
    }
    Row::new(merged)
}

/// Merged row after a field update, or `None` when the stored row does not change.
///
/// Not-null constraints are checked on the merged row before the no-op comparison.
pub(crate) fn changed_update_fields(
    latest: &Row,
    updates: Row,
    non_null_columns: &HashSet<String>,
) -> Result<Option<Row>, KalamDbError> {
    let new_fields = merge_row_updates(latest, updates);
    crate::utils::datafusion_dml::validate_not_null_with_set(
        non_null_columns,
        std::slice::from_ref(&new_fields),
    )?;
    if new_fields == *latest {
        Ok(None)
    } else {
        Ok(Some(new_fields))
    }
}

pub(crate) fn table_has_file_columns(table_def: &TableDefinition) -> bool {
    table_def
        .columns
        .iter()
        .any(|column| matches!(column.data_type(), KalamDataType::File))
}

pub(crate) fn rows_from_field_payloads(
    payloads: &[Vec<u8>],
    storage: &kalamdb_serialization::StorageSchema,
) -> Result<Vec<Row>, KalamDbError> {
    let names: Vec<String> = storage.fields.iter().map(|field| field.name.clone()).collect();
    payloads
        .iter()
        .map(|payload| {
            let columns = kalamdb_serialization::decode_row_fields_columns(payload, storage)
                .map_err(|error| {
                    KalamDbError::InvalidOperation(format!("ordinal insert decode failed: {error}"))
                })?;
            Ok(Row::from_aligned(&names, columns))
        })
        .collect()
}

pub(crate) fn coerce_storage_columns(
    columns: &mut [ScalarValue],
    storage: &kalamdb_serialization::StorageSchema,
    arrow: &SchemaRef,
) -> Result<(), KalamDbError> {
    for (value, field) in columns.iter_mut().zip(storage.fields.iter()) {
        if field.dropped {
            continue;
        }
        let Ok(arrow_field) = arrow.field_with_name(&field.name) else {
            continue;
        };
        let owned = std::mem::replace(value, ScalarValue::Null);
        *value = kalamdb_commons::conversions::coerce_scalar_to_field(owned, arrow_field).map_err(
            |error| KalamDbError::InvalidOperation(format!("Schema coercion failed: {error}")),
        )?;
    }
    Ok(())
}

pub(crate) struct DecodedOrdinalInsert {
    pub columns: Vec<Vec<ScalarValue>>,
    pub names: Vec<String>,
}

/// Decode ordinal payloads, coerce them to the Arrow schema, and check versions.
pub(crate) fn decode_ordinal_insert(
    payloads: &[Vec<u8>],
    storage: &kalamdb_serialization::StorageSchema,
    arrow: &SchemaRef,
    required: &HashSet<String>,
    versions: &[VersionId],
) -> Result<DecodedOrdinalInsert, KalamDbError> {
    let mut columns = Vec::with_capacity(payloads.len());
    for payload in payloads {
        let mut row = kalamdb_serialization::decode_row_fields_columns(payload, storage).map_err(
            |error| {
                KalamDbError::InvalidOperation(format!("ordinal insert decode failed: {error}"))
            },
        )?;
        coerce_storage_columns(&mut row, storage, arrow)?;
        columns.push(row);
    }
    validate_encoded_not_null(&columns, storage, required)?;
    require_version_count(versions.len(), columns.len())?;
    let names = storage.fields.iter().map(|field| field.name.clone()).collect();
    Ok(DecodedOrdinalInsert { columns, names })
}

pub(crate) fn validate_encoded_not_null(
    rows: &[Vec<ScalarValue>],
    storage: &kalamdb_serialization::StorageSchema,
    required: &HashSet<String>,
) -> Result<(), KalamDbError> {
    if required.is_empty() {
        return Ok(());
    }
    for (row_index, row) in rows.iter().enumerate() {
        for name in required {
            let Some(ordinal) = storage.fields.iter().position(|field| &field.name == name) else {
                continue;
            };
            if row.get(ordinal).is_none_or(ScalarValue::is_null) {
                return Err(KalamDbError::Coded(kalamdb_commons::CodedError::not_null(
                    name.clone(),
                    (row_index + 1) as i64,
                )));
            }
        }
    }
    Ok(())
}

pub(crate) fn warn_vector_upsert(
    table_id: &TableId,
    user_id: Option<&UserId>,
    version: VersionId,
    error: &impl std::fmt::Display,
) {
    match user_id {
        Some(user_id) => log::warn!(
            "Failed to stage vector upsert for table={}, user={}, seq={}: {}",
            table_id,
            user_id.as_str(),
            version.as_i64(),
            error
        ),
        None => log::warn!(
            "Failed to stage vector upsert for table={}, seq={}: {}",
            table_id,
            version.as_i64(),
            error
        ),
    }
}

pub(crate) fn warn_vector_delete(
    table_id: &TableId,
    user_id: Option<&UserId>,
    version: VersionId,
    pk: &str,
    error: &impl std::fmt::Display,
) {
    match user_id {
        Some(user_id) => log::warn!(
            "Failed to stage vector delete for table={}, user={}, seq={}, pk={}: {}",
            table_id,
            user_id.as_str(),
            version.as_i64(),
            pk,
            error
        ),
        None => log::warn!(
            "Failed to stage vector delete for table={}, seq={}, pk={}: {}",
            table_id,
            version.as_i64(),
            pk,
            error
        ),
    }
}

pub(crate) fn warn_vector_upsert_batch(
    table_id: &TableId,
    user_id: Option<&UserId>,
    error: &impl std::fmt::Display,
) {
    match user_id {
        Some(user_id) => log::warn!(
            "Failed to batch stage vector upserts for table={}, user={}: {}",
            table_id,
            user_id.as_str(),
            error
        ),
        None => {
            log::warn!("Failed to batch stage vector upserts for table={}: {}", table_id, error)
        },
    }
}

pub(crate) fn mark_table_pending_write(core: &TableProviderCore, user_id: Option<&UserId>) {
    if let Err(error) = core.services.manifest_service.mark_pending_write(core.table_id(), user_id)
    {
        log::warn!("Failed to mark manifest as pending_write for {}: {}", core.table_id(), error);
    }
}

/// Whether a table has topic routes, live subscribers, or both.
pub(crate) struct TableWatch {
    topics: bool,
    live: bool,
}

impl TableWatch {
    pub(crate) fn any(self) -> bool {
        self.topics || self.live
    }
}

pub(crate) fn table_watch(
    core: &TableProviderCore,
    subscriber_scope: Option<&UserId>,
) -> TableWatch {
    let table_id = core.table_id();
    TableWatch {
        topics: core.has_topic_routes(table_id),
        live: core.services.notification_service.has_subscribers(subscriber_scope, table_id),
    }
}

/// Publish one row change to topics, then to live subscribers when either is watching.
pub(crate) async fn fanout_row_change(
    core: &TableProviderCore,
    subscriber_scope: Option<&UserId>,
    topic_actor: Option<&UserId>,
    op: TopicOp,
    row: impl FnOnce() -> Row,
    live: impl FnOnce(TableId, Row) -> ChangeNotification,
) {
    let watch = table_watch(core, subscriber_scope);
    if !watch.topics && !watch.live {
        return;
    }
    let notification_service = core.services.notification_service.clone();
    let table_id = core.table_id().clone();
    let row = row();
    if watch.topics {
        core.publish_to_topics(&table_id, op, &row, topic_actor).await;
    }
    if watch.live {
        let notification = live(table_id.clone(), row);
        notification_service.notify_table_change(subscriber_scope.cloned(), table_id, notification);
    }
}

/// Publish an insert batch to topics and live subscribers when either is watching.
pub(crate) async fn fanout_insert_notifications(
    core: &TableProviderCore,
    subscriber_scope: Option<&UserId>,
    topic_actor: Option<&UserId>,
    rows: impl FnOnce() -> Vec<Row>,
) {
    let watch = table_watch(core, subscriber_scope);
    if !watch.topics && !watch.live {
        return;
    }
    let notification_service = core.services.notification_service.clone();
    let table_id = core.table_id().clone();
    let rows = rows();
    if watch.topics {
        core.publish_batch_to_topics(
            &table_id,
            kalamdb_commons::models::TopicOp::Insert,
            &rows,
            topic_actor,
        )
        .await;
    }
    if watch.live {
        let notify_scope = subscriber_scope.cloned();
        for row in rows {
            notification_service.notify_table_change(
                notify_scope.clone(),
                table_id.clone(),
                ChangeNotification::insert(table_id.clone(), row),
            );
        }
    }
}
