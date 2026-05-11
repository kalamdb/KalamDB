use kalamdb_commons::{models::TopicId, TableId};
use kalamdb_core::error::KalamDbError;
use kalamdb_system::JobType;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ScheduleErrorKind {
    AlreadyActive,
    PreValidationSkipped,
    Other,
}

pub(crate) fn hourly_date_key() -> String {
    chrono::Utc::now().format("%Y-%m-%d-%H").to_string()
}

pub(crate) fn hourly_table_idempotency_key(
    job_type: JobType,
    table_id: &TableId,
    date_key: &str,
) -> String {
    format!("{}:{}:{}", job_type.short_prefix(), table_id, date_key)
}

pub(crate) fn hourly_topic_idempotency_key(
    job_type: JobType,
    topic_id: &TopicId,
    date_key: &str,
) -> String {
    format!("{}:{}:{}", job_type.short_prefix(), topic_id.as_str(), date_key)
}

pub(crate) fn classify_schedule_error(error: &KalamDbError) -> ScheduleErrorKind {
    match error {
        KalamDbError::IdempotentConflict(_) => ScheduleErrorKind::AlreadyActive,
        KalamDbError::Other(message) if is_pre_validation_skip(message) => {
            ScheduleErrorKind::PreValidationSkipped
        },
        _ => {
            let message = error.to_string();
            if message.contains("already running") || message.contains("already exists") {
                ScheduleErrorKind::AlreadyActive
            } else if is_pre_validation_skip(&message) {
                ScheduleErrorKind::PreValidationSkipped
            } else {
                ScheduleErrorKind::Other
            }
        },
    }
}

fn is_pre_validation_skip(message: &str) -> bool {
    message.contains("pre-validation returned false")
}

#[cfg(test)]
mod tests {
    use kalamdb_commons::{models::TopicId, NamespaceId, TableName};

    use super::*;

    #[test]
    fn hourly_table_key_uses_job_prefix() {
        let table_id = TableId::new(NamespaceId::default(), TableName::new("events"));

        assert_eq!(
            hourly_table_idempotency_key(JobType::Flush, &table_id, "2026-04-29-10"),
            format!("FL:{}:2026-04-29-10", table_id)
        );
    }

    #[test]
    fn hourly_topic_key_uses_job_prefix() {
        let topic_id = TopicId::new("app.events");

        assert_eq!(
            hourly_topic_idempotency_key(JobType::TopicRetention, &topic_id, "2026-04-29-10"),
            "TR:app.events:2026-04-29-10"
        );
    }

    #[test]
    fn classify_idempotent_conflict() {
        let error = KalamDbError::IdempotentConflict("duplicate".to_string());

        assert_eq!(classify_schedule_error(&error), ScheduleErrorKind::AlreadyActive);
    }

    #[test]
    fn classify_pre_validation_skip() {
        let error = KalamDbError::Other(
            "Job flush skipped: pre-validation returned false (nothing to do)".to_string(),
        );

        assert_eq!(classify_schedule_error(&error), ScheduleErrorKind::PreValidationSkipped);
    }
}
