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

        let partition = self.partition_runtime(&topic.topic_id, partition_id);
        let mut stats = TopicRetentionDeletionStats::default();

        // Scans and deletes stay off the publish lock. Publishers take that
        // lock to assign the next offset, so holding it across a retention
        // batch stalls every checkout that publishes.
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
            self.delete_retention_batch(&partition, entries, &mut stats)?;
        }

        if let Some(max_bytes) = max_bytes {
            let max_bytes = max_bytes.max(0) as u64;
            while stats.messages_deleted < batch_size {
                let retained_bytes = self.retained_bytes_without_blocking_publish(
                    &partition,
                    &topic.topic_id,
                    partition_id,
                )?;
                if retained_bytes <= max_bytes {
                    break;
                }
                let remaining = batch_size - stats.messages_deleted;
                let entries = self
                    .message_store
                    .retention_entries_for_partition(&topic.topic_id, partition_id, remaining)
                    .map_err(|e| {
                        CommonError::Internal(format!("Failed to scan byte retention: {}", e))
                    })?;
                if entries.is_empty() {
                    partition.lock_write().set_retained_bytes(0);
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

                let before = stats.messages_deleted;
                self.delete_retention_batch(&partition, selected_entries, &mut stats)?;
                if stats.messages_deleted == before {
                    break;
                }
            }
        }

        if stats.messages_deleted > 0 {
            let log_start =
                match self.message_store.earliest_offset(&topic.topic_id, partition_id).map_err(
                    |e| CommonError::Internal(format!("Failed to fetch earliest offset: {}", e)),
                )? {
                    Some(offset) => offset,
                    None => partition.lock_write().peek_next().unwrap_or(0),
                };
            partition.lock_write().set_log_start(log_start);
            partition.trim_tail_below(log_start);
        }

        Ok(stats)
    }

    /// Delete expired messages in small RocksDB batches.
    ///
    /// The partition write lock is taken only to adjust the retained-byte
    /// counter, so a publish can allocate its offset between chunks.
    fn delete_retention_batch(
        &self,
        partition: &PartitionRuntime,
        entries: Vec<(crate::TopicRetentionIndexKey, crate::TopicRetentionIndexEntry)>,
        stats: &mut TopicRetentionDeletionStats,
    ) -> Result<()> {
        const CHUNK: usize = 512;
        let mut rest = entries;
        while !rest.is_empty() {
            let end = CHUNK.min(rest.len());
            let chunk = rest.drain(..end).collect();
            let deleted = self.message_store.delete_retention_entries(chunk).map_err(|e| {
                CommonError::Internal(format!("Failed to delete expired messages: {}", e))
            })?;
            if deleted.messages_deleted == 0 {
                break;
            }
            {
                let mut write = partition.lock_write();
                write.subtract_retained_bytes(deleted.bytes_freed);
            }
            stats.messages_deleted += deleted.messages_deleted;
            stats.bytes_freed += deleted.bytes_freed;
        }
        Ok(())
    }

    fn retained_bytes_without_blocking_publish(
        &self,
        partition: &PartitionRuntime,
        topic_id: &TopicId,
        partition_id: u32,
    ) -> Result<u64> {
        if partition.lock_write().retained_bytes().is_some() {
            let mut write = partition.lock_write();
            return self.load_retained_bytes(&mut write, topic_id, partition_id);
        }

        let bytes = self
            .message_store
            .retained_bytes_for_partition(topic_id, partition_id)
            .map_err(|e| CommonError::Internal(format!("Failed to read retained bytes: {}", e)))?;
        let mut write = partition.lock_write();
        if write.retained_bytes().is_none() {
            write.set_retained_bytes(bytes);
        }
        Ok(write.retained_bytes().unwrap_or(bytes))
    }

    pub fn restore_offset_counters(&self) {
        for entry in self.route_cache.iter_topics() {
            let topic = entry.value();
            for partition_id in 0..topic.partitions {
                match self.message_store.latest_offset(&topic.topic_id, partition_id) {
                    Ok(Some(last_offset)) => {
                        let next = last_offset + 1;
                        self.seed_next_offset(&topic.topic_id, partition_id, next);
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
