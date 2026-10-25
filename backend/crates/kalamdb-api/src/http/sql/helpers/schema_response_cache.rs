//! Cache Arrow→API schema conversion for SQL SELECT responses.
//!
//! Every SELECT used to rebuild `Vec<SchemaField>` and re-serialize it into the
//! JSON prefix. For repeated point lookups with a stable result schema (the
//! comparison / SDK hot path), reuse both the field descriptors and the
//! pre-serialized schema JSON.

use std::{
    hash::{Hash, Hasher},
    sync::{Arc, LazyLock},
};

use arrow::datatypes::SchemaRef;
use bytes::Bytes;
use kalamdb_commons::{conversions::schema_fields_from_arrow_schema, schemas::SchemaField};
use moka::sync::Cache;

const ROW_RESULT_PREFIX_HEAD: &str = "{\"status\":\"success\",\"results\":[{\"schema\":";
const ROW_RESULT_PREFIX_TAIL: &str = ",\"rows\":[";

/// Cached API schema projection for a given Arrow schema shape.
#[derive(Clone)]
pub struct CachedSqlSchema {
    pub fields: Arc<Vec<SchemaField>>,
    /// Prebuilt streaming/inline JSON prefix through `"rows":[`.
    pub row_result_prefix: Bytes,
}

fn row_result_prefix_bytes(schema_json: &str) -> Bytes {
    let mut prefix = String::with_capacity(
        ROW_RESULT_PREFIX_HEAD.len() + schema_json.len() + ROW_RESULT_PREFIX_TAIL.len(),
    );
    prefix.push_str(ROW_RESULT_PREFIX_HEAD);
    prefix.push_str(schema_json);
    prefix.push_str(ROW_RESULT_PREFIX_TAIL);
    Bytes::from(prefix)
}

static SCHEMA_RESPONSE_CACHE: LazyLock<Cache<u64, Arc<CachedSqlSchema>>> =
    LazyLock::new(|| Cache::builder().max_capacity(512).build());

fn schema_fingerprint(schema: &arrow::datatypes::Schema) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    schema.fields().len().hash(&mut hasher);
    for field in schema.fields() {
        field.name().hash(&mut hasher);
        field.data_type().hash(&mut hasher);
        field.is_nullable().hash(&mut hasher);
        // Kalam metadata drives SchemaField data_type/flags; include it in the key.
        if let Some(value) = field.metadata().get("kalam_data_type") {
            value.hash(&mut hasher);
        }
        if let Some(value) = field.metadata().get("kalam_column_flags") {
            value.hash(&mut hasher);
        }
    }
    hasher.finish()
}

/// Return cached SchemaField descriptors + pre-serialized schema JSON for `schema`.
pub fn cached_sql_schema(schema: &SchemaRef) -> Arc<CachedSqlSchema> {
    let key = schema_fingerprint(schema.as_ref());
    if let Some(hit) = SCHEMA_RESPONSE_CACHE.get(&key) {
        return hit;
    }

    let fields = schema_fields_from_arrow_schema(schema);
    let schema_json = serde_json::to_string(&fields).unwrap_or_else(|_| "[]".to_string());
    let entry = Arc::new(CachedSqlSchema {
        fields: Arc::new(fields),
        row_result_prefix: row_result_prefix_bytes(&schema_json),
    });
    SCHEMA_RESPONSE_CACHE.insert(key, Arc::clone(&entry));
    entry
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use arrow::datatypes::{DataType, Field, Schema};

    use super::*;

    #[test]
    fn caches_identical_schemas() {
        let schema = Arc::new(Schema::new(vec![
            Field::new("id", DataType::Int64, false),
            Field::new("data", DataType::Utf8, true),
        ]));

        let first = cached_sql_schema(&schema);
        let second = cached_sql_schema(&schema);
        assert!(Arc::ptr_eq(&first, &second));
        assert_eq!(first.fields.len(), 2);
        let prefix = std::str::from_utf8(&first.row_result_prefix).expect("utf8 prefix");
        assert!(prefix.contains("\"id\""));
        assert!(prefix.starts_with("{\"status\":\"success\""));
        assert!(prefix.ends_with(",\"rows\":["));
    }
}
