//! PostgreSQL-compatible `information_schema` extensions for wire clients.

use std::sync::Arc;

use async_trait::async_trait;
use datafusion::{
    catalog::{information_schema::InformationSchemaProvider, CatalogProviderList, SchemaProvider},
    datasource::TableProvider,
    error::Result as DataFusionResult,
    logical_expr::TableType,
};
use kalamdb_system::SystemTablesRegistry;

use crate::information_schema::relation::InfoSchemaSource;

pub mod catalog_metadata;
pub mod columns;
pub(crate) mod extend;
pub mod parameters;
pub mod relation;
pub mod routines;
pub mod tables;
pub mod triggers;
pub mod views;

/// Kalam columns appended to `information_schema.columns`, in `#[table]` order.
pub fn columns_extension_order() -> &'static [&'static str] {
    columns::extension_column_order()
}

/// `information_schema` provider that delegates to DataFusion for standard tables
/// and serves an extended `columns` view with SQL/PG types from `KalamDataType`.
#[derive(Debug)]
pub struct KalamInformationSchemaProvider {
    inner: InformationSchemaProvider,
    columns: Arc<dyn TableProvider>,
    tables: Arc<dyn TableProvider>,
    parameters: Arc<dyn TableProvider>,
    routines: Arc<dyn TableProvider>,
    views: Arc<dyn TableProvider>,
    triggers: Arc<dyn TableProvider>,
}

impl KalamInformationSchemaProvider {
    /// Stock DataFusion relations plus the Kalam extensions.
    pub fn new(
        catalog_list: Arc<dyn CatalogProviderList>,
        system_tables: Arc<SystemTablesRegistry>,
    ) -> Self {
        let relations: Vec<(&str, Arc<dyn TableProvider>)> = vec![
            (
                "tables",
                InfoSchemaSource::provider(tables::TablesRelation::new(
                    Arc::clone(&catalog_list),
                    Arc::clone(&system_tables),
                )),
            ),
            (
                "columns",
                InfoSchemaSource::provider(columns::ColumnsRelation::new(
                    Arc::clone(&catalog_list),
                    Arc::clone(&system_tables),
                )),
            ),
            (
                "parameters",
                InfoSchemaSource::provider(parameters::ParametersRelation::new(
                    Arc::clone(&catalog_list),
                    Arc::clone(&system_tables),
                )),
            ),
            (
                "routines",
                InfoSchemaSource::provider(routines::RoutinesRelation::new(
                    Arc::clone(&catalog_list),
                    system_tables,
                )),
            ),
            (
                "views",
                InfoSchemaSource::provider(views::ViewsRelation::new(Arc::clone(&catalog_list))),
            ),
            ("triggers", triggers::empty_triggers_provider()),
        ];
        let mut relations = relations.into_iter();
        Self {
            inner: InformationSchemaProvider::new(catalog_list),
            tables: relations.next().expect("tables").1,
            columns: relations.next().expect("columns").1,
            parameters: relations.next().expect("parameters").1,
            routines: relations.next().expect("routines").1,
            views: relations.next().expect("views").1,
            triggers: relations.next().expect("triggers").1,
        }
    }
}

#[async_trait]
impl SchemaProvider for KalamInformationSchemaProvider {
    fn table_names(&self) -> Vec<String> {
        let mut names = self.inner.table_names();
        for name in [
            "columns",
            "parameters",
            "routines",
            "tables",
            "views",
            "triggers",
        ] {
            if names.iter().all(|existing| !existing.eq_ignore_ascii_case(name)) {
                names.push(name.to_string());
            }
        }
        names
    }

    fn table_exist(&self, name: &str) -> bool {
        if name.eq_ignore_ascii_case("columns")
            || name.eq_ignore_ascii_case("parameters")
            || name.eq_ignore_ascii_case("routines")
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
            return Ok(Some(Arc::clone(&self.tables)));
        }
        if name.eq_ignore_ascii_case("parameters") {
            return Ok(Some(Arc::clone(&self.parameters)));
        }
        if name.eq_ignore_ascii_case("routines") {
            return Ok(Some(Arc::clone(&self.routines)));
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
            || name.eq_ignore_ascii_case("routines")
            || name.eq_ignore_ascii_case("tables")
            || name.eq_ignore_ascii_case("views")
            || name.eq_ignore_ascii_case("triggers")
        {
            return Ok(Some(TableType::View));
        }
        self.inner.table_type(name).await
    }
}
