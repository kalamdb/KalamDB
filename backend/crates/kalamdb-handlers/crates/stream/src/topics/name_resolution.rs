use kalamdb_commons::models::TopicId;
use kalamdb_core::sql::context::ExecutionContext;

pub fn resolve_topic_name(topic_name: &str, context: &ExecutionContext) -> String {
    if topic_name.contains('.') {
        topic_name.to_string()
    } else {
        format!("{}.{}", context.default_namespace().as_str(), topic_name)
    }
}

pub fn resolve_topic_id(topic_name: &str, context: &ExecutionContext) -> TopicId {
    TopicId::new(resolve_topic_name(topic_name, context))
}
