use std::sync::{Arc, OnceLock};

use datafusion::{
    arrow::{array::RecordBatch, datatypes::SchemaRef},
    logical_expr::Expr,
};
use kalamdb_commons::{ScheduleId, SystemTable};
use kalamdb_store::StorageBackend;

use super::{
    models::CatalogSchedule,
    scan::{scan_all_rows, scan_filtered_rows},
    CatalogStores,
};
use crate::{error::SystemError, providers::base::SimpleProviderDefinition};

#[derive(Clone)]
pub struct SchedulesTableProvider {
    stores: CatalogStores,
}

impl SchedulesTableProvider {
    pub fn new(backend: Arc<dyn StorageBackend>) -> Self {
        Self {
            stores: CatalogStores::new(backend),
        }
    }

    pub fn from_stores(stores: CatalogStores) -> Self {
        Self { stores }
    }

    fn scan_all_schedules(&self) -> Result<RecordBatch, SystemError> {
        scan_all_rows(&self.stores.schedules, &Self::schema(), &CatalogSchedule::definition())
    }

    fn scan_to_batch_filtered(
        &self,
        filters: &[Expr],
        limit: Option<usize>,
    ) -> Result<RecordBatch, SystemError> {
        scan_filtered_rows(
            &self.stores.schedules,
            &Self::schema(),
            &CatalogSchedule::definition(),
            "schedule_id",
            |value| Some(ScheduleId::new(value)),
            filters,
            limit,
        )
    }
}

crate::impl_system_table_provider_metadata!(
    simple,
    provider = SchedulesTableProvider,
    table_name = SystemTable::Schedules.table_name(),
    schema = CatalogSchedule::definition()
        .to_arrow_schema()
        .expect("failed to build schedules schema")
);

crate::impl_simple_system_table_provider!(
    provider = SchedulesTableProvider,
    key = ScheduleId,
    value = CatalogSchedule,
    definition = provider_definition,
    scan_all = scan_all_schedules,
    scan_filtered = scan_to_batch_filtered
);
