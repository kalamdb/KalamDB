//! Table Arrow overlay: named TypeId columns become interned Struct/List layouts.

use std::sync::Arc;

use arrow::datatypes::{DataType, Field, Fields};
use kalamdb_commons::{
    conversions::{
        nested_parquet_occurrence_id, with_kalam_data_type_metadata, with_kalam_type_id_metadata,
        with_logical_type_modifier, with_parquet_field_id,
    },
    models::schemas::TableDefinition,
};
use kalamdb_system::CatalogStores;

use super::{ResolvedType, TableLayoutKey, TypeRegistry};
use crate::error::KalamDbError;

impl TypeRegistry {
    /// Overlay named types, arrays, and builtin modifiers onto the table Arrow schema.
    ///
    /// The result is interned for `(namespace, table, schema_version)`.
    pub fn arrow_schema_for_table(
        &self,
        stores: &CatalogStores,
        table: &TableDefinition,
    ) -> Result<Arc<arrow::datatypes::Schema>, KalamDbError> {
        let key = TableLayoutKey {
            namespace_id:   table.namespace_id.clone(),
            table_name:     table.table_name.clone(),
            schema_version: table.schema_version,
        };
        if let Some(cached) = self.table_layouts.get(&key) {
            return Ok(Arc::clone(cached.value()));
        }
        let built = self.build_arrow_schema(stores, table)?;
        self.remember_table_layout(&key, table);
        let schema = {
            let stored = self.table_layouts.entry(key).or_insert(built);
            Arc::clone(stored.value())
        };
        self.evict_table_layouts();
        Ok(schema)
    }

    fn build_arrow_schema(
        &self,
        stores: &CatalogStores,
        table: &TableDefinition,
    ) -> Result<Arc<arrow::datatypes::Schema>, KalamDbError> {
        let base = table
            .to_arrow_schema()
            .map_err(|error| KalamDbError::InvalidSql(error.to_string()))?;
        let needs_overlay = table.columns.iter().any(|column| {
            column.named_type_id().is_some()
                || column.is_array()
                || column.logical_type.modifier_label().is_some()
        });
        if !needs_overlay {
            return Ok(base);
        }
        let mut fields = Vec::with_capacity(table.columns.len());
        for column in &table.columns {
            let layout = self.resolve(stores, &column.logical_type)?;
            let domain_not_null = !column.is_array()
                && layout
                    .nested
                    .as_ref()
                    .is_some_and(|r| r.domain_constraints().iter().any(|d| d.not_null));
            let mut occurrence = Field::new(
                column.column_name.clone(),
                layout.arrow_type,
                column.is_nullable && !domain_not_null,
            );
            occurrence = with_kalam_data_type_metadata(occurrence, &column.data_type());
            occurrence = with_logical_type_modifier(occurrence, &layout.type_ref);
            let parent_id = i32::try_from(column.column_id).unwrap_or(i32::MAX);
            occurrence = with_parquet_field_id(occurrence, parent_id);
            if let Some(type_id) = column.named_type_id() {
                occurrence = with_kalam_type_id_metadata(occurrence, type_id);
            }
            if let Some(kalamdb_commons::models::schemas::ColumnDefault::Literal(ref default_json)) =
                column.missing_default
            {
                occurrence =
                    kalamdb_commons::conversions::with_kalam_missing_default(occurrence, default_json);
            }
            if let Some(resolved) = layout.nested {
                if !column.is_array() {
                    occurrence = resolved.attach_metadata_to_occurrence(occurrence);
                }
                occurrence = stamp_occurrence_slots(occurrence, parent_id, &resolved);
            }
            fields.push(occurrence);
        }
        Ok(Arc::new(arrow::datatypes::Schema::new(fields)))
    }

    fn remember_table_layout(&self, key: &TableLayoutKey, table: &TableDefinition) {
        for column in &table.columns {
            for type_id in column.logical_type.referenced_type_ids() {
                let mut deps = self.table_deps.entry(type_id.clone()).or_default();
                if !deps.iter().any(|existing| existing == key) {
                    deps.push(key.clone());
                }
            }
        }
    }

    fn evict_table_layouts(&self) {
        if self.table_layouts.len() <= self.max_entries {
            return;
        }
        let victim_key = self.table_layouts.iter().next().map(|entry| entry.key().clone());
        if let Some(key) = victim_key {
            self.table_layouts.remove(&key);
        }
    }
}

fn stamp_occurrence_slots(field: Field, parent_id: i32, resolved: &ResolvedType) -> Field {
    let root = resolved.root_resolved_type();
    match field.data_type() {
        DataType::Struct(children) => {
            let stamped: Fields = children
                .iter()
                .enumerate()
                .map(|(index, child)| {
                    let slot = root
                        .fields
                        .get(index)
                        .map(|field| field.slot)
                        .unwrap_or((index as i32) + 1);
                    let id = nested_parquet_occurrence_id(parent_id, slot);
                    let nested_child = if let Some(nested) =
                        root.fields.get(index).and_then(|field| field.nested.as_ref())
                    {
                        stamp_occurrence_slots(child.as_ref().clone(), id, nested)
                    } else {
                        child.as_ref().clone()
                    };
                    Arc::new(with_parquet_field_id(nested_child, id))
                })
                .collect();
            field.with_data_type(DataType::Struct(stamped))
        },
        DataType::List(item) => {
            let stamped = stamp_occurrence_slots(item.as_ref().clone(), parent_id, resolved);
            field.with_data_type(DataType::List(Arc::new(stamped)))
        },
        _ => field,
    }
}
