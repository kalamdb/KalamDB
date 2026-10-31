//! Minimal pg_catalog compatibility views for PostgreSQL wire clients.

use std::{collections::BTreeMap, sync::Arc};

use async_trait::async_trait;
use datafusion::{
    arrow::{datatypes::SchemaRef, record_batch::RecordBatch},
    catalog::{SchemaProvider, Session},
    datasource::{TableProvider, TableType},
    error::{DataFusionError, Result as DataFusionResult},
    logical_expr::{Expr, TableProviderFilterPushDown},
    physical_plan::ExecutionPlan,
};
use kalamdb_commons::{constants::LOGICAL_DATABASE_NAME, Role, SystemTable};
use kalamdb_session_datafusion::SessionConfigValues;
use kalamdb_system::SystemTablesRegistry;

use crate::{
    catalog_provider::{CatalogScanCtx, CatalogSource, ComputedCatalogProvider},
    error::RegistryError,
    system::sessions::SessionsSnapshotCallback,
};

pub mod attrdef;
pub mod attribute;
pub mod class;
pub mod constraint;
pub mod database;
pub mod description;
pub mod empty;
pub mod empty_tables;
pub mod enum_view;
pub mod get_keywords;
pub mod index;
pub mod language;
pub mod namespace;
pub mod proc;
pub use proc::{function_definition, function_result, identity_arguments};
pub mod roles;
pub mod settings;
pub mod stat_activity;
pub mod tables;
pub mod r#type;
pub mod type_mapping;
pub mod views;

mod oid;
mod relations;

pub use oid::proc_oid;
pub(crate) use oid::{
    catalog_type_oid, class_oid, named_column_attnums, namespace_oid, oid_for_qualified_type_id,
    primary_constraint_oid, primary_index_oid, primary_index_relname, primary_key_attnums,
    scalar_index_attnums, scalar_index_oid, stable_oid,
};
pub(crate) use relations::{
    include_explicit_namespaces, pg_relkind, relation_is_view, supplemental_catalog_definitions,
    visible_table_definitions,
};

pub trait PgCatalogView: Send + Sync + std::fmt::Debug {
    fn name(&self) -> &'static str;
    fn schema(&self) -> SchemaRef;
    fn required_system_table(&self) -> Option<SystemTable> {
        None
    }
    fn compute_batch(&self, role: Role) -> Result<RecordBatch, RegistryError>;
    fn compute_session_batch(
        &self,
        role: Role,
        _settings: Option<&SessionConfigValues>,
    ) -> Result<RecordBatch, RegistryError> {
        self.compute_batch(role)
    }
}

/// Adapts a [`PgCatalogView`] to the shared catalog scan shell.
pub struct PgCatalogViewSource<V: PgCatalogView> {
    view: Arc<V>,
}

impl<V: PgCatalogView> std::fmt::Debug for PgCatalogViewSource<V> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PgCatalogViewSource")
            .field("view", &self.view.name())
            .finish()
    }
}

#[async_trait]
impl<V: PgCatalogView + 'static> CatalogSource for PgCatalogViewSource<V> {
    fn schema(&self) -> SchemaRef {
        self.view.schema()
    }

    fn required_system_table(&self) -> Option<SystemTable> {
        self.view.required_system_table()
    }

    async fn batch(&self, ctx: &CatalogScanCtx<'_>) -> DataFusionResult<RecordBatch> {
        self.view.compute_session_batch(ctx.role, ctx.settings).map_err(|error| {
            DataFusionError::Execution(format!(
                "failed to compute pg_catalog.{}: {}",
                self.view.name(),
                error
            ))
        })
    }
}

/// [`TableProvider`] for a [`PgCatalogView`].
pub struct PgCatalogViewTableProvider<V: PgCatalogView + 'static> {
    inner: ComputedCatalogProvider<PgCatalogViewSource<V>>,
}

impl<V: PgCatalogView + 'static> Clone for PgCatalogViewTableProvider<V> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}

impl<V: PgCatalogView + 'static> std::fmt::Debug for PgCatalogViewTableProvider<V> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PgCatalogViewTableProvider")
            .field("inner", &self.inner)
            .finish()
    }
}

impl<V: PgCatalogView + 'static> PgCatalogViewTableProvider<V> {
    /// Wrap a pg_catalog view.
    pub fn new(view: Arc<V>) -> Self {
        Self {
            inner: ComputedCatalogProvider::new(Arc::new(PgCatalogViewSource { view })),
        }
    }
}

#[async_trait]
impl<V: PgCatalogView + 'static> TableProvider for PgCatalogViewTableProvider<V> {
    fn schema(&self) -> SchemaRef {
        self.inner.schema()
    }

    fn table_type(&self) -> TableType {
        self.inner.table_type()
    }

    async fn scan(
        &self,
        state: &dyn Session,
        projection: Option<&Vec<usize>>,
        filters: &[Expr],
        limit: Option<usize>,
    ) -> DataFusionResult<Arc<dyn ExecutionPlan>> {
        self.inner.scan(state, projection, filters, limit).await
    }

    fn supports_filters_pushdown(
        &self,
        filters: &[&Expr],
    ) -> DataFusionResult<Vec<TableProviderFilterPushDown>> {
        self.inner.supports_filters_pushdown(filters)
    }
}

#[derive(Debug)]
pub struct PgCatalogSchemaProvider {
    providers: BTreeMap<String, Arc<dyn TableProvider>>,
}

