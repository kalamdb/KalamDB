//! Catalog load and intern for TypeRegistry.

use std::sync::Arc;

use arrow::datatypes::{DataType, Field, Fields};
use kalamdb_commons::{
    conversions::{
        with_kalam_data_type_metadata, with_kalam_type_id_metadata, with_logical_type_modifier,
    },
    datatypes::{KalamDataType, ToArrowType},
    models::{CatalogTypeKind, TypeId},
    LogicalTypeRef, TypeRevision,
};
use kalamdb_system::{CatalogStores, CatalogType, CatalogTypeField};

use super::{ResolvedLayout, ResolvedType, ResolvedTypeField, RevisionKey, TypeRegistry};
use crate::error::KalamDbError;

impl TypeRegistry {
    /// Resolve a logical type to an Arrow layout. Named types come from the
    /// interned revision cache. Builtin modifiers stay on the logical type.
    pub fn resolve(
        &self,
        stores: &CatalogStores,
        type_ref: &LogicalTypeRef,
    ) -> Result<ResolvedLayout, KalamDbError> {
        match type_ref {
            LogicalTypeRef::Builtin(_)
            | LogicalTypeRef::Jsonb
            | LogicalTypeRef::TimestampTz
            | LogicalTypeRef::Geometry => Ok(ResolvedLayout {
                type_ref:   type_ref.clone(),
                arrow_type: arrow_type_for_ref(type_ref, None)?,
                nested:     None,
            }),
            LogicalTypeRef::Named { type_id } => {
                let nested = self.get_or_load(stores, type_id)?;
                Ok(ResolvedLayout {
                    type_ref:   type_ref.clone(),
                    arrow_type: nested.arrow_type.clone(),
                    nested:     Some(nested),
                })
            },
            LogicalTypeRef::List {
                element,
                element_nullable,
            } => {
                let inner = self.resolve(stores, element)?;
                let mut item = Field::new("item", inner.arrow_type, *element_nullable);
                if let Some(builtin) = element.builtin() {
                    item = with_kalam_data_type_metadata(item, &builtin);
                }
                item = with_logical_type_modifier(item, element);
                if let Some(type_id) = element.named_type_id() {
                    item = with_kalam_type_id_metadata(item, type_id);
                }
                if let Some(resolved) = &inner.nested {
                    if !element.is_array() {
                        item = resolved.attach_metadata_to_occurrence(item);
                    }
                }
                Ok(ResolvedLayout {
                    type_ref:   type_ref.clone(),
                    arrow_type: DataType::List(Arc::new(item)),
                    nested:     inner.nested,
                })
            },
            LogicalTypeRef::Map { .. } => Err(KalamDbError::InvalidSql(
                "MAP types are reserved and cannot be stored".to_string(),
            )),
        }
    }

    pub(super) fn load_current(
        &self,
        stores: &CatalogStores,
        type_id: &TypeId,
    ) -> Result<Arc<ResolvedType>, KalamDbError> {
        let mut loading = Vec::new();
        let resolved = self.load_recursive(stores, type_id, &mut loading)?;
        self.publish_current(Arc::clone(&resolved));
        Ok(resolved)
    }

    fn load_recursive(
        &self,
        stores: &CatalogStores,
        type_id: &TypeId,
        loading: &mut Vec<TypeId>,
    ) -> Result<Arc<ResolvedType>, KalamDbError> {
        if let Some(cached) = self.current(type_id) {
            return Ok(cached);
        }
        if loading.iter().any(|id| id == type_id) {
            return Err(KalamDbError::InvalidSql(format!("cyclic type graph involving {type_id}")));
        }
        loading.push(type_id.clone());
        let catalog_type = stores
            .get_type(type_id)
            .map_err(|error| KalamDbError::ExecutionError(error.to_string()))?
            .ok_or_else(|| KalamDbError::NotFound(format!("type {type_id} not found")))?;
        let resolved = self.resolve_catalog_type(stores, catalog_type, loading)?;
        loading.pop();
        Ok(resolved)
    }

