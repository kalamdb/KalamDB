//! Interned layout for one TypeId revision.

use std::sync::Arc;

use arrow::datatypes::{DataType, Fields};
use kalamdb_commons::{
    models::{CatalogTypeKind, NamespaceId, TypeId},
    LogicalTypeRef, TypeRevision,
};

/// Shared layout for one TypeId revision. Nested children are interned.
#[derive(Debug, Clone)]
pub struct ResolvedType {
    pub type_id:      TypeId,
    pub revision:     TypeRevision,
    pub namespace_id: NamespaceId,
    pub name:         String,
    pub kind:         CatalogTypeKind,
    pub fields:       Arc<[ResolvedTypeField]>,
    /// Interned Arrow children for this type. Occurrence FieldRefs are built per column.
    pub arrow_fields: Arc<Fields>,
    pub arrow_type:   DataType,
}

#[derive(Debug, Clone)]
pub struct ResolvedTypeField {
    pub name:              String,
    pub slot:              i32,
    pub ordinal:           i32,
    pub type_ref:          LogicalTypeRef,
    pub not_null:          bool,
    pub dropped:           bool,
    pub nested:            Option<Arc<ResolvedType>>,
    pub arrow_child_index: usize,
}
