use crate::{
    emitter::{
        add_topic_source::emit_add_topic_source, clear_topic_retention::emit_clear_topic_retention,
        set_topic_retention::emit_set_topic_retention,
    },
    model::Topic,
};

pub(super) fn diff_existing_topic(current: &Topic, target: &Topic, out: &mut Vec<String>) {
    let start_len = out.len();

    if current.partitions != target.partitions {
        out.push(format!(
            "-- manual review required: topic {} changed partition count from {:?} to {:?}",
            target.name_sql, current.partitions, target.partitions
        ));
    }

    if current.retention != target.retention {
        if target.retention.is_empty() {
            out.push(emit_clear_topic_retention(&target.name_sql));
        } else {
            out.push(emit_set_topic_retention(&target.name_sql, &target.retention));
        }
    }

    for (source_key, target_source) in &target.sources {
        if !current.sources.contains_key(source_key) {
            out.push(emit_add_topic_source(target, target_source));
        }
    }

    for (source_key, current_source) in &current.sources {
        if !target.sources.contains_key(source_key) {
            out.push(format!(
                "-- manual review required: topic {} source {} on {} was removed from target \
                 schema",
                target.name_sql,
                current_source.table_sql,
                current_source.operation.as_sql()
            ));
        }
    }

    if out.len() > start_len {
        out.push(String::new());
    }
}
