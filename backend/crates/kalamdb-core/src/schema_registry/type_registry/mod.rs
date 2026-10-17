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
use kalamdb_commons::models::{SchemaId, TableName, TypeId};
use kalamdb_system::CatalogStores;
pub use resolved::{ResolvedLayout, ResolvedType, ResolvedTypeField};

use crate::error::KalamDbError;

const TYPE_CACHE_MAX_ENTRIES: usize = 256;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct RevisionKey {
    type_id:  TypeId,
    revision: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct TableLayoutKey {
    namespace_id:   SchemaId,
    table_name:     TableName,
    schema_version: u32,
}

/// DashMap cache keyed by TypeId+revision. Name lookup is not a cache key.
/// Table Arrow layouts are cached by schema revision and dropped when a named
/// type they use is invalidated.
pub struct TypeRegistry {
    current:       DashMap<TypeId, Arc<ResolvedType>>,
    revisions:     DashMap<RevisionKey, Arc<ResolvedType>>,
    reverse_deps:  DashMap<TypeId, Vec<TypeId>>,
    table_layouts: DashMap<TableLayoutKey, Arc<arrow::datatypes::Schema>>,
    table_deps:    DashMap<TypeId, Vec<TableLayoutKey>>,
    max_entries:   usize,
}

impl std::fmt::Debug for TypeRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TypeRegistry")
            .field("current", &self.current.len())
            .field("revisions", &self.revisions.len())
            .field("table_layouts", &self.table_layouts.len())
            .finish()
    }
}

impl TypeRegistry {
    pub fn new() -> Self {
        Self {
            current:       DashMap::with_capacity(32),
            revisions:     DashMap::with_capacity(32),
            reverse_deps:  DashMap::with_capacity(32),
            table_layouts: DashMap::with_capacity(32),
            table_deps:    DashMap::with_capacity(32),
            max_entries:   TYPE_CACHE_MAX_ENTRIES,
        }
    }

    pub fn current(&self, type_id: &TypeId) -> Option<Arc<ResolvedType>> {
        self.current.get(type_id).map(|entry| Arc::clone(entry.value()))
    }

