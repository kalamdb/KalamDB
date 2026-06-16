mod table;
mod topic;

use crate::{
    emitter::{
        table::{diff_existing_table, emit_create_table},
        topic::{diff_existing_topic, emit_add_topic_source, emit_create_topic},
    },
    model::Schema,
};

pub(crate) fn diff_schema(current: &Schema, target: &Schema, allow_drop: bool) -> Vec<String> {
    let mut out = vec![
        "-- Generated KalamDB schema evolution".to_string(),
        "-- Review before applying in production.".to_string(),
        String::new(),
    ];

    for namespace in target.namespaces.difference(&current.namespaces) {
        out.push(format!("CREATE NAMESPACE IF NOT EXISTS {namespace};"));
    }

    if !target.namespaces.is_empty() && out.last().map(String::as_str) != Some("") {
        out.push(String::new());
    }

    for (table_key, target_table) in &target.tables {
        match current.tables.get(table_key) {
            Some(current_table) => {
                diff_existing_table(current_table, target_table, allow_drop, &mut out);
            },
            None => {
                out.push(emit_create_table(target_table));
                out.push(String::new());
            },
        }
    }

    for (topic_key, target_topic) in &target.topics {
        match current.topics.get(topic_key) {
            Some(current_topic) => {
                diff_existing_topic(current_topic, target_topic, &mut out);
            },
            None => {
                out.push(emit_create_topic(target_topic));

                for source in target_topic.sources.values() {
                    out.push(emit_add_topic_source(target_topic, source));
                }

                out.push(String::new());
            },
        }
    }

    for (topic_key, current_topic) in &current.topics {
        if !target.topics.contains_key(topic_key) {
            if allow_drop {
                out.push(format!("DROP TOPIC {};", current_topic.name_sql));
            } else {
                out.push(format!(
                    "-- destructive change skipped: topic {} exists in current schema but not in target schema",
                    current_topic.name_sql
                ));
                out.push(format!(
                    "-- rerun with destructive changes enabled to emit: DROP TOPIC {};",
                    current_topic.name_sql
                ));
            }

            out.push(String::new());
        }
    }

    for (table_key, current_table) in &current.tables {
        if !target.tables.contains_key(table_key) {
            if allow_drop {
                out.push(format!("DROP TABLE {};", current_table.name_sql));
            } else {
                out.push(format!(
                    "-- destructive change skipped: table {} exists in current schema but not in target schema",
                    current_table.name_sql
                ));
                out.push(format!(
                    "-- rerun with destructive changes enabled to emit: DROP TABLE {};",
                    current_table.name_sql
                ));
            }
            out.push(String::new());
        }
    }

    if out.iter().all(|line| line.starts_with("--") || line.trim().is_empty()) {
        out.push("-- No schema changes.".to_string());
    }

    out
}
