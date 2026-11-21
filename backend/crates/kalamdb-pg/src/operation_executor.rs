use std::str::FromStr;

use arrow::record_batch::RecordBatch;
use arrow_ipc::writer::StreamWriter;
use async_trait::async_trait;
use kalamdb_backend::session::LiveSessionTransaction;
// Re-export domain types from kalamdb-commons (canonical location).
pub use kalamdb_commons::models::pg_operations::{
    DeleteRequest, InsertRequest, MutationResult, ScanRequest, ScanResult, UpdateRequest,
};
use kalamdb_commons::{
    models::{
        rows::{Row, StoredScalarValue},
        TransactionId, UserId,
    },
    TableId, TableType,
};
use serde_json::Value;
use tonic::Status;

use crate::{
    service::{ScanRpcRequest, ScanRpcResponse},
    DeleteRpcRequest, InsertRpcRequest, UpdateRpcRequest,
};

/// Domain-typed query executor.
///
/// `KalamPgService` translates gRPC wire types into domain types, calls this
/// trait, then encodes domain results back to gRPC responses.
#[async_trait]
pub trait OperationExecutor: Send + Sync + 'static {
    async fn execute_scan(&self, request: ScanRequest) -> Result<ScanResult, Status>;
    async fn execute_insert(&self, request: InsertRequest) -> Result<MutationResult, Status>;
    async fn execute_update(&self, request: UpdateRequest) -> Result<MutationResult, Status>;
    async fn execute_delete(&self, request: DeleteRequest) -> Result<MutationResult, Status>;
    async fn active_transaction(
        &self,
        _session_id: &str,
    ) -> Result<Option<LiveSessionTransaction>, Status> {
        Ok(None)
    }
    async fn begin_transaction(&self, _session_id: &str) -> Result<Option<TransactionId>, Status> {
        Ok(None)
    }
    async fn commit_transaction(
        &self,
        _session_id: &str,
        _transaction_id: &TransactionId,
    ) -> Result<Option<TransactionId>, Status> {
        Ok(None)
    }
    async fn rollback_transaction(
        &self,
        _session_id: &str,
        _transaction_id: &TransactionId,
    ) -> Result<Option<TransactionId>, Status> {
        Ok(None)
    }
    async fn execute_sql(&self, sql: &str) -> Result<String, Status>;
    /// Execute an arbitrary SQL statement and return (message, ipc_batches).
    ///
    /// For SELECT queries `ipc_batches` contains Arrow IPC-encoded RecordBatches.
    /// For DDL/DML `ipc_batches` is empty and `message` carries the status string.
    async fn execute_query(&self, sql: &str) -> Result<(String, Vec<bytes::Bytes>), Status>;
}

// ── Helpers for gRPC ↔ domain translation ──────────────────────────────────

pub fn parse_table_type(s: &str) -> Result<TableType, Status> {
    TableType::from_str(s)
        .map_err(|e| Status::invalid_argument(format!("invalid table_type: {}", e)))
}

pub fn parse_table_id(namespace: &str, table_name: &str) -> Result<TableId, Status> {
    use kalamdb_commons::models::{SchemaId, TableName};
    if namespace.trim().is_empty() {
        return Err(Status::invalid_argument("namespace must not be empty"));
    }
    if table_name.trim().is_empty() {
        return Err(Status::invalid_argument("table_name must not be empty"));
    }
    Ok(TableId::new(SchemaId::new(namespace.trim()), TableName::new(table_name.trim())))
}

pub fn parse_user_id(raw: Option<&str>) -> Option<UserId> {
    raw.map(str::trim).filter(|s| !s.is_empty()).map(UserId::new)
}

fn stored_scalar_from_json_value(value: &Value) -> StoredScalarValue {
    match value {
        Value::Null => StoredScalarValue::Null,
        Value::Bool(value) => StoredScalarValue::Boolean(Some(*value)),
        Value::Number(value) => {
            if let Some(value) = value.as_i64() {
                StoredScalarValue::Int64(Some(value.to_string()))
            } else if let Some(value) = value.as_u64() {
                StoredScalarValue::UInt64(Some(value.to_string()))
            } else if let Some(value) = value.as_f64() {
                StoredScalarValue::Float64(Some(value))
            } else {
                StoredScalarValue::Fallback(value.to_string())
            }
        },
        Value::String(value) => StoredScalarValue::Utf8(Some(value.clone())),
        Value::Array(_) | Value::Object(_) => StoredScalarValue::Fallback(value.to_string()),
    }
}

fn parse_row_value(value: &Value) -> Result<Row, Status> {
    match value {
        Value::Object(values) => {
            let row = values
                .iter()
                .map(|(column_name, value)| {
                    Ok((column_name.clone(), stored_scalar_from_json_value(value).into()))
                })
                .collect::<Result<Vec<_>, Status>>()?;
            Ok(Row::from_vec(row))
        },
        _ => Err(Status::invalid_argument("invalid row JSON: expected object payload")),
    }
}

