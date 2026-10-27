use std::sync::{Arc, OnceLock};

use arrow::{
    array::{ArrayRef, Int32Array, Int64Array, StringBuilder},
    datatypes::{DataType, Field, Schema, SchemaRef},
    record_batch::RecordBatch,
};
use kalamdb_core::{error::KalamDbError, sql::context::ExecutionResult};

fn ack_schema() -> SchemaRef {
    static SCHEMA: OnceLock<SchemaRef> = OnceLock::new();
    SCHEMA
        .get_or_init(|| {
            Arc::new(Schema::new(vec![
                Field::new("topic", DataType::Utf8, false),
                Field::new("group_id", DataType::Utf8, false),
                Field::new("partition", DataType::Int32, false),
                Field::new("committed_offset", DataType::Int64, false),
            ]))
        })
        .clone()
}

fn reset_consumer_group_schema() -> SchemaRef {
    static SCHEMA: OnceLock<SchemaRef> = OnceLock::new();
    SCHEMA
        .get_or_init(|| {
            Arc::new(Schema::new(vec![
                Field::new("topic", DataType::Utf8, false),
                Field::new("group_id", DataType::Utf8, false),
                Field::new("partition", DataType::Int32, false),
                Field::new("next_offset", DataType::Int64, false),
            ]))
        })
        .clone()
}

pub fn ack_result(
    topic_name: &str,
    group_id: &str,
    partition_id: u32,
    upto_offset: u64,
) -> Result<ExecutionResult, KalamDbError> {
    let schema = ack_schema();

    let mut topic_builder = StringBuilder::new();
    let mut group_builder = StringBuilder::new();
    topic_builder.append_value(topic_name);
    group_builder.append_value(group_id);

    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![
            Arc::new(topic_builder.finish()) as ArrayRef,
            Arc::new(group_builder.finish()) as ArrayRef,
            Arc::new(Int32Array::from(vec![partition_id as i32])) as ArrayRef,
            Arc::new(Int64Array::from(vec![upto_offset as i64])) as ArrayRef,
        ],
    )
    .map_err(|e| {
        KalamDbError::SerializationError(format!("Failed to create RecordBatch: {}", e))
    })?;

    Ok(ExecutionResult::Rows {
        batches: vec![batch],
        row_count: 1,
        schema: Some(schema),
    })
}

pub fn reset_consumer_group_result(
    topic_name: &str,
    group_id: &str,
    partition_id: u32,
    next_offset: u64,
) -> Result<ExecutionResult, KalamDbError> {
    let schema = reset_consumer_group_schema();

    let mut topic_builder = StringBuilder::new();
    let mut group_builder = StringBuilder::new();
    topic_builder.append_value(topic_name);
    group_builder.append_value(group_id);

    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![
            Arc::new(topic_builder.finish()) as ArrayRef,
            Arc::new(group_builder.finish()) as ArrayRef,
            Arc::new(Int32Array::from(vec![partition_id as i32])) as ArrayRef,
            Arc::new(Int64Array::from(vec![next_offset as i64])) as ArrayRef,
        ],
    )
    .map_err(|e| {
        KalamDbError::SerializationError(format!("Failed to create RecordBatch: {}", e))
    })?;

    Ok(ExecutionResult::Rows {
        batches: vec![batch],
        row_count: 1,
        schema: Some(schema),
    })
}
