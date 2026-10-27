//! Conversions between [`DomainValue`] and DataFusion [`ScalarValue`].
//!
//! Centralizes binary domain scalar mapping alongside JSON and Arrow conversions in
//! `kalamdb-commons::conversions` so query, table, and function boundaries share one
//! implementation.

use arrow::array::Array;
use arrow_schema::DataType;
use datafusion_common::ScalarValue;

use crate::{
    conversions::arrow_conversion::ToArrowType,
    models::{datatypes::KalamDataType, DomainValue},
};

/// Convert a [`DomainValue`] into a DataFusion [`ScalarValue`], using `data_type` to
/// preserve declared list element types on empty arrays.
pub fn domain_value_to_scalar(
    value: &DomainValue,
    data_type: Option<&KalamDataType>,
) -> Result<ScalarValue, String> {
    match value {
        DomainValue::Omitted | DomainValue::SqlNull | DomainValue::Void => Ok(ScalarValue::Null),
        DomainValue::JsonNull => Ok(ScalarValue::Utf8(Some("null".to_string()))),
        DomainValue::Bool(v) => Ok(ScalarValue::Boolean(Some(*v))),
        DomainValue::Int8(v) => Ok(ScalarValue::Int8(Some(*v))),
        DomainValue::Int16(v) => Ok(ScalarValue::Int16(Some(*v))),
        DomainValue::Int32(v) => Ok(ScalarValue::Int32(Some(*v))),
        DomainValue::Int64(v) => Ok(ScalarValue::Int64(Some(*v))),
        DomainValue::UInt8(v) => Ok(ScalarValue::UInt8(Some(*v))),
        DomainValue::UInt16(v) => Ok(ScalarValue::UInt16(Some(*v))),
        DomainValue::UInt32(v) => Ok(ScalarValue::UInt32(Some(*v))),
        DomainValue::UInt64(v) => Ok(ScalarValue::UInt64(Some(*v))),
        DomainValue::Float32(v) => Ok(ScalarValue::Float32(Some(*v))),
        DomainValue::Float64(v) => Ok(ScalarValue::Float64(Some(*v))),
        DomainValue::Decimal128 {
            unscaled,
            precision,
            scale,
        } => Ok(ScalarValue::Decimal128(Some(*unscaled), *precision, *scale)),
        DomainValue::Date32(v) => Ok(ScalarValue::Date32(Some(*v))),
        DomainValue::TimeMicros(v) => Ok(ScalarValue::Time64Microsecond(Some(*v))),
        DomainValue::TimestampMicros(v) => Ok(ScalarValue::TimestampMicrosecond(Some(*v), None)),
        DomainValue::TimestampTzMicros(v) => Ok(ScalarValue::TimestampMicrosecond(
            Some(*v),
            Some(std::sync::Arc::<str>::from("UTC")),
        )),
        DomainValue::Uuid(bytes) => Ok(ScalarValue::FixedSizeBinary(16, Some(bytes.to_vec()))),
        DomainValue::Utf8(s) | DomainValue::Json(s) => Ok(ScalarValue::Utf8(Some(s.clone()))),
        DomainValue::Bytes(bytes) => Ok(ScalarValue::Binary(Some(bytes.clone()))),
        DomainValue::Enum { label, .. } => Ok(ScalarValue::Utf8(Some(label.clone()))),
        DomainValue::FileRef(file) => {
            let json = serde_json::to_string(file).map_err(|e| e.to_string())?;
            Ok(ScalarValue::Utf8(Some(json)))
        },
        DomainValue::Array(items) => scalar_list_from_domain(items, data_type),
        DomainValue::Object(_) | DomainValue::Composite(_) | DomainValue::RowSet(_) => {
            Err("object, composite, and rowset domain values require structured field binding"
                .to_string())
        },
    }
}

