//! Shared utilities for converting between Arrow and JSON formats
//!
//! This module provides common functions for converting data between:
//! - Arrow RecordBatch (columnar format for DataFusion queries)
//! - JSON objects (row-oriented format for storage)
//!
//! These utilities are used by user_table_provider, shared_table_provider,
//! and stream_table_provider to avoid code duplication.
//!
//! # Conversion Architecture (Single Source of Truth)
//!
//! All ScalarValue → JSON conversions flow through ONE central function:
//! ```text
//! scalar_value_to_json()  ← SINGLE SOURCE OF TRUTH
//!          ↓
//!     ┌────┴────────────┐
//!     ↓                 ↓
//! row_to_json_map()   record_batch_to_json_arrays()
//!     ↓                 ↓
//! WebSocket:            REST API (general SELECT):
//! - notifications       - /v1/api/sql via RecordBatch
//! - subscriptions
//! - batch data
//!
//! rows_to_json_arrays()  ← REST cached PK point get (skip Arrow)
//! ```
//!
//! # Serialization Format
//!
//! All values are serialized as plain JSON values:
//! - Int64/UInt64 always serialized as strings to preserve precision
//! - Timestamps serialized as raw microsecond values (numbers)
//! - Boolean, String, Float values as native JSON types

use std::{collections::BTreeMap, convert::TryFrom, sync::Arc};

use arrow::{
    array::*,
    buffer::OffsetBuffer,
    datatypes::{DataType, Field, SchemaRef, TimeUnit},
    record_batch::RecordBatch,
};
use datafusion_common::{DataFusionError, ScalarValue};
use serde_json::Value as JsonValue;
use uuid::Uuid;

// Chrono no longer needed - DataFusion handles timestamp serialization natively
use crate::models::rows::Row;
use crate::{
    conversions::{
        scalar::json::{format_decimal_value, parse_decimal_value_bounded},
        schema_metadata::{
            read_character_maximum_length, read_kalam_data_type_metadata,
            read_kalam_domain_constraints, read_logical_type_modifier, validate_enum_label,
            DomainConstraintSpec,
        },
    },
    errors::CommonError,
    models::{datatypes::KalamDataType, FileRef, KalamCellValue},
};

/// Type alias for Arc<dyn Array> to improve readability
type ArrayRef = Arc<dyn Array>;

/// Coerce a list of rows to match the schema types and fill missing columns with typed NULLs.
///
/// Explicit SQL defaults must be applied before this step. This helper only ensures that every
/// schema column is present and correctly typed so downstream constraint validation can see
/// omitted columns instead of having them silently materialized to placeholder values.
pub fn coerce_rows(rows: Vec<Row>, schema: &SchemaRef) -> Result<Vec<Row>, String> {
    let _span = kalamdb_observability::kdb_debug_span_entered!(
        "coerce_rows",
        row_count = rows.len(),
        num_fields = schema.fields().len()
    );
    let typed_nulls = get_typed_nulls(schema);

    rows.into_iter()
        .map(|mut row| {
            // 1. Materialize missing schema columns as domain defaults (when present) or typed
            //    NULLs so constraint checks can distinguish omitted required fields from explicit
            //    defaults applied earlier.
            for (i, field) in schema.fields().iter().enumerate() {
                let field_name = field.name().as_str();
                if !row.values.contains_key(field_name) {
                    let initial = match materialize_domain_default_for_field(field.as_ref())? {
                        Some(default_val) => default_val,
                        None => typed_nulls[i].clone(),
                    };
                    row.values.insert(field_name.to_string(), initial);
                }
            }

            // 2. Coerce existing values in-place using get_mut + mem::replace. This avoids
            //    rebuilding the entire BTreeMap (no remove/re-insert, no new String key allocations
            //    for existing columns).
            for field in schema.fields() {
                if let Some(val) = row.values.get_mut(field.name().as_str()) {
                    let owned = std::mem::replace(val, ScalarValue::Null);
                    *val = coerce_scalar_to_field(owned, field)?;
                }
            }

            Ok(row)
        })
        .collect()
}

/// Convert ScalarValue-backed rows to Arrow RecordBatch
///
/// SELECT operations build row-oriented `Row` values (ScalarValue map) directly from storage.
/// This helper converts them into Arrow's columnar format without JSON round-trips.
pub fn json_rows_to_arrow_batch(schema: &SchemaRef, rows: Vec<Row>) -> Result<RecordBatch, String> {
    if rows.is_empty() {
        // Return empty batch with correct schema
        let empty_arrays: Vec<ArrayRef> = schema
            .fields()
            .iter()
            .map(|field| create_empty_array(field.data_type()))
            .collect();

        return RecordBatch::try_new(schema.clone(), empty_arrays)
            .map_err(|e| format!("Failed to create empty batch: {}", e));
    }

    // Pre-calculate default values for each column to avoid repeated logic in loop
    let defaults = get_column_defaults(schema);
    let typed_nulls = get_typed_nulls(schema);

    // Transpose rows to columns (Row-oriented -> Column-oriented)
    // We use move semantics (row.values.remove) to avoid cloning strings/blobs
    let mut columns: Vec<Vec<ScalarValue>> =
        (0..schema.fields().len()).map(|_| Vec::with_capacity(rows.len())).collect();

    for mut row in rows {
        for (i, field) in schema.fields().iter().enumerate() {
            let field_name = field.name().as_str();

            // Take value from map (move) instead of cloning
            let raw_value = row
                .values
                .remove(field_name)
                .or_else(|| defaults[i].clone())
                .unwrap_or_else(|| typed_nulls[i].clone());

            // We still need to coerce, but now we own the value
            let coerced = coerce_scalar_to_field_inner(raw_value, field, false)
                .map_err(|e| format!("Failed to coerce column '{}': {}", field.name(), e))?;

            columns[i].push(coerced);
        }
    }

    scalar_columns_to_record_batch(schema, columns)
}

/// Build a record batch from column-major scalars already coerced to `schema`.
pub fn scalar_columns_to_record_batch(
    schema: &SchemaRef,
    columns: Vec<Vec<ScalarValue>>,
) -> Result<RecordBatch, String> {
    let mut arrays: Vec<ArrayRef> = Vec::with_capacity(schema.fields().len());
    for (i, col_values) in columns.into_iter().enumerate() {
        let field = schema.field(i);
        let array = build_array_from_scalars(field, col_values)
            .map_err(|e| format!("Failed to build column '{}': {}", field.name(), e))?;
        let array = super::project::align_array_to_field(array, field)
            .map_err(|e| format!("Failed to align column '{}': {}", field.name(), e))?;
        arrays.push(array);
    }

    RecordBatch::try_new(schema.clone(), arrays)
        .map_err(|e| format!("Failed to build record batch: {}", e))
}

pub fn coerce_scalar_for_arrow_field(
    value: ScalarValue,
    field: &arrow::datatypes::Field,
) -> Result<ScalarValue, String> {
    coerce_scalar_to_field_inner(value, field, false).map_err(|error| error.to_string())
}

fn get_column_defaults(schema: &SchemaRef) -> Vec<Option<ScalarValue>> {
    schema
        .fields()
        .iter()
        .map(|field| {
            if let Some(default_json) =
                crate::conversions::schema_metadata::read_kalam_missing_default(field.as_ref())
            {
                let scalar = json_value_to_scalar(&default_json);
                if let Ok(coerced) = coerce_scalar_to_field_inner(scalar, field.as_ref(), false) {
                    return Some(coerced);
                }
            }
            if let Ok(Some(domain_default)) = materialize_domain_default_for_field(field.as_ref()) {
                return Some(domain_default);
            }
            if let Some(labels) =
                crate::conversions::schema_metadata::read_kalam_enum_labels(field.as_ref())
            {
                if let Some(first) = labels.first() {
                    return Some(ScalarValue::Utf8(Some(first.clone())));
                }
            }
            if field.is_nullable() {
                return None;
            }
            match field.data_type() {
                DataType::Boolean => Some(ScalarValue::Boolean(Some(false))),
                DataType::Int8 => Some(ScalarValue::Int8(Some(0))),
                DataType::Int16 => Some(ScalarValue::Int16(Some(0))),
                DataType::Int32 => Some(ScalarValue::Int32(Some(0))),
                DataType::Int64 => Some(ScalarValue::Int64(Some(0))),
                DataType::UInt8 => Some(ScalarValue::UInt8(Some(0))),
                DataType::UInt16 => Some(ScalarValue::UInt16(Some(0))),
                DataType::UInt32 => Some(ScalarValue::UInt32(Some(0))),
                DataType::UInt64 => Some(ScalarValue::UInt64(Some(0))),
                DataType::Float32 => Some(ScalarValue::Float32(Some(0.0))),
                DataType::Float64 => Some(ScalarValue::Float64(Some(0.0))),
                DataType::Utf8 => Some(ScalarValue::Utf8(Some("".to_string()))),
                DataType::LargeUtf8 => Some(ScalarValue::LargeUtf8(Some("".to_string()))),
                DataType::Timestamp(TimeUnit::Microsecond, tz_opt) => {
                    Some(ScalarValue::TimestampMicrosecond(Some(0), tz_opt.clone()))
                },
                DataType::Timestamp(TimeUnit::Millisecond, tz_opt) => {
                    Some(ScalarValue::TimestampMillisecond(Some(0), tz_opt.clone()))
                },
                DataType::Date32 => Some(ScalarValue::Date32(Some(0))),
                DataType::Date64 => Some(ScalarValue::Date64(Some(0))),
                _ => None,
            }
        })
        .collect()
}

fn typed_null_for_field(field: &Field) -> ScalarValue {
    match field.data_type() {
        DataType::FixedSizeList(child, _) if matches!(child.data_type(), DataType::Float32) => {
            ScalarValue::Null
        },
        _ => ScalarValue::try_from(field.data_type()).unwrap_or(ScalarValue::Null),
    }
}

fn get_typed_nulls(schema: &SchemaRef) -> Vec<ScalarValue> {
    schema
        .fields()
        .iter()
        .map(|field| typed_null_for_field(field.as_ref()))
        .collect()
}

fn create_empty_array(data_type: &DataType) -> ArrayRef {
    new_empty_array(data_type)
}

fn build_array_from_scalars(field: &Field, values: Vec<ScalarValue>) -> Result<ArrayRef, String> {
    if values.is_empty() {
        return Ok(create_empty_array(field.data_type()));
    }
    match field.data_type() {
        DataType::FixedSizeList(child, len) if matches!(child.data_type(), DataType::Float32) => {
            build_embedding_array(child.clone(), *len, values)
        },
        DataType::Struct(fields) if fields.is_empty() => {
            let len = values.len();
            let validity: Vec<bool> = values.iter().map(|v| !v.is_null()).collect();
            let nulls = if validity.iter().all(|valid| *valid) {
                None
            } else {
                Some(arrow::buffer::NullBuffer::from(validity))
            };
            let struct_arr =
                StructArray::try_new_with_length(fields.clone(), Vec::new(), nulls, len)
                    .map_err(|e| e.to_string())?;
            Ok(Arc::new(struct_arr) as ArrayRef)
        },
        _ => ScalarValue::iter_to_array(values).map_err(|e| format!("{}", e)),
    }
}

fn build_embedding_array(
    child: Arc<Field>,
    len: i32,
    values: Vec<ScalarValue>,
) -> Result<ArrayRef, String> {
    let dimensions = len as usize;
    let mut flat_values = Vec::with_capacity(values.len() * dimensions);
    let mut validity = Vec::with_capacity(values.len());

    for value in values {
        let maybe_vector = match value {
            ScalarValue::FixedSizeList(list) => fixed_size_list_scalar_to_vec(&list, dimensions),
            ScalarValue::List(list) => list_scalar_to_vec(&list, dimensions),
            ScalarValue::LargeList(list) => large_list_scalar_to_vec(&list, dimensions),
            other if other.is_null() => None,
            other => {
                return Err(format!(
                    "unsupported EMBEDDING scalar while building Arrow array: {:?}",
                    other.data_type()
                ));
            },
        };

        if let Some(vector) = maybe_vector {
            flat_values.extend(vector);
            validity.push(true);
        } else {
            flat_values.extend(std::iter::repeat_n(0.0_f32, dimensions));
            validity.push(false);
        }
    }

    let values_array: ArrayRef = Arc::new(Float32Array::from(flat_values));
    let nulls = if validity.iter().all(|is_valid| *is_valid) {
        None
    } else {
        Some(validity.into())
    };

    Ok(Arc::new(FixedSizeListArray::new(child, len, values_array, nulls)) as ArrayRef)
}

fn fixed_size_list_scalar_to_vec(list: &FixedSizeListArray, dimensions: usize) -> Option<Vec<f32>> {
    if list.is_empty() || list.is_null(0) {
        return None;
    }

    numeric_array_to_vec(list.value(0).as_ref(), dimensions)
}

fn list_scalar_to_vec(list: &ListArray, dimensions: usize) -> Option<Vec<f32>> {
    if list.is_empty() || list.is_null(0) {
        return None;
    }

    numeric_array_to_vec(list.value(0).as_ref(), dimensions)
}

fn large_list_scalar_to_vec(list: &LargeListArray, dimensions: usize) -> Option<Vec<f32>> {
    if list.is_empty() || list.is_null(0) {
        return None;
    }

    numeric_array_to_vec(list.value(0).as_ref(), dimensions)
}

fn numeric_array_to_vec(array: &dyn Array, dimensions: usize) -> Option<Vec<f32>> {
    let float_array = array.as_any().downcast_ref::<Float32Array>()?;
    if float_array.len() != dimensions || float_array.null_count() == float_array.len() {
        return None;
    }

    Some(
        (0..float_array.len())
            .map(|idx| {
                if float_array.is_null(idx) {
                    0.0
                } else {
                    float_array.value(idx)
                }
            })
            .collect(),
    )
}

pub fn coerce_scalar_to_field(value: ScalarValue, field: &Field) -> Result<ScalarValue, String> {
    coerce_scalar_to_field_inner(value, field, true)
}

