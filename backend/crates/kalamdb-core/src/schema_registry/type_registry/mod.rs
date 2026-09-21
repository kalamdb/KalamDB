//! Immutable named-type descriptors keyed by TypeId + revision.
//!
//! Layout lives next to [`crate::schema_registry::SchemaRegistry`]: columns use
//! [`kalamdb_commons::schemas::ColumnDefinition`], catalog rows stay in
//! `kalamdb-system`. This cache is not a name index.

mod load;
mod overlay;
mod resolved;

use std::sync::Arc;

use dashmap::DashMap;
use kalamdb_commons::models::TypeId;
use kalamdb_system::CatalogStores;
pub use resolved::{ResolvedType, ResolvedTypeField};

use crate::error::KalamDbError;

const TYPE_CACHE_MAX_ENTRIES: usize = 256;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct RevisionKey {
    type_id:  TypeId,
    revision: u32,
}

/// DashMap cache keyed by TypeId+revision. Name lookup is not a cache key.
pub struct TypeRegistry {
    current:      DashMap<TypeId, Arc<ResolvedType>>,
    revisions:    DashMap<RevisionKey, Arc<ResolvedType>>,
    reverse_deps: DashMap<TypeId, Vec<TypeId>>,
    max_entries:  usize,
}

impl std::fmt::Debug for TypeRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TypeRegistry")
            .field("current", &self.current.len())
            .field("revisions", &self.revisions.len())
            .finish()
    }
}

impl TypeRegistry {
    pub fn new() -> Self {
        Self {
            current:      DashMap::with_capacity(32),
            revisions:    DashMap::with_capacity(32),
            reverse_deps: DashMap::with_capacity(32),
            max_entries:  TYPE_CACHE_MAX_ENTRIES,
        }
    }

    pub fn current(&self, type_id: &TypeId) -> Option<Arc<ResolvedType>> {
        self.current.get(type_id).map(|entry| Arc::clone(entry.value()))
    }

    /// Load or intern the current revision. Name lookup happens in the catalog, not here.
    pub fn get_or_load(
        &self,
        stores: &CatalogStores,
        type_id: &TypeId,
    ) -> Result<Arc<ResolvedType>, KalamDbError> {
        if let Some(cached) = self.current(type_id) {
            return Ok(cached);
        }
        self.load_current(stores, type_id)
    }

    /// Drop current pointer, interned revisions, and reverse dependents.
    /// In-flight Arcs stay valid.
    pub fn invalidate(&self, type_id: &TypeId) {
        let mut drop_ids = vec![type_id.clone()];
        if let Some((_, dependents)) = self.reverse_deps.remove(type_id) {
            drop_ids.extend(dependents);
        }
        for id in &drop_ids {
            self.current.remove(id);
        }
        self.revisions.retain(|key, _| drop_ids.iter().all(|id| &key.type_id != id));
    }

    /// Types that currently store a field of `type_id` (nested composites).
    pub fn dependent_type_ids(&self, type_id: &TypeId) -> Vec<TypeId> {
        self.reverse_deps
            .get(type_id)
            .map(|entry| entry.value().clone())
            .unwrap_or_default()
    }
}

impl Default for TypeRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use kalamdb_commons::{
        models::{CatalogTypeKind, NamespaceId},
        TypeId,
    };
    use kalamdb_store::test_utils::InMemoryBackend;
    use kalamdb_system::{CatalogStores, CatalogType, CatalogTypeField};

    use super::TypeRegistry;

    #[test]
    fn interned_layout_is_shared_across_loads() {
        let stores = CatalogStores::new(Arc::new(InMemoryBackend::new()));
        let ns = NamespaceId::new("chat");
        let type_id = TypeId::from_parts(Some(&ns), "address");
        stores
            .upsert_type(CatalogType::named(
                type_id.clone(),
                ns.clone(),
                "address",
                CatalogTypeKind::Composite,
            ))
            .unwrap();
        stores
            .upsert_type_field(
                CatalogTypeField::new(
                    type_id.clone(),
                    "line1",
                    1,
                    None,
                    Some(kalamdb_commons::KalamDataType::Text),
                    "text",
                    false,
                    false,
                    false,
                )
                .unwrap(),
            )
            .unwrap();
        let registry = TypeRegistry::new();
        let first = registry.get_or_load(&stores, &type_id).unwrap();
        let second = registry.get_or_load(&stores, &type_id).unwrap();
        assert!(Arc::ptr_eq(&first, &second));
        assert!(matches!(first.arrow_type, arrow::datatypes::DataType::Struct(_)));
    }

    #[test]
    fn enum_layout_is_utf8_and_keeps_labels() {
        use kalamdb_commons::{
            conversions::read_kalam_enum_labels,
            schemas::{ColumnDefault, ColumnDefinition, TableDefinition, TableOptions, TableType},
            TableName,
        };

        let stores = CatalogStores::new(Arc::new(InMemoryBackend::new()));
        let ns = NamespaceId::new("chat");
        let type_id = TypeId::from_parts(Some(&ns), "status");
        stores
            .upsert_type(CatalogType::named(
                type_id.clone(),
                ns.clone(),
                "status",
                CatalogTypeKind::Enum,
            ))
            .unwrap();
        for (ordinal, label) in [(1, "active"), (2, "blocked")] {
            stores
                .upsert_type_field(
                    CatalogTypeField::new(
                        type_id.clone(),
                        label,
                        ordinal,
                        None,
                        None,
                        label,
                        false,
                        true,
                        false,
                    )
                    .unwrap(),
                )
                .unwrap();
        }
        let registry = TypeRegistry::new();
        let resolved = registry.get_or_load(&stores, &type_id).unwrap();
        assert!(matches!(resolved.arrow_type, arrow::datatypes::DataType::Utf8));
        assert_eq!(
            resolved.fields.iter().map(|field| field.name.as_str()).collect::<Vec<_>>(),
            vec!["active", "blocked"]
        );

        let table = TableDefinition::new(
            ns,
            TableName::new("tickets"),
            TableType::Shared,
            vec![
                ColumnDefinition::new(
                    1,
                    "id",
                    1,
                    kalamdb_commons::KalamDataType::Int,
                    false,
                    true,
                    false,
                    ColumnDefault::None,
                    None,
                ),
                ColumnDefinition::new(
                    2,
                    "status",
                    2,
                    kalamdb_commons::KalamDataType::Text,
                    false,
                    false,
                    false,
                    ColumnDefault::None,
                    None,
                )
                .with_named_type(type_id, false),
            ],
            TableOptions::shared(),
            None,
        )
        .unwrap();
        let overlay = registry.arrow_schema_for_table(&stores, &table).unwrap();
        let status = overlay.field_with_name("status").unwrap();
        assert_eq!(
            read_kalam_enum_labels(status).as_deref(),
            Some(["active".to_string(), "blocked".to_string()].as_slice())
        );
    }
}