    fn resolve_catalog_type(
        &self,
        stores: &CatalogStores,
        catalog_type: CatalogType,
        loading: &mut Vec<TypeId>,
    ) -> Result<Arc<ResolvedType>, KalamDbError> {
        let key = RevisionKey {
            type_id:  catalog_type.type_id.clone(),
            revision: catalog_type.type_revision,
        };

        let catalog_fields = stores
            .list_type_fields(&catalog_type.type_id)
            .map_err(|error| KalamDbError::ExecutionError(error.to_string()))?;

        if catalog_type.kind == CatalogTypeKind::Enum {
            return self.finish_enum_type(key, catalog_type, catalog_fields);
        }
        if catalog_type.kind == CatalogTypeKind::Domain {
            return self.finish_domain_type(stores, key, catalog_type, catalog_fields, loading);
        }

        let mut fields = Vec::new();
        let mut arrow_children = Vec::new();
        let mut live_index = 0usize;
        for catalog_field in catalog_fields {
            if catalog_field.dropped {
                continue;
            }
            let type_ref = catalog_field.type_ref().map_err(KalamDbError::InvalidSql)?;
            if type_ref.is_map() {
                return Err(KalamDbError::InvalidSql(format!(
                    "MAP types cannot be stored on {}.{}",
                    catalog_type.type_id, catalog_field.name
                )));
            }
            let nested = match type_ref.named_type_id() {
                Some(nested_id) => Some(self.load_recursive(stores, nested_id, loading)?),
                None => None,
            };
            if let Some(nested) = &nested {
                let mut deps = self.reverse_deps.entry(nested.type_id.clone()).or_default();
                if !deps.iter().any(|existing| existing == &catalog_type.type_id) {
                    deps.push(catalog_type.type_id.clone());
                }
            }
            let child_arrow = arrow_type_for_ref(&type_ref, nested.as_ref())?;
            let mut child_field =
                Field::new(catalog_field.name.clone(), child_arrow, !catalog_field.not_null);
            if let Some(builtin) = type_ref.builtin() {
                child_field = with_kalam_data_type_metadata(child_field, &builtin);
            }
            child_field = with_logical_type_modifier(child_field, &type_ref);
            if let Some(type_id) = type_ref.named_type_id() {
                child_field = with_kalam_type_id_metadata(child_field, type_id);
            }
            if let Some(resolved) = &nested {
                if !type_ref.is_array() {
                    child_field = resolved.attach_metadata_to_occurrence(child_field);
                }
            }
            arrow_children.push(Arc::new(child_field));
            fields.push(ResolvedTypeField {
                name: catalog_field.name.clone(),
                slot: catalog_field.physical_slot(),
                ordinal: catalog_field.ordinal,
                type_ref,
                not_null: catalog_field.not_null,
                dropped: false,
                nested,
                arrow_child_index: live_index,
                constraint_expr: None,
            });
            live_index += 1;
        }

        let arrow_fields = Arc::new(Fields::from(arrow_children));
        let arrow_type = match catalog_type.kind {
            CatalogTypeKind::Enum | CatalogTypeKind::TopicPayload => DataType::Utf8,
            _ => DataType::Struct(Arc::clone(&arrow_fields).as_ref().clone()),
        };
        let resolved = Arc::new(ResolvedType {
            type_id: catalog_type.type_id,
            revision: TypeRevision::from_u32(catalog_type.type_revision.max(1))
                .unwrap_or(TypeRevision::INITIAL),
            namespace_id: catalog_type.namespace_id,
            name: catalog_type.name,
            kind: catalog_type.kind,
            fields: fields.into(),
            arrow_fields,
            arrow_type,
        });
        self.revisions.insert(key, Arc::clone(&resolved));
        self.evict_if_needed();
        Ok(resolved)
    }

    fn finish_enum_type(
        &self,
        key: RevisionKey,
        catalog_type: CatalogType,
        catalog_fields: Vec<CatalogTypeField>,
    ) -> Result<Arc<ResolvedType>, KalamDbError> {
        let mut fields = Vec::new();
        for (live_index, catalog_field) in
            catalog_fields.into_iter().filter(|field| !field.dropped).enumerate()
        {
            fields.push(ResolvedTypeField {
                name:              catalog_field.name.clone(),
                slot:              catalog_field.physical_slot(),
                ordinal:           catalog_field.ordinal,
                type_ref:          LogicalTypeRef::from_builtin(KalamDataType::Text),
                not_null:          true,
                dropped:           false,
                nested:            None,
                arrow_child_index: live_index,
                constraint_expr:   None,
            });
        }
        let resolved = Arc::new(ResolvedType {
            type_id:      catalog_type.type_id,
            revision:     TypeRevision::from_u32(catalog_type.type_revision.max(1))
                .unwrap_or(TypeRevision::INITIAL),
            namespace_id: catalog_type.namespace_id,
            name:         catalog_type.name,
            kind:         CatalogTypeKind::Enum,
            fields:       fields.into(),
            arrow_fields: Arc::new(Fields::from(Vec::<Arc<Field>>::new())),
            arrow_type:   DataType::Utf8,
        });
        self.revisions.insert(key, Arc::clone(&resolved));
        self.evict_if_needed();
        Ok(resolved)
    }

