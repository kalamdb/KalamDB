use crate::model::Topic;

pub(super) fn emit_create_topic(topic: &Topic) -> String {
    let mut sql = "CREATE TOPIC ".to_string();

    if topic.if_not_exists {
        sql.push_str("IF NOT EXISTS ");
    }

    sql.push_str(&topic.name_sql);

    if let Some(partitions) = topic.partitions {
        sql.push_str(&format!(" PARTITIONS {partitions}"));
    }

    if !topic.retention.is_empty() {
        sql.push(' ');
        sql.push_str(&crate::emitter::set_topic_retention::emit_topic_retention(&topic.retention));
    }

    sql.push(';');
    sql
}
