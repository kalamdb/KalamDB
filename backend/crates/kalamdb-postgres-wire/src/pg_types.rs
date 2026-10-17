//! Logical-type to PostgreSQL wire OID mapping shared by statement describe and row encoding.

use arrow::datatypes::{DataType, Field};
use kalamdb_commons::{
    conversions::{
        read_kalam_data_type_metadata, read_kalam_type_id_metadata, read_logical_type_modifier,
    },
    models::datatypes::{KalamDataType, LogicalTypeRef},
};
use pgwire::{
    api::Type,
    error::{ErrorInfo, PgWireError, PgWireResult},
};

/// Resolve the canonical [`LogicalTypeRef`] for an Arrow [`Field`], honoring Kalam field metadata
/// (`kalam_type_id`, `kalam_type_modifier`, `kalam_data_type`) before falling back to the
/// physical Arrow [`DataType`].
pub fn logical_type_ref_for_field(field: &Field) -> PgWireResult<LogicalTypeRef> {
    match field.data_type() {
        DataType::List(item) | DataType::LargeList(item) => {
            return Ok(LogicalTypeRef::list(logical_type_ref_for_field(item)?, item.is_nullable()));
        },
        DataType::FixedSizeList(item, size)
            if matches!(item.data_type(), DataType::Float32) && *size > 0 =>
        {
            return Ok(LogicalTypeRef::Builtin(KalamDataType::Embedding(
                u16::try_from(*size).unwrap_or(u16::MAX),
            )));
        },
        _ => {},
    }

    if let Some(type_id) = read_kalam_type_id_metadata(field) {
        return Ok(LogicalTypeRef::named(type_id));
    }
    if let Some(modifier) = read_logical_type_modifier(field) {
        return Ok(match modifier {
            "jsonb" => LogicalTypeRef::Jsonb,
            "timestamptz" => LogicalTypeRef::TimestampTz,
            _ => LogicalTypeRef::Builtin(KalamDataType::Text),
        });
    }
    if let Some(kalam_type) = read_kalam_data_type_metadata(field) {
        if matches!(kalam_type, KalamDataType::Bytes) {
            return Err(unsupported_arrow_type_error(field.data_type()));
        }
        return Ok(LogicalTypeRef::Builtin(kalam_type));
    }

    logical_type_ref_for_arrow(field.data_type())
}

/// Resolve the canonical [`LogicalTypeRef`] for an unadorned Arrow [`DataType`].
pub fn logical_type_ref_for_arrow(data_type: &DataType) -> PgWireResult<LogicalTypeRef> {
    Ok(match data_type {
        DataType::Null => LogicalTypeRef::Builtin(KalamDataType::Text),
        DataType::Boolean => LogicalTypeRef::Builtin(KalamDataType::Boolean),
        DataType::Int8 | DataType::Int16 => LogicalTypeRef::Builtin(KalamDataType::SmallInt),
        DataType::Int32 | DataType::UInt8 | DataType::UInt16 => {
            LogicalTypeRef::Builtin(KalamDataType::Int)
        },
        DataType::Int64 | DataType::UInt32 => LogicalTypeRef::Builtin(KalamDataType::BigInt),
        DataType::UInt64 => LogicalTypeRef::Builtin(KalamDataType::Text),
        DataType::Float32 => LogicalTypeRef::Builtin(KalamDataType::Float),
        DataType::Float64 => LogicalTypeRef::Builtin(KalamDataType::Double),
        DataType::Date32 | DataType::Date64 => LogicalTypeRef::Builtin(KalamDataType::Date),
        DataType::Time32(_) | DataType::Time64(_) => LogicalTypeRef::Builtin(KalamDataType::Time),
        DataType::Timestamp(_, Some(_)) => LogicalTypeRef::TimestampTz,
        DataType::Timestamp(_, None) => LogicalTypeRef::Builtin(KalamDataType::Timestamp),
        DataType::Utf8 | DataType::LargeUtf8 | DataType::Utf8View => {
            LogicalTypeRef::Builtin(KalamDataType::Text)
        },
        DataType::FixedSizeBinary(16) => LogicalTypeRef::Builtin(KalamDataType::Uuid),
        DataType::Decimal128(precision, scale) | DataType::Decimal256(precision, scale) => {
            LogicalTypeRef::Builtin(KalamDataType::Decimal {
                precision: *precision,
                scale:     (*scale).max(0) as u8,
            })
        },
        DataType::FixedSizeList(item, size)
            if matches!(item.data_type(), DataType::Float32) && *size > 0 =>
        {
            LogicalTypeRef::Builtin(KalamDataType::Embedding(
                u16::try_from(*size).unwrap_or(u16::MAX),
            ))
        },
        DataType::List(item) | DataType::LargeList(item) => {
            LogicalTypeRef::list(logical_type_ref_for_field(item)?, item.is_nullable())
        },
        other => return Err(unsupported_arrow_type_error(other)),
    })
}

