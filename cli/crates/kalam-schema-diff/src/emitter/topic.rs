use crate::model::{Topic, TopicPayloadMode, TopicRetention, TopicSource};

pub(super) fn diff_existing_topic(current: &Topic, target: &Topic, out: &mut Vec<String>) {
    let mut emitted_for_topic = false;

    if current.partitions != target.partitions {
        out.push(format!(
            "-- manual review required: topic {} changed partition count from {:?} to {:?}",
            target.name_sql, current.partitions, target.partitions
        ));
        emitted_for_topic = true;
    }

    if current.retention != target.retention {
        if target.retention.is_empty() {
            out.push(format!("ALTER TOPIC {} CLEAR RETENTION;", target.name_sql));
        } else {
            out.push(format!(
                "ALTER TOPIC {} SET RETENTION {};",
                target.name_sql,
                emit_topic_retention(&target.retention)
            ));
        }

        emitted_for_topic = true;
    }

    for (source_key, target_source) in &target.sources {
        if !current.sources.contains_key(source_key) {
            out.push(emit_add_topic_source(target, target_source));
            emitted_for_topic = true;
        }
    }

    for (source_key, current_source) in &current.sources {
        if !target.sources.contains_key(source_key) {
            out.push(format!(
                "-- manual review required: topic {} source {} on {} was removed from target schema",
                target.name_sql,
                current_source.table_sql,
                current_source.operation.as_sql()
            ));
            emitted_for_topic = true;
        }
    }

    if emitted_for_topic {
        out.push(String::new());
    }
}

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
        sql.push_str(&emit_topic_retention(&topic.retention));
    }

    sql.push(';');
    sql
}

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

fn emit_topic_retention(retention: &TopicRetention) -> String {
    let mut options = Vec::new();

    if let Some(retention_seconds) = &retention.retention_seconds {
        options.push(format!("retention_seconds = {retention_seconds}"));
    }

    if let Some(retention_max_bytes) = &retention.retention_max_bytes {
        options.push(format!("retention_max_bytes = {retention_max_bytes}"));
    }

    format!("WITH ({})", options.join(", "))
}
