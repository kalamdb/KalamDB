use kalamdb_commons::models::{NamespaceId, ScheduleId};

use crate::ddl::{create_schedule::ScheduleParser, DdlResult};
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DropScheduleStatement {
    pub schedule_id: ScheduleId,
    pub if_exists:   bool,
}
impl DropScheduleStatement {
    pub fn parse(sql: &str, namespace: &NamespaceId) -> DdlResult<Self> {
        let mut p = ScheduleParser::new(sql)?;
        p.keyword("DROP")?;
        p.keyword("SCHEDULE")?;
        let if_exists = p.take_keyword("IF");
        if if_exists {
            p.keyword("EXISTS")?;
        }
        let (namespace, name) = p.name(namespace)?;
        p.end()?;
        Ok(Self {
            schedule_id: ScheduleId::from_parts(Some(&namespace), &name),
            if_exists,
        })
    }
}