fn coerce_scalar_to_field_inner(
    value: ScalarValue,
    field: &Field,
    enforce_domain_constraints: bool,
) -> Result<ScalarValue, String> {
    if value.is_null() {
        let typed_null = typed_null_for_field(field);
        if enforce_domain_constraints {
            validate_domain_constraints_for_field(field, &typed_null)?;
        }
        return Ok(typed_null);
    }

    reject_character_maximum_length(&value, field)?;

    if enforce_domain_constraints {
        validate_enum_scalar(field, &value)?;
    } else if &value.data_type() == field.data_type() {
        return Ok(value);
    }

    let coerced = if let Some(file_scalar) = if enforce_domain_constraints {
        coerce_file_column_scalar(&value, field)?
    } else {
        None
    } {
        file_scalar
    } else if let Some(json_scalar) = if enforce_domain_constraints {
        coerce_json_column_scalar(&value, field)?
    } else {
        None
    } {
        json_scalar
    } else if let Some(decimal_scalar) = coerce_decimal_scalar(&value, field)? {
        decimal_scalar
    } else if let Some(nested) =
        coerce_nested_json_scalar(value.clone(), field, enforce_domain_constraints)?
    {
        nested
    } else if &value.data_type() == field.data_type() {
        value
    } else if matches!(field.data_type(), DataType::FixedSizeBinary(16)) {
        let value_type = value.data_type();
        coerce_uuid_scalar(value, field)?.ok_or_else(|| {
            format!("Unable to coerce value {value_type:?} to UUID for column '{}'", field.name())
        })?
    } else if let Some(embedding) = coerce_embedding_scalar(value.clone(), field)? {
        embedding
    } else if matches!(field.data_type(), DataType::Int16 | DataType::Int32 | DataType::Int64)
        && matches!(
            &value,
            ScalarValue::Utf8(Some(_))
                | ScalarValue::LargeUtf8(Some(_))
                | ScalarValue::Utf8View(Some(_))
        )
    {
        let raw = scalar_as_str(&value).unwrap_or_default().trim();
        let parsed = raw.parse::<i64>().map_err(|_| {
            format!(
                "invalid input syntax for type {:?}: \"{raw}\" in column '{}'",
                field.data_type(),
                field.name()
            )
        })?;
        match field.data_type() {
            DataType::Int16 => {
                let v = i16::try_from(parsed).map_err(|_| {
                    format!(
                        "value \"{raw}\" is out of range for type smallint in column '{}'",
                        field.name()
                    )
                })?;
                ScalarValue::Int16(Some(v))
            },
            DataType::Int32 => {
                let v = i32::try_from(parsed).map_err(|_| {
                    format!(
                        "value \"{raw}\" is out of range for type integer in column '{}'",
                        field.name()
                    )
                })?;
                ScalarValue::Int32(Some(v))
            },
            DataType::Int64 => ScalarValue::Int64(Some(parsed)),
            _ => unreachable!(),
        }
    } else if matches!(field.data_type(), DataType::Float32 | DataType::Float64)
        && matches!(
            &value,
            ScalarValue::Utf8(Some(_))
                | ScalarValue::LargeUtf8(Some(_))
                | ScalarValue::Utf8View(Some(_))
        )
    {
        let raw = scalar_as_str(&value).unwrap_or_default().trim();
        let parsed = raw.parse::<f64>().map_err(|_| {
            format!(
                "invalid input syntax for type {:?}: \"{raw}\" in column '{}'",
                field.data_type(),
                field.name()
            )
        })?;
        match field.data_type() {
            DataType::Float32 => ScalarValue::Float32(Some(parsed as f32)),
            DataType::Float64 => ScalarValue::Float64(Some(parsed)),
            _ => unreachable!(),
        }
    } else if matches!(field.data_type(), DataType::Boolean)
        && matches!(
            &value,
            ScalarValue::Utf8(Some(_))
                | ScalarValue::LargeUtf8(Some(_))
                | ScalarValue::Utf8View(Some(_))
        )
    {
        let raw = scalar_as_str(&value).unwrap_or_default().trim();
        let b = match raw.to_ascii_lowercase().as_str() {
            "true" | "t" | "1" | "yes" | "on" => true,
            "false" | "f" | "0" | "no" | "off" => false,
            _ => {
                return Err(format!(
                    "invalid input syntax for type boolean: \"{raw}\" in column '{}'",
                    field.name()
                ))
            },
        };
        ScalarValue::Boolean(Some(b))
    } else {
        let casted = value.cast_to(field.data_type()).map_err(|e| {
            format!(
                "Unable to cast value {:?} to {:?} for column '{}': {}",
                value.data_type(),
                field.data_type(),
                field.name(),
                e
            )
        })?;
        if casted.is_null() && !value.is_null() {
            return Err(format!(
                "invalid input syntax or out of range when casting {:?} to {:?} for column '{}'",
                value.data_type(),
                field.data_type(),
                field.name()
            ));
        }
        casted
    };

    if enforce_domain_constraints {
        validate_domain_constraints_for_field(field, &coerced)?;
    }

    reject_character_maximum_length(&coerced, field)?;
    Ok(coerced)
}

fn coerce_file_column_scalar(
    value: &ScalarValue,
    field: &Field,
) -> Result<Option<ScalarValue>, String> {
    if !matches!(field.data_type(), DataType::Utf8 | DataType::LargeUtf8 | DataType::Utf8View)
        || read_kalam_data_type_metadata(field) != Some(KalamDataType::File)
    {
        return Ok(None);
    }
    let raw_text = match value {
        ScalarValue::Utf8(Some(text))
        | ScalarValue::LargeUtf8(Some(text))
        | ScalarValue::Utf8View(Some(text)) => text.as_str(),
        other => {
            return Err(format!(
                "column '{}' is of type pg_catalog.file and cannot be constructed from {:?}; use \
                 file_put()",
                field.name(),
                other.data_type()
            ));
        },
    };
    let parsed = FileRef::try_from_json(raw_text).map_err(|error| {
        format!(
            "invalid or forged pg_catalog.file reference in column '{}': {error}",
            field.name()
        )
    })?;
    if !parsed.is_well_formed() {
        return Err(format!(
            "invalid or forged pg_catalog.file reference in column '{}': required metadata fields \
             are missing",
            field.name()
        ));
    }
    Ok(Some(ScalarValue::Utf8(Some(raw_text.to_string()))))
}

fn coerce_json_column_scalar(
    value: &ScalarValue,
    field: &Field,
) -> Result<Option<ScalarValue>, String> {
    if !matches!(field.data_type(), DataType::Utf8 | DataType::LargeUtf8 | DataType::Utf8View)
        || read_kalam_data_type_metadata(field) != Some(KalamDataType::Json)
    {
        return Ok(None);
    }
    let raw_text = match value {
        ScalarValue::Utf8(Some(text))
        | ScalarValue::LargeUtf8(Some(text))
        | ScalarValue::Utf8View(Some(text)) => text.as_str(),
        other => {
            return Err(format!(
                "column '{}' is of type json but expression is of type {:?}",
                field.name(),
                other.data_type()
            ));
        },
    };
    let type_label = read_logical_type_modifier(field).unwrap_or("json");
    let parsed: JsonValue = serde_json::from_str(raw_text).map_err(|error| {
        format!(
            "invalid input syntax for type {type_label} in column '{}': {error}",
            field.name()
        )
    })?;
    if type_label == "jsonb" {
        let normalized = serde_json::to_string(&parsed).map_err(|error| error.to_string())?;
        Ok(Some(ScalarValue::Utf8(Some(normalized))))
    } else {
        Ok(Some(ScalarValue::Utf8(Some(raw_text.to_string()))))
    }
}

fn coerce_decimal_scalar(
    value: &ScalarValue,
    field: &Field,
) -> Result<Option<ScalarValue>, String> {
    let DataType::Decimal128(precision, scale) = field.data_type() else {
        return Ok(None);
    };
    let text = match value {
        ScalarValue::Decimal128(Some(v), _src_p, src_s) => {
            format_decimal_value(*v, (*src_s).max(0) as u8)
        },
        ScalarValue::Utf8(Some(s))
        | ScalarValue::LargeUtf8(Some(s))
        | ScalarValue::Utf8View(Some(s)) => s.clone(),
        ScalarValue::Int8(Some(n)) => n.to_string(),
        ScalarValue::Int16(Some(n)) => n.to_string(),
        ScalarValue::Int32(Some(n)) => n.to_string(),
        ScalarValue::Int64(Some(n)) => n.to_string(),
        ScalarValue::UInt8(Some(n)) => n.to_string(),
        ScalarValue::UInt16(Some(n)) => n.to_string(),
        ScalarValue::UInt32(Some(n)) => n.to_string(),
        ScalarValue::UInt64(Some(n)) => n.to_string(),
        ScalarValue::Float32(Some(f)) => {
            if !f.is_finite() {
                return Err(format!(
                    "numeric value '{f}' is not supported by bounded NUMERIC({precision}, \
                     {scale}) in column '{}'",
                    field.name()
                ));
            }
            f.to_string()
        },
        ScalarValue::Float64(Some(f)) => {
            if !f.is_finite() {
                return Err(format!(
                    "numeric value '{f}' is not supported by bounded NUMERIC({precision}, \
                     {scale}) in column '{}'",
                    field.name()
                ));
            }
            f.to_string()
        },
        _ => return Ok(None),
    };
    let parsed = parse_decimal_value_bounded(&text, *precision, *scale)
        .map_err(|error| format!("column '{}': {error}", field.name()))?;
    Ok(Some(ScalarValue::Decimal128(Some(parsed), *precision, *scale)))
}

fn coerce_embedding_scalar(
    value: ScalarValue,
    field: &Field,
) -> Result<Option<ScalarValue>, String> {
    let DataType::FixedSizeList(child, len) = field.data_type() else {
        return Ok(None);
    };

    if !matches!(child.data_type(), DataType::Float32) {
        return Ok(None);
    }

    let raw = match value {
        ScalarValue::Utf8(Some(s)) | ScalarValue::LargeUtf8(Some(s)) => s,
        _ => return Ok(None),
    };

    let normalized = raw.trim();
    let normalized = normalized
        .strip_prefix('\'')
        .and_then(|s| s.strip_suffix('\''))
        .unwrap_or(normalized);

    let parsed = match serde_json::from_str::<Vec<f32>>(normalized) {
        Ok(v) => v,
        Err(json_err) => parse_debug_embedding_repr(normalized).ok_or_else(|| {
            format!("Invalid embedding JSON for column '{}': {}", field.name(), json_err)
        })?,
    };

    if parsed.len() != (*len as usize) {
        return Err(format!(
            "Embedding dimension mismatch for column '{}': expected {}, got {}",
            field.name(),
            len,
            parsed.len()
        ));
    }

    let values = Float32Array::from(parsed);
    let list = FixedSizeListArray::new(child.clone(), *len, Arc::new(values), None);

    Ok(Some(ScalarValue::FixedSizeList(Arc::new(list))))
}

fn coerce_nested_json_scalar(
    value: ScalarValue,
    field: &Field,
    enforce_domain_constraints: bool,
) -> Result<Option<ScalarValue>, String> {
    match field.data_type() {
        DataType::Struct(_) | DataType::List(_) | DataType::LargeList(_) => {},
        _ => return Ok(None),
    }
    let json = match value {
        ScalarValue::Utf8(Some(text)) | ScalarValue::LargeUtf8(Some(text)) => {
            serde_json::from_str::<JsonValue>(&text).map_err(|error| {
                format!("invalid nested JSON for column '{}': {error}", field.name())
            })?
        },
        ScalarValue::List(list_arr) => {
            if !enforce_domain_constraints {
                let aligned = super::project::align_array_to_field(list_arr as ArrayRef, field)?;
                let rebuilt = aligned
                    .as_any()
                    .downcast_ref::<ListArray>()
                    .ok_or_else(|| "expected ListArray after alignment".to_string())?
                    .clone();
                return Ok(Some(ScalarValue::List(Arc::new(rebuilt))));
            }
            let DataType::List(item_field) = field.data_type() else {
                return Ok(Some(ScalarValue::List(list_arr)));
            };
            let values_arr = list_arr.value(0);
            let mut coerced_items = Vec::with_capacity(values_arr.len());
            for idx in 0..values_arr.len() {
                let elem_scalar =
                    arrow_value_to_scalar(&values_arr, idx).map_err(|e| e.to_string())?;
                if elem_scalar.is_null() && !item_field.is_nullable() {
                    return Err(format!(
                        "array element in column '{}' cannot be null",
                        field.name()
                    ));
                }
                coerced_items.push(coerce_scalar_to_field(elem_scalar, item_field)?);
            }
            let child_array = if coerced_items.is_empty() {
                new_empty_array(item_field.data_type())
            } else {
                build_array_from_scalars(item_field.as_ref(), coerced_items)?
            };
            let offsets = OffsetBuffer::from_lengths([child_array.len()]);
            return Ok(Some(ScalarValue::List(Arc::new(ListArray::new(
                item_field.clone(),
                offsets,
                child_array,
                None,
            )))));
        },
        ScalarValue::Struct(struct_arr) => {
            if !enforce_domain_constraints {
                let aligned = super::project::align_array_to_field(struct_arr as ArrayRef, field)?;
                let rebuilt = aligned
                    .as_any()
                    .downcast_ref::<StructArray>()
                    .ok_or_else(|| "expected StructArray after alignment".to_string())?
                    .clone();
                return Ok(Some(ScalarValue::Struct(Arc::new(rebuilt))));
            }
            let DataType::Struct(fields) = field.data_type() else {
                return Ok(Some(ScalarValue::Struct(struct_arr)));
            };
            if struct_arr.num_columns() != fields.len() {
                return Err(format!(
                    "composite value for column '{}' has {} fields, expected {}",
                    field.name(),
                    struct_arr.num_columns(),
                    fields.len()
                ));
            }
            let mut child_arrays = Vec::with_capacity(fields.len());
            for (idx, child_field) in fields.iter().enumerate() {
                let col = struct_arr.column(idx);
                let child_scalar = arrow_value_to_scalar(col, 0).map_err(|e| e.to_string())?;
                if child_scalar.is_null() && !child_field.is_nullable() {
                    return Err(format!(
                        "field '{}' of composite column '{}' cannot be null",
                        child_field.name(),
                        field.name()
                    ));
                }
                let coerced = coerce_scalar_to_field(child_scalar, child_field)?;
                child_arrays.push(coerced.to_array().map_err(|e| e.to_string())?);
            }
            let rebuilt = StructArray::try_new_with_length(
                fields.clone(),
                child_arrays,
                struct_arr.nulls().cloned(),
                1,
            )
            .map_err(|e| e.to_string())?;
            return Ok(Some(ScalarValue::Struct(Arc::new(rebuilt))));
        },
        ScalarValue::LargeList(_) => {
            return Ok(Some(value));
        },
        _ => return Ok(None),
    };
    Ok(Some(json_value_to_scalar_for_field(&json, field)?))
}

/// Convert JSON into a ScalarValue matching an Arrow field, including named structs and lists.
pub fn json_value_to_scalar_for_field(
    value: &JsonValue,
    field: &Field,
) -> Result<ScalarValue, String> {
    if value.is_null() {
        return Ok(typed_null_for_field(field));
    }
    if matches!(field.data_type(), DataType::Utf8 | DataType::LargeUtf8 | DataType::Utf8View)
        && read_kalam_data_type_metadata(field) == Some(KalamDataType::Json)
    {
        let raw = match value {
            JsonValue::String(s) => s.clone(),
            other => serde_json::to_string(other).map_err(|e| e.to_string())?,
        };
        return coerce_scalar_to_field(ScalarValue::Utf8(Some(raw)), field);
    }
    let scalar = json_value_to_scalar_for_data_type(value, field.data_type(), field.is_nullable())?;
    coerce_scalar_to_field(scalar, field)
}

fn json_value_to_scalar_for_data_type(
    value: &JsonValue,
    data_type: &DataType,
    _nullable: bool,
) -> Result<ScalarValue, String> {
    if value.is_null() {
        return ScalarValue::try_from(data_type).map_err(|error| error.to_string());
    }
    match data_type {
        DataType::Struct(fields) => {
            let object = value
                .as_object()
                .ok_or_else(|| "named type values must be JSON objects".to_string())?;
            let mut arrays = Vec::with_capacity(fields.len());
            for child in fields {
                let child_json = object.get(child.name()).unwrap_or(&JsonValue::Null);
                if child_json.is_null() && !child.is_nullable() {
                    return Err(format!(
                        "field '{}' of composite value cannot be null",
                        child.name()
                    ));
                }
                let scalar = json_value_to_scalar_for_field(child_json, child)?;
                arrays.push(scalar.to_array().map_err(|error| error.to_string())?);
            }
            let struct_array = StructArray::try_new_with_length(fields.clone(), arrays, None, 1)
                .map_err(|error| error.to_string())?;
            Ok(ScalarValue::Struct(Arc::new(struct_array)))
        },
        DataType::List(item) | DataType::LargeList(item) => {
            let array = value
                .as_array()
                .ok_or_else(|| "array columns must be JSON arrays".to_string())?;
            if array.len() > 1_048_576 {
                return Err("decoded list exceeds max length".to_string());
            }
            let mut values = Vec::with_capacity(array.len());
            for element in array {
                if element.is_null() && !item.is_nullable() {
                    return Err("array element cannot be null".to_string());
                }
                values.push(json_value_to_scalar_for_field(element, item)?);
            }
            let child_array = if values.is_empty() {
                new_empty_array(item.data_type())
            } else {
                build_array_from_scalars(item.as_ref(), values)?
            };
            let offsets = OffsetBuffer::from_lengths([child_array.len()]);
            Ok(ScalarValue::List(Arc::new(ListArray::new(
                item.clone(),
                offsets,
                child_array,
                None,
            ))))
        },
        _ => Ok(json_value_to_scalar(value)),
    }
}

