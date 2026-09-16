use kalamdb_commons::ScheduleId;
use kalamdb_system::CatalogSchedule;
use serde::{Deserialize, Serialize};

/// One independent compare-and-swap in a bounded schedule batch.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScheduleUpdate {
    pub schedule_id:      ScheduleId,
    pub expected_version: Option<String>,
    pub replacement:      Option<CatalogSchedule>,
}
