//! Arrow schema metadata helpers
//!
//! Centralized helpers for reading/writing Arrow Field metadata keys used by KalamDB.
//! This ensures consistent key names and serialization across the codebase.

use arrow_schema::Field;

use crate::{models::datatypes::KalamDataType, schemas::FieldFlags};

/// Metadata key for serialized KalamDataType.
pub const KALAM_DATA_TYPE_METADATA_KEY: &str = "kalam_data_type";
/// Metadata key for serialized `FieldFlags`.
pub const KALAM_COLUMN_FLAGS_METADATA_KEY: &str = "kalam_column_flags";
/// Parquet occurrence identity. Distinct from SQL TypeId; two columns of the same type get
/// different ids.
pub const PARQUET_FIELD_ID_METADATA_KEY: &str = "PARQUET:field_id";
/// Opaque catalog TypeId for named CREATE TYPE columns. Survives SQL rename.
pub const KALAM_TYPE_ID_METADATA_KEY: &str = "kalam_type_id";
/// Allowed labels for a named ENUM column (JSON string array).
pub const KALAM_ENUM_LABELS_METADATA_KEY: &str = "kalam_enum_labels";

/// Read `KalamDataType` from Arrow field metadata.
pub fn read_kalam_data_type_metadata(field: &Field) -> Option<KalamDataType> {
    field
        .metadata()
        .get(KALAM_DATA_TYPE_METADATA_KEY)
        .and_then(|s| serde_json::from_str::<KalamDataType>(s).ok())
}

/// Attach `KalamDataType` metadata to an Arrow field.
///
/// Preserves any existing metadata on the field.
pub fn with_kalam_data_type_metadata(mut field: Field, kalam_type: &KalamDataType) -> Field {
    let kalam_type_json =
        serde_json::to_string(kalam_type).unwrap_or_else(|_| "\"Text\"".to_string());
    let mut metadata = field.metadata().clone();
    metadata.insert(KALAM_DATA_TYPE_METADATA_KEY.to_string(), kalam_type_json);
    field = field.with_metadata(metadata);
    field
}

/// Read typed `FieldFlags` from Arrow field metadata.
pub fn read_kalam_column_flags_metadata(field: &Field) -> Option<FieldFlags> {
    field
        .metadata()
        .get(KALAM_COLUMN_FLAGS_METADATA_KEY)
        .and_then(|s| serde_json::from_str::<FieldFlags>(s).ok())
}

/// Attach typed `FieldFlags` to an Arrow field.
pub fn with_kalam_column_flags_metadata(mut field: Field, flags: &FieldFlags) -> Field {
    let mut metadata = field.metadata().clone();
    let flags_json = serde_json::to_string(flags).unwrap_or_else(|_| "[]".to_string());
    metadata.insert(KALAM_COLUMN_FLAGS_METADATA_KEY.to_string(), flags_json);
    field = field.with_metadata(metadata);
    field
}

/// Nested Parquet occurrence identity: `parent * 1000 + slot`.
///
/// Used by table columns (`column_id` as parent) and named-type fields (`TypeFieldSlot`).
pub fn nested_parquet_occurrence_id(parent_id: i32, slot: i32) -> i32 {
    parent_id.saturating_mul(1_000).saturating_add(slot)
}

/// Attach a Parquet field_id occurrence. Nested children use `parent * 1000 + slot`.
pub fn with_parquet_field_id(mut field: Field, field_id: i32) -> Field {
    let mut metadata = field.metadata().clone();
    metadata.insert(PARQUET_FIELD_ID_METADATA_KEY.to_string(), field_id.to_string());
    field = field.with_metadata(metadata);
    field
}

/// Read a Parquet field_id from Arrow metadata.
pub fn read_parquet_field_id(field: &Field) -> Option<i32> {
    field
        .metadata()
        .get(PARQUET_FIELD_ID_METADATA_KEY)
        .and_then(|value| value.parse().ok())
}

/// Attach the opaque catalog TypeId so pgwire OIDs and codecs follow identity, not SQL name.
pub fn with_kalam_type_id_metadata(mut field: Field, type_id: &crate::models::TypeId) -> Field {
    let mut metadata = field.metadata().clone();
    metadata.insert(KALAM_TYPE_ID_METADATA_KEY.to_string(), type_id.as_str().to_string());
    field = field.with_metadata(metadata);
    field
}

