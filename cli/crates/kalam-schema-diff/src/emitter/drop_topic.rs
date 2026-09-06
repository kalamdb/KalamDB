use crate::model::Topic;

pub(super) fn emit_drop_topic(topic: &Topic, allow_drop: bool, out: &mut Vec<String>) {
    if allow_drop {
        out.push(format!("DROP TOPIC {};", topic.name_sql));
    } else {
        out.push(format!(
            "-- destructive change skipped: topic {} exists in current schema but not in target \
             schema",
            topic.name_sql
        ));
        out.push(format!(
            "-- rerun with destructive changes enabled to emit: DROP TOPIC {};",
            topic.name_sql
        ));
    }

    out.push(String::new());
}
