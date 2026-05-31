use super::*;

impl TopicPublisherService {
    pub fn fetch_messages(
        &self,
        topic_id: &TopicId,
        partition_id: u32,
        offset: u64,
        limit: usize,
    ) -> Result<Vec<TopicMessage>> {
        if limit == 0 {
            return Ok(Vec::new());
        }

        let earliest = self.earliest_available_offset(topic_id, partition_id)?;
        if offset < earliest {
            let latest =
                self.latest_offset(topic_id, partition_id)?.map(|last| last + 1).unwrap_or(0);
            return Err(CommonError::InvalidInput(format!(
                "OffsetOutOfRange: requested offset {} is before earliest available offset {} \
                 for topic {} partition {} (latest next offset {})",
                offset,
                earliest,
                topic_id.as_str(),
                partition_id,
                latest
            )));
        }

        let messages = self
            .message_store
            .fetch_messages(topic_id, partition_id, offset, limit)
            .map_err(|e| CommonError::Internal(format!("Failed to fetch messages: {}", e)))?;
        let payload_bytes = messages.iter().map(|message| message.payload.len() as u64).sum();
        record_pubsub_messages_consumed(messages.len() as u64, payload_bytes);
        Ok(messages)
    }

    pub fn fetch_messages_for_group(
        &self,
        topic_id: &TopicId,
        group_id: &ConsumerGroupId,
        partition_id: u32,
        start_offset: u64,
        limit: usize,
    ) -> Result<Vec<TopicMessage>> {
        if limit == 0 {
            return Ok(Vec::new());
        }

        self.register_consumer_group(topic_id, group_id);

        let cursor_key = GroupPartitionKey::new(topic_id, group_id, partition_id);

        loop {
            let (effective_start, effective_limit) = {
                let mut state = self
                    .group_claim_state
                    .entry(cursor_key.clone())
                    .or_insert_with(|| ClaimState::new(start_offset));

                state.expire_stale_claims(Instant::now(), self.visibility_timeout);
                state.next_available_window(limit)
            };

            if effective_limit == 0 {
                return Ok(Vec::new());
            }

            let earliest = self.earliest_available_offset(topic_id, partition_id)?;
            if effective_start < earliest {
                let latest =
                    self.latest_offset(topic_id, partition_id)?.map(|last| last + 1).unwrap_or(0);
                return Err(CommonError::InvalidInput(format!(
                    "OffsetOutOfRange: requested offset {} is before earliest available offset {} \
                     for topic {} partition {} (latest next offset {})",
                    effective_start,
                    earliest,
                    topic_id.as_str(),
                    partition_id,
                    latest
                )));
            }

            let messages = self
                .message_store
                .fetch_messages(topic_id, partition_id, effective_start, effective_limit)
                .map_err(|e| CommonError::Internal(format!("Failed to fetch messages: {}", e)))?;

            let Some(last_message) = messages.last() else {
                return Ok(messages);
            };

            let claim_start =
                messages.first().map(|message| message.offset).unwrap_or(effective_start);
            let end_exclusive = last_message.offset + 1;
            let claimed_at = Instant::now();
            let mut state = self
                .group_claim_state
                .entry(cursor_key.clone())
                .or_insert_with(|| ClaimState::new(start_offset));

            state.expire_stale_claims(claimed_at, self.visibility_timeout);
            let (current_start, _) = state.next_available_window(limit);
            if current_start != effective_start {
                continue;
            }

            state.cursor = end_exclusive;
            state.pending.push(PendingClaim {
                start: claim_start,
                end_exclusive,
                claimed_at,
            });

            let payload_bytes = messages.iter().map(|message| message.payload.len() as u64).sum();
            record_pubsub_messages_consumed(messages.len() as u64, payload_bytes);

            return Ok(messages);
        }
    }

    pub fn latest_offset(&self, topic_id: &TopicId, partition_id: u32) -> Result<Option<u64>> {
        let next_offset = self.offset_allocator.peek_next_offset(topic_id, partition_id);

        if let Some(next) = next_offset {
            return Ok(next.checked_sub(1));
        }

        self.message_store
            .latest_offset(topic_id, partition_id)
            .map_err(|e| CommonError::Internal(format!("Failed to fetch latest offset: {}", e)))
    }

    pub fn earliest_available_offset(&self, topic_id: &TopicId, partition_id: u32) -> Result<u64> {
        if let Some(offset) = self
            .message_store
            .earliest_offset(topic_id, partition_id)
            .map_err(|e| CommonError::Internal(format!("Failed to fetch earliest offset: {}", e)))?
        {
            return Ok(offset);
        }

        Ok(self.offset_allocator.peek_next_offset(topic_id, partition_id).unwrap_or(0))
    }

    pub fn ack_offset(
        &self,
        topic_id: &TopicId,
        group_id: &ConsumerGroupId,
        partition_id: u32,
        offset: u64,
    ) -> Result<()> {
        self.register_consumer_group(topic_id, group_id);

        self.offset_store
            .ack_offset(topic_id, group_id, partition_id, offset)
            .map_err(|e| CommonError::Internal(format!("Failed to ack offset: {}", e)))?;

        let cursor_key = GroupPartitionKey::new(topic_id, group_id, partition_id);
        if let Some(mut state) = self.group_claim_state.get_mut(&cursor_key) {
            state.ack_up_to(offset);
        } else {
            self.group_claim_state.insert(cursor_key, ClaimState::new(offset + 1));
        }

        Ok(())
    }

    pub fn reset_group_offset(
        &self,
        topic_id: &TopicId,
        group_id: &ConsumerGroupId,
        partition_id: u32,
        next_offset: u64,
    ) -> Result<()> {
        self.register_consumer_group(topic_id, group_id);

        self.offset_store
            .reset_offset(topic_id, group_id, partition_id, next_offset)
            .map_err(|e| CommonError::Internal(format!("Failed to reset offset: {}", e)))?;

        let cursor_key = GroupPartitionKey::new(topic_id, group_id, partition_id);
        self.group_claim_state.insert(cursor_key, ClaimState::new(next_offset));

        Ok(())
    }

    pub fn get_group_offsets(
        &self,
        topic_id: &TopicId,
        group_id: &ConsumerGroupId,
    ) -> Result<Vec<TopicOffset>> {
        self.offset_store
            .get_group_offsets(topic_id, group_id)
            .map_err(|e| CommonError::Internal(format!("Failed to get offsets: {}", e)))
    }
}
