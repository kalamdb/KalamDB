//! Minimal pg_catalog compatibility views for PostgreSQL wire clients.

use std::{collections::BTreeMap, sync::Arc};

use async_trait::async_trait;
use datafusion::{
    arrow::{datatypes::SchemaRef, record_batch::RecordBatch},
    catalog::{SchemaProvider, Session},
    common::DFSchema,
    datasource::{TableProvider, TableType},
    error::{DataFusionError, Result as DataFusionResult},
    logical_expr::{Expr, TableProviderFilterPushDown},
    physical_expr::PhysicalExpr,
    physical_plan::ExecutionPlan,
};
use kalamdb_commons::{constants::LOGICAL_DATABASE_NAME, Role, SystemTable};
use kalamdb_datafusion_sources::{
    exec::{finalize_deferred_batch, DeferredBatchExec, DeferredBatchSource},
    provider::{combined_filter, pushdown_results_for_filters, FilterCapability},
};
use kalamdb_session_datafusion::{extract_user_role, PermissionChecker, SessionConfigValues};
use kalamdb_system::SystemTablesRegistry;

use crate::{error::RegistryError, system::sessions::SessionsSnapshotCallback};

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

#[derive(Debug, Clone)]
pub struct PgCatalogViewTableProvider<V: PgCatalogView> {
    view: Arc<V>,
}

impl<V: PgCatalogView> PgCatalogViewTableProvider<V> {
    pub fn new(view: Arc<V>) -> Self {
        Self { view }
    }
}

struct PgCatalogScanSource<V: PgCatalogView> {
    view:            Arc<V>,
    physical_filter: Option<Arc<dyn PhysicalExpr>>,
    projection:      Option<Vec<usize>>,
    limit:           Option<usize>,
    output_schema:   SchemaRef,
    role:            Role,
    settings:        Option<SessionConfigValues>,
}

impl<V: PgCatalogView> std::fmt::Debug for PgCatalogScanSource<V> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PgCatalogScanSource")
            .field("view_name", &self.view.name())
            .field("projection", &self.projection)
            .field("limit", &self.limit)
            .finish()
    }
}

#[async_trait]
impl<V: PgCatalogView + 'static> DeferredBatchSource for PgCatalogScanSource<V> {
    fn source_name(&self) -> &'static str {
        "pg_catalog_view_scan"
    }

    fn schema(&self) -> SchemaRef {
        Arc::clone(&self.output_schema)
    }

    async fn produce_batch(&self) -> DataFusionResult<RecordBatch> {
        let batch = self.view.compute_session_batch(self.role, self.settings.as_ref()).map_err(
            |error| {
                DataFusionError::Execution(format!(
                    "failed to compute pg_catalog.{}: {}",
                    self.view.name(),
                    error
                ))
            },
        )?;

        finalize_deferred_batch(
            batch,
            self.physical_filter.as_ref(),
            self.projection.as_deref(),
            self.limit,
            self.source_name(),
        )
    }
}

#[async_trait]
impl<V: PgCatalogView + 'static> TableProvider for PgCatalogViewTableProvider<V> {
    fn schema(&self) -> SchemaRef {
        self.view.schema()
    }

    fn table_type(&self) -> TableType {
        TableType::View
    }

    async fn scan(
        &self,
        state: &dyn Session,
        projection: Option<&Vec<usize>>,
        filters: &[Expr],
        limit: Option<usize>,
    ) -> DataFusionResult<Arc<dyn ExecutionPlan>> {
        if let Some(system_table) = self.view.required_system_table() {
            PermissionChecker::check_system_table(state, &system_table.table_id())?;
        }

        let base_schema = self.view.schema();
        let output_schema = match projection {
            Some(indices) => base_schema
                .project(indices)
                .map(Arc::new)
                .map_err(|error| DataFusionError::ArrowError(Box::new(error), None))?,
            None => Arc::clone(&base_schema),
        };
        let physical_filter = if let Some(filter) = combined_filter(filters) {
            let df_schema = DFSchema::try_from(Arc::clone(&base_schema))?;
            Some(state.create_physical_expr(filter, &df_schema)?)
        } else {
            None
        };
        let role = extract_user_role(state);

        Ok(Arc::new(DeferredBatchExec::new(Arc::new(PgCatalogScanSource {
            view: Arc::clone(&self.view),
            physical_filter,
            projection: projection.cloned(),
            limit,
            output_schema,
            role,
            settings: state.config_options().extensions.get::<SessionConfigValues>().cloned(),
        }))))
    }

    fn supports_filters_pushdown(
        &self,
        filters: &[&Expr],
    ) -> DataFusionResult<Vec<TableProviderFilterPushDown>> {
        Ok(pushdown_results_for_filters(filters, |_| FilterCapability::Exact))
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
