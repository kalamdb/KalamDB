//! Minimal pg_catalog compatibility views for PostgreSQL wire clients.

use std::{
    collections::BTreeMap,
    hash::{Hash, Hasher},
    sync::Arc,
};

use async_trait::async_trait;
use datafusion::{
    arrow::{
        datatypes::SchemaRef,
        record_batch::RecordBatch,
    },
    catalog::{SchemaProvider, Session},
    common::DFSchema,
    datasource::{TableProvider, TableType},
    error::{DataFusionError, Result as DataFusionResult},
    logical_expr::{Expr, TableProviderFilterPushDown},
    physical_expr::PhysicalExpr,
    physical_plan::ExecutionPlan,
};
use kalamdb_commons::{
    schemas::{TableDefinition, TableType as KalamTableType},
    Role, SystemTable,
};
use kalamdb_datafusion_sources::{
    exec::{finalize_deferred_batch, DeferredBatchExec, DeferredBatchSource},
    provider::{combined_filter, pushdown_results_for_filters, FilterCapability},
};
use kalamdb_session::{can_access_system_table, can_access_user_table, is_admin_role};
use kalamdb_session_datafusion::{extract_user_role, PermissionChecker};
use kalamdb_system::SystemTablesRegistry;

use crate::{
    error::RegistryError,
    system::{sessions::SessionsSnapshotCallback, system_view_table_definition},
};

pub mod attribute;
pub mod class;
pub mod database;
pub mod empty;
pub mod namespace;
pub mod stat_activity;
pub mod tables;
pub mod r#type;
pub mod type_mapping;
pub mod views;

pub trait PgCatalogView: Send + Sync + std::fmt::Debug {
    fn name(&self) -> &'static str;
    fn schema(&self) -> SchemaRef;
    fn required_system_table(&self) -> Option<SystemTable> {
        None
    }
    fn compute_batch(&self, role: Role) -> Result<RecordBatch, RegistryError>;
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
        let batch = self.view.compute_batch(self.role).map_err(|error| {
            DataFusionError::Execution(format!(
                "failed to compute pg_catalog.{}: {}",
                self.view.name(),
                error
            ))
        })?;

        finalize_deferred_batch(
            batch,
            self.physical_filter.as_ref(),
            self.projection.as_deref(),
            self.limit,
            self.source_name())
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
        limit: Option<usize>) -> DataFusionResult<Arc<dyn ExecutionPlan>> {
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
        }))))
    }

    fn supports_filters_pushdown(
        &self,
        filters: &[&Expr]) -> DataFusionResult<Vec<TableProviderFilterPushDown>> {
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
        sessions_snapshot_callback: SessionsSnapshotCallback) -> Self {
        let mut providers = BTreeMap::<String, Arc<dyn TableProvider>>::new();
        providers.insert(
            "pg_namespace".to_string(),
            Arc::new(PgCatalogViewTableProvider::new(Arc::new(namespace::PgNamespaceView::new(
                Arc::clone(&system_registry))))));
        providers.insert(
            "pg_class".to_string(),
            Arc::new(PgCatalogViewTableProvider::new(Arc::new(class::PgClassView::new(
                Arc::clone(&system_registry))))));
        providers.insert(
            "pg_attribute".to_string(),
            Arc::new(PgCatalogViewTableProvider::new(Arc::new(attribute::PgAttributeView::new(
                Arc::clone(&system_registry))))));
        providers.insert(
            "pg_type".to_string(),
            Arc::new(PgCatalogViewTableProvider::new(Arc::new(r#type::PgTypeView::new(
                Arc::clone(&system_registry))))));
        providers.insert(
            "pg_database".to_string(),
            Arc::new(PgCatalogViewTableProvider::new(Arc::new(database::PgDatabaseView::new(
                "kalam")))));
        providers.insert(
            "pg_stat_activity".to_string(),
            Arc::new(PgCatalogViewTableProvider::new(Arc::new(
                stat_activity::PgStatActivityView::new(sessions_snapshot_callback)))));
        providers.insert(
            "pg_tables".to_string(),
            Arc::new(PgCatalogViewTableProvider::new(Arc::new(tables::PgTablesView::new(
                Arc::clone(&system_registry))))));
        providers.insert(
            "pg_views".to_string(),
            Arc::new(PgCatalogViewTableProvider::new(Arc::new(views::PgViewsView::new(
                Arc::clone(&system_registry))))));
        register_empty_pg_catalog_views(&mut providers);

        Self { providers }
    }
}

fn register_empty_pg_catalog_views(providers: &mut BTreeMap<String, Arc<dyn TableProvider>>) {
    // Single source of truth: kalamdb-postgres-wire client_catalog (compiled in-place via path).
    #[allow(dead_code)]
    #[path = "../../../kalamdb-postgres-wire/src/client_catalog/empty_tables.rs"]
    mod wire_empty_tables;

    for (name, fields) in wire_empty_tables::empty_pg_catalog_table_defs() {
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

pub(crate) fn stable_oid(parts: &[&str]) -> i64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    for part in parts {
        part.hash(&mut hasher);
    }
    (hasher.finish() & 0x7fff_ffff) as i64
}

pub(crate) fn visible_table_definitions(
    system_registry: &SystemTablesRegistry,
    role: Role) -> Result<Vec<TableDefinition>, RegistryError> {
    let mut definitions = BTreeMap::<(String, String), TableDefinition>::new();

    let tables = system_registry
        .tables()
        .list_tables()
        .map_err(|error| RegistryError::Other(format!("failed to list tables: {error}")))?;
    for table in tables {
        insert_catalog_definition(&mut definitions, table);
    }

    for table in supplemental_catalog_definitions(system_registry) {
        insert_catalog_definition(&mut definitions, table);
    }

    Ok(definitions
        .into_values()
        .filter(|table| table_visible_to_role(table, role))
        .collect())
}

fn insert_catalog_definition(
    definitions: &mut BTreeMap<(String, String), TableDefinition>,
    table: TableDefinition) {
    let key = (table.namespace_id.as_str().to_string(), table.table_name.as_str().to_string());
    definitions.entry(key).or_insert(table);
}

fn supplemental_catalog_definitions(
    system_registry: &SystemTablesRegistry) -> Vec<TableDefinition> {
    let mut definitions = system_registry
        .expected_system_table_definitions()
        .into_iter()
        .map(|definition| definition.as_ref().clone())
        .collect::<Vec<_>>();

    for system_table in SystemTable::all_views() {
        definitions.push(system_view_table_definition(*system_table));
    }

    definitions
}

pub(crate) fn include_explicit_namespaces(role: Role) -> bool {
    is_admin_role(role)
}

fn table_visible_to_role(table: &TableDefinition, role: Role) -> bool {
    match table.table_type {
        KalamTableType::System => can_access_system_table(role),
        KalamTableType::Shared => !matches!(role, Role::Anonymous),
        KalamTableType::User | KalamTableType::Stream => can_access_user_table(role),
    }
}

/// Whether a visible relation should be classified as a PostgreSQL view.
pub(crate) fn relation_is_view(table: &TableDefinition) -> bool {
    if table.namespace_id.as_str() != "system" {
        return false;
    }

    SystemTable::from_name(table.table_name.as_str())
        .map(|system_table| system_table.is_view())
        .unwrap_or(false)
}

/// PostgreSQL `pg_class.relkind` for a visible relation.
pub(crate) fn pg_relkind(table: &TableDefinition) -> &'static str {
    if relation_is_view(table) {
        "v"
    } else {
        "r"
    }
}
