use super::*;

impl TopicPublisherService {
    pub(super) fn retained_bytes_for_partition(
        &self,
        topic_id: &TopicId,
        partition_id: u32,
    ) -> Result<u64> {
        let partition = self.partition_runtime(topic_id, partition_id);
        let mut write = partition.lock_write();
        self.load_retained_bytes(&mut write, topic_id, partition_id)
    }

    pub(super) fn load_retained_bytes(
        &self,
        write: &mut PartitionWriteState,
        topic_id: &TopicId,
        partition_id: u32,
    ) -> Result<u64> {
        if let Some(bytes) = write.retained_bytes() {
            return Ok(bytes);
        }

        let bytes = self
            .message_store
            .retained_bytes_for_partition(topic_id, partition_id)
            .map_err(|e| CommonError::Internal(format!("Failed to read retained bytes: {}", e)))?;
        write.set_retained_bytes(bytes);
        Ok(bytes)
    }

    pub fn publish_message(
        &self,
        table_id: &TableId,
        operation: TopicOp,
        row: &Row,
        user_id: Option<&UserId>,
    ) -> Result<usize> {
        let span = tracing::debug_span!(
            "topic.publish",
            table_id = %table_id,
            operation = ?operation,
            has_user_id = user_id.is_some(),
            row_value_count = row.values.len(),
            published_count = tracing::field::Empty
        );
        let _span_guard = span.entered();

        let matching = self.route_cache.get_matching_routes(table_id, &operation);
        if matching.is_empty() {
            return Ok(0);
        }
        let primary_key_columns = self.primary_key_columns_for(table_id)?;
        let prepared = Self::prepare_row(row, table_id, &matching)?;
        let key = prepared.extract_key(&primary_key_columns)?;

        let mut total_published = 0;

        for entry in matching {
            if !Self::route_matches_row(&entry, row) {
                continue;
            }

            let topic_span = tracing::debug_span!(
                "publish_to_topic",
                topic_name = entry.topic_id().as_str(),
                topic_partitions = entry.topic_partitions(),
                operation = ?entry.route().op
            );
            let _topic_span_guard = topic_span.entered();

            let payload_bytes = prepared.extract_payload(entry.route(), table_id)?;

            let partition_id = if let Some(ref key) = key {
                (payload::hash_key(key) % entry.topic_partitions() as u64) as u32
            } else {
                (prepared.hash_row() % entry.topic_partitions() as u64) as u32
            };

            let partition = self.partition_runtime(entry.topic_id(), partition_id);
            let mut write = partition.lock_write();

            let offset = write.allocate(1);

            let timestamp_ms = chrono::Utc::now().timestamp_millis();
            let message = TopicMessage::new_with_user(
                entry.topic_id().clone(),
                partition_id,
                offset,
                payload_bytes,
                key.clone(),
                timestamp_ms,
                user_id.cloned(),
                operation.clone(),
            );

            let message_bytes =
                self.message_store.put_message_with_retention_index(&message).map_err(|e| {
                    CommonError::Internal(format!("Failed to store topic message: {}", e))
                })?;
            write.add_retained_bytes(message_bytes);
            record_pubsub_messages_published(1, message_bytes);

            tracing::debug!(
                topic_name = entry.topic_id().as_str(),
                partition_id = partition_id,
                offset = offset,
                payload_bytes = message.payload.len(),
                "Published message to topic"
            );
            partition.append_to_tail(message);

            total_published += 1;
        }

        tracing::Span::current().record("published_count", total_published);
        Ok(total_published)
    }