pub fn parse_row(json: &str) -> Result<Row, Status> {
    if let Ok(row) = serde_json::from_str::<Row>(json) {
        return Ok(row);
    }

    let value = serde_json::from_str::<Value>(json)
        .map_err(|e| Status::invalid_argument(format!("invalid row JSON: {}", e)))?;

    parse_row_value(&value)
}

fn parse_rows_json(json: &str) -> Result<Vec<Row>, Status> {
    if let Ok(rows) = serde_json::from_str::<Vec<Row>>(json) {
        return Ok(rows);
    }

    if let Ok(row) = serde_json::from_str::<Row>(json) {
        return Ok(vec![row]);
    }

    let value = serde_json::from_str::<Value>(json)
        .map_err(|e| Status::invalid_argument(format!("invalid row JSON: {}", e)))?;

    match value {
        Value::Array(rows) => rows.iter().map(parse_row_value).collect(),
        other => Ok(vec![parse_row_value(&other)?]),
    }
}

/// Encode all Arrow RecordBatches as one IPC stream for gRPC transport.
///
/// Keeping one stream avoids repeating the schema and dictionary state per batch.
/// The existing gRPC repeated-bytes response and clients accept a stream containing
/// multiple batches, so this does not change the wire contract.
pub fn encode_batches(
    batches: &[RecordBatch],
) -> Result<(Vec<bytes::Bytes>, Option<bytes::Bytes>), Status> {
    let Some(first) = batches.first() else {
        // ScanResult currently has no independent schema for zero-batch results.
        return Ok((Vec::new(), None));
    };

    let schema = first.schema();
    let mut buf = Vec::new();
    {
        let mut writer = StreamWriter::try_new(&mut buf, &schema)
            .map_err(|e| Status::internal(format!("failed to create IPC writer: {}", e)))?;
        for batch in batches {
            writer
                .write(batch)
                .map_err(|e| Status::internal(format!("failed to write IPC batch: {}", e)))?;
        }
        writer
            .finish()
            .map_err(|e| Status::internal(format!("failed to finish IPC writer: {}", e)))?;
    }

    Ok((vec![bytes::Bytes::from(buf)], None))
}

/// Convert a `ScanRpcRequest` into a domain `ScanRequest`.
pub fn scan_request_from_rpc(rpc: &ScanRpcRequest) -> Result<ScanRequest, Status> {
    let table_id = parse_table_id(&rpc.namespace, &rpc.table_name)?;
    let table_type = parse_table_type(&rpc.table_type)?;
    let filters = rpc
        .filters
        .iter()
        .filter(|f| f.op == "eq")
        .map(|f| (f.column.clone(), f.value.clone()))
        .collect();
    Ok(ScanRequest {
        table_id,
        table_type,
        session_id: Some(rpc.session_id.clone()),
        columns: rpc.columns.clone(),
        limit: rpc.limit.map(|l| l as usize),
        user_id: parse_user_id(rpc.user_id.as_deref()),
        filters,
    })
}

/// Convert a domain `ScanResult` into a `ScanRpcResponse`.
pub fn scan_result_to_rpc(result: ScanResult) -> Result<ScanRpcResponse, Status> {
    let (ipc_batches, schema_ipc) = encode_batches(&result.batches)?;
    Ok(ScanRpcResponse {
        ipc_batches,
        schema_ipc,
    })
}

/// Convert an `InsertRpcRequest` into a domain `InsertRequest`.
pub fn insert_request_from_rpc(rpc: &InsertRpcRequest) -> Result<InsertRequest, Status> {
    let table_id = parse_table_id(&rpc.namespace, &rpc.table_name)?;
    let table_type = parse_table_type(&rpc.table_type)?;
    let mut rows = Vec::with_capacity(rpc.rows_json.len());
    for json in &rpc.rows_json {
        rows.push(parse_row(json)?);
    }
    Ok(InsertRequest {
        table_id,
        table_type,
        session_id: Some(rpc.session_id.clone()),
        user_id: parse_user_id(rpc.user_id.as_deref()),
        rows,
    })
}

