//! Centralized datatype and value conversion utilities
//!
//! This module provides unified conversion functions for mapping between different data
//! representations (ScalarValue, bytes, Arrow types, etc.) used throughout the KalamDB codebase.
//!
//! The goal is to eliminate duplication of similar conversion logic scattered across multiple files
//! and provide a single source of truth for all datatype conversions.

use datafusion_common::ScalarValue;

/// Encode a scalar value to bytes for use in index keys or storage
///
/// Converts ScalarValue to a byte representation suitable for use as:
/// - Primary key index keys
/// - Secondary index keys
/// - Storage key prefixes
///
/// Width of [`order_preserving_i128_bytes`].
pub const ORDER_PRESERVING_INT_LEN: usize = 16;

/// # Numeric Types
/// Integers use a fixed-width order-preserving encoding so index range seeks
/// match numeric order. Decimal text does not: `"10"` sorts before `"2"`.
/// `Int32(10)` and `Int64(10)` encode to the same bytes.
///
/// # String Types
/// UTF-8 strings are converted directly to bytes.
///
/// # Other Types
/// All other types are converted to their string representation then bytes.
///
/// # Example
///
/// ```rust,ignore
/// use kalamdb_commons::conversions::scalar_value_to_bytes;
/// use arrow_schema::ScalarValue;
///
/// let small = scalar_value_to_bytes(&ScalarValue::Int32(Some(2)));
/// let large = scalar_value_to_bytes(&ScalarValue::Int64(Some(10)));
/// assert!(small < large);
///
/// let str_value = ScalarValue::Utf8(Some("hello".to_string()));
/// let str_bytes = scalar_value_to_bytes(&str_value);
/// assert_eq!(str_bytes, b"hello".to_vec());
/// ```
pub fn scalar_value_to_bytes(value: &ScalarValue) -> Vec<u8> {
    if value.is_null() {
        return Vec::new();
    }
    if let Some(integer) = scalar_index_integer(value) {
        return order_preserving_i128_bytes(integer).to_vec();
    }
    match value {
        ScalarValue::Utf8(Some(s))
        | ScalarValue::LargeUtf8(Some(s))
        | ScalarValue::Utf8View(Some(s)) => s.as_bytes().to_vec(),
        _ => value.to_string().into_bytes(),
    }
}

/// Integer value of a scalar that can share one index encoding.
pub fn scalar_index_integer(value: &ScalarValue) -> Option<i128> {
    match value {
        ScalarValue::Int8(Some(n)) => Some(i128::from(*n)),
        ScalarValue::Int16(Some(n)) => Some(i128::from(*n)),
        ScalarValue::Int32(Some(n)) => Some(i128::from(*n)),
        ScalarValue::Int64(Some(n)) => Some(i128::from(*n)),
        ScalarValue::UInt8(Some(n)) => Some(i128::from(*n)),
        ScalarValue::UInt16(Some(n)) => Some(i128::from(*n)),
        ScalarValue::UInt32(Some(n)) => Some(i128::from(*n)),
        ScalarValue::UInt64(Some(n)) => Some(i128::from(*n)),
        _ => None,
    }
}

/// Big-endian sign-magnitude bytes. Numeric order is byte order, including negatives.
pub fn order_preserving_i128_bytes(value: i128) -> [u8; ORDER_PRESERVING_INT_LEN] {
    ((value as u128) ^ (1u128 << 127)).to_be_bytes()
}

/// Inverse of [`order_preserving_i128_bytes`]. `None` when `bytes` is not 16 bytes long.
pub fn decode_order_preserving_i128(bytes: &[u8]) -> Option<i128> {
    let raw: [u8; ORDER_PRESERVING_INT_LEN] = bytes.try_into().ok()?;
    Some((u128::from_be_bytes(raw) ^ (1u128 << 127)) as i128)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_encode_int64() {
        let value = ScalarValue::Int64(Some(12345));
        let bytes = scalar_value_to_bytes(&value);
        assert_eq!(bytes, order_preserving_i128_bytes(12345).to_vec());
    }

    #[test]
    fn integer_index_bytes_sort_numerically_across_widths() {
        let encoded = |n: i128| order_preserving_i128_bytes(n).to_vec();
        assert!(encoded(-2) < encoded(-1));
        assert!(encoded(-1) < encoded(0));
        assert!(encoded(0) < encoded(2));
        assert!(encoded(2) < encoded(10));
        assert!(encoded(10) < encoded(20));
        assert_eq!(
            scalar_value_to_bytes(&ScalarValue::Int32(Some(10))),
            scalar_value_to_bytes(&ScalarValue::Int64(Some(10)))
        );
        assert_eq!(decode_order_preserving_i128(&encoded(-42)), Some(-42));
    }

    #[test]
    fn test_encode_utf8() {
        let value = ScalarValue::Utf8(Some("hello".to_string()));
        let bytes = scalar_value_to_bytes(&value);
        assert_eq!(bytes, b"hello".to_vec());
    }

    #[test]
    fn test_encode_null() {
        let value = ScalarValue::Utf8(None);
        let bytes = scalar_value_to_bytes(&value);
        assert_eq!(bytes, b"".to_vec());
    }

    #[test]
    fn test_encode_uint64() {
        let value = ScalarValue::UInt64(Some(999));
        let bytes = scalar_value_to_bytes(&value);
        assert_eq!(bytes, order_preserving_i128_bytes(999).to_vec());
    }

    #[test]
    fn test_encode_int32() {
        let value = ScalarValue::Int32(Some(-42));
        let bytes = scalar_value_to_bytes(&value);
        assert_eq!(bytes, order_preserving_i128_bytes(-42).to_vec());
        assert!(bytes < scalar_value_to_bytes(&ScalarValue::Int32(Some(0))));
    }
}
