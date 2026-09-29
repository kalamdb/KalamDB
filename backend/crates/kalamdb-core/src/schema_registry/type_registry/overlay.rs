//! Table Arrow overlay: named TypeId columns become interned Struct/List layouts.

use std::sync::Arc;

use arrow::datatypes::{DataType, Field, Fields};
use kalamdb_commons::{
    conversions::{
        nested_parquet_occurrence_id, with_kalam_data_type_metadata, with_kalam_enum_labels,
        with_kalam_type_id_metadata, with_parquet_field_id,
    },
    models::{schemas::TableDefinition, CatalogTypeKind},
};
use kalamdb_system::CatalogStores;

use super::{ResolvedType, TypeRegistry};
use crate::error::KalamDbError;

impl TypeRegistry {
    /// Overlay named TypeId / array columns onto the table Arrow schema.
    pub fn arrow_schema_for_table(
        &self,
        stores: &CatalogStores,
        table: &TableDefinition,
    ) -> Result<Arc<arrow::datatypes::Schema>, KalamDbError> {
        let base = table
            .to_arrow_schema()
            .map_err(|error| KalamDbError::InvalidSql(error.to_string()))?;
        if !table
            .columns
            .iter()
            .any(|column| column.named_type_id.is_some() || column.is_array)
        {
            return Ok(base);
        }
        let mut fields = Vec::with_capacity(table.columns.len());
        for (column, field) in table.columns.iter().zip(base.fields()) {
            let mut data_type = field.data_type().clone();
            let mut nested = None;
            if let Some(type_id) = &column.named_type_id {
                let resolved = self.get_or_load(stores, type_id)?;
                data_type = resolved.arrow_type.clone();
                nested = Some(resolved);
            }
            if column.is_array {
                data_type = DataType::List(Arc::new(Field::new(
                    "item",
                    data_type,
                    column.element_nullable,
                )));
            }
            let mut occurrence =
                Field::new(column.column_name.clone(), data_type, column.is_nullable);
            occurrence = with_kalam_data_type_metadata(occurrence, &column.data_type);
            let parent_id = i32::try_from(column.column_id).unwrap_or(i32::MAX);
            occurrence = with_parquet_field_id(occurrence, parent_id);
            if let Some(type_id) = &column.named_type_id {
                occurrence = with_kalam_type_id_metadata(occurrence, type_id);
            }
            if let Some(resolved) = nested {
                occurrence = attach_enum_labels(occurrence, &resolved);
                occurrence = stamp_occurrence_slots(occurrence, parent_id, &resolved);
            }
            fields.push(occurrence);
        }
        Ok(Arc::new(arrow::datatypes::Schema::new(fields)))
    }
}

fn stamp_occurrence_slots(field: Field, parent_id: i32, resolved: &ResolvedType) -> Field {
    match field.data_type() {
        DataType::Struct(children) => {
            let stamped: Fields = children
                .iter()
                .enumerate()
                .map(|(index, child)| {
                    let slot = resolved
                        .fields
                        .get(index)
                        .map(|field| field.slot)
                        .unwrap_or((index as i32) + 1);
                    let id = nested_parquet_occurrence_id(parent_id, slot);
                    let nested_child = if let Some(nested) =
                        resolved.fields.get(index).and_then(|field| field.nested.as_ref())
                    {
                        let stamped = stamp_occurrence_slots(child.as_ref().clone(), id, nested);
                        attach_enum_labels(stamped, nested)
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
            field.with_data_type(DataType::List(Arc::new(attach_enum_labels(stamped, resolved))))
        },
        _ => attach_enum_labels(field, resolved),
    }
}

fn attach_enum_labels(field: Field, resolved: &ResolvedType) -> Field {
    if resolved.kind != CatalogTypeKind::Enum {
        return field;
    }
    let labels: Vec<String> = resolved.fields.iter().map(|field| field.name.clone()).collect();
    match field.data_type() {
        DataType::List(item) => {
            let item = with_kalam_enum_labels(item.as_ref().clone(), &labels);
            field.with_data_type(DataType::List(Arc::new(item)))
        },
        DataType::LargeList(item) => {
            let item = with_kalam_enum_labels(item.as_ref().clone(), &labels);
            field.with_data_type(DataType::LargeList(Arc::new(item)))
        },
        _ => with_kalam_enum_labels(field, &labels),
    }
}
