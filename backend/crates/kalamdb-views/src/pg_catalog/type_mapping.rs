use kalamdb_commons::datatypes::KalamDataType;

use crate::pg_catalog::catalog_type_oid;

/// PostgreSQL `pg_type.oid` for `void`.
pub(crate) const PG_VOID_OID: i64 = 2278;

/// One built-in `pg_type` row. OID, `typname`, `typlen`, and `format_type` stay together.
pub(crate) struct BuiltinPgType {
    pub typname:     &'static str,
    pub oid:         i64,
    pub typlen:      i64,
    pub format_type: &'static str,
}

pub(crate) const BUILTIN_PG_TYPES: &[BuiltinPgType] = &[
    BuiltinPgType {
        typname:     "bool",
        oid:         16,
        typlen:      1,
        format_type: "boolean",
    },
    BuiltinPgType {
        typname:     "bytea",
        oid:         17,
        typlen:      -1,
        format_type: "bytea",
    },
    BuiltinPgType {
        typname:     "name",
        oid:         19,
        typlen:      64,
        format_type: "name",
    },
    BuiltinPgType {
        typname:     "int8",
        oid:         20,
        typlen:      8,
        format_type: "bigint",
    },
    BuiltinPgType {
        typname:     "int2",
        oid:         21,
        typlen:      2,
        format_type: "smallint",
    },
    BuiltinPgType {
        typname:     "int4",
        oid:         23,
        typlen:      4,
        format_type: "integer",
    },
    BuiltinPgType {
        typname:     "text",
        oid:         25,
        typlen:      -1,
        format_type: "text",
    },
    BuiltinPgType {
        typname:     "bpchar",
        oid:         1042,
        typlen:      -1,
        format_type: "character",
    },
    BuiltinPgType {
        typname:     "varchar",
        oid:         1043,
        typlen:      -1,
        format_type: "character varying",
    },
    BuiltinPgType {
        typname:     "oid",
        oid:         26,
        typlen:      4,
        format_type: "oid",
    },
    BuiltinPgType {
        typname:     "json",
        oid:         114,
        typlen:      -1,
        format_type: "json",
    },
    BuiltinPgType {
        typname:     "float4",
        oid:         700,
        typlen:      4,
        format_type: "real",
    },
    BuiltinPgType {
        typname:     "float8",
        oid:         701,
        typlen:      8,
        format_type: "double precision",
    },
    BuiltinPgType {
        typname:     "date",
        oid:         1082,
        typlen:      4,
        format_type: "date",
    },
    BuiltinPgType {
        typname:     "time",
        oid:         1083,
        typlen:      8,
        format_type: "time without time zone",
    },
    BuiltinPgType {
        typname:     "timestamp",
        oid:         1114,
        typlen:      8,
        format_type: "timestamp without time zone",
    },
    BuiltinPgType {
        typname:     "numeric",
        oid:         1700,
        typlen:      -1,
        format_type: "numeric",
    },
    BuiltinPgType {
        typname:     "uuid",
        oid:         2950,
        typlen:      16,
        format_type: "uuid",
    },
    BuiltinPgType {
        typname:     "void",
        oid:         PG_VOID_OID,
        typlen:      4,
        format_type: "void",
    },
];

pub(crate) fn builtin_oid(typname: &str) -> i64 {
    BUILTIN_PG_TYPES
        .iter()
        .find(|entry| entry.typname == typname)
        .map(|entry| entry.oid)
        .expect("builtin pg type is registered")
}

pub(crate) fn builtin_typlen(oid: i64) -> i64 {
    BUILTIN_PG_TYPES
        .iter()
        .find(|entry| entry.oid == oid)
        .map(|entry| entry.typlen)
        .unwrap_or(-1)
}

fn extension_type_oid(name: &str) -> i64 {
    catalog_type_oid(kalamdb_commons::constants::PG_CATALOG_NAMESPACE, name)
}

