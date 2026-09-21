//! Named-type OID lookup shared by describe and row encode.

use arrow::datatypes::Field;
use kalamdb_commons::conversions::read_kalam_type_id_metadata;
use pgwire::api::Type;

/// OID follows opaque TypeId when the Arrow field carries `kalam_type_id`.
pub fn pg_type_for_named_field(field: &Field) -> Option<Type> {
    let type_id = read_kalam_type_id_metadata(field)?;
    Some(Type::from_oid(type_id.pg_oid()).unwrap_or(Type::TEXT))
}
