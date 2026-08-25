//! PostgreSQL-compatible `information_schema` extensions for wire clients.

use std::sync::Arc;

use async_trait::async_trait;
use columns::ExtendedInformationSchemaColumnsProvider;
use datafusion::{
    catalog::{information_schema::InformationSchemaProvider, CatalogProviderList, SchemaProvider},
    datasource::TableProvider,
    error::Result as DataFusionResult,
    logical_expr::TableType,
};
use kalamdb_system::SystemTablesRegistry;
use parameters::ExtendedInformationSchemaParametersProvider;
use tables::ExtendedInformationSchemaTablesProvider;
use views::InformationSchemaViewsProvider;

pub mod catalog_metadata;
pub mod columns;
pub(crate) mod extend;
pub mod parameters;
pub mod tables;
pub mod triggers;
pub mod views;

/// `information_schema` provider that delegates to DataFusion for standard tables
/// and serves an extended `columns` view with SQL/PG types from `KalamDataType`.
#[derive(Debug)]
pub struct KalamInformationSchemaProvider {
    inner:      InformationSchemaProvider,
    columns:    Arc<dyn TableProvider>,
    tables:     Arc<ExtendedInformationSchemaTablesProvider>,
    parameters: Arc<dyn TableProvider>,
    views:      Arc<dyn TableProvider>,
    triggers:   Arc<dyn TableProvider>,
}

impl KalamInformationSchemaProvider {
    pub fn new(
        catalog_list: Arc<dyn CatalogProviderList>,
        system_tables: Arc<SystemTablesRegistry>) -> Self {
        let tables = Arc::new(ExtendedInformationSchemaTablesProvider::new(
            Arc::clone(&catalog_list),
            Arc::clone(&system_tables)));
        Self {
            inner:      InformationSchemaProvider::new(Arc::clone(&catalog_list)),
            columns:    Arc::new(ExtendedInformationSchemaColumnsProvider::new(
                Arc::clone(&catalog_list),
                system_tables)),
            tables:     Arc::clone(&tables),
            parameters: Arc::new(ExtendedInformationSchemaParametersProvider::new(Arc::clone(
                &catalog_list))),
            views:      Arc::new(InformationSchemaViewsProvider::new(tables.inner())),
            triggers:   triggers::empty_triggers_provider(),
        }
    }
}

#[async_trait]
impl SchemaProvider for KalamInformationSchemaProvider {
    fn table_names(&self) -> Vec<String> {
        let mut names = self.inner.table_names();
        for name in ["columns", "parameters", "tables", "views", "triggers"] {
            if names.iter().all(|existing| !existing.eq_ignore_ascii_case(name)) {
                names.push(name.to_string());
            }
        }
        names
    }

    fn table_exist(&self, name: &str) -> bool {
        if name.eq_ignore_ascii_case("columns")
            || name.eq_ignore_ascii_case("parameters")
            || name.eq_ignore_ascii_case("tables")
            || name.eq_ignore_ascii_case("views")
            || name.eq_ignore_ascii_case("triggers")
        {
            return true;
        }
        self.inner.table_exist(name)
    }

    async fn table(&self, name: &str) -> DataFusionResult<Option<Arc<dyn TableProvider>>> {
        if name.eq_ignore_ascii_case("columns") {
            return Ok(Some(Arc::clone(&self.columns)));
        }
        if name.eq_ignore_ascii_case("tables") {
            return Ok(Some(Arc::clone(&self.tables) as Arc<dyn TableProvider>));
        }
        if name.eq_ignore_ascii_case("parameters") {
            return Ok(Some(Arc::clone(&self.parameters)));
        }
        if name.eq_ignore_ascii_case("views") {
            return Ok(Some(Arc::clone(&self.views)));
        }
        if name.eq_ignore_ascii_case("triggers") {
            return Ok(Some(Arc::clone(&self.triggers)));
        }
        self.inner.table(name).await
    }

    async fn table_type(&self, name: &str) -> DataFusionResult<Option<TableType>> {
        if name.eq_ignore_ascii_case("columns")
            || name.eq_ignore_ascii_case("parameters")
            || name.eq_ignore_ascii_case("tables")
            || name.eq_ignore_ascii_case("views")
            || name.eq_ignore_ascii_case("triggers")
        {
            return Ok(Some(TableType::View));
        }
        self.inner.table_type(name).await
    }
}
