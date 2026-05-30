mod notification;

use kalamdb_commons::schemas::TableDefinition;
pub use notification::NotificationRow;

use crate::error::Result;

pub const DBA_NAMESPACE: &str = "dba";

pub fn bootstrap_table_definitions() -> Result<Vec<TableDefinition>> {
    Ok(vec![NotificationRow::definition()])
}