/// Read the opaque catalog TypeId from Arrow field metadata.
pub fn read_kalam_type_id_metadata(field: &Field) -> Option<crate::models::TypeId> {
    field
        .metadata()
        .get(KALAM_TYPE_ID_METADATA_KEY)
        .and_then(|value| crate::models::TypeId::try_new(value.clone()).ok())
}

/// Attach allowed ENUM labels so DML can fail closed without a catalog lookup.
pub fn with_kalam_enum_labels(mut field: Field, labels: &[String]) -> Field {
    let encoded = serde_json::to_string(labels).unwrap_or_else(|_| "[]".to_string());
    let mut metadata = field.metadata().clone();
    metadata.insert(KALAM_ENUM_LABELS_METADATA_KEY.to_string(), encoded);
    field = field.with_metadata(metadata);
    field
}

/// Read allowed ENUM labels from Arrow field metadata.
pub fn read_kalam_enum_labels(field: &Field) -> Option<Vec<String>> {
    let raw = field.metadata().get(KALAM_ENUM_LABELS_METADATA_KEY)?;
    serde_json::from_str(raw).ok()
}

/// Reject a label that is not in the field's ENUM metadata.
///
/// Fields without enum metadata are left unchanged.
pub fn validate_enum_label(field: &Field, label: &str) -> Result<(), String> {
    let Some(labels) = read_kalam_enum_labels(field) else {
        return Ok(());
    };
    if labels.iter().any(|existing| existing == label) {
        return Ok(());
    }
    Err(format!(
        "invalid enum label '{label}' for {}; supported values: {}",
        field.name(),
        labels.join(", ")
    ))
}

/// Convert an Arrow schema into KalamDB `SchemaField` descriptors.
///
/// Reads `KalamDataType` from field metadata when present, otherwise infers
/// from the Arrow `DataType`. Falls back to `Text` for unsupported types.
#[cfg(feature = "arrow-conversion")]
pub fn schema_fields_from_arrow_schema(
    arrow_schema: &arrow_schema::SchemaRef,
) -> Vec<crate::schemas::SchemaField> {
    use crate::{conversions::arrow_conversion::FromArrowType, models::datatypes::KalamDataType};

    arrow_schema
        .fields()
        .iter()
        .enumerate()
        .map(|(index, field)| {
            let kalam_type = read_kalam_data_type_metadata(field).unwrap_or_else(|| {
                KalamDataType::from_arrow_type(field.data_type()).unwrap_or(KalamDataType::Text)
            });
            crate::schemas::SchemaField::from_arrow_field(field, kalam_type, index)
        })
        .collect()
}

/// Mask sensitive columns (`credentials`, `password_hash`) for non-admin roles.
///
/// Admin roles (`Dba`, `System`) see unmasked values. All other roles see `"***"`.
#[cfg(feature = "arrow-conversion")]
pub fn mask_sensitive_rows_for_role(
    rows: &mut [Vec<crate::models::KalamCellValue>],
    schema_fields: &[crate::schemas::SchemaField],
    user_role: crate::models::Role,
) {
    use crate::models::KalamCellValue;

    if matches!(user_role, crate::models::Role::Dba | crate::models::Role::System) {
        return;
    }
    for target in &["credentials", "password_hash"] {
        if let Some(col_idx) =
            schema_fields.iter().position(|f| f.name.eq_ignore_ascii_case(target))
        {
            for row in rows.iter_mut() {
                if let Some(v) = row.get_mut(col_idx) {
                    if !v.is_null() {
                        *v = KalamCellValue::text("***");
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nested_occurrence_is_parent_times_1000_plus_slot() {
        assert_eq!(nested_parquet_occurrence_id(7, 3), 7003);
    }

    #[test]
    fn enum_labels_round_trip_and_reject_unknown() {
        let field = with_kalam_enum_labels(
            Field::new("status", arrow_schema::DataType::Utf8, false),
            &["active".to_string(), "blocked".to_string()],
        );
        assert_eq!(
            read_kalam_enum_labels(&field).as_deref(),
            Some(["active".to_string(), "blocked".to_string()].as_slice())
        );
        validate_enum_label(&field, "active").unwrap();
        let err = validate_enum_label(&field, "nope").unwrap_err();
        assert!(err.contains("invalid enum label 'nope'"), "{err}");
        assert!(err.contains("active") && err.contains("blocked"), "{err}");
    }
}