impl PgCatalogSchemaProvider {
    pub fn new(
        system_registry: Arc<SystemTablesRegistry>,
        sessions_snapshot_callback: SessionsSnapshotCallback,
    ) -> Self {
        let mut providers = BTreeMap::<String, Arc<dyn TableProvider>>::new();
        providers.insert(
            "pg_namespace".to_string(),
            Arc::new(PgCatalogViewTableProvider::new(Arc::new(namespace::PgNamespaceView::new(
                Arc::clone(&system_registry),
            )))),
        );
        providers.insert(
            "pg_class".to_string(),
            Arc::new(PgCatalogViewTableProvider::new(Arc::new(class::PgClassView::new(
                Arc::clone(&system_registry),
            )))),
        );
        providers.insert(
            "pg_attribute".to_string(),
            Arc::new(PgCatalogViewTableProvider::new(Arc::new(attribute::PgAttributeView::new(
                Arc::clone(&system_registry),
            )))),
        );
        providers.insert(
            "pg_type".to_string(),
            Arc::new(PgCatalogViewTableProvider::new(Arc::new(r#type::PgTypeView::new(
                Arc::clone(&system_registry),
            )))),
        );
        providers.insert(
            "pg_database".to_string(),
            Arc::new(PgCatalogViewTableProvider::new(Arc::new(database::PgDatabaseView::new(
                LOGICAL_DATABASE_NAME,
            )))),
        );
        providers.insert(
            "pg_stat_activity".to_string(),
            Arc::new(PgCatalogViewTableProvider::new(Arc::new(
                stat_activity::PgStatActivityView::new(sessions_snapshot_callback),
            ))),
        );
        providers.insert(
            "pg_tables".to_string(),
            Arc::new(PgCatalogViewTableProvider::new(Arc::new(tables::PgTablesView::new(
                Arc::clone(&system_registry),
            )))),
        );
        providers.insert(
            "pg_views".to_string(),
            Arc::new(PgCatalogViewTableProvider::new(Arc::new(views::PgViewsView::new(
                Arc::clone(&system_registry),
            )))),
        );
        providers.insert(
            "pg_get_keywords".to_string(),
            Arc::new(PgCatalogViewTableProvider::new(Arc::new(get_keywords::PgGetKeywordsView))),
        );
        providers.insert(
            "pg_index".to_string(),
            Arc::new(PgCatalogViewTableProvider::new(Arc::new(index::PgIndexView::new(
                Arc::clone(&system_registry),
            )))),
        );
        providers.insert(
            "pg_constraint".to_string(),
            Arc::new(PgCatalogViewTableProvider::new(Arc::new(constraint::PgConstraintView::new(
                Arc::clone(&system_registry),
            )))),
        );
        providers.insert(
            "pg_proc".to_string(),
            Arc::new(PgCatalogViewTableProvider::new(Arc::new(proc::PgProcView::new(Arc::clone(
                &system_registry,
            ))))),
        );
        providers.insert(
            "pg_settings".to_string(),
            Arc::new(PgCatalogViewTableProvider::new(Arc::new(settings::PgSettingsView))),
        );
        providers.insert(
            "pg_enum".to_string(),
            Arc::new(PgCatalogViewTableProvider::new(Arc::new(enum_view::PgEnumView::new(
                Arc::clone(&system_registry),
            )))),
        );
        providers.insert(
            "pg_language".to_string(),
            Arc::new(PgCatalogViewTableProvider::new(Arc::new(language::PgLanguageView))),
        );
        providers.insert(
            "pg_attrdef".to_string(),
            Arc::new(PgCatalogViewTableProvider::new(Arc::new(attrdef::PgAttrdefView::new(
                Arc::clone(&system_registry),
            )))),
        );
        providers.insert(
            "pg_description".to_string(),
            Arc::new(PgCatalogViewTableProvider::new(Arc::new(
                description::PgDescriptionView::new(Arc::clone(&system_registry)),
            ))),
        );
        providers.insert(
            "pg_roles".to_string(),
            Arc::new(PgCatalogViewTableProvider::new(Arc::new(roles::PgRolesView::roles(
                Arc::clone(&system_registry),
            )))),
        );
        providers.insert(
            "pg_authid".to_string(),
            Arc::new(PgCatalogViewTableProvider::new(Arc::new(roles::PgRolesView::authid(
                Arc::clone(&system_registry),
            )))),
        );
        register_empty_pg_catalog_views(&mut providers);

        Self { providers }
    }
}

fn register_empty_pg_catalog_views(providers: &mut BTreeMap<String, Arc<dyn TableProvider>>) {
    for (name, fields) in empty_tables::empty_pg_catalog_table_defs() {
        if providers.contains_key(name) {
            continue;
        }
        providers.insert(
            name.to_string(),
            Arc::new(PgCatalogViewTableProvider::new(Arc::new(empty::EmptyPgCatalogView::new(
                name, fields,
            )))),
        );
    }
}

/// Column names for a `#[table]` pg_catalog relation, in ordinal order.
pub fn relation_column_order(name: &str) -> Option<&'static [&'static str]> {
    match name {
        "pg_class" => Some(class::column_order()),
        "pg_attribute" => Some(attribute::column_order()),
        "pg_namespace" => Some(namespace::column_order()),
        _ => None,
    }
}

#[async_trait]
impl SchemaProvider for PgCatalogSchemaProvider {
    fn table_names(&self) -> Vec<String> {
        self.providers.keys().cloned().collect()
    }

    fn table_exist(&self, name: &str) -> bool {
        self.providers.contains_key(name)
    }

    async fn table(&self, name: &str) -> DataFusionResult<Option<Arc<dyn TableProvider>>> {
        Ok(self.providers.get(name).cloned())
    }

    async fn table_type(&self, name: &str) -> DataFusionResult<Option<TableType>> {
        Ok(self.providers.get(name).map(|provider| provider.table_type()))
    }
}