    fn finish_domain_type(
        &self,
        stores: &CatalogStores,
        key: RevisionKey,
        catalog_type: CatalogType,
        catalog_fields: Vec<CatalogTypeField>,
        loading: &mut Vec<TypeId>,
    ) -> Result<Arc<ResolvedType>, KalamDbError> {
        let base_catalog_field = catalog_fields
            .iter()
            .find(|field| field.name == "__base" && !field.dropped)
            .ok_or_else(|| {
                KalamDbError::InvalidSql(format!(
                    "domain {} is missing __base field",
                    catalog_type.type_id
                ))
            })?;
        let base_type_ref = base_catalog_field.type_ref().map_err(KalamDbError::InvalidSql)?;
        let nested = match base_type_ref.named_type_id() {
            Some(nested_id) => Some(self.load_recursive(stores, nested_id, loading)?),
            None => None,
        };
        if let Some(nested) = &nested {
            let mut deps = self.reverse_deps.entry(nested.type_id.clone()).or_default();
            if !deps.iter().any(|existing| existing == &catalog_type.type_id) {
                deps.push(catalog_type.type_id.clone());
            }
        }
        let arrow_type = arrow_type_for_ref(&base_type_ref, nested.as_ref())?;
        let arrow_fields = nested
            .as_ref()
            .map(|n| Arc::clone(&n.arrow_fields))
            .unwrap_or_else(|| Arc::new(Fields::from(Vec::<Arc<Field>>::new())));

        let mut fields = Vec::new();
        for (live_index, catalog_field) in
            catalog_fields.into_iter().filter(|field| !field.dropped).enumerate()
        {
            let is_base = catalog_field.name == "__base";
            let type_ref = if is_base {
                base_type_ref.clone()
            } else {
                LogicalTypeRef::from_builtin(KalamDataType::Text)
            };
            let constraint_expr = if is_base {
                None
            } else {
                Some(catalog_field.type_name.clone())
            };
            let slot = catalog_field.physical_slot();
            fields.push(ResolvedTypeField {
                name: catalog_field.name,
                slot,
                ordinal: catalog_field.ordinal,
                type_ref,
                not_null: catalog_field.not_null,
                dropped: false,
                nested: if is_base { nested.clone() } else { None },
                arrow_child_index: live_index,
                constraint_expr,
            });
        }
        let resolved = Arc::new(ResolvedType {
            type_id: catalog_type.type_id,
            revision: TypeRevision::from_u32(catalog_type.type_revision.max(1))
                .unwrap_or(TypeRevision::INITIAL),
            namespace_id: catalog_type.namespace_id,
            name: catalog_type.name,
            kind: CatalogTypeKind::Domain,
            fields: fields.into(),
            arrow_fields,
            arrow_type,
        });
        self.revisions.insert(key, Arc::clone(&resolved));
        self.evict_if_needed();
        Ok(resolved)
    }

    fn publish_current(&self, resolved: Arc<ResolvedType>) {
        self.current.insert(resolved.type_id.clone(), resolved);
    }

    fn evict_if_needed(&self) {
        if self.revisions.len() <= self.max_entries {
            return;
        }
        let victim_key = self.revisions.iter().next().map(|entry| entry.key().clone());
        if let Some(key) = victim_key {
            self.revisions.remove(&key);
        }
    }
}

fn arrow_type_for_ref(
    type_ref: &LogicalTypeRef,
    nested: Option<&Arc<ResolvedType>>,
) -> Result<DataType, KalamDbError> {
    match type_ref {
        LogicalTypeRef::Builtin(data_type) => data_type
            .to_arrow_type()
            .map_err(|error| KalamDbError::InvalidSql(error.to_string())),
        LogicalTypeRef::Jsonb => KalamDataType::Json
            .to_arrow_type()
            .map_err(|error| KalamDbError::InvalidSql(error.to_string())),
        LogicalTypeRef::TimestampTz => KalamDataType::DateTime
            .to_arrow_type()
            .map_err(|error| KalamDbError::InvalidSql(error.to_string())),
        LogicalTypeRef::Geometry => KalamDataType::Text
            .to_arrow_type()
            .map_err(|error| KalamDbError::InvalidSql(error.to_string())),
        LogicalTypeRef::Named { .. } => {
            let nested = nested.ok_or_else(|| {
                KalamDbError::InvalidSql("named type missing nested layout".to_string())
            })?;
            Ok(nested.arrow_type.clone())
        },
        LogicalTypeRef::List {
            element,
            element_nullable,
        } => {
            let inner = arrow_type_for_ref(element, nested)?;
            let mut item = Field::new("item", inner, *element_nullable);
            if let Some(builtin) = element.builtin() {
                item = with_kalam_data_type_metadata(item, &builtin);
            }
            item = with_logical_type_modifier(item, element);
            if let Some(type_id) = element.named_type_id() {
                item = with_kalam_type_id_metadata(item, type_id);
            }
            if let Some(resolved) = nested {
                if !element.is_array() {
                    item = resolved.attach_metadata_to_occurrence(item);
                }
            }
            Ok(DataType::List(Arc::new(item)))
        },
        LogicalTypeRef::Map { .. } => Err(KalamDbError::InvalidSql(
            "MAP types are reserved and cannot be stored".to_string(),
        )),
    }
}
