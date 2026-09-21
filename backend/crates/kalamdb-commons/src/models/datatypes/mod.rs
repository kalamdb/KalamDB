//! Canonical catalog type system.
//!
//! [`KalamDataType`] is the only place to add a builtin. Table columns, CREATE
//! TYPE fields, routine parameters/returns, and typed defaults all store this
//! enum. Named `CREATE TYPE` values are [`crate::models::TypeId`] plus
//! [`LogicalTypeRef`] (same vocabulary as [`crate::schemas::ColumnDefinition`]),
//! not a separate crate — catalog rows live in `kalamdb-system`, the runtime
//! cache in `kalamdb-core::schema_registry::TypeRegistry`.
//!
//! ```text
//! SQL name / sqlparser  →  KalamDataType  →  Arrow DataType
//!                                       ↘  WireFormat (RocksDB / catalogs)
//!                                       ↘  StorageDataType (row codec)
//! ```

pub mod kalam_data_type;
pub mod logical_type_ref;
pub mod type_graph;
pub mod wire_format;

pub use kalam_data_type::KalamDataType;
pub use logical_type_ref::LogicalTypeRef;
pub use type_graph::{
    assert_finite_type_graph, assert_finite_type_graph_with_limits, TypeGraphError, TypeGraphNode,
    MAX_RESOLVED_ARROW_FIELDS, MAX_TYPE_GRAPH_DEPTH,
};
pub use wire_format::{WireFormat, WireFormatError};

#[cfg(feature = "arrow-conversion")]
pub use crate::conversions::arrow_conversion::{ArrowConversionError, FromArrowType, ToArrowType};