fn validate_enum_scalar(field: &Field, value: &ScalarValue) -> Result<(), String> {
    let label = match value {
        ScalarValue::Utf8(Some(text))
        | ScalarValue::LargeUtf8(Some(text))
        | ScalarValue::Utf8View(Some(text)) => text.as_str(),
        _ => return Ok(()),
    };
    validate_enum_label(field, label)
}

/// Validate all domain `NOT NULL` and `CHECK` constraints attached to `field`.
pub fn validate_domain_constraints_for_field(
    field: &Field,
    value: &ScalarValue,
) -> Result<(), String> {
    let Some(specs) = read_kalam_domain_constraints(field) else {
        return Ok(());
    };
    validate_domain_specs(&specs, value)
}

/// Validate an explicit slice of [`DomainConstraintSpec`] against `value`.
pub fn validate_domain_specs(
    specs: &[DomainConstraintSpec],
    value: &ScalarValue,
) -> Result<(), String> {
    for spec in specs {
        if spec.not_null && value.is_null() {
            return Err(format!("domain {} does not allow null values", spec.domain_name));
        }
        for check in &spec.checks {
            if evaluate_domain_check_predicate(&check.expr, value)? == Some(false) {
                return Err(format!(
                    "value for domain {} violates check constraint \"{}\"",
                    spec.domain_name, check.name
                ));
            }
        }
    }
    Ok(())
}

/// Materialize the outermost domain default attached to `field`, if any.
pub fn materialize_domain_default_for_field(field: &Field) -> Result<Option<ScalarValue>, String> {
    let Some(specs) = read_kalam_domain_constraints(field) else {
        return Ok(None);
    };
    let Some(default_expr) = specs.iter().rev().find_map(|spec| spec.default_expr.as_deref())
    else {
        return Ok(None);
    };
    let scalar = eval_domain_default_expr(default_expr, field)?;
    coerce_scalar_to_field(scalar, field).map(Some)
}

fn eval_domain_default_expr(raw: &str, field: &Field) -> Result<ScalarValue, String> {
    let trimmed = strip_trailing_type_cast(raw.trim());
    let trimmed = strip_balanced_parens(trimmed);
    if trimmed.eq_ignore_ascii_case("NULL") {
        return Ok(ScalarValue::Null);
    }
    if let DataType::Struct(children) = field.data_type() {
        let tuple_inner = if let Some(rest) = strip_keyword_prefix(trimmed, "ROW") {
            let rest = rest.trim();
            if rest.starts_with('(') && rest.ends_with(')') {
                Some(&rest[1..rest.len() - 1])
            } else {
                None
            }
        } else if raw.trim().starts_with('(') && raw.trim().ends_with(')') {
            let r = strip_trailing_type_cast(raw.trim());
            Some(&r[1..r.len() - 1])
        } else {
            None
        };
        if let Some(inner) = tuple_inner {
            let parts = split_top_level_commas(inner);
            if parts.len() != children.len() {
                return Err(format!(
                    "domain default composite value has {} fields, expected {}",
                    parts.len(),
                    children.len()
                ));
            }
            let mut child_arrays = Vec::with_capacity(children.len());
            for (part, child_field) in parts.iter().zip(children.iter()) {
                let child_scalar = eval_domain_default_expr(part, child_field.as_ref())?;
                let coerced = coerce_scalar_to_field(child_scalar, child_field.as_ref())?;
                child_arrays.push(coerced.to_array().map_err(|e| e.to_string())?);
            }
            let sa = StructArray::try_new_with_length(children.clone(), child_arrays, None, 1)
                .map_err(|e| e.to_string())?;
            return Ok(ScalarValue::Struct(Arc::new(sa)));
        }
    }
    if let DataType::List(item_field) = field.data_type() {
        if let Some(rest) = strip_keyword_prefix(trimmed, "ARRAY") {
            let rest = rest.trim();
            if rest.starts_with('[') && rest.ends_with(']') {
                let inner = &rest[1..rest.len() - 1];
                let parts = split_top_level_commas(inner);
                let mut items = Vec::with_capacity(parts.len());
                for part in parts {
                    let elem = eval_domain_default_expr(part, item_field.as_ref())?;
                    items.push(coerce_scalar_to_field(elem, item_field.as_ref())?);
                }
                let child_array = if items.is_empty() {
                    new_empty_array(item_field.data_type())
                } else {
                    build_array_from_scalars(item_field.as_ref(), items)?
                };
                let offsets = OffsetBuffer::from_lengths([child_array.len()]);
                return Ok(ScalarValue::List(Arc::new(ListArray::new(
                    item_field.clone(),
                    offsets,
                    child_array,
                    None,
                ))));
            }
        }
    }
    eval_domain_operand(trimmed, &ScalarValue::Null)
}

/// Evaluate a PostgreSQL domain `CHECK (expr)` predicate with three-valued SQL logic:
/// `Some(true)` = TRUE, `Some(false)` = FALSE (violates constraint), `None` = UNKNOWN (passes).
pub fn evaluate_domain_check_predicate(
    expr: &str,
    value: &ScalarValue,
) -> Result<Option<bool>, String> {
    let trimmed = strip_balanced_parens(expr.trim());
    if trimmed.is_empty() {
        return Ok(Some(true));
    }

    let or_parts = split_top_level_keyword(trimmed, "OR");
    if or_parts.len() > 1 {
        let mut saw_unknown = false;
        for part in or_parts {
            match evaluate_domain_check_predicate(part, value)? {
                Some(true) => return Ok(Some(true)),
                Some(false) => {},
                None => saw_unknown = true,
            }
        }
        return Ok(if saw_unknown { None } else { Some(false) });
    }

    let and_parts = split_top_level_keyword(trimmed, "AND");
    if and_parts.len() > 1 {
        let mut saw_unknown = false;
        for part in and_parts {
            match evaluate_domain_check_predicate(part, value)? {
                Some(false) => return Ok(Some(false)),
                Some(true) => {},
                None => saw_unknown = true,
            }
        }
        return Ok(if saw_unknown { None } else { Some(true) });
    }

    if let Some(rest) = strip_keyword_prefix(trimmed, "NOT") {
        return Ok(evaluate_domain_check_predicate(rest, value)?.map(|b| !b));
    }

    if let Some(lhs_str) = strip_keyword_suffix(trimmed, "IS NOT NULL") {
        let lhs = eval_domain_operand(lhs_str, value)?;
        return Ok(Some(!lhs.is_null()));
    }
    if let Some(lhs_str) = strip_keyword_suffix(trimmed, "IS NULL") {
        let lhs = eval_domain_operand(lhs_str, value)?;
        return Ok(Some(lhs.is_null()));
    }
    if let Some(lhs_str) = strip_keyword_suffix(trimmed, "IS TRUE") {
        let lhs = eval_domain_operand(lhs_str, value)?;
        return Ok(Some(matches!(lhs, ScalarValue::Boolean(Some(true)))));
    }
    if let Some(lhs_str) = strip_keyword_suffix(trimmed, "IS FALSE") {
        let lhs = eval_domain_operand(lhs_str, value)?;
        return Ok(Some(matches!(lhs, ScalarValue::Boolean(Some(false)))));
    }

    if let Some((lhs_str, list_str, negated)) = split_in_list(trimmed) {
        let lhs = eval_domain_operand(lhs_str, value)?;
        if lhs.is_null() {
            return Ok(None);
        }
        let items = split_top_level_commas(list_str);
        let mut saw_null = false;
        let mut matched = false;
        for item_str in items {
            let item = eval_domain_operand(item_str, value)?;
            if item.is_null() {
                saw_null = true;
            } else if compare_domain_scalars(&lhs, &item)? == Some(std::cmp::Ordering::Equal) {
                matched = true;
                break;
            }
        }
        return Ok(if matched {
            Some(!negated)
        } else if saw_null {
            None
        } else {
            Some(negated)
        });
    }

    if let Some((lhs_str, rhs_str, case_insensitive, negated)) = split_like_op(trimmed) {
        let lhs = eval_domain_operand(lhs_str, value)?;
        let rhs = eval_domain_operand(rhs_str, value)?;
        if lhs.is_null() || rhs.is_null() {
            return Ok(None);
        }
        let Some(text) = scalar_as_str(&lhs) else {
            return Err("LIKE operand must be text".to_string());
        };
        let Some(pattern) = scalar_as_str(&rhs) else {
            return Err("LIKE pattern must be text".to_string());
        };
        let is_match = sql_like_match(text, pattern, case_insensitive);
        return Ok(Some(if negated { !is_match } else { is_match }));
    }

    if let Some((lhs_str, op, rhs_str)) = split_top_level_comparison(trimmed) {
        let lhs = eval_domain_operand(lhs_str, value)?;
        let rhs = eval_domain_operand(rhs_str, value)?;
        if lhs.is_null() || rhs.is_null() {
            return Ok(None);
        }
        let Some(ord) = compare_domain_scalars(&lhs, &rhs)? else {
            return Ok(None);
        };
        let result = match op {
            "=" => ord == std::cmp::Ordering::Equal,
            "<>" | "!=" => ord != std::cmp::Ordering::Equal,
            "<" => ord == std::cmp::Ordering::Less,
            "<=" => ord != std::cmp::Ordering::Greater,
            ">" => ord == std::cmp::Ordering::Greater,
            ">=" => ord != std::cmp::Ordering::Less,
            _ => return Err(format!("unsupported domain check operator '{op}'")),
        };
        return Ok(Some(result));
    }

    let val = eval_domain_operand(trimmed, value)?;
    match val {
        ScalarValue::Boolean(Some(b)) => Ok(Some(b)),
        v if v.is_null() => Ok(None),
        other => Err(format!(
            "domain CHECK expression '{trimmed}' did not evaluate to boolean (got {:?})",
            other.data_type()
        )),
    }
}

fn eval_domain_operand(raw: &str, value: &ScalarValue) -> Result<ScalarValue, String> {
    let without_cast = strip_trailing_type_cast(raw.trim());
    let trimmed = strip_balanced_parens(without_cast);

    if let Some((lhs_str, op, rhs_str)) = split_top_level_arith(trimmed) {
        let lhs = eval_domain_operand(lhs_str, value)?;
        let rhs = eval_domain_operand(rhs_str, value)?;
        return eval_domain_arith(&lhs, op, &rhs);
    }

    if trimmed.eq_ignore_ascii_case("VALUE") {
        return Ok(value.clone());
    }
    if trimmed.eq_ignore_ascii_case("NULL") {
        return Ok(ScalarValue::Null);
    }
    if trimmed.eq_ignore_ascii_case("TRUE") {
        return Ok(ScalarValue::Boolean(Some(true)));
    }
    if trimmed.eq_ignore_ascii_case("FALSE") {
        return Ok(ScalarValue::Boolean(Some(false)));
    }

    if let Some(field_path) = parse_value_field_access(trimmed) {
        let mut current = value.clone();
        for segment in field_path {
            if current.is_null() {
                return Ok(ScalarValue::Null);
            }
            let ScalarValue::Struct(sa) = &current else {
                return Err(format!(
                    "cannot access field '{segment}' on non-composite domain value"
                ));
            };
            let Some((idx, _)) = sa
                .fields()
                .iter()
                .enumerate()
                .find(|(_, f)| f.name().eq_ignore_ascii_case(segment))
            else {
                return Err(format!("composite domain value has no field '{segment}'"));
            };
            current = arrow_value_to_scalar(sa.column(idx), 0).map_err(|e| e.to_string())?;
        }
        return Ok(current);
    }

    if trimmed.starts_with('\'') && trimmed.ends_with('\'') && trimmed.len() >= 2 {
        let inner = &trimmed[1..trimmed.len() - 1];
        return Ok(ScalarValue::Utf8(Some(inner.replace("''", "'"))));
    }

    if let Some((fn_name, args_str)) = split_function_call(trimmed) {
        let args = split_top_level_commas(args_str);
        let lower_fn = fn_name.to_ascii_lowercase();
        match lower_fn.as_str() {
            "length" | "char_length" | "character_length" => {
                if args.len() != 1 {
                    return Err(format!("{fn_name}() requires 1 argument"));
                }
                let arg = eval_domain_operand(args[0], value)?;
                if arg.is_null() {
                    return Ok(ScalarValue::Null);
                }
                let Some(s) = scalar_as_str(&arg) else {
                    return Err(format!("{fn_name}() requires text argument"));
                };
                return Ok(ScalarValue::Int64(Some(s.chars().count() as i64)));
            },
            "octet_length" => {
                if args.len() != 1 {
                    return Err("octet_length() requires 1 argument".to_string());
                }
                let arg = eval_domain_operand(args[0], value)?;
                if arg.is_null() {
                    return Ok(ScalarValue::Null);
                }
                let Some(s) = scalar_as_str(&arg) else {
                    return Err("octet_length() requires text argument".to_string());
                };
                return Ok(ScalarValue::Int64(Some(s.len() as i64)));
            },
            "lower" => {
                if args.len() != 1 {
                    return Err("lower() requires 1 argument".to_string());
                }
                let arg = eval_domain_operand(args[0], value)?;
                if arg.is_null() {
                    return Ok(ScalarValue::Null);
                }
                let Some(s) = scalar_as_str(&arg) else {
                    return Err("lower() requires text argument".to_string());
                };
                return Ok(ScalarValue::Utf8(Some(s.to_lowercase())));
            },
            "upper" => {
                if args.len() != 1 {
                    return Err("upper() requires 1 argument".to_string());
                }
                let arg = eval_domain_operand(args[0], value)?;
                if arg.is_null() {
                    return Ok(ScalarValue::Null);
                }
                let Some(s) = scalar_as_str(&arg) else {
                    return Err("upper() requires text argument".to_string());
                };
                return Ok(ScalarValue::Utf8(Some(s.to_uppercase())));
            },
            "trim" | "btrim" => {
                if args.len() != 1 {
                    return Err(format!("{fn_name}() requires 1 argument"));
                }
                let arg = eval_domain_operand(args[0], value)?;
                if arg.is_null() {
                    return Ok(ScalarValue::Null);
                }
                let Some(s) = scalar_as_str(&arg) else {
                    return Err(format!("{fn_name}() requires text argument"));
                };
                return Ok(ScalarValue::Utf8(Some(s.trim().to_string())));
            },
            "abs" => {
                if args.len() != 1 {
                    return Err("abs() requires 1 argument".to_string());
                }
                let arg = eval_domain_operand(args[0], value)?;
                if arg.is_null() {
                    return Ok(ScalarValue::Null);
                }
                if let Some(n) = scalar_as_f64(&arg) {
                    return Ok(ScalarValue::Float64(Some(n.abs())));
                }
                return Err("abs() requires numeric argument".to_string());
            },
            "cardinality" | "array_length" => {
                if args.is_empty() {
                    return Err(format!("{fn_name}() requires at least 1 argument"));
                }
                let arg = eval_domain_operand(args[0], value)?;
                if arg.is_null() {
                    return Ok(ScalarValue::Null);
                }
                if let ScalarValue::List(list) = &arg {
                    return Ok(ScalarValue::Int64(Some(list.value(0).len() as i64)));
                }
                return Err(format!("{fn_name}() requires array argument"));
            },
            "coalesce" => {
                for arg_expr in args {
                    let arg = eval_domain_operand(arg_expr, value)?;
                    if !arg.is_null() {
                        return Ok(arg);
                    }
                }
                return Ok(ScalarValue::Null);
            },
            _ => {},
        }
    }

    if let Ok(int_val) = trimmed.parse::<i64>() {
        return Ok(ScalarValue::Int64(Some(int_val)));
    }
    if let Ok(float_val) = trimmed.parse::<f64>() {
        return Ok(ScalarValue::Float64(Some(float_val)));
    }

    Err(format!("unsupported operand '{trimmed}' in domain expression"))
}

