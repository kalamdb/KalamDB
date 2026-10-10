//! Arrow to JSON conversion helpers

use arrow::record_batch::RecordBatch;
use kalamdb_commons::{
    conversions::mask_sensitive_rows_for_role,
    models::{rows::Row, Role},
};
use kalamdb_core::providers::arrow_json_conversion::{
    record_batch_to_json_arrays, rows_to_json_arrays,
};

use super::{super::models::QueryResult, schema_response_cache::cached_sql_schema};

/// Convert Arrow RecordBatches to QueryResult
pub fn record_batch_to_query_result(
    batches: Vec<RecordBatch>,
    schema: Option<arrow::datatypes::SchemaRef>,
    user_role: Option<Role>,
) -> Result<QueryResult, Box<dyn std::error::Error>> {
    let arrow_schema = match resolve_arrow_schema(&batches, schema) {
        Some(schema) => schema,
        None => return Ok(QueryResult::with_message("Query executed successfully".to_string())),
    };

    let cached = cached_sql_schema(&arrow_schema);
    let schema_fields = cached.fields.as_ref().clone();

    let mut rows = Vec::new();
    for batch in &batches {
        let batch_rows = record_batch_to_json_arrays(batch)
            .map_err(|e| format!("Failed to convert batch to JSON: {}", e))?;
        rows.extend(batch_rows);
    }

    if let Some(role) = user_role {
        mask_sensitive_rows_for_role(&mut rows, &schema_fields, role);
    }

    let result = QueryResult::with_rows_and_schema(rows, schema_fields);
    Ok(result)
}

/// JSON for cached point-get rows. Skips the Arrow batch those rows used to
/// be rebuilt into before the same cells were read back out.
pub fn scalar_rows_to_query_result(
    rows: Vec<Row>,
    schema: arrow::datatypes::SchemaRef,
    user_role: Option<Role>,
) -> Result<QueryResult, Box<dyn std::error::Error>> {
    let cached = cached_sql_schema(&schema);
    let mut json_rows = rows_to_json_arrays(&schema, rows)?;
    if let Some(role) = user_role {
        mask_sensitive_rows_for_role(&mut json_rows, cached.fields.as_ref(), role);
    }
    let schema_fields = cached.fields.as_ref().clone();
    Ok(QueryResult::with_rows_and_schema(json_rows, schema_fields))
}

pub fn resolve_arrow_schema(
    batches: &[RecordBatch],
    schema: Option<arrow::datatypes::SchemaRef>,
) -> Option<arrow::datatypes::SchemaRef> {
    if !batches.is_empty() {
        Some(batches[0].schema())
    } else {
        schema
    }
}

pub fn success_response_suffix(row_count: usize, as_user: &str, took: f64) -> String {
    let rounded = (took * 1000.0).round() / 1000.0;
    format!(
        "],\"row_count\":{},\"as_user\":{}}}],\"took\":{},\"error\":null}}",
        row_count,
        serde_json::to_string(as_user).unwrap_or_else(|_| "\"unknown\"".to_string()),
        rounded
    )
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use arrow::{
        array::RecordBatch,
        datatypes::{DataType, Field, Schema},
    };
    use kalamdb_commons::{
        conversions::{with_kalam_column_flags_metadata, with_kalam_data_type_metadata},
        models::datatypes::KalamDataType,
        schemas::{FieldFlag, FieldFlags},
    };

    use super::*;

    #[test]
    fn test_record_batch_to_query_result_includes_flags_and_omits_empty() {
        let id_field = with_kalam_column_flags_metadata(
            with_kalam_data_type_metadata(
                Field::new("id", DataType::FixedSizeBinary(16), false),
                &KalamDataType::Uuid,
            ),
            &FieldFlags::from([FieldFlag::PrimaryKey, FieldFlag::NonNull, FieldFlag::Unique]),
        );
        let tenant_field = with_kalam_column_flags_metadata(
            with_kalam_data_type_metadata(
                Field::new("tenant_id", DataType::Utf8, false),
                &KalamDataType::Text,
            ),
            &FieldFlags::from([FieldFlag::NonNull]),
        );
        let payload_field = with_kalam_data_type_metadata(
            Field::new("payload", DataType::Utf8, true),
            &KalamDataType::Text,
        );

        let schema = Arc::new(Schema::new(vec![id_field, tenant_field, payload_field]));

        let result = record_batch_to_query_result(vec![], Some(schema), None).unwrap();

        assert_eq!(result.schema.len(), 3);
        assert_eq!(result.schema[0].name, "id");
        assert!(matches!(
            result.schema[0].flags,
            Some(ref flags)
                if flags.contains(&FieldFlag::PrimaryKey)
                    && flags.contains(&FieldFlag::NonNull)
                    && flags.contains(&FieldFlag::Unique)
        ));
        assert_eq!(result.schema[1].name, "tenant_id");
        assert!(matches!(
            result.schema[1].flags,
            Some(ref flags) if flags.contains(&FieldFlag::NonNull)
        ));
        assert_eq!(result.schema[2].name, "payload");
        assert!(result.schema[2].flags.is_none());
    }

    #[test]
    fn scalar_rows_match_arrow_json() {
        use std::collections::BTreeMap;

        use arrow::array::Int64Array;
        use kalamdb_commons::models::rows::Row;
        use kalamdb_core::sql::ScalarValue;

        let schema = Arc::new(Schema::new(vec![Field::new("id", DataType::Int64, false)]));
        let batch = RecordBatch::try_new(
            Arc::clone(&schema),
            vec![Arc::new(Int64Array::from(vec![7_i64]))],
        )
        .expect("batch");
        let from_arrow = record_batch_to_query_result(vec![batch], None, None).expect("arrow json");

        let mut values = BTreeMap::new();
        values.insert("id".to_string(), ScalarValue::Int64(Some(7)));
        let from_rows =
            scalar_rows_to_query_result(vec![Row::new(values)], Arc::clone(&schema), None)
                .expect("scalar json");

        assert_eq!(
            serde_json::to_value(&from_arrow).expect("arrow value"),
            serde_json::to_value(&from_rows).expect("scalar value")
        );
    }

    #[test]
    fn test_record_batch_to_query_result_without_column_flags_metadata() {
        let schema = Arc::new(Schema::new(vec![Field::new("name", DataType::Utf8, true)]));
        let empty_batch = RecordBatch::new_empty(schema);

        let result = record_batch_to_query_result(vec![empty_batch], None, None).unwrap();
        assert_eq!(result.schema.len(), 1);
        assert_eq!(result.schema[0].name, "name");
        assert!(result.schema[0].flags.is_none());
    }
}
