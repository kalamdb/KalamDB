//! Canonical domain value model shared across backend, `link-common`, WASM, and FRB.

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

/// Typed stored file reference in domain payloads (`FR-005`, `FR-040`).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct DomainFileRef {
    pub id: String,
    pub sub: String,
    pub name: String,
    pub size: i64,
    pub mime: String,
    pub sha256: String,
    pub shard: Option<i32>,
}

/// Canonical domain value model shared across backend, `link-common`, WASM, and FRB (`FR-040`,
/// `FR-048`).
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum DomainValue {
    /// Field was omitted from input/patch; distinct from `SqlNull` and `JsonNull`.
    Omitted,
    /// SQL `NULL` value.
    SqlNull,
    /// JSON `null` value inside a `JSON`/`JSONB` column or parameter.
    JsonNull,
    /// `VOID` result from a procedure or void-returning function.
    Void,
    Bool(bool),
    Int8(i8),
    Int16(i16),
    Int32(i32),
    Int64(i64),
    UInt8(u8),
    UInt16(u16),
    UInt32(u32),
    UInt64(u64),
    Float32(f32),
    Float64(f64),
    Decimal128 {
        unscaled: i128,
        precision: u8,
        scale: i8,
    },
    Date32(i32),
    TimeMicros(i64),
    TimestampMicros(i64),
    TimestampTzMicros(i64),
    Uuid([u8; 16]),
    Utf8(String),
    Bytes(Vec<u8>),
    Json(String),
    FileRef(DomainFileRef),
    Enum {
        type_id: u32,
        label: String,
    },
    /// Ordinal-tagged composite fields `Vec<(slot, value)>`.
    Composite(Vec<(u16, DomainValue)>),
    /// Named record. Keys are the caller-visible field names, in declaration order.
    Object(Vec<(String, DomainValue)>),
    /// Homogeneous or composite array elements.
    Array(Vec<DomainValue>),
    /// Tabular row set (`SETOF` / `RETURNS TABLE` / query rows).
    RowSet(Vec<Vec<(u16, DomainValue)>>),
}