fn reject_character_maximum_length(value: &ScalarValue, field: &Field) -> Result<(), String> {
    let Some(max_chars) = read_character_maximum_length(field) else {
        return Ok(());
    };
    let Some(text) = scalar_as_str(value) else {
        return Ok(());
    };
    if text.chars().count() > usize::try_from(max_chars).unwrap_or(usize::MAX) {
        return Err(format!("value too long for type character varying({max_chars})"));
    }
    Ok(())
}

fn scalar_as_str(val: &ScalarValue) -> Option<&str> {
    match val {
        ScalarValue::Utf8(Some(s))
        | ScalarValue::LargeUtf8(Some(s))
        | ScalarValue::Utf8View(Some(s)) => Some(s.as_str()),
        _ => None,
    }
}

fn scalar_as_f64(val: &ScalarValue) -> Option<f64> {
    match val {
        ScalarValue::Int8(Some(v)) => Some(*v as f64),
        ScalarValue::Int16(Some(v)) => Some(*v as f64),
        ScalarValue::Int32(Some(v)) => Some(*v as f64),
        ScalarValue::Int64(Some(v)) => Some(*v as f64),
        ScalarValue::UInt8(Some(v)) => Some(*v as f64),
        ScalarValue::UInt16(Some(v)) => Some(*v as f64),
        ScalarValue::UInt32(Some(v)) => Some(*v as f64),
        ScalarValue::UInt64(Some(v)) => Some(*v as f64),
        ScalarValue::Float32(Some(v)) => Some(*v as f64),
        ScalarValue::Float64(Some(v)) => Some(*v),
        ScalarValue::Decimal128(Some(v), _, s) => {
            let scale = (*s).max(0) as u32;
            Some((*v as f64) / 10_f64.powi(scale as i32))
        },
        _ => None,
    }
}

fn compare_domain_scalars(
    lhs: &ScalarValue,
    rhs: &ScalarValue,
) -> Result<Option<std::cmp::Ordering>, String> {
    if let (Some(l), Some(r)) = (scalar_as_f64(lhs), scalar_as_f64(rhs)) {
        return Ok(l.partial_cmp(&r));
    }
    if let (Some(l), Some(r)) = (scalar_as_str(lhs), scalar_as_str(rhs)) {
        return Ok(Some(l.cmp(r)));
    }
    if let (ScalarValue::Boolean(Some(l)), ScalarValue::Boolean(Some(r))) = (lhs, rhs) {
        return Ok(Some(l.cmp(r)));
    }
    Err(format!(
        "cannot compare domain values of types {:?} and {:?}",
        lhs.data_type(),
        rhs.data_type()
    ))
}

fn eval_domain_arith(
    lhs: &ScalarValue,
    op: &str,
    rhs: &ScalarValue,
) -> Result<ScalarValue, String> {
    if lhs.is_null() || rhs.is_null() {
        return Ok(ScalarValue::Null);
    }
    if let (ScalarValue::Int64(Some(l)), ScalarValue::Int64(Some(r))) = (lhs, rhs) {
        return match op {
            "+" => Ok(ScalarValue::Int64(Some(l.saturating_add(*r)))),
            "-" => Ok(ScalarValue::Int64(Some(l.saturating_sub(*r)))),
            "*" => Ok(ScalarValue::Int64(Some(l.saturating_mul(*r)))),
            "/" => {
                if *r == 0 {
                    Err("division by zero in domain constraint".to_string())
                } else {
                    Ok(ScalarValue::Int64(Some(l / r)))
                }
            },
            "%" => {
                if *r == 0 {
                    Err("division by zero in domain constraint".to_string())
                } else {
                    Ok(ScalarValue::Int64(Some(l % r)))
                }
            },
            _ => Err(format!("unsupported arithmetic operator '{op}'")),
        };
    }
    if let (Some(l), Some(r)) = (scalar_as_f64(lhs), scalar_as_f64(rhs)) {
        return match op {
            "+" => Ok(ScalarValue::Float64(Some(l + r))),
            "-" => Ok(ScalarValue::Float64(Some(l - r))),
            "*" => Ok(ScalarValue::Float64(Some(l * r))),
            "/" => {
                if r == 0.0 {
                    Err("division by zero in domain constraint".to_string())
                } else {
                    Ok(ScalarValue::Float64(Some(l / r)))
                }
            },
            "%" => {
                if r == 0.0 {
                    Err("division by zero in domain constraint".to_string())
                } else {
                    Ok(ScalarValue::Float64(Some(l % r)))
                }
            },
            _ => Err(format!("unsupported arithmetic operator '{op}'")),
        };
    }
    Err("arithmetic in domain constraint requires numeric operands".to_string())
}

fn strip_balanced_parens(mut s: &str) -> &str {
    loop {
        let trimmed = s.trim();
        if !trimmed.starts_with('(') || !trimmed.ends_with(')') {
            return trimmed;
        }
        let mut depth = 0i32;
        let mut in_single = false;
        let mut in_double = false;
        let bytes = trimmed.as_bytes();
        let mut closes_at_end = true;
        for (i, &b) in bytes.iter().enumerate() {
            match b {
                b'\'' if !in_double => in_single = !in_single,
                b'"' if !in_single => in_double = !in_double,
                b'(' if !in_single && !in_double => depth += 1,
                b')' if !in_single && !in_double => {
                    depth -= 1;
                    if depth == 0 && i + 1 < bytes.len() {
                        closes_at_end = false;
                        break;
                    }
                },
                _ => {},
            }
        }
        if closes_at_end && depth == 0 {
            s = &trimmed[1..trimmed.len() - 1];
        } else {
            return trimmed;
        }
    }
}

fn strip_trailing_type_cast(s: &str) -> &str {
    let mut depth = 0i32;
    let mut in_single = false;
    let mut in_double = false;
    let bytes = s.as_bytes();
    let mut i = 0usize;
    let mut last_cast = None;
    while i + 1 < bytes.len() {
        match bytes[i] {
            b'\'' if !in_double => in_single = !in_single,
            b'"' if !in_single => in_double = !in_double,
            b'(' | b'[' if !in_single && !in_double => depth += 1,
            b')' | b']' if !in_single && !in_double => depth -= 1,
            b':' if !in_single && !in_double && depth == 0 && bytes[i + 1] == b':' => {
                last_cast = Some(i);
                i += 2;
                continue;
            },
            _ => {},
        }
        i += 1;
    }
    if let Some(pos) = last_cast {
        let suffix = s[pos + 2..].trim();
        if !suffix.is_empty()
            && suffix.chars().all(|c| {
                c.is_ascii_alphanumeric()
                    || matches!(c, '_' | '.' | '[' | ']' | ' ' | '(' | ')' | ',')
            })
        {
            return s[..pos].trim();
        }
    }
    s
}

fn split_top_level_keyword<'a>(s: &'a str, keyword: &str) -> Vec<&'a str> {
    let mut parts = Vec::new();
    let mut depth = 0i32;
    let mut in_single = false;
    let mut in_double = false;
    let bytes = s.as_bytes();
    let kw_len = keyword.len();
    let mut start = 0usize;
    let mut i = 0usize;
    while i < bytes.len() {
        match bytes[i] {
            b'\'' if !in_double => in_single = !in_single,
            b'"' if !in_single => in_double = !in_double,
            b'(' | b'[' if !in_single && !in_double => depth += 1,
            b')' | b']' if !in_single && !in_double => depth -= 1,
            _ if !in_single && !in_double && depth == 0 && i + kw_len <= bytes.len() => {
                let prev_ok = i == 0 || bytes[i - 1].is_ascii_whitespace() || bytes[i - 1] == b')';
                let next_ok = i + kw_len == bytes.len()
                    || bytes[i + kw_len].is_ascii_whitespace()
                    || bytes[i + kw_len] == b'(';
                if prev_ok && next_ok && s[i..i + kw_len].eq_ignore_ascii_case(keyword) {
                    parts.push(s[start..i].trim());
                    i += kw_len;
                    start = i;
                    continue;
                }
            },
            _ => {},
        }
        i += 1;
    }
    if parts.is_empty() {
        vec![s]
    } else {
        parts.push(s[start..].trim());
        parts
    }
}

fn strip_keyword_prefix<'a>(s: &'a str, keyword: &str) -> Option<&'a str> {
    let trimmed = s.trim();
    if trimmed.len() <= keyword.len() {
        return None;
    }
    if !trimmed[..keyword.len()].eq_ignore_ascii_case(keyword) {
        return None;
    }
    let next = trimmed.as_bytes()[keyword.len()];
    if next.is_ascii_whitespace() || next == b'(' || next == b'[' {
        Some(trimmed[keyword.len()..].trim())
    } else {
        None
    }
}

fn strip_keyword_suffix<'a>(s: &'a str, suffix_kw: &str) -> Option<&'a str> {
    let trimmed = s.trim();
    let parts: Vec<&str> = suffix_kw.split_whitespace().collect();
    let mut rem = trimmed;
    for kw in parts.iter().rev() {
        rem = rem.trim_end();
        if rem.len() <= kw.len() {
            return None;
        }
        let split_at = rem.len() - kw.len();
        if !rem.is_char_boundary(split_at) || !rem[split_at..].eq_ignore_ascii_case(kw) {
            return None;
        }
        let prev = rem.as_bytes()[split_at - 1];
        if !prev.is_ascii_whitespace() && prev != b')' {
            return None;
        }
        rem = &rem[..split_at];
    }
    Some(rem.trim())
}

fn split_top_level_commas(s: &str) -> Vec<&str> {
    let trimmed = s.trim();
    if trimmed.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::new();
    let mut depth = 0i32;
    let mut in_single = false;
    let mut in_double = false;
    let mut start = 0usize;
    for (i, b) in trimmed.bytes().enumerate() {
        match b {
            b'\'' if !in_double => in_single = !in_single,
            b'"' if !in_single => in_double = !in_double,
            b'(' | b'[' if !in_single && !in_double => depth += 1,
            b')' | b']' if !in_single && !in_double => depth -= 1,
            b',' if !in_single && !in_double && depth == 0 => {
                out.push(trimmed[start..i].trim());
                start = i + 1;
            },
            _ => {},
        }
    }
    out.push(trimmed[start..].trim());
    out
}

fn split_in_list(s: &str) -> Option<(&str, &str, bool)> {
    let trimmed = s.trim();
    if !trimmed.ends_with(')') {
        return None;
    }
    let mut depth = 0i32;
    let mut in_single = false;
    let mut in_double = false;
    let bytes = trimmed.as_bytes();
    let mut open_paren = None;
    for (i, &b) in bytes.iter().enumerate() {
        match b {
            b'\'' if !in_double => in_single = !in_single,
            b'"' if !in_single => in_double = !in_double,
            b'(' if !in_single && !in_double => {
                if depth == 0 {
                    open_paren = Some(i);
                }
                depth += 1;
            },
            b')' if !in_single && !in_double => {
                depth -= 1;
                if depth == 0 && i + 1 < bytes.len() {
                    open_paren = None;
                }
            },
            _ => {},
        }
    }
    let open = open_paren?;
    let before_paren = trimmed[..open].trim();
    let list_inner = &trimmed[open + 1..trimmed.len() - 1];
    if let Some(lhs) = strip_keyword_suffix(before_paren, "NOT IN") {
        return Some((lhs, list_inner, true));
    }
    if let Some(lhs) = strip_keyword_suffix(before_paren, "IN") {
        return Some((lhs, list_inner, false));
    }
    None
}

fn split_like_op(s: &str) -> Option<(&str, &str, bool, bool)> {
    for (kw, ci, neg) in [
        ("NOT ILIKE", true, true),
        ("NOT LIKE", false, true),
        ("ILIKE", true, false),
        ("LIKE", false, false),
    ] {
        let parts = split_top_level_keyword(s, kw);
        if parts.len() == 2 {
            return Some((parts[0], parts[1], ci, neg));
        }
    }
    None
}

fn sql_like_match(text: &str, pattern: &str, case_insensitive: bool) -> bool {
    let t: Vec<char> = if case_insensitive {
        text.to_lowercase().chars().collect()
    } else {
        text.chars().collect()
    };
    let p: Vec<char> = if case_insensitive {
        pattern.to_lowercase().chars().collect()
    } else {
        pattern.chars().collect()
    };
    let mut dp = vec![vec![false; p.len() + 1]; t.len() + 1];
    dp[0][0] = true;
    for j in 1..=p.len() {
        if p[j - 1] == '%' {
            dp[0][j] = dp[0][j - 1];
        }
    }
    for i in 1..=t.len() {
        for j in 1..=p.len() {
            if p[j - 1] == '%' {
                dp[i][j] = dp[i][j - 1] || dp[i - 1][j];
            } else if p[j - 1] == '_' || p[j - 1] == t[i - 1] {
                dp[i][j] = dp[i - 1][j - 1];
            }
        }
    }
    dp[t.len()][p.len()]
}

fn split_top_level_comparison(s: &str) -> Option<(&str, &str, &str)> {
    let mut depth = 0i32;
    let mut in_single = false;
    let mut in_double = false;
    let bytes = s.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        match bytes[i] {
            b'\'' if !in_double => in_single = !in_single,
            b'"' if !in_single => in_double = !in_double,
            b'(' | b'[' if !in_single && !in_double => depth += 1,
            b')' | b']' if !in_single && !in_double => depth -= 1,
            _ if !in_single && !in_double && depth == 0 => {
                if i + 1 < bytes.len() {
                    let two = &s[i..i + 2];
                    if matches!(two, "<>" | "!=" | "<=" | ">=") {
                        return Some((s[..i].trim(), two, s[i + 2..].trim()));
                    }
                }
                let one = &s[i..i + 1];
                if matches!(one, "=" | "<" | ">") {
                    return Some((s[..i].trim(), one, s[i + 1..].trim()));
                }
            },
            _ => {},
        }
        i += 1;
    }
    None
}

fn split_top_level_arith(s: &str) -> Option<(&str, &str, &str)> {
    let mut depth = 0i32;
    let mut in_single = false;
    let mut in_double = false;
    let bytes = s.as_bytes();
    let mut add_sub_pos = None;
    let mut mul_div_pos = None;
    for (i, &b) in bytes.iter().enumerate() {
        match b {
            b'\'' if !in_double => in_single = !in_single,
            b'"' if !in_single => in_double = !in_double,
            b'(' | b'[' if !in_single && !in_double => depth += 1,
            b')' | b']' if !in_single && !in_double => depth -= 1,
            b'+' | b'-' if !in_single && !in_double && depth == 0 && i > 0 => {
                let prev = s[..i].trim_end();
                if !prev.is_empty() {
                    add_sub_pos = Some(i);
                }
            },
            b'*' | b'/' | b'%' if !in_single && !in_double && depth == 0 && i > 0 => {
                mul_div_pos = Some(i);
            },
            _ => {},
        }
    }
    let pos = add_sub_pos.or(mul_div_pos)?;
    Some((s[..pos].trim(), &s[pos..pos + 1], s[pos + 1..].trim()))
}

