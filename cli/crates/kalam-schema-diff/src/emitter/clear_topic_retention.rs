pub(super) fn emit_clear_topic_retention(name_sql: &str) -> String {
    format!("ALTER TOPIC {name_sql} CLEAR RETENTION;")
}
