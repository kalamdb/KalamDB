//! Compact physical type metadata cached with each v2 schema layout.

use datafusion_common::ScalarValue;

use super::{
    scalar::*,
    schema::StorageDataType,
    value::{decode_value, Reader},
};
use crate::error::{Result, SerializationError};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct FixedValueEncoding {
    tag: u8,
    precision: u8,
    scale: i8,
}

impl FixedValueEncoding {
    pub(crate) fn for_type(data_type: &StorageDataType) -> Option<Self> {
        let tag = match data_type {
            StorageDataType::Boolean => TAG_BOOL,
            StorageDataType::Int8 => TAG_I8,
            StorageDataType::Int16 => TAG_I16,
            StorageDataType::Int32 => TAG_I32,
            StorageDataType::Int64 => TAG_I64,
            StorageDataType::UInt8 => TAG_U8,
            StorageDataType::UInt16 => TAG_U16,
            StorageDataType::UInt32 => TAG_U32,
            StorageDataType::UInt64 => TAG_U64,
            StorageDataType::Float32 => TAG_F32,
            StorageDataType::Float64 => TAG_F64,
            StorageDataType::Date32 => TAG_DATE32,
            StorageDataType::Time64Microsecond => TAG_TIME64_US,
            StorageDataType::TimestampMillisecond => TAG_TS_MS,
            StorageDataType::TimestampMicrosecond => TAG_TS_US,
            StorageDataType::TimestampNanosecond => TAG_TS_NS,
            StorageDataType::Decimal { precision, scale } => {
                return Some(Self {
                    tag: TAG_DECIMAL128,
                    precision: *precision,
                    scale: *scale,
                });
            },
            _ => return None,
        };
        Some(Self {
            tag,
            precision: 0,
            scale: 0,
        })
    }

    /// Reuse the tagged row codec's checked schema coercions without a heap allocation.
    pub(crate) fn decode_as(
        &self,
        bytes: &[u8],
        expected: &StorageDataType,
    ) -> Result<ScalarValue> {
        let mut tagged = [0u8; 19]; // tag + decimal metadata + largest fixed value (i128)
        tagged[0] = self.tag;
        let start = if self.tag == TAG_DECIMAL128 {
            tagged[1] = self.precision;
            tagged[2] = self.scale as u8;
            3
        } else {
            1
        };
        let end = start + bytes.len();
        tagged
            .get_mut(start..end)
            .ok_or(SerializationError::Truncated)?
            .copy_from_slice(bytes);
        let mut reader = Reader::new(&tagged[..end]);
        let value = decode_value(&mut reader, expected)?;
        if !reader.is_empty() {
            return Err(SerializationError::Decode("invalid v2 fixed value width".to_string()));
        }
        Ok(value)
    }
}
