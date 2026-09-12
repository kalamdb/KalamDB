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
                "OffsetOutOfRange: requested offset {} is before earliest available offset {} for \
                 topic {} partition {} (latest next offset {})",
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

        let cursor_key = GroupPartitionKey::new(topic_id, group_id, partition_id);

        loop {
            let earliest = self.earliest_available_offset(topic_id, partition_id)?;
            let (fetch_start, fetch_limit, reservation_id) = {
                let initial_start =
                    self.group_fetch_start(topic_id, group_id, partition_id, start_offset)?;
                let mut state = self
                    .group_claim_state
                    .entry(cursor_key.clone())
                    .or_insert_with(|| ClaimState::new(initial_start));

                state.expire_stale_claims(Instant::now(), self.visibility_timeout);
                if let Some(last_acked) = self.durable_last_acked(topic_id, group_id, partition_id)?
                {
                    state.ack_up_to(last_acked);
                }

                let (current_start, available_limit) = state.next_available_window(limit);
                if available_limit == 0 {
                    return Ok(Vec::new());
                }

                if current_start < earliest {
                    state.cursor = earliest;
                    continue;
                }

                let claimed_at = Instant::now();
                let reservation_id =
                    state.reserve_window(current_start, available_limit, claimed_at);
                self.register_consumer_group(topic_id, group_id);
                (current_start, available_limit, reservation_id)
            };

            let messages = self
                .message_store
                .fetch_messages(topic_id, partition_id, fetch_start, fetch_limit)
                .map_err(|e| CommonError::Internal(format!("Failed to fetch messages: {}", e)))?;

            if messages.is_empty() {
                let pin_tail = fetch_start > earliest;
                if let Some(mut state) = self.group_claim_state.get_mut(&cursor_key) {
                    state.cancel_reservation(reservation_id);
                    if pin_tail {
                        state.cursor = fetch_start;
                    } else if state.pending.is_empty() {
                        drop(state);
                        self.group_claim_state.remove(&cursor_key);
                        self.unregister_consumer_group_if_idle(topic_id, group_id);
                        self.trim_consumer_runtime_maps();
                    }
                }
                return Ok(messages);
            }

            let claim_start = messages.first().map(|message| message.offset).unwrap_or(fetch_start);
            let end_exclusive = messages.last().map(|message| message.offset + 1).unwrap_or(fetch_start);
            let claimed_at = Instant::now();

            let initial_start =
                self.group_fetch_start(topic_id, group_id, partition_id, start_offset)?;
            let mut state = self
                .group_claim_state
                .entry(cursor_key.clone())
                .or_insert_with(|| ClaimState::new(initial_start));

            state.expire_stale_claims(claimed_at, self.visibility_timeout);
            if state.has_reservation(reservation_id) {
                state.finalize_reservation(reservation_id, claim_start, end_exclusive);
            } else if state.overlaps_pending(claim_start, end_exclusive) {
                continue;
            } else {
                state.register_delivered_claim(claim_start, end_exclusive, claimed_at);
            }

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

        Ok(self.log_start_offset(topic_id, partition_id))
    }

    pub(crate) fn log_start_offset(&self, topic_id: &TopicId, partition_id: u32) -> u64 {
        self.log_start_offsets
            .get(&TopicPartitionKey::new(topic_id, partition_id))
            .map(|offset| *offset)
            .unwrap_or(0)
    }

    pub(crate) fn set_log_start_offset(&self, topic_id: &TopicId, partition_id: u32, offset: u64) {
        self.log_start_offsets
            .entry(TopicPartitionKey::new(topic_id, partition_id))
            .and_modify(|current| {
                if offset > *current {
                    *current = offset;
                }
            })
            .or_insert(offset);
    }

    pub fn ack_offset(
        &self,
        topic_id: &TopicId,
        group_id: &ConsumerGroupId,
        partition_id: u32,
        offset: u64,
    ) -> Result<()> {
        self.offset_store
            .ack_offset(topic_id, group_id, partition_id, offset)
            .map_err(|e| CommonError::Internal(format!("Failed to ack offset: {}", e)))?;

        let cursor_key = GroupPartitionKey::new(topic_id, group_id, partition_id);
        let became_idle = {
            if let Some(mut state) = self.group_claim_state.get_mut(&cursor_key) {
                state.ack_up_to(offset);
                state.pending.is_empty()
            } else {
                false
            }
        };
        if became_idle {
            self.group_claim_state
                .remove_if(&cursor_key, |_, state| state.pending.is_empty());
            self.unregister_consumer_group_if_idle(topic_id, group_id);
            self.trim_consumer_runtime_maps();
        }

        Ok(())
    }

    fn durable_last_acked(
        &self,
        topic_id: &TopicId,
        group_id: &ConsumerGroupId,
        partition_id: u32,
    ) -> Result<Option<u64>> {
        match self.offset_store.get_offset(topic_id, group_id, partition_id) {
            Ok(Some(offset)) => Ok(Some(offset.last_acked_offset)),
            Ok(None) => Ok(None),
            Err(e) => Err(CommonError::Internal(format!("Failed to read group offset: {}", e))),
        }
    }

    fn group_fetch_start(
        &self,
        topic_id: &TopicId,
        group_id: &ConsumerGroupId,
        partition_id: u32,
        start_offset: u64,
    ) -> Result<u64> {
        match self.durable_last_acked(topic_id, group_id, partition_id)? {
            Some(last_acked) => Ok(last_acked.saturating_add(1)),
            None => Ok(start_offset),
        }
    }

    fn unregister_consumer_group_if_idle(&self, topic_id: &TopicId, group_id: &ConsumerGroupId) {
        let still_claimed = self
            .group_claim_state
            .iter()
            .any(|entry| entry.key().topic_id == *topic_id && entry.key().group_id == *group_id);
        if !still_claimed {
            self.consumer_groups.remove(&ConsumerGroupKey::new(topic_id, group_id));
        }
    }

    fn trim_consumer_runtime_maps(&self) {
        shrink_dashmap_if_sparse(&self.group_claim_state);
        shrink_dashmap_if_sparse(&self.consumer_groups);
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
