use kalamdb_commons::datatypes::KalamDataType;

/// PostgreSQL `pg_type.oid` for a Kalam column type.
pub(crate) fn pg_type_oid(data_type: &KalamDataType) -> i64 {
    match data_type {
        KalamDataType::Boolean => 16,
        KalamDataType::SmallInt => 21,
        KalamDataType::Int => 23,
        KalamDataType::BigInt => 20,
        KalamDataType::Float => 700,
        KalamDataType::Double => 701,
        KalamDataType::Timestamp | KalamDataType::DateTime => 1114,
        KalamDataType::Date => 1082,
        KalamDataType::Time => 1083,
        KalamDataType::Decimal { .. } => 1700,
        KalamDataType::Json => 114,
        KalamDataType::Uuid => 2950,
        KalamDataType::Bytes => 17,
        KalamDataType::Text | KalamDataType::Embedding(_) | KalamDataType::File => 25,
    }
}

/// SQL `information_schema.columns.data_type` for a Kalam column type.
pub(crate) fn info_schema_data_type(data_type: &KalamDataType) -> &'static str {
    match data_type {
        KalamDataType::Boolean => "boolean",
        KalamDataType::SmallInt => "smallint",
        KalamDataType::Int => "integer",
        KalamDataType::BigInt => "bigint",
        KalamDataType::Float => "real",
        KalamDataType::Double => "double precision",
        KalamDataType::Timestamp | KalamDataType::DateTime => "timestamp without time zone",
        KalamDataType::Date => "date",
        KalamDataType::Time => "time without time zone",
        KalamDataType::Decimal { .. } => "numeric",
        KalamDataType::Json => "json",
        KalamDataType::Uuid => "uuid",
        KalamDataType::Bytes => "bytea",
        KalamDataType::Text | KalamDataType::Embedding(_) | KalamDataType::File => "text",
    }
}

/// Numeric metadata for `information_schema.columns` (precision, radix, scale).
pub(crate) fn info_schema_numeric_metadata(
    data_type: &KalamDataType,
) -> (Option<u64>, Option<u64>, Option<u64>) {
    match data_type {
        KalamDataType::SmallInt => (Some(16), Some(2), None),
        KalamDataType::Int => (Some(32), Some(2), None),
        KalamDataType::BigInt => (Some(64), Some(2), None),
        KalamDataType::Float => (Some(24), Some(2), None),
        KalamDataType::Double => (Some(53), Some(2), None),
        KalamDataType::Decimal { precision, scale } => {
            (Some(*precision as u64), Some(10), Some(*scale as u64))
        },
        _ => (None, None, None),
    }
}

/// Map DataFusion `information_schema.columns.data_type` strings to PostgreSQL `udt_name`.
pub(crate) fn arrow_info_type_to_udt_name(data_type: &str) -> &'static str {
    match data_type {
        "Boolean" => "bool",
        "Int8" | "UInt8" => "int2",
        "Int16" | "UInt16" => "int2",
        "Int32" | "UInt32" => "int4",
        "Int64" | "UInt64" => "int8",
        "Float32" => "float4",
        "Float64" => "float8",
        _ if data_type.starts_with("Timestamp") => "timestamp",
        _ if data_type.starts_with("Date") => "date",
        _ if data_type.starts_with("Time") => "time",
        _ if data_type.starts_with("Decimal") => "numeric",
        "Utf8" | "LargeUtf8" => "text",
        "Binary" | "LargeBinary" | "FixedSizeBinary(_)" => "bytea",
        "List(_)" | "LargeList(_)" | "FixedSizeList(_, _)" => "_text",
        _ if data_type.starts_with("List(") => "_text",
        _ => "text",
    }
}

