use std::sync::Arc;

use arrow::{
    array::{ArrayRef, BooleanArray, StringArray, UInt64Array},
    record_batch::RecordBatch,
};
use kalamdb_commons::arrow_utils::{field_boolean, field_uint64, field_utf8, schema};
use kalamdb_core::{error::KalamDbError, sql::context::ExecutionResult};

pub fn cluster_group_action_rows<I>(
    action: &str,
    target_node_id: Option<u64>,
    upto: Option<u64>,
    snapshots_dir: Option<String>,
    results: I,
) -> Result<ExecutionResult, KalamDbError>
where
    I: IntoIterator<Item = (String, bool, Option<String>, Option<u64>)>,
{
    let mut group_ids = Vec::new();
    let mut successes = Vec::new();
    let mut errors = Vec::new();
    let mut snapshot_indices = Vec::new();

    for (group_id, success, error, snapshot_index) in results {
        group_ids.push(Some(group_id));
        successes.push(Some(success));
        errors.push(error);
        snapshot_indices.push(snapshot_index);
    }

    if group_ids.is_empty() {
        group_ids.push(None);
        successes.push(None);
        errors.push(None);
        snapshot_indices.push(None);
    }

    let row_count = group_ids.len();
    let schema = schema(vec![
        field_utf8("action", false),
        field_utf8("group_id", true),
        field_boolean("success", true),
        field_utf8("error", true),
        field_uint64("snapshot_index", true),
        field_uint64("target_node_id", true),
        field_uint64("upto", true),
        field_utf8("snapshots_dir", true),
    ]);

    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![
            Arc::new(StringArray::from(vec![action.to_string(); row_count])) as ArrayRef,
            Arc::new(StringArray::from(group_ids)) as ArrayRef,
            Arc::new(BooleanArray::from(successes)) as ArrayRef,
            Arc::new(StringArray::from(errors)) as ArrayRef,
            Arc::new(UInt64Array::from(snapshot_indices)) as ArrayRef,
            Arc::new(UInt64Array::from(vec![target_node_id; row_count])) as ArrayRef,
            Arc::new(UInt64Array::from(vec![upto; row_count])) as ArrayRef,
            Arc::new(StringArray::from(vec![snapshots_dir; row_count])) as ArrayRef,
        ],
    )
    .map_err(|e| {
        KalamDbError::SerializationError(format!(
            "Failed to build cluster action result rows: {}",
            e
        ))
    })?;

    Ok(ExecutionResult::Rows {
        batches: vec![batch],
        row_count,
        schema: Some(schema),
    })
}

pub fn cluster_join_rows(
    node_id: u64,
    rpc_addr: &str,
    api_addr: &str,
    rebalance_requested: bool,
) -> Result<ExecutionResult, KalamDbError> {
    let schema = schema(vec![
        field_utf8("action", false),
        field_uint64("node_id", false),
        field_utf8("rpc_addr", false),
        field_utf8("api_addr", false),
        field_boolean("rebalance_requested", false),
    ]);

    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![
            Arc::new(StringArray::from(vec!["join"])) as ArrayRef,
            Arc::new(UInt64Array::from(vec![node_id])) as ArrayRef,
            Arc::new(StringArray::from(vec![rpc_addr])) as ArrayRef,
            Arc::new(StringArray::from(vec![api_addr])) as ArrayRef,
            Arc::new(BooleanArray::from(vec![rebalance_requested])) as ArrayRef,
        ],
    )
    .map_err(|e| {
        KalamDbError::SerializationError(format!("Failed to build cluster join result rows: {}", e))
    })?;

    Ok(ExecutionResult::Rows {
        batches: vec![batch],
        row_count: 1,
        schema: Some(schema),
    })
}

pub fn cluster_clear_rows(
    snapshots_dir: String,
    snapshots_dir_exists: bool,
    total_snapshots_found: u64,
    total_size_bytes: u64,
    snapshots_cleared: u64,
    cleared_size_bytes: u64,
    errors: Vec<String>,
) -> Result<ExecutionResult, KalamDbError> {
    let row_count = errors.len().max(1);
    let error_count = errors.len() as u64;
    let error_rows = if errors.is_empty() {
        vec![None]
    } else {
        errors.into_iter().map(Some).collect()
    };

    let schema = schema(vec![
        field_utf8("action", false),
        field_utf8("snapshots_dir", false),
        field_boolean("snapshots_dir_exists", false),
        field_uint64("total_snapshots_found", false),
        field_uint64("total_size_bytes", false),
        field_uint64("snapshots_cleared", false),
        field_uint64("cleared_size_bytes", false),
        field_uint64("error_count", false),
        field_utf8("error", true),
    ]);

    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![
            Arc::new(StringArray::from(vec!["clear"; row_count])) as ArrayRef,
            Arc::new(StringArray::from(vec![snapshots_dir; row_count])) as ArrayRef,
            Arc::new(BooleanArray::from(vec![snapshots_dir_exists; row_count])) as ArrayRef,
            Arc::new(UInt64Array::from(vec![total_snapshots_found; row_count])) as ArrayRef,
            Arc::new(UInt64Array::from(vec![total_size_bytes; row_count])) as ArrayRef,
            Arc::new(UInt64Array::from(vec![snapshots_cleared; row_count])) as ArrayRef,
            Arc::new(UInt64Array::from(vec![cleared_size_bytes; row_count])) as ArrayRef,
            Arc::new(UInt64Array::from(vec![error_count; row_count])) as ArrayRef,
            Arc::new(StringArray::from(error_rows)) as ArrayRef,
        ],
    )
    .map_err(|e| {
        KalamDbError::SerializationError(format!(
            "Failed to build cluster clear result rows: {}",
            e
        ))
    })?;

    Ok(ExecutionResult::Rows {
        batches: vec![batch],
        row_count,
        schema: Some(schema),
    })
}