fn parse_value_field_access(s: &str) -> Option<Vec<&str>> {
    let trimmed = s.trim();
    let rest = if let Some(after) = trimmed.strip_prefix('(') {
        let close = after.find(')')?;
        let inner = after[..close].trim();
        if !inner.eq_ignore_ascii_case("VALUE") {
            return None;
        }
        after[close + 1..].trim()
    } else if trimmed.len() > 6 && trimmed[..5].eq_ignore_ascii_case("VALUE") {
        trimmed[5..].trim()
    } else {
        return None;
    };
    let rest = rest.strip_prefix('.')?;
    let segments: Vec<&str> = rest
        .split('.')
        .map(|seg| seg.trim().trim_matches('"'))
        .filter(|seg| !seg.is_empty())
        .collect();
    if segments.is_empty() {
        None
    } else {
        Some(segments)
    }
}

fn split_function_call(s: &str) -> Option<(&str, &str)> {
    let trimmed = s.trim();
    if !trimmed.ends_with(')') {
        return None;
    }
    let open = trimmed.find('(')?;
    let fn_name = trimmed[..open].trim();
    if fn_name.is_empty() || !fn_name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return None;
    }
    let mut depth = 0i32;
    let mut in_single = false;
    for (i, b) in trimmed[open..].bytes().enumerate() {
        match b {
            b'\'' => in_single = !in_single,
            b'(' if !in_single => depth += 1,
            b')' if !in_single => {
                depth -= 1;
                if depth == 0 && open + i + 1 < trimmed.len() {
                    return None;
                }
            },
            _ => {},
        }
    }
    Some((fn_name, &trimmed[open + 1..trimmed.len() - 1]))
}

fn parse_debug_embedding_repr(raw: &str) -> Option<Vec<f32>> {
    let mut values = Vec::new();
    for line in raw.lines() {
        let token = line.trim().trim_end_matches(',');
        if token.is_empty()
            || token.starts_with('[')
            || token.starts_with(']')
            || token.contains("Array")
            || token.contains('<')
            || token.contains('>')
        {
            continue;
        }
        if let Ok(parsed) = token.parse::<f32>() {
            values.push(parsed);
        }
    }
    if values.is_empty() {
        None
    } else {
        Some(values)
    }
}

fn coerce_uuid_scalar(value: ScalarValue, field: &Field) -> Result<Option<ScalarValue>, String> {
    match value {
        ScalarValue::Utf8(Some(raw))
        | ScalarValue::LargeUtf8(Some(raw))
        | ScalarValue::Utf8View(Some(raw)) => {
            let uuid = Uuid::parse_str(&raw).map_err(|e| {
                format!("Invalid UUID literal '{}' for column '{}': {}", raw, field.name(), e)
            })?;
            Ok(Some(ScalarValue::FixedSizeBinary(16, Some(uuid.as_bytes().to_vec()))))
        },
        ScalarValue::Binary(Some(bytes))
        | ScalarValue::LargeBinary(Some(bytes))
        | ScalarValue::BinaryView(Some(bytes)) => uuid_bytes_to_scalar(bytes, field).map(Some),
        ScalarValue::FixedSizeBinary(size, Some(bytes)) => {
            if size != 16 {
                return Err(format!(
                    "UUID fixed binary literal must be 16 bytes for column '{}', got size {}",
                    field.name(),
                    size
                ));
            }
            uuid_bytes_to_scalar(bytes, field).map(Some)
        },
        _ => Ok(None),
    }
}

fn uuid_bytes_to_scalar(bytes: Vec<u8>, field: &Field) -> Result<ScalarValue, String> {
    if bytes.len() != 16 {
        return Err(format!(
            "UUID binary literal must be 16 bytes for column '{}', got {}",
            field.name(),
            bytes.len()
        ));
    }
    Ok(ScalarValue::FixedSizeBinary(16, Some(bytes)))
}

#[cfg(test)]
mod tests {
    use std::{collections::BTreeMap, sync::Arc};

    use arrow::datatypes::{DataType, Field, Schema};

    use super::*;

    fn make_row(entries: Vec<(&str, ScalarValue)>) -> Row {
        let mut values = BTreeMap::new();
        for (key, value) in entries {
            values.insert(key.to_string(), value);
        }
        Row::new(values)
    }

    #[test]
    fn varchar_limit_rejects_one_extra_character() {
        use crate::conversions::with_character_maximum_length;

        let field = with_character_maximum_length(Field::new("string", DataType::Utf8, true), 255);
        let accepted = coerce_scalar_to_field(ScalarValue::Utf8(Some("0".repeat(255))), &field)
            .expect("255 characters fit varchar(255)");
        match accepted {
            ScalarValue::Utf8(Some(text)) => assert_eq!(text.chars().count(), 255),
            other => panic!("unexpected value {other:?}"),
        }

        let rejected = coerce_scalar_to_field(ScalarValue::Utf8(Some("0".repeat(256))), &field)
            .expect_err("256 characters overflow varchar(255)");
        assert!(rejected.contains("value too long for type character varying(255)"));

        let missing = coerce_scalar_to_field(ScalarValue::Utf8(None), &field).unwrap();
        assert!(missing.is_null());

        let unlimited = Field::new("string", DataType::Utf8, true);
        assert!(
            coerce_scalar_to_field(ScalarValue::Utf8(Some("0".repeat(256))), &unlimited).is_ok()
        );
    }

    #[test]
    fn test_coerce_rows_keeps_omitted_required_columns_null() {
        let schema = Arc::new(Schema::new(vec![
            Field::new("id", DataType::Int64, false),
            Field::new("email", DataType::Utf8, false),
        ]));

        let rows = vec![make_row(vec![("id", ScalarValue::Int64(Some(1)))])];

        let coerced = coerce_rows(rows, &schema).unwrap();
        let email = coerced[0].values.get("email").expect("email should be materialized");

        assert!(
            email.is_null(),
            "missing required columns must remain NULL so NOT NULL validation can reject them"
        );
    }

    #[test]
    fn coerce_rejects_invalid_enum_label() {
        use crate::conversions::schema_metadata::with_kalam_enum_labels;

        let field = with_kalam_enum_labels(
            Field::new("status", DataType::Utf8, false),
            &["active".to_string(), "blocked".to_string()],
        );
        let err = coerce_scalar_to_field(ScalarValue::Utf8(Some("nope".into())), &field)
            .expect_err("invalid enum");
        assert!(err.contains("invalid enum label 'nope'"), "{err}");
        coerce_scalar_to_field(ScalarValue::Utf8(Some("active".into())), &field)
            .expect("valid enum label");
    }

    #[test]
    fn test_json_rows_to_arrow_batch_basic() {
        let schema = Arc::new(Schema::new(vec![
            Field::new("id", DataType::Int64, false),
            Field::new("name", DataType::Utf8, true),
        ]));

        let rows = vec![
            make_row(vec![
                ("id", ScalarValue::Int64(Some(1))),
                ("name", ScalarValue::Utf8(Some("Alice".into()))),
            ]),
            make_row(vec![
                ("id", ScalarValue::Int64(Some(2))),
                ("name", ScalarValue::Null),
            ]),
            make_row(vec![
                ("id", ScalarValue::Int64(Some(3))),
                ("name", ScalarValue::Utf8(Some("Charlie".into()))),
            ]),
        ];

        let batch = json_rows_to_arrow_batch(&schema, rows).unwrap();
        assert_eq!(batch.num_rows(), 3);
        assert_eq!(batch.num_columns(), 2);

        let id_col = batch.column(0).as_any().downcast_ref::<Int64Array>().unwrap();
        assert_eq!(id_col.value(0), 1);
        assert_eq!(id_col.value(1), 2);
        assert_eq!(id_col.value(2), 3);

        let name_col = batch.column(1).as_any().downcast_ref::<StringArray>().unwrap();
        assert_eq!(name_col.value(0), "Alice");
        assert!(name_col.is_null(1));
        assert_eq!(name_col.value(2), "Charlie");
    }

    #[test]
    fn test_empty_batch() {
        let schema = Arc::new(Schema::new(vec![
            Field::new("id", DataType::Int64, false),
            Field::new("name", DataType::Utf8, true),
        ]));

        let batch = json_rows_to_arrow_batch(&schema, vec![]).unwrap();
        assert_eq!(batch.num_rows(), 0);
        assert_eq!(batch.num_columns(), 2);

        let empty_columns = vec![Vec::new(), Vec::new()];
        let direct = scalar_columns_to_record_batch(&schema, empty_columns).unwrap();
        assert_eq!(direct.num_rows(), 0);
        assert_eq!(direct.num_columns(), 2);
    }

    #[test]
    fn test_timestamp_conversion() {
        let schema = Arc::new(Schema::new(vec![Field::new(
            "ts",
            DataType::Timestamp(TimeUnit::Millisecond, None),
            false,
        )]));

        // Test i64 timestamp
        let rows = vec![make_row(vec![(
            "ts",
            ScalarValue::Int64(Some(1609459200000)),
        )])];
        let batch = json_rows_to_arrow_batch(&schema, rows).unwrap();
        let ts_col = batch.column(0).as_any().downcast_ref::<TimestampMillisecondArray>().unwrap();
        assert_eq!(ts_col.value(0), 1609459200000i64);

        // Test RFC3339 string
        let rows = vec![make_row(vec![(
            "ts",
            ScalarValue::Utf8(Some("2021-01-01T00:00:00Z".into())),
        )])];
        let batch = json_rows_to_arrow_batch(&schema, rows).unwrap();
        let ts_col = batch.column(0).as_any().downcast_ref::<TimestampMillisecondArray>().unwrap();
        assert_eq!(ts_col.value(0), 1609459200000i64);
    }

    #[test]
    fn arrow_value_to_scalar_preserves_fixed_size_list_embeddings() {
        let child = Arc::new(Field::new("item", DataType::Float32, true));
        let values = Float32Array::from(vec![1.0, 0.0, 0.0, 0.0, 1.0, 0.0]);
        let list = FixedSizeListArray::new(child, 3, Arc::new(values), None);

        let first = arrow_value_to_scalar(&list, 0).unwrap();
        let second = arrow_value_to_scalar(&list, 1).unwrap();

        assert!(
            matches!(&first, ScalarValue::FixedSizeList(_)),
            "cold-row embeddings must stay FixedSizeList, got {first:?}"
        );
        assert!(
            matches!(&second, ScalarValue::FixedSizeList(_)),
            "later parquet rows must not stringify embeddings, got {second:?}"
        );
        assert!(
            !matches!(first, ScalarValue::Utf8(_)) && !matches!(second, ScalarValue::Utf8(_)),
            "Debug-formatting embeddings corrupts DELETE tombstones"
        );

        let first_vec = match &first {
            ScalarValue::FixedSizeList(array) => {
                fixed_size_list_scalar_to_vec(array.as_ref(), 3).expect("first embedding")
            },
            other => panic!("expected FixedSizeList, got {other:?}"),
        };
        assert_eq!(first_vec, vec![1.0, 0.0, 0.0]);
    }

    #[test]
    fn embedding_typed_null_uses_plain_null_scalar() {
        let field = Field::new(
            "embedding",
            DataType::FixedSizeList(Arc::new(Field::new("item", DataType::Float32, true)), 3),
            true,
        );

        let typed_null = coerce_scalar_to_field(ScalarValue::Null, &field).unwrap();
        assert!(matches!(typed_null, ScalarValue::Null));
    }

    #[test]
    fn json_rows_to_arrow_batch_preserves_null_embedding_rows() {
        let schema = Arc::new(Schema::new(vec![
            Field::new("id", DataType::Int64, false),
            Field::new(
                "embedding",
                DataType::FixedSizeList(Arc::new(Field::new("item", DataType::Float32, true)), 3),
                true,
            ),
        ]));

        let rows = vec![
            make_row(vec![
                ("id", ScalarValue::Int64(Some(1))),
                ("embedding", coerce_scalar_to_field(ScalarValue::Null, schema.field(1)).unwrap()),
            ]),
            make_row(vec![
                ("id", ScalarValue::Int64(Some(2))),
                (
                    "embedding",
                    ScalarValue::FixedSizeList(Arc::new(FixedSizeListArray::new(
                        Arc::new(Field::new("item", DataType::Float32, true)),
                        3,
                        Arc::new(Float32Array::from(vec![1.0_f32, 0.0, 0.0])),
                        None,
                    ))),
                ),
            ]),
        ];

        let batch = json_rows_to_arrow_batch(&schema, rows).unwrap();
        let embedding = batch
            .column(1)
            .as_any()
            .downcast_ref::<FixedSizeListArray>()
            .expect("embedding array");

        assert!(embedding.is_null(0));
        assert!(!embedding.is_null(1));
    }

    // ─── No-op UPDATE detection tests ────────────────────────────────────────
    // These tests verify that `coerce_updates` produces values with proper types,
    // ensuring that the Row equality check in `update_by_pk_value` correctly
    // detects no-op UPDATEs (same logical value, potentially different ScalarValue type).

    /// Build a schema matching the user's chat.messages table:
    /// id TEXT PK, thread_id TEXT, role TEXT, content TEXT, created_at TIMESTAMP, _seq INT64,
    /// _commit_seq UINT64, _deleted BOOLEAN
    fn chat_messages_schema() -> Arc<Schema> {
        Arc::new(Schema::new(vec![
            Field::new("id", DataType::Utf8, false),
            Field::new("thread_id", DataType::Utf8, true),
            Field::new("role", DataType::Utf8, true),
            Field::new("content", DataType::Utf8, true),
            Field::new(
                "created_at",
                DataType::Timestamp(arrow::datatypes::TimeUnit::Microsecond, None),
                true,
            ),
            Field::new("_seq", DataType::Int64, false),
            Field::new("_commit_seq", DataType::UInt64, false),
            Field::new("_deleted", DataType::Boolean, false),
        ]))
    }

    /// Helper: simulate what coerce_rows produces during INSERT for a stored row.
    fn stored_row() -> Row {
        make_row(vec![
            ("id", ScalarValue::Utf8(Some("abc123".into()))),
            ("thread_id", ScalarValue::Utf8(Some("thread-1".into()))),
            ("role", ScalarValue::Utf8(Some("user".into()))),
            ("content", ScalarValue::Utf8(Some("hello world".into()))),
            ("created_at", ScalarValue::TimestampMicrosecond(Some(1704067200000000), None)),
            ("_seq", ScalarValue::Int64(Some(0))),
            ("_commit_seq", ScalarValue::UInt64(Some(0))),
            ("_deleted", ScalarValue::Boolean(Some(false))),
        ])
    }

    #[test]
    fn noop_update_text_same_value_detected() {
        // UPDATE chat.messages SET role = 'user' WHERE id = 'abc123'
        // The raw value from expr_to_scalar is Utf8, same as stored → must be no-op.
        let schema = chat_messages_schema();
        let update_row = make_row(vec![("role", ScalarValue::Utf8(Some("user".into())))]);

        let coerced = coerce_updates(update_row, &schema).unwrap();

        // Merge coerced onto stored
        let stored = stored_row();
        let mut merged = stored.values.clone();
        for (k, v) in coerced.values {
            merged.insert(k, v);
        }
        let merged_row = Row::new(merged);

        assert_eq!(merged_row, stored, "no-op TEXT update should produce identical row");
    }