/// Map Arrow physical type names to Kalam SQL type names for user-facing display.
pub(crate) fn arrow_info_type_to_kalam_sql_name(data_type: &str) -> String {
    match data_type {
        "Boolean" => "BOOLEAN".to_string(),
        "Int8" | "UInt8" | "Int16" | "UInt16" => "SMALLINT".to_string(),
        "Int32" | "UInt32" => "INT".to_string(),
        "Int64" | "UInt64" => "BIGINT".to_string(),
        "Float32" => "FLOAT".to_string(),
        "Float64" => "DOUBLE".to_string(),
        arrow if arrow.starts_with("Timestamp") => "TIMESTAMP".to_string(),
        arrow if arrow.starts_with("Date") && !arrow.starts_with("DateTime") => "DATE".to_string(),
        arrow if arrow.starts_with("Time") => "TIME".to_string(),
        arrow if arrow.starts_with("Decimal") => "DECIMAL".to_string(),
        "Utf8" | "LargeUtf8" => "TEXT".to_string(),
        "Binary" | "LargeBinary" | "FixedSizeBinary(_)" => "BYTES".to_string(),
        arrow if arrow.starts_with("List(") => "JSON".to_string(),
        _ => "TEXT".to_string(),
    }
}

/// Map PostgreSQL `udt_name` to SQL `information_schema.columns.data_type` values.
pub(crate) fn udt_name_to_info_schema_data_type(udt_name: &str) -> &'static str {
    match udt_name {
        "bool" => "boolean",
        "int2" => "smallint",
        "int4" => "integer",
        "int8" => "bigint",
        "float4" => "real",
        "float8" => "double precision",
        "timestamp" => "timestamp without time zone",
        "date" => "date",
        "time" => "time without time zone",
        "numeric" => "numeric",
        "json" => "json",
        "uuid" => "uuid",
        "bytea" => "bytea",
        _ => "text",
    }
}

/// PostgreSQL `format_type()` display name for a `pg_type.oid`.
pub fn pg_format_type(oid: i64) -> &'static str {
    match oid {
        16 => "boolean",
        17 => "bytea",
        20 => "bigint",
        21 => "smallint",
        23 => "integer",
        25 => "text",
        114 => "json",
        700 => "real",
        701 => "double precision",
        1082 => "date",
        1083 => "time without time zone",
        1114 => "timestamp without time zone",
        1700 => "numeric",
        2950 => "uuid",
        _ => "unknown",
    }
}

/// PostgreSQL `pg_type.typname` for a Kalam column type.
pub(crate) fn pg_type_name(data_type: &KalamDataType) -> &'static str {
    match data_type {
        KalamDataType::Boolean => "bool",
        KalamDataType::SmallInt => "int2",
        KalamDataType::Int => "int4",
        KalamDataType::BigInt => "int8",
        KalamDataType::Float => "float4",
        KalamDataType::Double => "float8",
        KalamDataType::Timestamp | KalamDataType::DateTime => "timestamp",
        KalamDataType::Date => "date",
        KalamDataType::Time => "time",
        KalamDataType::Decimal { .. } => "numeric",
        KalamDataType::Json => "json",
        KalamDataType::Uuid => "uuid",
        KalamDataType::Bytes => "bytea",
        KalamDataType::Text | KalamDataType::Embedding(_) | KalamDataType::File => "text",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arrow_info_type_to_kalam_sql_name_maps_physical_types() {
        assert_eq!(arrow_info_type_to_kalam_sql_name("Int64"), "BIGINT");
        assert_eq!(arrow_info_type_to_kalam_sql_name("Utf8"), "TEXT");
        assert_eq!(arrow_info_type_to_kalam_sql_name("Boolean"), "BOOLEAN");
        assert_eq!(
            arrow_info_type_to_kalam_sql_name("Timestamp(Microsecond, None)"),
            "TIMESTAMP"
        );
    }

    #[test]
    fn udt_name_to_info_schema_data_type_maps_pg_names() {
        assert_eq!(udt_name_to_info_schema_data_type("int8"), "bigint");
        assert_eq!(udt_name_to_info_schema_data_type("text"), "text");
    }
}
