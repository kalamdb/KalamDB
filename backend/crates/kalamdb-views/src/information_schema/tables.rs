//! Extends DataFusion `information_schema.tables` with Kalam `kdb_*` metadata.

use std::sync::Arc;

use datafusion::{catalog::CatalogProviderList, error::Result as DataFusionResult};
use kalamdb_macros::table;
use kalamdb_system::SystemTablesRegistry;

use super::{
    catalog_metadata::CatalogMetadataIndex,
    relation::{InformationSchemaRelation, StockRow},
};
use crate::system::common::Timestamp;

#[table(
    name = "tables_extension",
    namespace = SchemaId::information_schema(),
    view = true,
    comment = "Kalam columns appended to information_schema.tables"
)]
#[derive(Clone, Debug, Default)]
pub(crate) struct TablesExtension {
    /// Namespace that owns the table
    kdb_namespace_id: Option<String>,
    /// USER, SHARED, STREAM, or SYSTEM
    kdb_table_type: Option<String>,
    /// Storage placement
    kdb_storage_id: Option<String>,
    /// Schema version
    kdb_version: Option<i64>,
    /// Table options JSON
    kdb_options: Option<String>,
    /// Table comment
    kdb_comment: Option<String>,
    /// Last schema change
    kdb_updated_at: Option<Timestamp>,
    /// Table creation time
    kdb_created_at: Option<Timestamp>,
}

/// Appends `kdb_*` metadata to DataFusion `information_schema.tables`.
#[derive(Debug)]
pub(crate) struct TablesRelation {
    catalog_list: Arc<dyn CatalogProviderList>,
    system_tables: Arc<SystemTablesRegistry>,
}

impl TablesRelation {
    /// Relation backed by the process catalog list and system registry.
    pub fn new(
        catalog_list: Arc<dyn CatalogProviderList>,
        system_tables: Arc<SystemTablesRegistry>,
    ) -> Self {
        Self {
            catalog_list,
            system_tables,
        }
    }
}

impl InformationSchemaRelation for TablesRelation {
    type Ext = TablesExtension;
    type Extra = ();

    const NAME: &'static str = "tables";
    const STOCK: Option<&'static str> = Some("tables");

    fn catalog_list(&self) -> Option<Arc<dyn CatalogProviderList>> {
        Some(Arc::clone(&self.catalog_list))
    }

    fn system_tables(&self) -> Option<&SystemTablesRegistry> {
        Some(&self.system_tables)
    }

    fn delegate_filter_pushdown(&self) -> bool {
        true
    }

    fn extend(
        &self,
        row: &StockRow<'_>,
        index: &CatalogMetadataIndex,
    ) -> DataFusionResult<Option<Self::Ext>> {
        let Some(meta) = index.table(row.utf8("table_schema")?, row.utf8("table_name")?) else {
            return Ok(None);
        };
        Ok(Some(TablesExtension {
            kdb_namespace_id: Some(meta.namespace_id.clone()),
            kdb_table_type: Some(meta.kalam_table_type.clone()),
            kdb_storage_id: meta.storage_id.clone(),
            kdb_version: Some(meta.version),
            kdb_options: meta.options.clone(),
            kdb_comment: meta.comment.clone(),
            kdb_updated_at: Some(meta.updated_at_micros),
            kdb_created_at: Some(meta.created_at_micros),
        }))
    }
}
