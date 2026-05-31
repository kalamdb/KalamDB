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
        self.offset_allocator.clear();
        self.group_claim_state.clear();
        self.consumer_groups.clear();
        self.partition_write_locks.clear();
        self.retained_bytes.clear();
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
        self.offset_allocator.clear_topic(topic_id);

        let claim_keys: Vec<_> = self
            .group_claim_state
            .iter()
            .filter(|entry| entry.key().topic_id == *topic_id)
            .map(|entry| entry.key().clone())
            .collect();
        for key in claim_keys {
            self.group_claim_state.remove(&key);
        }

        let consumer_keys: Vec<_> = self
            .consumer_groups
            .iter()
            .filter(|entry| entry.key().topic_id == *topic_id)
            .map(|entry| entry.key().clone())
            .collect();
        for key in consumer_keys {
            self.consumer_groups.remove(&key);
        }

        let lock_keys: Vec<_> = self
            .partition_write_locks
            .iter()
            .filter(|entry| entry.key().topic_id == *topic_id)
            .map(|entry| entry.key().clone())
            .collect();
        for key in lock_keys {
            self.partition_write_locks.remove(&key);
        }

        let retained_keys: Vec<_> = self
            .retained_bytes
            .iter()
            .filter(|entry| entry.key().topic_id == *topic_id)
            .map(|entry| entry.key().clone())
            .collect();
        for key in retained_keys {
            self.retained_bytes.remove(&key);
        }
    }
}