fn scalar_list_from_domain(
    items: &[DomainValue],
    data_type: Option<&KalamDataType>,
) -> Result<ScalarValue, String> {
    let mut scalars = Vec::with_capacity(items.len());
    for item in items {
        scalars.push(domain_value_to_scalar(item, data_type)?);
    }
    let elem_ty = if let Some(declared) = data_type.and_then(|ty| ty.to_arrow_type().ok()) {
        declared
    } else {
        let mut found = None;
        for scalar in &scalars {
            if scalar.is_null() {
                continue;
            }
            let ty = scalar.data_type();
            match &found {
                Some(previous) if previous != &ty => {
                    return Err(format!(
                        "array element type mismatch: {previous} then {}",
                        scalar.data_type()
                    ));
                },
                Some(_) => {},
                None => found = Some(ty),
            }
        }
        found.unwrap_or(DataType::Null)
    };
    let mut aligned = Vec::with_capacity(scalars.len());
    for scalar in scalars {
        aligned.push(align_list_element(scalar, &elem_ty)?);
    }
    Ok(ScalarValue::List(ScalarValue::new_list(&aligned, &elem_ty, true)))
}

fn align_list_element(scalar: ScalarValue, elem_ty: &DataType) -> Result<ScalarValue, String> {
    if scalar.is_null() {
        return ScalarValue::try_from(elem_ty).map_err(|error| error.to_string());
    }
    if scalar.data_type() == *elem_ty {
        return Ok(scalar);
    }
    scalar
        .cast_to(elem_ty)
        .map_err(|error| format!("array element type mismatch: {error}"))
}

/// Convert a DataFusion [`ScalarValue`] into a [`DomainValue`].
pub fn scalar_to_domain_value(scalar: &ScalarValue) -> Result<DomainValue, String> {
    Ok(match scalar {
        ScalarValue::Null => DomainValue::SqlNull,
        ScalarValue::Boolean(Some(v)) => DomainValue::Bool(*v),
        ScalarValue::Boolean(None) => DomainValue::SqlNull,
        ScalarValue::Int8(Some(v)) => DomainValue::Int8(*v),
        ScalarValue::Int16(Some(v)) => DomainValue::Int16(*v),
        ScalarValue::Int32(Some(v)) => DomainValue::Int32(*v),
        ScalarValue::Int64(Some(v)) => DomainValue::Int64(*v),
        ScalarValue::UInt8(Some(v)) => DomainValue::UInt8(*v),
        ScalarValue::UInt16(Some(v)) => DomainValue::UInt16(*v),
        ScalarValue::UInt32(Some(v)) => DomainValue::UInt32(*v),
        ScalarValue::UInt64(Some(v)) => DomainValue::UInt64(*v),
        ScalarValue::Float32(Some(v)) => DomainValue::Float32(*v),
        ScalarValue::Float64(Some(v)) => DomainValue::Float64(*v),
        ScalarValue::Decimal128(Some(unscaled), precision, scale) => DomainValue::Decimal128 {
            unscaled: *unscaled,
            precision: *precision,
            scale: *scale,
        },
        ScalarValue::Date32(Some(v)) => DomainValue::Date32(*v),
        ScalarValue::Time64Microsecond(Some(v)) => DomainValue::TimeMicros(*v),
        ScalarValue::TimestampMillisecond(Some(ms), tz) => {
            timestamp_domain(ms.saturating_mul(1_000), tz.is_some())
        },
        ScalarValue::TimestampMicrosecond(Some(us), tz) => timestamp_domain(*us, tz.is_some()),
        ScalarValue::TimestampNanosecond(Some(ns), tz) => {
            timestamp_domain(ns.div_euclid(1_000), tz.is_some())
        },
        ScalarValue::FixedSizeBinary(16, Some(bytes)) if bytes.len() == 16 => {
            let mut arr = [0u8; 16];
            arr.copy_from_slice(bytes);
            DomainValue::Uuid(arr)
        },
        ScalarValue::Binary(Some(bytes))
        | ScalarValue::LargeBinary(Some(bytes))
        | ScalarValue::BinaryView(Some(bytes))
        | ScalarValue::FixedSizeBinary(_, Some(bytes)) => DomainValue::Bytes(bytes.clone()),
        ScalarValue::Utf8(Some(s))
        | ScalarValue::LargeUtf8(Some(s))
        | ScalarValue::Utf8View(Some(s)) => DomainValue::Utf8(s.clone()),
        ScalarValue::Struct(struct_arr) => {
            if struct_arr.is_empty() || struct_arr.is_null(0) {
                return Ok(DomainValue::SqlNull);
            }
            let mut fields = Vec::with_capacity(struct_arr.num_columns());
            for (col_idx, field) in struct_arr.fields().iter().enumerate() {
                let cell = ScalarValue::try_from_array(struct_arr.column(col_idx), 0)
                    .map_err(|e| e.to_string())?;
                fields.push((field.name().to_string(), scalar_to_domain_value(&cell)?));
            }
            DomainValue::Object(fields)
        },
        ScalarValue::List(list) => return list_scalar_to_domain(list.as_ref(), list.value(0)),
        ScalarValue::LargeList(list) => return list_scalar_to_domain(list.as_ref(), list.value(0)),
        ScalarValue::FixedSizeList(list) => {
            return list_scalar_to_domain(list.as_ref(), list.value(0))
        },
        ScalarValue::Dictionary(_, value) => return scalar_to_domain_value(value),
        other if other.is_null() => DomainValue::SqlNull,
        other => return Err(format!("unsupported scalar for domain encoding: {other:?}")),
    })
}