    /// Look up a specific pinned revision without mutating the current pointer.
    pub fn get_revision(
        &self,
        type_id: &TypeId,
        revision: kalamdb_commons::TypeRevision,
    ) -> Option<Arc<ResolvedType>> {
        let key = RevisionKey {
            type_id:  type_id.clone(),
            revision: revision.get(),
        };
        self.revisions.get(&key).map(|entry| Arc::clone(entry.value()))
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

    /// Drop current pointer and reverse dependents.
    /// Historical revisions stay pinned by `(TypeId, revision)` and in-flight Arcs stay valid.
    pub fn invalidate(&self, type_id: &TypeId) {
        let mut drop_ids = self.dependent_type_ids(type_id);
        drop_ids.insert(0, type_id.clone());
        for id in &drop_ids {
            self.current.remove(id);
        }
        for id in &drop_ids {
            let Some((_, keys)) = self.table_deps.remove(id) else {
                continue;
            };
            for key in keys {
                self.table_layouts.remove(&key);
            }
        }
    }

    /// Types that transitively store a field of `type_id` (nested composites).
    pub fn dependent_type_ids(&self, type_id: &TypeId) -> Vec<TypeId> {
        let mut out = Vec::new();
        let mut queue = vec![type_id.clone()];
        while let Some(current) = queue.pop() {
            if let Some(entry) = self.reverse_deps.get(&current) {
                for dep in entry.value() {
                    if dep != type_id && !out.contains(dep) {
                        out.push(dep.clone());
                        queue.push(dep.clone());
                    }
                }
            }
        }
        out
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
        models::{CatalogTypeKind, SchemaId},
        TypeId,
    };
    use kalamdb_store::test_utils::InMemoryBackend;
    use kalamdb_system::{CatalogStores, CatalogType, CatalogTypeField};

    use super::TypeRegistry;

    #[test]
    fn interned_layout_is_shared_across_loads() {
        let stores = CatalogStores::new(Arc::new(InMemoryBackend::new()));
        let ns = SchemaId::new("chat");
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
        let ns = SchemaId::new("chat");
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

    #[test]
    fn modifiers_resolve_onto_the_schema_revision_cache() {
        use kalamdb_commons::{
            conversions::read_logical_type_modifier,
            schemas::{ColumnDefault, ColumnDefinition, TableDefinition, TableOptions, TableType},
            KalamDataType, LogicalTypeRef, TableName,
        };

        let stores = CatalogStores::new(Arc::new(InMemoryBackend::new()));
        let ns = SchemaId::new("chat");
        let mut payload = ColumnDefinition::new(
            1,
            "payload",
            1,
            KalamDataType::Json,
            true,
            false,
            false,
            ColumnDefault::None,
            None,
        );
        payload.logical_type = LogicalTypeRef::jsonb();
        let mut charged_at = ColumnDefinition::new(
            2,
            "charged_at",
            2,
            KalamDataType::Timestamp,
            true,
            false,
            false,
            ColumnDefault::None,
            None,
        );
        charged_at.logical_type = LogicalTypeRef::timestamptz();
        let mut table = TableDefinition::new(
            ns,
            TableName::new("invoices"),
            TableType::Shared,
            vec![payload, charged_at],
            TableOptions::shared(),
            None,
        )
        .unwrap();
        let registry = TypeRegistry::new();
        let first = registry.arrow_schema_for_table(&stores, &table).unwrap();
        let second = registry.arrow_schema_for_table(&stores, &table).unwrap();
        assert!(Arc::ptr_eq(&first, &second));
        assert_eq!(
            read_logical_type_modifier(first.field_with_name("payload").unwrap()),
            Some("jsonb")
        );
        assert_eq!(
            read_logical_type_modifier(first.field_with_name("charged_at").unwrap()),
            Some("timestamptz")
        );

        table.schema_version = table.schema_version.saturating_add(1);
        let revised = registry.arrow_schema_for_table(&stores, &table).unwrap();
        assert!(!Arc::ptr_eq(&first, &revised));
    }

    #[test]
    fn named_type_invalidation_drops_the_table_layout() {
        use kalamdb_commons::{
            schemas::{ColumnDefault, ColumnDefinition, TableDefinition, TableOptions, TableType},
            KalamDataType, TableName,
        };

        let stores = CatalogStores::new(Arc::new(InMemoryBackend::new()));
        let ns = SchemaId::new("chat");
        let type_id = TypeId::from_parts(Some(&ns), "status");
        stores
            .upsert_type(CatalogType::named(
                type_id.clone(),
                ns.clone(),
                "status",
                CatalogTypeKind::Enum,
            ))
            .unwrap();
        stores
            .upsert_type_field(
                CatalogTypeField::new(
                    type_id.clone(),
                    "open",
                    1,
                    None,
                    None,
                    "open",
                    false,
                    true,
                    false,
                )
                .unwrap(),
            )
            .unwrap();
        let table = TableDefinition::new(
            ns,
            TableName::new("invoices"),
            TableType::Shared,
            vec![ColumnDefinition::new(
                1,
                "status",
                1,
                KalamDataType::Text,
                false,
                false,
                false,
                ColumnDefault::None,
                None,
            )
            .with_named_type(type_id.clone(), false)],
            TableOptions::shared(),
            None,
        )
        .unwrap();
        let registry = TypeRegistry::new();
        let first = registry.arrow_schema_for_table(&stores, &table).unwrap();
        registry.invalidate(&type_id);
        let reloaded = registry.arrow_schema_for_table(&stores, &table).unwrap();
        assert!(!Arc::ptr_eq(&first, &reloaded));
        assert_eq!(
            reloaded.field_with_name("status").unwrap().data_type(),
            &arrow::datatypes::DataType::Utf8
        );
    }

    #[test]
    fn eviction_past_max_entries_does_not_deadlock() {
        use kalamdb_commons::{
            schemas::{ColumnDefault, ColumnDefinition, TableDefinition, TableOptions, TableType},
            KalamDataType, TableName,
        };

        let stores = CatalogStores::new(Arc::new(InMemoryBackend::new()));
        let ns = SchemaId::new("chat");
        let registry = TypeRegistry::new();

        for idx in 0..(super::TYPE_CACHE_MAX_ENTRIES + 8) {
            let type_name = format!("status_{idx}");
            let type_id = TypeId::from_parts(Some(&ns), &type_name);
            stores
                .upsert_type(CatalogType::named(
                    type_id.clone(),
                    ns.clone(),
                    &type_name,
                    CatalogTypeKind::Enum,
                ))
                .unwrap();
            stores
                .upsert_type_field(
                    CatalogTypeField::new(
                        type_id.clone(),
                        "open",
                        1,
                        None,
                        None,
                        "open",
                        false,
                        true,
                        false,
                    )
                    .unwrap(),
                )
                .unwrap();
            let table = TableDefinition::new(
                ns.clone(),
                TableName::new(format!("tbl_{idx}")),
                TableType::Shared,
                vec![ColumnDefinition::new(
                    1,
                    "status",
                    1,
                    KalamDataType::Text,
                    false,
                    false,
                    false,
                    ColumnDefault::None,
                    None,
                )
                .with_named_type(type_id, false)],
                TableOptions::shared(),
                None,
            )
            .unwrap();
            let _ = registry.arrow_schema_for_table(&stores, &table).unwrap();
        }
        assert!(registry.table_layouts.len() <= super::TYPE_CACHE_MAX_ENTRIES);
        assert!(registry.revisions.len() <= super::TYPE_CACHE_MAX_ENTRIES);
    }
}