/// Derive the PostgreSQL wire [`Type`] (OID + format descriptor) from a [`LogicalTypeRef`].
pub fn pg_type_for_logical_type(type_ref: &LogicalTypeRef) -> PgWireResult<Type> {
    Ok(match type_ref {
        LogicalTypeRef::Builtin(kalam_type) => match kalam_type {
            KalamDataType::Boolean => Type::BOOL,
            KalamDataType::SmallInt => Type::INT2,
            KalamDataType::Int => Type::INT4,
            KalamDataType::BigInt => Type::INT8,
            KalamDataType::Float => Type::FLOAT4,
            KalamDataType::Double => Type::FLOAT8,
            KalamDataType::Text | KalamDataType::File | KalamDataType::Decimal { .. } => Type::TEXT,
            KalamDataType::Timestamp => Type::TIMESTAMP,
            KalamDataType::DateTime => Type::TIMESTAMPTZ,
            KalamDataType::Date => Type::DATE,
            KalamDataType::Time => Type::TIME,
            KalamDataType::Uuid => Type::UUID,
            KalamDataType::Json => Type::JSON,
            KalamDataType::Embedding(_) => Type::FLOAT4_ARRAY,
            KalamDataType::Bytes => return Err(unsupported_logical_type_error(type_ref)),
        },
        LogicalTypeRef::Jsonb => Type::JSONB,
        LogicalTypeRef::TimestampTz => Type::TIMESTAMPTZ,
        LogicalTypeRef::Named { type_id } => pg_type_for_type_id(type_id),
        LogicalTypeRef::List { element, .. } => pg_array_type_for_logical_type(element)?,
        LogicalTypeRef::Map { .. } => return Err(unsupported_logical_type_error(type_ref)),
    })
}

/// Construct a PostgreSQL wire [`Type`] whose OID is the stable user-defined OID of `type_id`.
pub fn pg_type_for_type_id(type_id: &kalamdb_commons::models::TypeId) -> Type {
    let oid = type_id.pg_oid();
    Type::from_oid(oid).unwrap_or_else(|| {
        Type::new(type_id.as_str().to_string(), oid, Type::TEXT.kind().clone(), String::new())
    })
}

fn pg_array_type_for_logical_type(element: &LogicalTypeRef) -> PgWireResult<Type> {
    Ok(match element {
        LogicalTypeRef::Builtin(KalamDataType::Boolean) => Type::BOOL_ARRAY,
        LogicalTypeRef::Builtin(KalamDataType::SmallInt) => Type::INT2_ARRAY,
        LogicalTypeRef::Builtin(KalamDataType::Int) => Type::INT4_ARRAY,
        LogicalTypeRef::Builtin(KalamDataType::BigInt) => Type::INT8_ARRAY,
        LogicalTypeRef::Builtin(KalamDataType::Float) => Type::FLOAT4_ARRAY,
        LogicalTypeRef::Builtin(KalamDataType::Double) => Type::FLOAT8_ARRAY,
        LogicalTypeRef::Builtin(
            KalamDataType::Text | KalamDataType::File | KalamDataType::Decimal { .. },
        ) => Type::TEXT_ARRAY,
        LogicalTypeRef::Builtin(KalamDataType::Timestamp) => Type::TIMESTAMP_ARRAY,
        LogicalTypeRef::Builtin(KalamDataType::DateTime) | LogicalTypeRef::TimestampTz => {
            Type::TIMESTAMPTZ_ARRAY
        },
        LogicalTypeRef::Builtin(KalamDataType::Date) => Type::DATE_ARRAY,
        LogicalTypeRef::Builtin(KalamDataType::Time) => Type::TIME_ARRAY,
        LogicalTypeRef::Builtin(KalamDataType::Uuid) => Type::UUID_ARRAY,
        LogicalTypeRef::Builtin(KalamDataType::Json) => Type::JSON_ARRAY,
        LogicalTypeRef::Jsonb => Type::JSONB_ARRAY,
        LogicalTypeRef::Named { .. } => Type::TEXT_ARRAY,
        other => {
            return Err(unsupported_logical_type_error(&LogicalTypeRef::list(other.clone(), true)))
        },
    })
}

/// Derive the PostgreSQL wire [`Type`] for an Arrow [`Field`] via [`LogicalTypeRef`].
pub fn pg_type_for_field(field: &Field) -> PgWireResult<Type> {
    let logical_type = logical_type_ref_for_field(field)?;
    pg_type_for_logical_type(&logical_type)
}

/// Derive the PostgreSQL wire [`Type`] for an Arrow [`DataType`] via [`LogicalTypeRef`].
pub fn pg_type_for_arrow(data_type: &DataType) -> PgWireResult<Type> {
    let logical_type = logical_type_ref_for_arrow(data_type)?;
    pg_type_for_logical_type(&logical_type)
}

pub fn unsupported_arrow_type_error(data_type: &DataType) -> PgWireError {
    PgWireError::UserError(Box::new(ErrorInfo::new(
        "ERROR".to_string(),
        "0A000".to_string(),
        format!("unsupported Arrow type for PostgreSQL wire row encoding: {data_type}"),
    )))
}

pub fn unsupported_logical_type_error(type_ref: &LogicalTypeRef) -> PgWireError {
    PgWireError::UserError(Box::new(ErrorInfo::new(
        "ERROR".to_string(),
        "0A000".to_string(),
        format!("unsupported logical type for PostgreSQL wire row encoding: {type_ref:?}"),
    )))
}