/// PostgreSQL `pg_type.oid` for a Kalam column type.
pub(crate) fn pg_type_oid(data_type: &KalamDataType) -> i64 {
    match data_type {
        KalamDataType::File => extension_type_oid("file"),
        KalamDataType::Embedding(_) => extension_type_oid("embedding"),
        other => builtin_oid(pg_type_name(other)),
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
    if let Some(entry) = BUILTIN_PG_TYPES.iter().find(|entry| entry.oid == oid) {
        return entry.format_type;
    }
    if oid == extension_type_oid("file") {
        return "file";
    }
    if oid == extension_type_oid("embedding") {
        return "embedding";
    }
    if oid == extension_type_oid("geometry") {
        return "geometry";
    }
    "unknown"
}

/// PostgreSQL `pg_attribute` `(atttypid, atttypmod)` for a column.
///
/// `varchar(n)` is oid 1043 with typmod `n + 4`. `numeric(p, s)` packs
/// precision and scale the same way PostgreSQL does. Unlimited `text` stays
/// oid 25 with typmod `-1`.
pub(crate) fn pg_attribute_type(
    data_type: &KalamDataType,
    character_maximum_length: Option<u32>,
    modifier: Option<&str>,
) -> (i64, i64) {
    if modifier == Some("geometry") {
        return (extension_type_oid("geometry"), -1);
    }
    match data_type {
        KalamDataType::Text => match character_maximum_length {
            Some(length) => (1043, i64::from(length).saturating_add(4)),
            None => (pg_type_oid(data_type), -1),
        },
        KalamDataType::Decimal { precision, scale } => {
            let packed = (i64::from(*precision) << 16) | (i64::from(*scale) & 0xffff);
            (1700, packed.saturating_add(4))
        },
        other => (pg_type_oid(other), -1),
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
        KalamDataType::Text => "text",
        KalamDataType::File => "file",
        KalamDataType::Embedding(_) => "embedding",
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
        assert_eq!(arrow_info_type_to_kalam_sql_name("Timestamp(Microsecond, None)"), "TIMESTAMP");
    }

    #[test]
    fn udt_name_to_info_schema_data_type_maps_pg_names() {
        assert_eq!(udt_name_to_info_schema_data_type("int8"), "bigint");
        assert_eq!(udt_name_to_info_schema_data_type("text"), "text");
    }

    #[test]
    fn kalam_scalars_use_the_builtin_type_table() {
        let scalars = [
            KalamDataType::Boolean,
            KalamDataType::SmallInt,
            KalamDataType::Int,
            KalamDataType::BigInt,
            KalamDataType::Float,
            KalamDataType::Double,
            KalamDataType::Timestamp,
            KalamDataType::DateTime,
            KalamDataType::Date,
            KalamDataType::Time,
            KalamDataType::Decimal {
                precision: 10,
                scale:     2,
            },
            KalamDataType::Json,
            KalamDataType::Uuid,
            KalamDataType::Bytes,
            KalamDataType::Text,
        ];
        for data_type in scalars {
            let oid = pg_type_oid(&data_type);
            assert!(
                BUILTIN_PG_TYPES.iter().any(|entry| entry.oid == oid),
                "{data_type:?} oid {oid} is missing from the builtin table"
            );
            assert_ne!(pg_format_type(oid), "unknown");
        }
        assert_eq!(pg_format_type(pg_type_oid(&KalamDataType::File)), "file");
        assert_eq!(pg_format_type(pg_type_oid(&KalamDataType::Embedding(3))), "embedding");
    }

    #[test]
    fn varchar_limit_is_a_postgres_typmod() {
        assert_eq!(pg_attribute_type(&KalamDataType::Text, Some(255), None), (1043, 259));
        assert_eq!(pg_attribute_type(&KalamDataType::Text, None, None), (25, -1));
        assert_eq!(pg_format_type(1043), "character varying");
        assert_eq!(
            pg_attribute_type(
                &KalamDataType::Decimal {
                    precision: 10,
                    scale:     5,
                },
                None,
                None
            ),
            (1700, ((10_i64 << 16) | 5) + 4)
        );
        let (geometry_oid, geometry_typmod) =
            pg_attribute_type(&KalamDataType::Text, None, Some("geometry"));
        assert_eq!(geometry_typmod, -1);
        assert_eq!(pg_format_type(geometry_oid), "geometry");
    }
}