    #[test]
    fn noop_update_text_different_value_detected() {
        // UPDATE chat.messages SET role = 'admin' WHERE id = 'abc123'
        let schema = chat_messages_schema();
        let update_row = make_row(vec![("role", ScalarValue::Utf8(Some("admin".into())))]);

        let coerced = coerce_updates(update_row, &schema).unwrap();

        let stored = stored_row();
        let mut merged = stored.values.clone();
        for (k, v) in coerced.values {
            merged.insert(k, v);
        }
        let merged_row = Row::new(merged);

        assert_ne!(merged_row, stored, "changed TEXT update should not equal stored row");
    }

    #[test]
    fn noop_update_timestamp_same_value_via_int_literal() {
        // UPDATE ... SET created_at = 1704067200000000 (raw Int64 from SQL literal)
        // After coercion: Int64 → TimestampMicrosecond(1704067200000000)
        let schema = chat_messages_schema();
        let update_row = make_row(vec![("created_at", ScalarValue::Int64(Some(1704067200000000)))]);

        let coerced = coerce_updates(update_row, &schema).unwrap();

        // Verify the coerced value is TimestampMicrosecond, not Int64
        let coerced_ts = coerced.values.get("created_at").unwrap();
        assert!(
            matches!(coerced_ts, ScalarValue::TimestampMicrosecond(Some(1704067200000000), None)),
            "Int64 should be coerced to TimestampMicrosecond, got {:?}",
            coerced_ts
        );

        // Merge and verify no-op
        let stored = stored_row();
        let mut merged = stored.values.clone();
        for (k, v) in coerced.values {
            merged.insert(k, v);
        }
        let merged_row = Row::new(merged);

        assert_eq!(merged_row, stored, "same TIMESTAMP value via Int64 should be no-op");
    }

    #[test]
    fn noop_update_without_coercion_would_fail_for_timestamp() {
        // Demonstrates the bug: without coercion, Int64(1704067200000000) !=
        // TimestampMicrosecond(1704067200000000)
        let update_val = ScalarValue::Int64(Some(1704067200000000));
        let stored_val = ScalarValue::TimestampMicrosecond(Some(1704067200000000), None);

        assert_ne!(
            update_val, stored_val,
            "without coercion, Int64 != TimestampMicrosecond (the bug this fix addresses)"
        );
    }

    #[test]
    fn noop_update_multiple_columns_same_values() {
        // UPDATE ... SET role = 'user', content = 'hello world'
        let schema = chat_messages_schema();
        let update_row = make_row(vec![
            ("role", ScalarValue::Utf8(Some("user".into()))),
            ("content", ScalarValue::Utf8(Some("hello world".into()))),
        ]);

        let coerced = coerce_updates(update_row, &schema).unwrap();

        let stored = stored_row();
        let mut merged = stored.values.clone();
        for (k, v) in coerced.values {
            merged.insert(k, v);
        }
        let merged_row = Row::new(merged);

        assert_eq!(merged_row, stored, "multi-column no-op should be detected");
    }

    #[test]
    fn noop_update_one_changed_one_same() {
        // UPDATE ... SET role = 'admin', content = 'hello world'
        // role changed, content same → overall row changed
        let schema = chat_messages_schema();
        let update_row = make_row(vec![
            ("role", ScalarValue::Utf8(Some("admin".into()))),
            ("content", ScalarValue::Utf8(Some("hello world".into()))),
        ]);

        let coerced = coerce_updates(update_row, &schema).unwrap();

        let stored = stored_row();
        let mut merged = stored.values.clone();
        for (k, v) in coerced.values {
            merged.insert(k, v);
        }
        let merged_row = Row::new(merged);

        assert_ne!(merged_row, stored, "partial change should produce different row");
    }

    #[test]
    fn coerce_updates_preserves_utf8_type() {
        // TEXT columns: expr_to_scalar produces Utf8, stored is Utf8 → no coercion needed
        let schema = chat_messages_schema();
        let update_row = make_row(vec![("role", ScalarValue::Utf8(Some("test".into())))]);

        let coerced = coerce_updates(update_row, &schema).unwrap();
        let val = coerced.values.get("role").unwrap();
        assert!(matches!(val, ScalarValue::Utf8(Some(s)) if s == "test"));
    }

    #[test]
    fn coerce_typed_binary_null_to_uuid_field() {
        let field = Field::new("col_uuid", DataType::FixedSizeBinary(16), true);
        let coerced = coerce_scalar_to_field(ScalarValue::Binary(None), &field).unwrap();
        assert!(
            matches!(coerced, ScalarValue::FixedSizeBinary(16, None)),
            "stored Binary(None) UUID slots must become typed FixedSizeBinary nulls: {coerced:?}"
        );
    }

    #[test]
    fn coerce_binary_uuid_bytes_to_fixed_size() {
        let field = Field::new("col_uuid", DataType::FixedSizeBinary(16), true);
        let bytes = vec![0u8; 16];
        let coerced =
            coerce_scalar_to_field(ScalarValue::Binary(Some(bytes.clone())), &field).unwrap();
        assert_eq!(coerced, ScalarValue::FixedSizeBinary(16, Some(bytes)));
    }

    #[test]
    fn coerce_updates_casts_int64_to_timestamp() {
        // TIMESTAMP columns: expr_to_scalar produces Int64, stored is TimestampMicrosecond
        let schema = chat_messages_schema();
        let update_row = make_row(vec![("created_at", ScalarValue::Int64(Some(1704067200000000)))]);

        let coerced = coerce_updates(update_row, &schema).unwrap();
        let val = coerced.values.get("created_at").unwrap();
        assert!(
            matches!(val, ScalarValue::TimestampMicrosecond(Some(1704067200000000), None)),
            "expected TimestampMicrosecond, got {:?}",
            val
        );
    }

    #[test]
    fn coerce_updates_null_to_typed_null() {
        // SET content = NULL should produce typed Utf8(None), not bare ScalarValue::Null
        let schema = chat_messages_schema();
        let update_row = make_row(vec![("content", ScalarValue::Null)]);

        let coerced = coerce_updates(update_row, &schema).unwrap();
        let val = coerced.values.get("content").unwrap();
        assert!(
            matches!(val, ScalarValue::Utf8(None)),
            "bare NULL should be coerced to typed Utf8(None), got {:?}",
            val
        );
    }

    #[test]
    fn coerce_updates_ignores_unknown_columns() {
        // Columns not in schema should pass through unchanged
        let schema = chat_messages_schema();
        let update_row = make_row(vec![("nonexistent_col", ScalarValue::Utf8(Some("val".into())))]);

        let coerced = coerce_updates(update_row, &schema).unwrap();
        let val = coerced.values.get("nonexistent_col").unwrap();
        assert!(matches!(val, ScalarValue::Utf8(Some(s)) if s == "val"));
    }
}

/// Convert Arrow array value at given index to DataFusion ScalarValue
pub fn arrow_value_to_scalar(
    array: &dyn Array,
    row_idx: usize,
) -> Result<ScalarValue, DataFusionError> {
    use arrow::{array::*, datatypes::*};

    if array.is_null(row_idx) {
        return Ok(ScalarValue::try_from(array.data_type()).unwrap_or(ScalarValue::Null));
    }

    match array.data_type() {
        DataType::Boolean => {
            let arr = array.as_any().downcast_ref::<BooleanArray>().unwrap();
            Ok(ScalarValue::Boolean(Some(arr.value(row_idx))))
        },
        DataType::Int8 => {
            let arr = array.as_any().downcast_ref::<Int8Array>().unwrap();
            Ok(ScalarValue::Int8(Some(arr.value(row_idx))))
        },
        DataType::Int16 => {
            let arr = array.as_any().downcast_ref::<Int16Array>().unwrap();
            Ok(ScalarValue::Int16(Some(arr.value(row_idx))))
        },
        DataType::Int32 => {
            let arr = array.as_any().downcast_ref::<Int32Array>().unwrap();
            Ok(ScalarValue::Int32(Some(arr.value(row_idx))))
        },
        DataType::Int64 => {
            let arr = array.as_any().downcast_ref::<Int64Array>().unwrap();
            Ok(ScalarValue::Int64(Some(arr.value(row_idx))))
        },
        DataType::UInt8 => {
            let arr = array.as_any().downcast_ref::<UInt8Array>().unwrap();
            Ok(ScalarValue::UInt8(Some(arr.value(row_idx))))
        },
        DataType::UInt16 => {
            let arr = array.as_any().downcast_ref::<UInt16Array>().unwrap();
            Ok(ScalarValue::UInt16(Some(arr.value(row_idx))))
        },
        DataType::UInt32 => {
            let arr = array.as_any().downcast_ref::<UInt32Array>().unwrap();
            Ok(ScalarValue::UInt32(Some(arr.value(row_idx))))
        },
        DataType::UInt64 => {
            let arr = array.as_any().downcast_ref::<UInt64Array>().unwrap();
            Ok(ScalarValue::UInt64(Some(arr.value(row_idx))))
        },
        DataType::Float32 => {
            let arr = array.as_any().downcast_ref::<Float32Array>().unwrap();
            Ok(ScalarValue::Float32(Some(arr.value(row_idx))))
        },
        DataType::Float64 => {
            let arr = array.as_any().downcast_ref::<Float64Array>().unwrap();
            Ok(ScalarValue::Float64(Some(arr.value(row_idx))))
        },
        DataType::Utf8 => {
            let arr = array.as_any().downcast_ref::<StringArray>().unwrap();
            Ok(ScalarValue::Utf8(Some(arr.value(row_idx).to_string())))
        },
        DataType::LargeUtf8 => {
            let arr = array.as_any().downcast_ref::<LargeStringArray>().unwrap();
            Ok(ScalarValue::LargeUtf8(Some(arr.value(row_idx).to_string())))
        },
        DataType::Timestamp(TimeUnit::Millisecond, tz) => {
            let arr = array.as_any().downcast_ref::<TimestampMillisecondArray>().unwrap();
            Ok(ScalarValue::TimestampMillisecond(Some(arr.value(row_idx)), tz.clone()))
        },
        DataType::Timestamp(TimeUnit::Microsecond, tz) => {
            let arr = array.as_any().downcast_ref::<TimestampMicrosecondArray>().unwrap();
            Ok(ScalarValue::TimestampMicrosecond(Some(arr.value(row_idx)), tz.clone()))
        },
        DataType::Timestamp(TimeUnit::Nanosecond, tz) => {
            let arr = array.as_any().downcast_ref::<TimestampNanosecondArray>().unwrap();
            Ok(ScalarValue::TimestampNanosecond(Some(arr.value(row_idx)), tz.clone()))
        },
        DataType::Timestamp(TimeUnit::Second, tz) => {
            let arr = array.as_any().downcast_ref::<TimestampSecondArray>().unwrap();
            Ok(ScalarValue::TimestampSecond(Some(arr.value(row_idx)), tz.clone()))
        },
        DataType::Date32 => {
            let arr = array.as_any().downcast_ref::<Date32Array>().unwrap();
            Ok(ScalarValue::Date32(Some(arr.value(row_idx))))
        },
        DataType::Date64 => {
            let arr = array.as_any().downcast_ref::<Date64Array>().unwrap();
            Ok(ScalarValue::Date64(Some(arr.value(row_idx))))
        },
        DataType::Time32(TimeUnit::Second) => {
            let arr = array.as_any().downcast_ref::<Time32SecondArray>().unwrap();
            Ok(ScalarValue::Time32Second(Some(arr.value(row_idx))))
        },
        DataType::Time32(TimeUnit::Millisecond) => {
            let arr = array.as_any().downcast_ref::<Time32MillisecondArray>().unwrap();
            Ok(ScalarValue::Time32Millisecond(Some(arr.value(row_idx))))
        },
        DataType::Time64(TimeUnit::Microsecond) => {
            let arr = array.as_any().downcast_ref::<Time64MicrosecondArray>().unwrap();
            Ok(ScalarValue::Time64Microsecond(Some(arr.value(row_idx))))
        },
        DataType::Time64(TimeUnit::Nanosecond) => {
            let arr = array.as_any().downcast_ref::<Time64NanosecondArray>().unwrap();
            Ok(ScalarValue::Time64Nanosecond(Some(arr.value(row_idx))))
        },
        DataType::FixedSizeBinary(size) => {
            let arr = array.as_any().downcast_ref::<FixedSizeBinaryArray>().unwrap();
            Ok(ScalarValue::FixedSizeBinary(*size, Some(arr.value(row_idx).to_vec())))
        },
        DataType::Decimal128(precision, scale) => {
            let arr = array.as_any().downcast_ref::<Decimal128Array>().unwrap();
            Ok(ScalarValue::Decimal128(Some(arr.value(row_idx)), *precision, *scale))
        },
        DataType::Binary => {
            let arr = array.as_any().downcast_ref::<BinaryArray>().unwrap();
            Ok(ScalarValue::Binary(Some(arr.value(row_idx).to_vec())))
        },
        DataType::LargeBinary => {
            let arr = array.as_any().downcast_ref::<LargeBinaryArray>().unwrap();
            Ok(ScalarValue::LargeBinary(Some(arr.value(row_idx).to_vec())))
        },
        _ => ScalarValue::try_from_array(array, row_idx),
    }
}

/// Convert JSON Object to Row
///
/// Used for converting storage JSON rows to Row format for live query notifications.
pub fn json_to_row(json: &JsonValue) -> Option<Row> {
    if let JsonValue::Object(map) = json {
        let mut values = BTreeMap::new();
        for (k, v) in map {
            values.insert(k.clone(), json_value_to_scalar(v));
        }
        Some(Row::new(values))
    } else {
        None
    }
}

/// Convert serde_json::Value to DataFusion ScalarValue
///
/// This variant uses fallbacks for complex types (arrays/objects are converted to JSON strings).
/// For strict validation (e.g., SQL parameter binding), use `json_value_to_scalar_strict`.
pub fn json_value_to_scalar(v: &JsonValue) -> ScalarValue {
    match v {
        JsonValue::Null => ScalarValue::Null,
        JsonValue::Bool(b) => ScalarValue::Boolean(Some(*b)),
        JsonValue::Number(n) => {
            if let Some(i) = n.as_i64() {
                ScalarValue::Int64(Some(i))
            } else if let Some(f) = n.as_f64() {
                ScalarValue::Float64(Some(f))
            } else {
                ScalarValue::Float64(None)
            }
        },
        JsonValue::String(s) => ScalarValue::Utf8(Some(s.clone())),
        JsonValue::Array(_) => ScalarValue::Utf8(Some(v.to_string())), // Fallback for arrays
        JsonValue::Object(_) => ScalarValue::Utf8(Some(v.to_string())), // Fallback for objects
    }
}

/// Convert serde_json::Value to DataFusion ScalarValue with strict validation
///
/// Unlike `json_value_to_scalar`, this function returns an error for unsupported types
/// (arrays and objects). Use this for API parameter binding where strict validation is needed.
///
/// # Errors
/// Returns an error string if the JSON value is an array or object.
pub fn json_value_to_scalar_strict(v: &JsonValue) -> Result<ScalarValue, String> {
    match v {
        JsonValue::Null => Ok(ScalarValue::Utf8(None)),
        JsonValue::Bool(b) => Ok(ScalarValue::Boolean(Some(*b))),
        JsonValue::Number(n) => {
            if let Some(i) = n.as_i64() {
                Ok(ScalarValue::Int64(Some(i)))
            } else if let Some(f) = n.as_f64() {
                Ok(ScalarValue::Float64(Some(f)))
            } else {
                Err(format!("Unsupported number format: {}", n))
            }
        },
        JsonValue::String(s) => Ok(ScalarValue::Utf8(Some(s.clone()))),
        JsonValue::Array(_) => Err("Array parameters not yet supported".to_string()),
        JsonValue::Object(_) => Err("Object parameters not yet supported".to_string()),
    }
}

