use crate::model::{Topic, TopicPayloadMode, TopicSource};

pub(super) fn emit_add_topic_source(topic: &Topic, source: &TopicSource) -> String {
    let mut sql = format!(
        "ALTER TOPIC {} ADD SOURCE {} ON {}",
        topic.name_sql,
        source.table_sql,
        source.operation.as_sql()
    );

    if let Some(filter_expr) = &source.filter_expr {
        sql.push_str(" WHERE ");
        sql.push_str(filter_expr);
    }

    if source.payload_explicit || source.payload_mode != TopicPayloadMode::Full {
        sql.push_str(&format!(" WITH (payload = '{}')", source.payload_mode.as_sql()));
    }

    sql.push(';');
    sql
}
