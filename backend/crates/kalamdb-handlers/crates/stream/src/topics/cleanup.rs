use std::sync::Arc;

use kalamdb_commons::models::TopicId;
use kalamdb_core::{app_context::AppContext, error::KalamDbError};

pub(super) fn clear_topic_data(
    app_context: &Arc<AppContext>,
    topic_id: &TopicId,
) -> Result<(usize, usize), KalamDbError> {
    app_context
        .topic_publisher()
        .clear_topic_data(topic_id)
        .map_err(|e| KalamDbError::ExecutionError(e.to_string()))
}