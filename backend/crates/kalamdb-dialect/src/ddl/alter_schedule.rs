use kalamdb_commons::models::{NamespaceId, ScheduleId};

use crate::ddl::{create_schedule::ScheduleParser, DdlResult};
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AlterScheduleStatement {
    pub schedule_id: ScheduleId,
    pub enabled:     bool,
}
impl AlterScheduleStatement {
    pub fn parse(sql: &str, namespace: &NamespaceId) -> DdlResult<Self> {
        let mut p = ScheduleParser::new(sql)?;
        p.keyword("ALTER")?;
        p.keyword("SCHEDULE")?;
        let (namespace, name) = p.name(namespace)?;
        let enabled = if p.take_keyword("ENABLE") {
            true
        } else {
            p.keyword("DISABLE")?;
            false
        };
        p.end()?;
        Ok(Self {
            schedule_id: ScheduleId::from_parts(Some(&namespace), &name),
            enabled,
        })
    }
}