/// Convert DataFusion ScalarValue to [`KalamCellValue`]
///
/// **SINGLE SOURCE OF TRUTH** for converting ScalarValue to a cell value.
/// All ScalarValue → cell-value conversions in the codebase flow through
/// this function.
///
/// Plain JSON values with Int64/UInt64 as strings for precision.
/// - Example: `"123"` for Int64(123), `"Alice"` for Utf8("Alice")
/// - Timestamps serialized as raw microsecond values (numbers)
pub fn scalar_value_to_json(value: &ScalarValue) -> Result<KalamCellValue, CommonError> {
    let json = match value {
        ScalarValue::Null => JsonValue::Null,
        ScalarValue::Boolean(Some(b)) => JsonValue::Bool(*b),
        ScalarValue::Boolean(None) => JsonValue::Null,
        ScalarValue::Int8(Some(i)) => JsonValue::Number((*i).into()),
        ScalarValue::Int8(None) => JsonValue::Null,
        ScalarValue::Int16(Some(i)) => JsonValue::Number((*i).into()),
        ScalarValue::Int16(None) => JsonValue::Null,
        ScalarValue::Int32(Some(i)) => JsonValue::Number((*i).into()),
        ScalarValue::Int32(None) => JsonValue::Null,
        // Int64 always as string for precision (no MAX_SAFE_INTEGER check)
        ScalarValue::Int64(Some(i)) => JsonValue::String(i.to_string()),
        ScalarValue::Int64(None) => JsonValue::Null,
        ScalarValue::UInt8(Some(i)) => JsonValue::Number((*i).into()),
        ScalarValue::UInt8(None) => JsonValue::Null,
        ScalarValue::UInt16(Some(i)) => JsonValue::Number((*i).into()),
        ScalarValue::UInt16(None) => JsonValue::Null,
        ScalarValue::UInt32(Some(i)) => JsonValue::Number((*i).into()),
        ScalarValue::UInt32(None) => JsonValue::Null,
        // UInt64 always as string for precision
        ScalarValue::UInt64(Some(i)) => JsonValue::String(i.to_string()),
        ScalarValue::UInt64(None) => JsonValue::Null,
        ScalarValue::Float32(Some(f)) => {
            return serde_json::Number::from_f64(*f as f64)
                .map(|n| KalamCellValue(JsonValue::Number(n)))
                .ok_or_else(|| CommonError::invalid_input("Invalid float value"));
        },
        ScalarValue::Float32(None) => JsonValue::Null,
        ScalarValue::Float64(Some(f)) => {
            return serde_json::Number::from_f64(*f)
                .map(|n| KalamCellValue(JsonValue::Number(n)))
                .ok_or_else(|| CommonError::invalid_input("Invalid float value"));
        },
        ScalarValue::Float64(None) => JsonValue::Null,
        ScalarValue::Utf8(Some(s))
        | ScalarValue::LargeUtf8(Some(s))
        | ScalarValue::Utf8View(Some(s)) => JsonValue::String(s.clone()),
        ScalarValue::Utf8(None) | ScalarValue::LargeUtf8(None) | ScalarValue::Utf8View(None) => {
            JsonValue::Null
        },
        ScalarValue::Date32(Some(d)) => JsonValue::Number((*d).into()),
        ScalarValue::Date32(None) => JsonValue::Null,
        ScalarValue::Date64(Some(d)) => JsonValue::Number((*d).into()),
        ScalarValue::Date64(None) => JsonValue::Null,
        ScalarValue::TimestampSecond(Some(ts), _) => {
            JsonValue::Number(checked_unit_to_micros(*ts, 1_000_000, "seconds")?.into())
        },
        ScalarValue::TimestampSecond(None, _) => JsonValue::Null,
        ScalarValue::TimestampMillisecond(Some(ts), _) => {
            JsonValue::Number(checked_unit_to_micros(*ts, 1_000, "milliseconds")?.into())
        },
        ScalarValue::TimestampMillisecond(None, _) => JsonValue::Null,
        ScalarValue::TimestampMicrosecond(Some(ts), _) => JsonValue::Number((*ts).into()),
        ScalarValue::TimestampMicrosecond(None, _) => JsonValue::Null,
        ScalarValue::TimestampNanosecond(Some(ts), _) => JsonValue::Number((ts / 1_000).into()),
        ScalarValue::TimestampNanosecond(None, _) => JsonValue::Null,
        ScalarValue::Decimal128(Some(v), _precision, scale) => {
            JsonValue::String(format_decimal_value(*v, (*scale).max(0) as u8))
        },
        ScalarValue::Decimal128(None, _, _) => JsonValue::Null,
        ScalarValue::FixedSizeBinary(_, Some(bytes)) => {
            // If it looks like a UUID (16 bytes), try to format as UUID string
            if bytes.len() == 16 {
                if let Ok(uuid) = uuid::Uuid::from_slice(bytes) {
                    return Ok(KalamCellValue(JsonValue::String(uuid.to_string())));
                }
            }
            // Otherwise, use array of numbers for generic binary
            JsonValue::Array(bytes.iter().map(|&b| JsonValue::Number(b.into())).collect())
        },
        ScalarValue::FixedSizeBinary(_, None) => JsonValue::Null,
        ScalarValue::Binary(Some(bytes)) | ScalarValue::LargeBinary(Some(bytes)) => {
            JsonValue::Array(bytes.iter().map(|&b| JsonValue::Number(b.into())).collect())
        },
        ScalarValue::Binary(None) | ScalarValue::LargeBinary(None) => JsonValue::Null,
        ScalarValue::List(list) => list_scalar_value_to_json(list.as_ref())?,
        ScalarValue::LargeList(list) => large_list_scalar_value_to_json(list.as_ref())?,
        ScalarValue::FixedSizeList(list) => fixed_size_list_scalar_value_to_json(list.as_ref())?,
        // Time types are serialized as raw microsecond values (integer).
        ScalarValue::Time32Second(Some(t)) => {
            JsonValue::Number(checked_unit_to_micros(i64::from(*t), 1_000_000, "seconds")?.into())
        },
        ScalarValue::Time32Second(None) => JsonValue::Null,
        ScalarValue::Time32Millisecond(Some(t)) => {
            JsonValue::Number(checked_unit_to_micros(i64::from(*t), 1_000, "milliseconds")?.into())
        },
        ScalarValue::Time32Millisecond(None) => JsonValue::Null,
        ScalarValue::Time64Microsecond(Some(t)) => JsonValue::Number((*t).into()),
        ScalarValue::Time64Microsecond(None) => JsonValue::Null,
        ScalarValue::Time64Nanosecond(Some(t)) => JsonValue::Number((t / 1_000).into()),
        ScalarValue::Time64Nanosecond(None) => JsonValue::Null,
        ScalarValue::Struct(array) => struct_scalar_value_to_json(array.as_ref())?,
        _ => {
            return Err(CommonError::invalid_input(format!(
                "Unsupported ScalarValue conversion to JSON: {:?}",
                value
            )));
        },
    };
    Ok(KalamCellValue(json))
}

/// JSON for the in-memory function JS boundary.
///
/// REST/SQL JSON keeps Int64/UInt64 as strings so JavaScript clients do not
/// lose precision. V8 `input + 1` is arithmetic, so values within
/// `Number.MAX_SAFE_INTEGER` must be JSON numbers or they concatenate.
pub fn scalar_value_to_js_json(value: &ScalarValue) -> Result<KalamCellValue, CommonError> {
    Ok(KalamCellValue(scalar_to_js_json_value(value)?))
}

const JS_MAX_SAFE_INTEGER: i64 = (1i64 << 53) - 1;

fn scalar_to_js_json_value(value: &ScalarValue) -> Result<JsonValue, CommonError> {
    match value {
        ScalarValue::Int64(Some(n)) if *n >= -JS_MAX_SAFE_INTEGER && *n <= JS_MAX_SAFE_INTEGER => {
            Ok(JsonValue::Number((*n).into()))
        },
        ScalarValue::UInt64(Some(n)) if *n <= JS_MAX_SAFE_INTEGER as u64 => {
            Ok(JsonValue::Number((*n).into()))
        },
        ScalarValue::List(list) => list_scalar_value_to_js_json(list.as_ref()),
        ScalarValue::LargeList(list) => large_list_scalar_value_to_js_json(list.as_ref()),
        ScalarValue::FixedSizeList(list) => fixed_size_list_scalar_value_to_js_json(list.as_ref()),
        ScalarValue::Struct(array) => struct_scalar_value_to_js_json(array.as_ref()),
        _ => Ok(scalar_value_to_json(value)?.0),
    }
}

#[inline]
fn checked_unit_to_micros(value: i64, factor: i64, source_unit: &str) -> Result<i64, CommonError> {
    value.checked_mul(factor).ok_or_else(|| {
        CommonError::invalid_input(format!(
            "{} value {} overflows when converted to microseconds",
            source_unit, value
        ))
    })
}

fn list_scalar_value_to_json(list: &ListArray) -> Result<JsonValue, CommonError> {
    nested_array_to_json(list, |array| array.value(0), scalar_value_to_json)
}

fn large_list_scalar_value_to_json(list: &LargeListArray) -> Result<JsonValue, CommonError> {
    nested_array_to_json(list, |array| array.value(0), scalar_value_to_json)
}

fn fixed_size_list_scalar_value_to_json(
    list: &FixedSizeListArray,
) -> Result<JsonValue, CommonError> {
    nested_array_to_json(list, |array| array.value(0), scalar_value_to_json)
}

fn list_scalar_value_to_js_json(list: &ListArray) -> Result<JsonValue, CommonError> {
    nested_array_to_json(list, |array| array.value(0), scalar_value_to_js_json)
}

fn large_list_scalar_value_to_js_json(list: &LargeListArray) -> Result<JsonValue, CommonError> {
    nested_array_to_json(list, |array| array.value(0), scalar_value_to_js_json)
}

fn fixed_size_list_scalar_value_to_js_json(
    list: &FixedSizeListArray,
) -> Result<JsonValue, CommonError> {
    nested_array_to_json(list, |array| array.value(0), scalar_value_to_js_json)
}

fn struct_scalar_value_to_json(array: &StructArray) -> Result<JsonValue, CommonError> {
    struct_scalar_value_to_json_converted(array, scalar_value_to_json)
}

fn struct_scalar_value_to_js_json(array: &StructArray) -> Result<JsonValue, CommonError> {
    struct_scalar_value_to_json_converted(array, scalar_value_to_js_json)
}

fn struct_scalar_value_to_json_converted(
    array: &StructArray,
    convert: fn(&ScalarValue) -> Result<KalamCellValue, CommonError>,
) -> Result<JsonValue, CommonError> {
    if array.is_empty() || array.is_null(0) {
        return Ok(JsonValue::Null);
    }
    let mut object = serde_json::Map::with_capacity(array.num_columns());
    for (index, field) in array.fields().iter().enumerate() {
        let scalar = ScalarValue::try_from_array(array.column(index), 0).map_err(|error| {
            CommonError::invalid_input(format!(
                "Failed to extract struct field '{}': {}",
                field.name(),
                error
            ))
        })?;
        object.insert(field.name().clone(), convert(&scalar)?.0);
    }
    Ok(JsonValue::Object(object))
}

fn nested_array_to_json<A, F>(
    array: &A,
    extract_values: F,
    convert: fn(&ScalarValue) -> Result<KalamCellValue, CommonError>,
) -> Result<JsonValue, CommonError>
where
    A: Array,
    F: FnOnce(&A) -> ArrayRef,
{
    if array.is_empty() || array.is_null(0) {
        return Ok(JsonValue::Null);
    }

    let values = extract_values(array);
    let mut json_values = Vec::with_capacity(values.len());
    for index in 0..values.len() {
        let scalar = ScalarValue::try_from_array(values.as_ref(), index).map_err(|error| {
            CommonError::invalid_input(format!("Failed to extract list element scalar: {}", error))
        })?;
        json_values.push(convert(&scalar)?.0);
    }

    Ok(JsonValue::Array(json_values))
}

/// Convert Arrow RecordBatch to JSON rows
///
/// **Used by:** REST API `/v1/api/sql` endpoint for query results
///
/// For each cell in the batch, extracts ScalarValue and calls `scalar_value_to_json()`
/// (the single source of truth for all conversions).
///
/// # Arguments
/// * `batch` - Arrow RecordBatch from DataFusion query execution
///
/// # Architecture Note
/// ✅ NO direct conversions - all cells go through scalar_value_to_json()
pub fn record_batch_to_json_rows(
    batch: &RecordBatch,
) -> Result<Vec<std::collections::HashMap<String, KalamCellValue>>, CommonError> {
    let schema = batch.schema();
    let column_names: Vec<String> = schema.fields().iter().map(|f| f.name().clone()).collect();
    let num_rows = batch.num_rows();
    let num_cols = batch.num_columns();

    let mut rows = Vec::with_capacity(num_rows);

    for row_idx in 0..num_rows {
        let mut json_row = std::collections::HashMap::with_capacity(num_cols);

        for (col_idx, col_name) in column_names.iter().enumerate() {
            let column = batch.column(col_idx);

            let scalar = ScalarValue::try_from_array(column.as_ref(), row_idx).map_err(|e| {
                CommonError::invalid_input(format!("Failed to extract scalar: {}", e))
            })?;

            // ✅ SINGLE SOURCE: All conversions flow through this function
            let json_value = scalar_value_to_json(&scalar)?;
            json_row.insert(col_name.clone(), json_value);
        }

        rows.push(json_row);
    }

    Ok(rows)
}

/// Convert Arrow RecordBatch to JSON arrays (one array per row, values ordered by column index)
///
/// **Used by:** REST API `/v1/api/sql` endpoint for query results
///
/// Returns rows as arrays of values where each value's position corresponds to the schema field
/// index. Example output: `[["123", "Alice", 1699000000000000], ["456", "Bob", 1699000001000000]]`
///
/// # Arguments
/// * `batch` - Arrow RecordBatch from DataFusion query execution
///
/// # Architecture Note
/// ✅ NO direct conversions - all cells go through scalar_value_to_json()
/// Write one JSON array per row directly from Arrow columns.
///
/// Returns false when a column type has no direct writer. Callers then use
/// [`record_batch_to_json_arrays`] so masking and uncommon types stay unchanged.
pub fn try_append_record_batch_json_rows(
    batch: &RecordBatch,
    out: &mut Vec<u8>,
    row_separator_needed: &mut bool,
) -> Result<bool, CommonError> {
    if !batch.columns().iter().all(|column| arrow_column_has_direct_json(column.as_ref())) {
        return Ok(false);
    }
    for row_idx in 0..batch.num_rows() {
        if *row_separator_needed {
            out.push(b',');
        } else {
            *row_separator_needed = true;
        }
        out.push(b'[');
        for (col_idx, column) in batch.columns().iter().enumerate() {
            if col_idx > 0 {
                out.push(b',');
            }
            append_arrow_cell_json(column.as_ref(), row_idx, out)?;
        }
        out.push(b']');
    }
    Ok(true)
}

fn arrow_column_has_direct_json(column: &dyn arrow::array::Array) -> bool {
    matches!(
        column.data_type(),
        arrow::datatypes::DataType::Boolean
            | arrow::datatypes::DataType::Int8
            | arrow::datatypes::DataType::Int16
            | arrow::datatypes::DataType::Int32
            | arrow::datatypes::DataType::Int64
            | arrow::datatypes::DataType::UInt8
            | arrow::datatypes::DataType::UInt16
            | arrow::datatypes::DataType::UInt32
            | arrow::datatypes::DataType::UInt64
            | arrow::datatypes::DataType::Float32
            | arrow::datatypes::DataType::Float64
            | arrow::datatypes::DataType::Utf8
            | arrow::datatypes::DataType::LargeUtf8
            | arrow::datatypes::DataType::Null
    )
}