    pub fn publish_batch(
        &self,
        table_id: &TableId,
        operation: TopicOp,
        rows: &[Row],
        user_id: Option<&UserId>,
    ) -> Result<usize> {
        if rows.is_empty() {
            return Ok(0);
        }

        let span = tracing::debug_span!(
            "topic.publish_batch",
            table_id = %table_id,
            operation = ?operation,
            row_count = rows.len(),
            published_count = tracing::field::Empty
        );
        let _span_guard = span.entered();

        let matching = self.route_cache.get_matching_routes(table_id, &operation);
        if matching.is_empty() {
            return Ok(0);
        }
        let primary_key_columns = self.primary_key_columns_for(table_id)?;

        let prepared: Vec<payload::PreparedRow> = rows
            .iter()
            .map(|row| Self::prepare_row(row, table_id, &matching))
            .collect::<Result<Vec<_>>>()?;

        let prepared_keys: Vec<Option<String>> = prepared
            .iter()
            .map(|prep| prep.extract_key(&primary_key_columns))
            .collect::<Result<Vec<_>>>()?;

        let mut total_published = 0;
        let timestamp_ms = chrono::Utc::now().timestamp_millis();

        for entry in &matching {
            let mut partition_groups: std::collections::HashMap<u32, Vec<usize>> =
                std::collections::HashMap::new();

            for (idx, prep) in prepared.iter().enumerate() {
                if !Self::route_matches_row(entry, &rows[idx]) {
                    continue;
                }

                let partition_hash = match prepared_keys[idx].as_deref() {
                    Some(key) => payload::hash_key(key),
                    None => prep.hash_row(),
                };
                let partition_id = (partition_hash % entry.topic_partitions() as u64) as u32;
                partition_groups.entry(partition_id).or_default().push(idx);
            }

            if partition_groups.is_empty() {
                continue;
            }

            for (partition_id, row_indices) in &partition_groups {
                let count = row_indices.len() as u64;

                let mut pre_encoded: Vec<(Vec<u8>, Option<String>)> =
                    Vec::with_capacity(row_indices.len());
                for &row_idx in row_indices {
                    let prep = &prepared[row_idx];
                    let payload_bytes = prep.extract_payload(entry.route(), table_id)?;
                    let key = prepared_keys[row_idx].clone();
                    pre_encoded.push((payload_bytes, key));
                }

                let partition = self.partition_runtime(entry.topic_id(), *partition_id);
                let mut write = partition.lock_write();

                let start_offset = write.allocate(count);

                let mut raw_entries = Vec::with_capacity(pre_encoded.len());
                let mut cached_messages = Vec::with_capacity(pre_encoded.len());
                for (i, (payload_bytes, key)) in pre_encoded.into_iter().enumerate() {
                    let offset = start_offset + i as u64;

                    let message = TopicMessage::new_with_user(
                        entry.topic_id().clone(),
                        *partition_id,
                        offset,
                        payload_bytes,
                        key,
                        timestamp_ms,
                        user_id.cloned(),
                        operation.clone(),
                    );
                    let msg_id = message.id();

                    let key_encoded = kalamdb_commons::StorageKey::storage_key(&msg_id);
                    let value_encoded = kalamdb_store::encode_entity(&message).map_err(|e| {
                        CommonError::Internal(format!("Failed to serialize topic message: {}", e))
                    })?;
                    let retention_entry = kalamdb_tables::TopicRetentionIndexEntry::new_raw(
                        entry.topic_id().clone(),
                        *partition_id,
                        timestamp_ms,
                        offset,
                        value_encoded.len() as u64,
                    );
                    raw_entries.push((retention_entry, key_encoded, value_encoded));
                    cached_messages.push(message);
                }

                let message_bytes =
                    self.message_store.batch_put_raw_with_retention(raw_entries).map_err(|e| {
                        CommonError::Internal(format!(
                            "Failed to batch store topic messages: {}",
                            e
                        ))
                    })?;
                write.add_retained_bytes(message_bytes);
                record_pubsub_messages_published(row_indices.len() as u64, message_bytes);
                partition.append_messages_to_tail(cached_messages);

                total_published += row_indices.len();
            }
        }

        tracing::Span::current().record("published_count", total_published);
        Ok(total_published)
    }

    /// Persist a typed procedure payload on an explicit topic.
    pub fn publish_typed(
        &self,
        topic_id: &TopicId,
        payload: Vec<u8>,
        user_id: Option<&UserId>,
    ) -> Result<u64> {
        if !self.topic_exists(topic_id) {
            return Err(CommonError::NotFound(format!("topic {topic_id} not found")));
        }
        let partition_id = 0u32;
        let partition = self.partition_runtime(topic_id, partition_id);
        let mut write = partition.lock_write();
        let offset = write.allocate(1);
        let timestamp_ms = chrono::Utc::now().timestamp_millis();
        let message = TopicMessage::new_with_user(
            topic_id.clone(),
            partition_id,
            offset,
            payload,
            None,
            timestamp_ms,
            user_id.cloned(),
            TopicOp::Insert,
        );
        let message_bytes =
            self.message_store.put_message_with_retention_index(&message).map_err(|e| {
                CommonError::Internal(format!("Failed to store typed topic message: {}", e))
            })?;
        write.add_retained_bytes(message_bytes);
        record_pubsub_messages_published(1, message_bytes);
        partition.append_to_tail(message);
        Ok(offset)
    }

    fn prepare_row(
        row: &Row,
        table_id: &TableId,
        matching: &[RouteEntry],
    ) -> Result<payload::PreparedRow> {
        let needs_full_payload = matching.iter().any(|entry| {
            matches!(
                entry.route().payload_mode,
                kalamdb_commons::models::PayloadMode::Full
                    | kalamdb_commons::models::PayloadMode::Diff
            )
        });
        if needs_full_payload {
            payload::PreparedRow::from_row_with_table(row, table_id)
        } else {
            payload::PreparedRow::from_row(row)
        }
    }
}