fn timestamp_domain(micros: i64, with_timezone: bool) -> DomainValue {
    if with_timezone {
        DomainValue::TimestampTzMicros(micros)
    } else {
        DomainValue::TimestampMicros(micros)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn null_leading_array_uses_the_first_typed_element() {
        let value = DomainValue::Array(vec![DomainValue::SqlNull, DomainValue::Int32(1)]);
        let scalar = domain_value_to_scalar(&value, Some(&KalamDataType::Int)).unwrap();
        let ScalarValue::List(list) = scalar else {
            panic!("expected list, got {scalar:?}");
        };
        assert!(!list.is_null(0));
        assert_eq!(list.value(0).len(), 2);
        assert!(list.value(0).is_null(0));
    }

    #[test]
    fn mixed_array_elements_are_rejected() {
        let value = DomainValue::Array(vec![DomainValue::Utf8("x".into()), DomainValue::Int32(1)]);
        let error = domain_value_to_scalar(&value, None).unwrap_err();
        assert!(error.contains("array element type mismatch"), "{error}");
    }

    #[test]
    fn timestamp_without_zone_stays_untimed() {
        let scalar = ScalarValue::TimestampMicrosecond(Some(42), None);
        assert_eq!(scalar_to_domain_value(&scalar).unwrap(), DomainValue::TimestampMicros(42));
    }

    #[test]
    fn struct_keeps_field_names() {
        let fields = arrow_schema::Fields::from(vec![arrow_schema::Field::new(
            "ok",
            DataType::Boolean,
            true,
        )]);
        let column = ScalarValue::Boolean(Some(true)).to_array().unwrap();
        let struct_array = arrow::array::StructArray::new(fields, vec![column], None);
        let scalar = ScalarValue::Struct(std::sync::Arc::new(struct_array));
        assert_eq!(
            scalar_to_domain_value(&scalar).unwrap(),
            DomainValue::Object(vec![("ok".into(), DomainValue::Bool(true))])
        );
    }
}

fn list_scalar_to_domain(
    list: &dyn Array,
    values: std::sync::Arc<dyn Array>,
) -> Result<DomainValue, String> {
    if list.is_empty() || list.is_null(0) {
        return Ok(DomainValue::SqlNull);
    }
    let mut items = Vec::with_capacity(values.len());
    for idx in 0..values.len() {
        let cell = ScalarValue::try_from_array(values.as_ref(), idx).map_err(|e| e.to_string())?;
        items.push(scalar_to_domain_value(&cell)?);
    }
    Ok(DomainValue::Array(items))
}
