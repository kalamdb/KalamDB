//! Interned layout for one TypeId revision.

use std::sync::Arc;

use arrow::datatypes::{DataType, Field, Fields};
use kalamdb_commons::{
    conversions::{
        with_kalam_data_type_metadata, with_kalam_domain_constraints, with_kalam_enum_labels,
        with_logical_type_modifier, DomainCheckSpec, DomainConstraintSpec,
    },
    models::{CatalogTypeKind, SchemaId, TypeId},
    LogicalTypeRef, TypeRevision,
};

/// Shared layout for one TypeId revision. Nested children are interned.
#[derive(Debug, Clone)]
pub struct ResolvedType {
    pub type_id:      TypeId,
    pub revision:     TypeRevision,
    pub namespace_id: SchemaId,
    pub name:         String,
    pub kind:         CatalogTypeKind,
    pub fields:       Arc<[ResolvedTypeField]>,
    /// Interned Arrow children for this type. Occurrence FieldRefs are built per column.
    pub arrow_fields: Arc<Fields>,
    pub arrow_type:   DataType,
}

impl ResolvedType {
    /// Peel domain wrappers to the underlying non-domain resolved type (or self if base is builtin).
    pub fn root_resolved_type(&self) -> &ResolvedType {
        let mut current = self;
        while current.kind == CatalogTypeKind::Domain {
            if let Some(base_nested) = current
                .fields
                .iter()
                .find(|field| field.name == "__base" && !field.dropped)
                .and_then(|field| field.nested.as_deref())
            {
                current = base_nested;
            } else {
                break;
            }
        }
        current
    }

    /// Peel domain wrappers to the innermost base [`LogicalTypeRef`].
    pub fn root_type_ref(&self) -> Option<&LogicalTypeRef> {
        if self.kind != CatalogTypeKind::Domain {
            return None;
        }
        let mut current = self;
        let mut last_ref = None;
        while current.kind == CatalogTypeKind::Domain {
            let Some(base_field) = current
                .fields
                .iter()
                .find(|field| field.name == "__base" && !field.dropped)
            else {
                break;
            };
            last_ref = Some(&base_field.type_ref);
            if let Some(nested) = base_field.nested.as_deref() {
                current = nested;
            } else {
                break;
            }
        }
        last_ref
    }

    /// Collect domain constraints from outermost base domain to this domain.
    pub fn domain_constraints(&self) -> Vec<DomainConstraintSpec> {
        if self.kind != CatalogTypeKind::Domain {
            return Vec::new();
        }
        let mut out = self
            .fields
            .iter()
            .find(|field| field.name == "__base" && !field.dropped)
            .and_then(|field| field.nested.as_deref())
            .map(ResolvedType::domain_constraints)
            .unwrap_or_default();

        let not_null = self
            .fields
            .iter()
            .find(|field| field.name == "__base" && !field.dropped)
            .map(|field| field.not_null)
            .unwrap_or(false);
        let default_expr = self
            .fields
            .iter()
            .find(|field| field.name == "__default" && !field.dropped)
            .and_then(|field| field.constraint_expr.clone());
        let checks = self
            .fields
            .iter()
            .filter(|field| field.name != "__base" && field.name != "__default" && !field.dropped)
            .map(|field| DomainCheckSpec {
                name:      field.name.clone(),
                expr:      field.constraint_expr.clone().unwrap_or_default(),
                validated: field.not_null,
            })
            .collect();

        out.push(DomainConstraintSpec {
            domain_name: self.name.clone(),
            not_null,
            default_expr,
            checks,
        });
        out
    }

    /// Attach enum labels and/or domain metadata onto a single occurrence `Field`.
    pub fn attach_metadata_to_occurrence(&self, mut field: Field) -> Field {
        let root = self.root_resolved_type();
        if root.kind == CatalogTypeKind::Enum {
            let labels: Vec<String> = root.fields.iter().map(|f| f.name.clone()).collect();
            field = with_kalam_enum_labels(field, &labels);
        }
        if self.kind == CatalogTypeKind::Domain {
            if let Some(root_ref) = self.root_type_ref() {
                if let Some(builtin) = root_ref.builtin() {
                    field = with_kalam_data_type_metadata(field, &builtin);
                }
                field = with_logical_type_modifier(field, root_ref);
            }
            let constraints = self.domain_constraints();
            field = with_kalam_domain_constraints(field, &constraints);
        }
        field
    }
}

/// Arrow layout for one [`LogicalTypeRef`], including builtin modifiers.
///
/// Named types point at an interned [`ResolvedType`]. The modifier stays on
/// `type_ref`; Arrow does not have a separate jsonb or timestamptz type.
#[derive(Debug, Clone)]
pub struct ResolvedLayout {
    pub type_ref:   LogicalTypeRef,
    pub arrow_type: DataType,
    pub nested:     Option<Arc<ResolvedType>>,
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
    pub constraint_expr:   Option<String>,
}
