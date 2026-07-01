//! Convert PostgreSQL wire bind parameters into DataFusion `ScalarValue`s.

use kalamdb_core::sql::ScalarValue;
use pgwire::{
    api::{portal::Portal, Type},
    error::PgWireResult,
};

/// Decode all portal parameters as DataFusion scalars.
///
/// DBeaver and other clients typically send metadata filters as text (`$1` = schema name).
pub fn portal_parameters_to_scalar_values<S>(
    portal: &Portal<S>,
    parameter_types: &[Type],
) -> PgWireResult<Vec<ScalarValue>>
where
    S: Clone,
{
    let mut values = Vec::with_capacity(portal.parameter_len());
    for index in 0..portal.parameter_len() {
        let parameter_type = parameter_types.get(index).unwrap_or(&Type::TEXT);
        values.push(parse_portal_parameter(portal, index, parameter_type)?);
    }
    Ok(values)
}

fn parse_portal_parameter<S>(
    portal: &Portal<S>,
    index: usize,
    parameter_type: &Type,
) -> PgWireResult<ScalarValue>
where
    S: Clone,
{
    match *parameter_type {
        Type::BOOL => {
            let value: Option<bool> = portal.parameter(index, &Type::BOOL)?;
            Ok(value.map_or(ScalarValue::Null, |value| ScalarValue::Boolean(Some(value))))
        },
        Type::INT2 => {
            let value: Option<i16> = portal.parameter(index, &Type::INT2)?;
            Ok(value.map_or(ScalarValue::Null, |value| ScalarValue::Int16(Some(value))))
        },
        Type::INT4 => {
            let value: Option<i32> = portal.parameter(index, &Type::INT4)?;
            Ok(value.map_or(ScalarValue::Null, |value| ScalarValue::Int32(Some(value))))
        },
        Type::INT8 => {
            let value: Option<i64> = portal.parameter(index, &Type::INT8)?;
            Ok(value.map_or(ScalarValue::Null, |value| ScalarValue::Int64(Some(value))))
        },
        Type::FLOAT4 => {
            let value: Option<f32> = portal.parameter(index, &Type::FLOAT4)?;
            Ok(value.map_or(ScalarValue::Null, |value| ScalarValue::Float32(Some(value))))
        },
        Type::FLOAT8 => {
            let value: Option<f64> = portal.parameter(index, &Type::FLOAT8)?;
            Ok(value.map_or(ScalarValue::Null, |value| ScalarValue::Float64(Some(value))))
        },
        _ => {
            let text: Option<String> = portal.parameter(index, &Type::TEXT)?;
            Ok(match text {
                None => ScalarValue::Null,
                Some(value) => ScalarValue::Utf8(Some(value)),
            })
        },
    }
}
