use super::*;

impl TopicPublisherService {
    pub fn enforce_retention(
        &self,
        topic: &Topic,
        partition_id: u32,
        cutoff_timestamp_ms: Option<i64>,
        max_bytes: Option<i64>,
        batch_size: usize,
    ) -> Result<TopicRetentionDeletionStats> {
        if batch_size == 0 {
            return Err(CommonError::InvalidInput("batch_size must be greater than 0".to_string()));
        }
        if partition_id >= topic.partitions {
            return Err(CommonError::InvalidInput(format!(
                "Partition {} is outside topic {} partition count {}",
                partition_id,
                topic.topic_id.as_str(),
                topic.partitions
            )));
        }

        let lock = self.partition_write_lock(&topic.topic_id, partition_id);
        let _guard = lock.lock().unwrap_or_else(|e| e.into_inner());

        let mut stats = TopicRetentionDeletionStats::default();

        if let Some(cutoff) = cutoff_timestamp_ms {
            let entries = self
                .message_store
                .retention_entries_before(
                    &topic.topic_id,
                    partition_id,
                    cutoff,
                    batch_size.saturating_sub(stats.messages_deleted),
                )
                .map_err(|e| {
                    CommonError::Internal(format!("Failed to scan age retention: {}", e))
                })?;
            let deleted = self.message_store.delete_retention_entries(entries).map_err(|e| {
                CommonError::Internal(format!("Failed to delete expired messages: {}", e))
            })?;
            self.subtract_retained_bytes(&topic.topic_id, partition_id, deleted.bytes_freed);
            stats.messages_deleted += deleted.messages_deleted;
            stats.bytes_freed += deleted.bytes_freed;
        }

        if let Some(max_bytes) = max_bytes {
            let max_bytes = max_bytes.max(0) as u64;
            let mut retained_bytes =
                self.retained_bytes_for_partition(&topic.topic_id, partition_id)?;
            while retained_bytes > max_bytes && stats.messages_deleted < batch_size {
                let remaining = batch_size - stats.messages_deleted;
                let entries = self
                    .message_store
                    .retention_entries_for_partition(&topic.topic_id, partition_id, remaining)
                    .map_err(|e| {
                        CommonError::Internal(format!("Failed to scan byte retention: {}", e))
                    })?;
                if entries.is_empty() {
                    self.set_retained_bytes(&topic.topic_id, partition_id, 0);
                    break;
                }

                let mut selected_entries = Vec::new();
                let mut selected_bytes = 0u64;
                for entry in entries {
                    selected_bytes = selected_bytes.saturating_add(entry.1.message_bytes);
                    selected_entries.push(entry);
                    if retained_bytes.saturating_sub(selected_bytes) <= max_bytes {
                        break;
                    }
                }

                let deleted =
                    self.message_store.delete_retention_entries(selected_entries).map_err(|e| {
                        CommonError::Internal(format!("Failed to delete oversized messages: {}", e))
                    })?;
                if deleted.messages_deleted == 0 {
                    break;
                }

                retained_bytes = retained_bytes.saturating_sub(deleted.bytes_freed);
                self.subtract_retained_bytes(&topic.topic_id, partition_id, deleted.bytes_freed);
                stats.messages_deleted += deleted.messages_deleted;
                stats.bytes_freed += deleted.bytes_freed;
            }
        }

        Ok(stats)
    }

    pub fn restore_offset_counters(&self) {
        for entry in self.route_cache.iter_topics() {
            let topic = entry.value();
            for partition_id in 0..topic.partitions {
                match self.message_store.latest_offset(&topic.topic_id, partition_id) {
                    Ok(Some(last_offset)) => {
                        let next = last_offset + 1;
                        self.offset_allocator.seed(&topic.topic_id, partition_id, next);
                        log::debug!(
                            "Restored offset counter for topic={} partition={}: next_offset={}",
                            topic.topic_id.as_str(),
                            partition_id,
                            next,
                        );
                    },
                    Ok(None) => {},
                    Err(e) => {
                        log::warn!(
                            "Failed to restore offset for topic={} partition={}: {}",
                            topic.topic_id.as_str(),
                            partition_id,
                            e,
                        );
                    },
                }
            }
        }
    }
}
