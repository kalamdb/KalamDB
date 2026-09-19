use super::*;

impl TopicPublisherService {
    #[inline]
    pub fn has_topics_for_table(&self, table_id: &TableId) -> bool {
        self.route_cache.has_topics_for_table(table_id)
    }

    #[inline]
    pub fn has_topics_for_table_op(&self, table_id: &TableId, operation: &TopicOp) -> bool {
        self.route_cache.has_topics_for_table_op(table_id, operation)
    }

    pub fn topic_exists(&self, topic_id: &TopicId) -> bool {
        self.route_cache.topic_exists(topic_id)
    }

    pub fn get_topic(&self, topic_id: &TopicId) -> Option<Topic> {
        self.route_cache.get_topic(topic_id)
    }

    pub fn get_topic_ids_for_table(&self, table_id: &TableId) -> Vec<TopicId> {
        self.route_cache.get_topic_ids_for_table(table_id)
    }

    pub fn refresh_topics_cache(&self, topics: Vec<Topic>) {
        self.route_cache.refresh(topics);
    }

    pub fn add_topic(&self, topic: Topic) {
        self.route_cache.add_topic(topic);
    }

    pub fn remove_topic(&self, topic_id: &TopicId) {
        self.clear_topic_runtime_state(topic_id);
        self.route_cache.remove_topic(topic_id);
    }

    pub fn update_topic(&self, topic: Topic) {
        self.route_cache.update_topic(topic);
    }

    pub fn clear_cache(&self) {
        self.route_cache.clear();
        self.partitions.clear();
        self.group_claim_state.clear();
    }

    pub fn clear_topic_data(&self, topic_id: &TopicId) -> Result<(usize, usize)> {
        let offsets_deleted = self
            .offset_store
            .delete_topic_offsets(topic_id)
            .map_err(|e| CommonError::Internal(format!("Failed to delete topic offsets: {}", e)))?;
        let messages_deleted = self.message_store.delete_topic_messages(topic_id).map_err(|e| {
            CommonError::Internal(format!("Failed to delete topic messages: {}", e))
        })?;

        self.clear_topic_runtime_state(topic_id);

        Ok((offsets_deleted, messages_deleted))
    }

    fn clear_topic_runtime_state(&self, topic_id: &TopicId) {
        self.partitions.retain(|key, _| key.topic_id != *topic_id);
        self.group_claim_state.retain(|key, _| key.topic_id != *topic_id);
        shrink_dashmap_if_sparse(&self.partitions);
        shrink_dashmap_if_sparse(&self.group_claim_state);
    }
}
