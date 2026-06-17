use std::cmp::Ordering;

use super::StoredScalarValue;

/// Compare two stored scalar bounds when both sides share a comparable type.
pub fn stored_scalar_cmp(
    left: &StoredScalarValue,
    right: &StoredScalarValue,
) -> Option<Ordering> {
    match (left, right) {
        (StoredScalarValue::Int8(Some(left)), StoredScalarValue::Int8(Some(right))) => {
            Some(left.cmp(right))
        },
        (StoredScalarValue::Int16(Some(left)), StoredScalarValue::Int16(Some(right))) => {
            Some(left.cmp(right))
        },
        (StoredScalarValue::Int32(Some(left)), StoredScalarValue::Int32(Some(right))) => {
            Some(left.cmp(right))
        },
        (StoredScalarValue::Int64(Some(left)), StoredScalarValue::Int64(Some(right))) => {
            Some(left.parse::<i64>().ok()?.cmp(&right.parse::<i64>().ok()?))
        },
        (StoredScalarValue::UInt8(Some(left)), StoredScalarValue::UInt8(Some(right))) => {
            Some(left.cmp(right))
        },
        (StoredScalarValue::UInt16(Some(left)), StoredScalarValue::UInt16(Some(right))) => {
            Some(left.cmp(right))
        },
        (StoredScalarValue::UInt32(Some(left)), StoredScalarValue::UInt32(Some(right))) => {
            Some(left.cmp(right))
        },
        (StoredScalarValue::UInt64(Some(left)), StoredScalarValue::UInt64(Some(right))) => {
            Some(left.parse::<u64>().ok()?.cmp(&right.parse::<u64>().ok()?))
        },
        (StoredScalarValue::Utf8(Some(left)), StoredScalarValue::Utf8(Some(right)))
        | (StoredScalarValue::LargeUtf8(Some(left)), StoredScalarValue::LargeUtf8(Some(right))) => {
            Some(left.cmp(right))
        },
        (StoredScalarValue::Float32(Some(left)), StoredScalarValue::Float32(Some(right))) => {
            left.partial_cmp(right)
        },
        (StoredScalarValue::Float64(Some(left)), StoredScalarValue::Float64(Some(right))) => {
            left.partial_cmp(right)
        },
        (StoredScalarValue::Boolean(Some(left)), StoredScalarValue::Boolean(Some(right))) => {
            Some(left.cmp(right))
        },
        (StoredScalarValue::Date32(Some(left)), StoredScalarValue::Date32(Some(right))) => {
            Some(left.cmp(right))
        },
        (
            StoredScalarValue::Time64Microsecond(Some(left)),
            StoredScalarValue::Time64Microsecond(Some(right)),
        ) => Some(left.cmp(right)),
        (
            StoredScalarValue::TimestampMillisecond {
                value: Some(left), ..
            },
            StoredScalarValue::TimestampMillisecond {
                value: Some(right), ..
            },
        )
        | (
            StoredScalarValue::TimestampMicrosecond {
                value: Some(left), ..
            },
            StoredScalarValue::TimestampMicrosecond {
                value: Some(right), ..
            },
        )
        | (
            StoredScalarValue::TimestampNanosecond {
                value: Some(left), ..
            },
            StoredScalarValue::TimestampNanosecond {
                value: Some(right), ..
            },
        ) => Some(left.cmp(right)),
        _ => None,
    }
}

/// Keep the smaller of two optional stored scalar bounds.
pub fn choose_min_stored_scalar(
    current: Option<StoredScalarValue>,
    next: Option<StoredScalarValue>,
) -> Option<StoredScalarValue> {
    choose_stored_scalar(current, next, Ordering::Less)
}

/// Keep the larger of two optional stored scalar bounds.
pub fn choose_max_stored_scalar(
    current: Option<StoredScalarValue>,
    next: Option<StoredScalarValue>,
) -> Option<StoredScalarValue> {
    choose_stored_scalar(current, next, Ordering::Greater)
}

fn choose_stored_scalar(
    current: Option<StoredScalarValue>,
    next: Option<StoredScalarValue>,
    preferred_ordering: Ordering,
) -> Option<StoredScalarValue> {
    match (current, next) {
        (None, value) | (value, None) => value,
        (Some(left), Some(right)) => match stored_scalar_cmp(&left, &right) {
            Some(ordering) if ordering == preferred_ordering => Some(left),
            Some(Ordering::Equal) => Some(left),
            Some(_) => Some(right),
            None => Some(left),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn min_and_max_merge_numeric_bounds() {
        let low = StoredScalarValue::Int32(Some(1));
        let high = StoredScalarValue::Int32(Some(9));

        assert_eq!(
            choose_min_stored_scalar(Some(low.clone()), Some(high.clone())),
            Some(low.clone())
        );
        assert_eq!(
            choose_max_stored_scalar(Some(low), Some(high)),
            Some(StoredScalarValue::Int32(Some(9)))
        );
    }
}