/// Convert an `UpdateRpcRequest` into a domain `UpdateRequest`.
pub fn update_request_from_rpc(rpc: &UpdateRpcRequest) -> Result<UpdateRequest, Status> {
    let table_id = parse_table_id(&rpc.namespace, &rpc.table_name)?;
    let table_type = parse_table_type(&rpc.table_type)?;
    let updates = parse_rows_json(&rpc.updates_json)?;
    Ok(UpdateRequest {
        table_id,
        table_type,
        session_id: Some(rpc.session_id.clone()),
        user_id: parse_user_id(rpc.user_id.as_deref()),
        updates,
        pk_value: rpc.pk_value.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ipc_multiple_batches_share_one_stream_and_round_trip() {
        use std::{io::Cursor, sync::Arc};
        use arrow::{
            array::Int64Array,
            datatypes::{DataType, Field, Schema},
        };
        use arrow_ipc::reader::StreamReader;

        let schema = Arc::new(Schema::new(vec![Field::new("id", DataType::Int64, true)]));
        let first = RecordBatch::try_new(
            Arc::clone(&schema),
            vec![Arc::new(Int64Array::from(vec![Some(1), None]))],
        ).unwrap();
        let second = RecordBatch::try_new(
            Arc::clone(&schema),
            vec![Arc::new(Int64Array::from(vec![Some(3)]))],
        ).unwrap();

        let (ipc, schema_ipc) = encode_batches(&[first.clone(), second.clone()]).unwrap();
        assert_eq!(ipc.len(), 1, "one stream should carry both batches");
        assert!(schema_ipc.is_none());
        let decoded = StreamReader::try_new(Cursor::new(ipc[0].as_ref()), None)
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(decoded, vec![first, second]);
    }

    #[test]
    fn ipc_zero_row_batch_preserves_schema() {
        use std::{io::Cursor, sync::Arc};
        use arrow::datatypes::{DataType, Field, Schema};
        use arrow_ipc::reader::StreamReader;

        let schema = Arc::new(Schema::new(vec![Field::new("id", DataType::Int64, true)]));
        let empty = RecordBatch::new_empty(schema.clone());
        let (ipc, _) = encode_batches(&[empty]).unwrap();
        assert_eq!(ipc.len(), 1);
        let mut reader = StreamReader::try_new(Cursor::new(ipc[0].as_ref()), None).unwrap();
        assert_eq!(reader.schema().as_ref(), schema.as_ref());
        assert_eq!(reader.next().unwrap().unwrap().num_rows(), 0);
        assert!(reader.next().is_none());
    }

    #[test]
    fn ipc_no_batches_still_returns_no_payload() {
        let (ipc, schema) = encode_batches(&[]).unwrap();
        assert!(ipc.is_empty());
        assert!(schema.is_none());
    }

    #[test]
    fn parse_row_accepts_plain_json_scalars() {
        let row = parse_row(r#"{"active":true,"count":7,"id":"msg-1","missing":null}"#)
            .expect("plain row JSON should parse");

        assert_eq!(row.len(), 4);
        assert_eq!(
            row.get("id").map(StoredScalarValue::from),
            Some(StoredScalarValue::Utf8(Some("msg-1".to_string())))
        );
        assert_eq!(
            row.get("count").map(StoredScalarValue::from),
            Some(StoredScalarValue::Int64(Some("7".to_string())))
        );
        assert_eq!(
            row.get("active").map(StoredScalarValue::from),
            Some(StoredScalarValue::Boolean(Some(true)))
        );
        assert_eq!(row.get("missing").map(StoredScalarValue::from), Some(StoredScalarValue::Null));
    }

    #[test]
    fn update_request_from_rpc_accepts_row_arrays() {
        let request = UpdateRpcRequest {
            namespace: "app".to_string(),
            table_name: "messages".to_string(),
            table_type: "shared".to_string(),
            session_id: "session-1".to_string(),
            user_id: None,
            pk_value: "row-1".to_string(),
            updates_json: r#"[{"name":{"Utf8":"updated"}}]"#.to_string(),
        };

        let parsed = update_request_from_rpc(&request).expect("update array payload should parse");

        assert_eq!(parsed.updates.len(), 1);
        assert_eq!(
            parsed.updates[0].get("name").map(StoredScalarValue::from),
            Some(StoredScalarValue::Utf8(Some("updated".to_string())))
        );
    }

    #[test]
    fn update_request_from_rpc_accepts_single_typed_row_object() {
        let request = UpdateRpcRequest {
            namespace: "app".to_string(),
            table_name: "messages".to_string(),
            table_type: "shared".to_string(),
            session_id: "session-1".to_string(),
            user_id: None,
            pk_value: "row-1".to_string(),
            updates_json: r#"{"name":{"Utf8":"updated"},"value":{"Int32":110}}"#.to_string(),
        };

        let parsed =
            update_request_from_rpc(&request).expect("single typed update payload should parse");

        assert_eq!(parsed.updates.len(), 1);
        assert_eq!(
            parsed.updates[0].get("name").map(StoredScalarValue::from),
            Some(StoredScalarValue::Utf8(Some("updated".to_string())))
        );
        assert_eq!(
            parsed.updates[0].get("value").map(StoredScalarValue::from),
            Some(StoredScalarValue::Int32(Some(110)))
        );
    }
}

/// Convert a `DeleteRpcRequest` into a domain `DeleteRequest`.
pub fn delete_request_from_rpc(rpc: &DeleteRpcRequest) -> Result<DeleteRequest, Status> {
    let table_id = parse_table_id(&rpc.namespace, &rpc.table_name)?;
    let table_type = parse_table_type(&rpc.table_type)?;
    Ok(DeleteRequest {
        table_id,
        table_type,
        session_id: Some(rpc.session_id.clone()),
        user_id: parse_user_id(rpc.user_id.as_deref()),
        pk_value: rpc.pk_value.clone(),
    })
}
