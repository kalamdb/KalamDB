use crate::model::TopicRetention;

pub(super) fn emit_set_topic_retention(name_sql: &str, retention: &TopicRetention) -> String {
    format!("ALTER TOPIC {name_sql} SET RETENTION {};", emit_topic_retention(retention))
}

pub(super) fn emit_topic_retention(retention: &TopicRetention) -> String {
    let mut options = Vec::new();

    if let Some(retention_seconds) = &retention.retention_seconds {
        options.push(format!("retention_seconds = {retention_seconds}"));
    }

    if let Some(retention_max_bytes) = &retention.retention_max_bytes {
        options.push(format!("retention_max_bytes = {retention_max_bytes}"));
    }

    format!("WITH ({})", options.join(", "))
}