fn append_arrow_cell_json(
    column: &dyn arrow::array::Array,
    row_idx: usize,
    out: &mut Vec<u8>,
) -> Result<(), CommonError> {
    use arrow::array::{
        BooleanArray, Float32Array, Float64Array, Int16Array, Int32Array, Int64Array, Int8Array,
        LargeStringArray, StringArray, UInt16Array, UInt32Array, UInt64Array, UInt8Array,
    };

    if column.is_null(row_idx) {
        out.extend_from_slice(b"null");
        return Ok(());
    }
    match column.data_type() {
        arrow::datatypes::DataType::Boolean => {
            let value = column.as_any().downcast_ref::<BooleanArray>().unwrap().value(row_idx);
            out.extend_from_slice(if value { b"true" } else { b"false" });
        },
        arrow::datatypes::DataType::Int8 => {
            let value = column.as_any().downcast_ref::<Int8Array>().unwrap().value(row_idx);
            append_i64(out, i64::from(value));
        },
        arrow::datatypes::DataType::Int16 => {
            let value = column.as_any().downcast_ref::<Int16Array>().unwrap().value(row_idx);
            append_i64(out, i64::from(value));
        },
        arrow::datatypes::DataType::Int32 => {
            let value = column.as_any().downcast_ref::<Int32Array>().unwrap().value(row_idx);
            append_i64(out, i64::from(value));
        },
        arrow::datatypes::DataType::Int64 => {
            let value = column.as_any().downcast_ref::<Int64Array>().unwrap().value(row_idx);
            out.push(b'"');
            append_i64(out, value);
            out.push(b'"');
        },
        arrow::datatypes::DataType::UInt8 => {
            let value = column.as_any().downcast_ref::<UInt8Array>().unwrap().value(row_idx);
            append_i64(out, i64::from(value));
        },
        arrow::datatypes::DataType::UInt16 => {
            let value = column.as_any().downcast_ref::<UInt16Array>().unwrap().value(row_idx);
            append_i64(out, i64::from(value));
        },
        arrow::datatypes::DataType::UInt32 => {
            let value = column.as_any().downcast_ref::<UInt32Array>().unwrap().value(row_idx);
            append_i64(out, i64::from(value));
        },
        arrow::datatypes::DataType::UInt64 => {
            let value = column.as_any().downcast_ref::<UInt64Array>().unwrap().value(row_idx);
            out.push(b'"');
            append_u64(out, value);
            out.push(b'"');
        },
        arrow::datatypes::DataType::Float32 => {
            let value = column.as_any().downcast_ref::<Float32Array>().unwrap().value(row_idx);
            append_json_f64(out, f64::from(value));
        },
        arrow::datatypes::DataType::Float64 => {
            let value = column.as_any().downcast_ref::<Float64Array>().unwrap().value(row_idx);
            append_json_f64(out, value);
        },
        arrow::datatypes::DataType::Utf8 => {
            let value = column.as_any().downcast_ref::<StringArray>().unwrap().value(row_idx);
            append_json_string(out, value);
        },
        arrow::datatypes::DataType::LargeUtf8 => {
            let value = column.as_any().downcast_ref::<LargeStringArray>().unwrap().value(row_idx);
            append_json_string(out, value);
        },
        arrow::datatypes::DataType::Null => out.extend_from_slice(b"null"),
        _ => {
            return Err(CommonError::invalid_input(
                "column type has no direct JSON writer",
            ));
        },
    }
    Ok(())
}

fn append_json_f64(out: &mut Vec<u8>, value: f64) {
    if value.is_finite() {
        out.extend_from_slice(format!("{value}").as_bytes());
    } else {
        out.extend_from_slice(b"null");
    }
}

fn append_u64(out: &mut Vec<u8>, mut remaining: u64) {
    if remaining == 0 {
        out.push(b'0');
        return;
    }
    let mut buf = [0u8; 20];
    let mut cursor = buf.len();
    while remaining > 0 {
        cursor -= 1;
        buf[cursor] = b'0' + (remaining % 10) as u8;
        remaining /= 10;
    }
    out.extend_from_slice(&buf[cursor..]);
}

fn append_i64(out: &mut Vec<u8>, value: i64) {
    if value == i64::MIN {
        out.extend_from_slice(b"-9223372036854775808");
        return;
    }
    let mut buf = [0u8; 20];
    let mut cursor = buf.len();
    let negative = value < 0;
    let mut remaining = if negative { -value } else { value } as u64;
    if remaining == 0 {
        out.push(b'0');
        return;
    }
    while remaining > 0 {
        cursor -= 1;
        buf[cursor] = b'0' + (remaining % 10) as u8;
        remaining /= 10;
    }
    if negative {
        cursor -= 1;
        buf[cursor] = b'-';
    }
    out.extend_from_slice(&buf[cursor..]);
}

fn append_json_string(out: &mut Vec<u8>, value: &str) {
    out.push(b'"');
    for ch in value.chars() {
        match ch {
            '"' => out.extend_from_slice(b"\\\""),
            '\\' => out.extend_from_slice(b"\\\\"),
            '\n' => out.extend_from_slice(b"\\n"),
            '\r' => out.extend_from_slice(b"\\r"),
            '\t' => out.extend_from_slice(b"\\t"),
            ch if (ch as u32) < 0x20 => {
                let mut hex = [0u8; 6];
                hex[0] = b'\\';
                hex[1] = b'u';
                hex[2] = b'0';
                hex[3] = b'0';
                let byte = ch as u8;
                hex[4] = hex_digit(byte >> 4);
                hex[5] = hex_digit(byte & 0x0f);
                out.extend_from_slice(&hex);
            },
            ch => {
                let mut buf = [0u8; 4];
                out.extend_from_slice(ch.encode_utf8(&mut buf).as_bytes());
            },
        }
    }
    out.push(b'"');
}

fn hex_digit(value: u8) -> u8 {
    b"0123456789abcdef"[usize::from(value)]
}

pub fn record_batch_to_json_arrays(
    batch: &RecordBatch,
) -> Result<Vec<Vec<KalamCellValue>>, CommonError> {
    let num_rows = batch.num_rows();
    let num_cols = batch.num_columns();

    let mut rows = Vec::with_capacity(num_rows);

    for row_idx in 0..num_rows {
        let mut json_row = Vec::with_capacity(num_cols);

        for col_idx in 0..num_cols {
            let column = batch.column(col_idx);

            let scalar = ScalarValue::try_from_array(column.as_ref(), row_idx).map_err(|e| {
                CommonError::invalid_input(format!("Failed to extract scalar: {}", e))
            })?;

            // ✅ SINGLE SOURCE: All conversions flow through this function
            let json_value = scalar_value_to_json(&scalar)?;
            json_row.push(json_value);
        }

        rows.push(json_row);
    }

    Ok(rows)
}

/// Convert schema-aligned [`Row`] maps to JSON arrays without building Arrow.
///
/// Cached HTTP point gets use this instead of `json_rows_to_arrow_batch` +
/// `record_batch_to_json_arrays`. Do not route that path through Arrow.
pub fn rows_to_json_arrays(
    schema: &SchemaRef,
    rows: Vec<Row>,
) -> Result<Vec<Vec<KalamCellValue>>, CommonError> {
    let mut json_rows = Vec::with_capacity(rows.len());
    for mut row in rows {
        let mut json_row = Vec::with_capacity(schema.fields().len());
        for field in schema.fields() {
            // Output field names (including aliases) must already match Row
            // keys. Missing keys become JSON null — that is how
            // `SELECT name AS title` regressed on skip-Arrow.
            let scalar = row.values.remove(field.name()).unwrap_or(ScalarValue::Null);
            let coerced = coerce_scalar_to_field_inner(scalar, field, false)
                .map_err(CommonError::invalid_input)?;
            json_row.push(scalar_value_to_json(&coerced)?);
        }
        json_rows.push(json_row);
    }
    Ok(json_rows)
}

/// Convert Row to HashMap<String, JsonValue>
///
/// **Used by:** WebSocket subscriptions, change notifications, and batch data
///
/// For each column in the row, calls `scalar_value_to_json()` (the single source of truth).
///
/// # Arguments
/// * `row` - Row from live query system (ScalarValue map)
///
/// # Returns
/// HashMap mapping column names to JSON values
///
/// # Architecture Note
/// ✅ NO direct conversions - all columns go through scalar_value_to_json()
pub fn row_to_json_map(
    row: &Row,
) -> Result<std::collections::HashMap<String, KalamCellValue>, CommonError> {
    let mut json_row = std::collections::HashMap::with_capacity(row.values.len());

    for (col_name, scalar_value) in &row.values {
        // ✅ SINGLE SOURCE: All conversions flow through this function
        let json_value = scalar_value_to_json(scalar_value)?;
        json_row.insert(col_name.clone(), json_value);
    }

    Ok(json_row)
}

/// Zero-clone variant of [`row_to_json_map`] that takes ownership of the Row,
/// reusing the existing String allocations for column names instead of cloning.
///
/// Prefer this over `row_to_json_map(&row)` when the Row is no longer needed.
pub fn row_into_json_map(
    row: Row,
) -> Result<std::collections::HashMap<String, KalamCellValue>, CommonError> {
    let mut json_row = std::collections::HashMap::with_capacity(row.values.len());

    for (col_name, scalar_value) in row.values {
        let json_value = scalar_value_to_json(&scalar_value)?;
        json_row.insert(col_name, json_value);
    }

    Ok(json_row)
}

/// Coerce a single row of updates to match the schema types.
///
/// Unlike `coerce_rows`, this does NOT fill in default values for missing columns.
/// It only coerces the columns that are present in the update row.
pub fn coerce_updates(row: Row, schema: &SchemaRef) -> Result<Row, String> {
    let mut new_values = BTreeMap::new();

    for (col_name, value) in row.values {
        if let Ok(field) = schema.field_with_name(&col_name) {
            let coerced = coerce_scalar_to_field(value, field)?;
            new_values.insert(col_name, coerced);
        } else {
            // If column not in schema, keep it as is
            new_values.insert(col_name, value);
        }
    }

    Ok(Row::new(new_values))
}

#[cfg(test)]
mod serialization_tests {
    use super::*;

    #[test]
    fn test_int64_always_string() {
        // Int64 should always be serialized as string for precision
        let value = ScalarValue::Int64(Some(123));
        let json = scalar_value_to_json(&value).unwrap();
        assert_eq!(json, serde_json::json!("123").into());

        // Even small values should be strings
        let value = ScalarValue::Int64(Some(42));
        let json = scalar_value_to_json(&value).unwrap();
        assert_eq!(json, serde_json::json!("42").into());
    }

    #[test]
    fn test_int64_js_boundary_is_number_when_safe() {
        let json = scalar_value_to_js_json(&ScalarValue::Int64(Some(41))).unwrap();
        assert_eq!(json, serde_json::json!(41).into());
        let json = scalar_value_to_js_json(&ScalarValue::UInt64(Some(41))).unwrap();
        assert_eq!(json, serde_json::json!(41).into());
        let json =
            scalar_value_to_js_json(&ScalarValue::Int64(Some(JS_MAX_SAFE_INTEGER + 1))).unwrap();
        assert_eq!(json, serde_json::json!((JS_MAX_SAFE_INTEGER + 1).to_string()).into());
    }

    #[test]
    fn test_uint64_always_string() {
        let value = ScalarValue::UInt64(Some(123));
        let json = scalar_value_to_json(&value).unwrap();
        assert_eq!(json, serde_json::json!("123").into());
    }

    #[test]
    fn test_utf8_plain_string() {
        let value = ScalarValue::Utf8(Some("hello".to_string()));
        let json = scalar_value_to_json(&value).unwrap();
        assert_eq!(json, serde_json::json!("hello").into());
    }

    #[test]
    fn test_utf8view_plain_string() {
        let value = ScalarValue::Utf8View(Some("bigint".to_string()));
        let json = scalar_value_to_json(&value).unwrap();
        assert_eq!(json, serde_json::json!("bigint").into());
    }

    #[test]
    fn test_boolean_plain_value() {
        let value = ScalarValue::Boolean(Some(true));
        let json = scalar_value_to_json(&value).unwrap();
        assert_eq!(json, serde_json::json!(true).into());
    }

    #[test]
    fn test_timestamp_as_number() {
        let value = ScalarValue::TimestampMicrosecond(Some(1765741510326539), None);
        let json = scalar_value_to_json(&value).unwrap();
        // Timestamp should be raw number (microseconds)
        assert_eq!(json, serde_json::json!(1765741510326539_i64).into());
    }

    #[test]
    fn test_timestamp_units_serialize_as_microseconds() {
        let expected: KalamCellValue = serde_json::json!(1735689600000000_i64).into();

        assert_eq!(
            scalar_value_to_json(&ScalarValue::TimestampSecond(Some(1735689600), None)).unwrap(),
            expected.clone()
        );
        assert_eq!(
            scalar_value_to_json(&ScalarValue::TimestampMillisecond(Some(1735689600000), None))
                .unwrap(),
            expected.clone()
        );
        assert_eq!(
            scalar_value_to_json(&ScalarValue::TimestampMicrosecond(Some(1735689600000000), None))
                .unwrap(),
            expected.clone()
        );
        assert_eq!(
            scalar_value_to_json(&ScalarValue::TimestampNanosecond(
                Some(1735689600000000000),
                None,
            ))
            .unwrap(),
            expected
        );
    }

    #[test]
    fn test_time_units_serialize_as_microseconds() {
        let expected: KalamCellValue = serde_json::json!(45_296_123_456_i64).into();

        assert_eq!(
            scalar_value_to_json(&ScalarValue::Time32Second(Some(45_296))).unwrap(),
            serde_json::json!(45_296_000_000_i64).into()
        );
        assert_eq!(
            scalar_value_to_json(&ScalarValue::Time32Millisecond(Some(45_296_123))).unwrap(),
            serde_json::json!(45_296_123_000_i64).into()
        );
        assert_eq!(
            scalar_value_to_json(&ScalarValue::Time64Microsecond(Some(45_296_123_456))).unwrap(),
            expected.clone()
        );
        assert_eq!(
            scalar_value_to_json(&ScalarValue::Time64Nanosecond(Some(45_296_123_456_789))).unwrap(),
            expected
        );
    }

    #[test]
    fn test_null_values() {
        let null = ScalarValue::Null;
        let json = scalar_value_to_json(&null).unwrap();
        assert_eq!(json, serde_json::json!(null).into());
    }

    #[test]
    fn test_list_values_serialize_to_json_arrays() {
        let mut builder = ListBuilder::new(StringBuilder::new());
        builder.values().append_value("a");
        builder.values().append_value("b");
        builder.append(true);

        let value = ScalarValue::List(Arc::new(builder.finish()));
        let json = scalar_value_to_json(&value).unwrap();

        assert_eq!(json, serde_json::json!(["a", "b"]).into());
    }

    #[test]
    fn test_float64_plain_number() {
        let value = ScalarValue::Float64(Some(3.25));
        let json = scalar_value_to_json(&value).unwrap();
        // Check it's a number approximately equal to 3.25
        assert!(json.inner().as_f64().unwrap() > 3.24 && json.inner().as_f64().unwrap() < 3.26);
    }

    #[test]
    fn test_int32_plain_number() {
        let value = ScalarValue::Int32(Some(42));
        let json = scalar_value_to_json(&value).unwrap();
        assert_eq!(json, serde_json::json!(42).into());
    }
}
