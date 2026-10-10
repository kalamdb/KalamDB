impl StreamTableProvider {
    /// Create a new stream table provider
    ///
    /// # Arguments
    /// * `core` - Shared core with services, schema, pk_name, etc.
    /// * `store` - StreamTableStore for this table (hot-only)
    /// * `ttl_seconds` - Optional TTL for event eviction
    pub fn new(
        core: Arc<TableProviderCore>,
        store: Arc<StreamTableStore>,
        ttl_seconds: Option<u64>,
    ) -> Self {
        Self {
            core,
            store,
            ttl_seconds,
        }
    }

    /// Build a complete Row from StreamTableRow including system column (_seq)
    ///
    /// This ensures live query notifications include all columns, not just user-defined fields.
    /// Stream tables don't have _deleted column.
    fn build_notification_row(entity: &StreamTableRow) -> Row {
        crate::utils::base::build_stream_notification_row(&entity.fields, entity._version)
    }

    fn now_millis() -> Result<u64, KalamDbError> {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .into_invalid_operation("System time error")
            .map(|d| d.as_millis() as u64)
    }

    /// Expose the underlying store (used by maintenance jobs such as stream eviction)
    pub fn store_arc(&self) -> Arc<StreamTableStore> {
        self.store.clone()
    }
}

impl SourceProvider for StreamTableProvider {
    fn filter_capability(&self, _filter: &Expr) -> FilterCapability {
        FilterCapability::Exact
    }

    fn scan_descriptor(
        &self,
        projection: Option<&Vec<usize>>,
        filters: &[Expr],
        limit: Option<usize>,
    ) -> ScanDescriptor {
        merged_projection_scan_descriptor(self.schema_ref(), projection, filters, limit)
    }
}

#[async_trait]
impl BaseTableProvider<StreamTableRowId, StreamTableRow> for StreamTableProvider {
    fn core(&self) -> &crate::utils::base::TableProviderCore {
        &self.core
    }

    fn construct_row_from_parquet_data(
        &self,
        user_id: &UserId,
        row_data: &crate::utils::version_resolution::ParquetRowData,
    ) -> Result<Option<(StreamTableRowId, StreamTableRow)>, KalamDbError> {
        let row_key = StreamTableRowId::new(user_id.clone(), row_data.version);
        let row = StreamTableRow {
            user_id: user_id.clone(),
            _version:    row_data.version,
            _timestamp: match row_data.fields.get(kalamdb_commons::constants::SystemColumnNames::TIMESTAMP) {
                Some(datafusion::common::ScalarValue::Int64(Some(ts))) => *ts,
                _ => 0,
            },
            fields:  row_data.fields.clone(),
        };
        Ok(Some((row_key, row)))
    }

    /// Stream tables are append-only and do not support UPDATE-by-key lookups.
    /// Hard delete by PK is handled by a scan in `delete_by_pk_value`.
    async fn find_row_key_by_id_field(
        &self,
        _user_id: &UserId,
        _id_value: &str,
    ) -> Result<Option<StreamTableRowId>, KalamDbError> {
        // Stream tables do not maintain a mutable PK lookup path for UPDATE.
        Ok(None)
    }

    async fn insert(
        &self,
        user_id: &UserId,
        row_data: Row,
        version: VersionId,
    ) -> Result<StreamTableRowId, KalamDbError> {
        let user_id = user_id.clone();
        let now_ms = Self::now_millis()?;
        let entity = StreamTableRow {
            user_id: user_id.clone(),
            _version: version,
            _timestamp: now_ms as i64,
            fields: row_data,
        };
        let row_key = StreamTableRowId::new(user_id.clone(), version);

        self.store.put(&row_key, &entity).map_err(|e| {
            KalamDbError::InvalidOperation(format!("Failed to insert stream event: {}", e))
        })?;

        let manager = self.core.services.notification_service.clone();
        let table_id = self.core.table_id().clone();

        let has_topics = self.core.has_topic_routes(&table_id);
        let has_live_subs = manager.has_subscribers(Some(&user_id), &table_id);
        if has_topics || has_live_subs {
            let row = Self::build_notification_row(&entity);

            if has_topics {
                self.core
                    .publish_to_topics(
                        &table_id,
                        kalamdb_commons::models::TopicOp::Insert,
                        &row,
                        Some(&user_id),
                    )
                    .await;
            }

            if has_live_subs {
                let notification = ChangeNotification::insert(table_id.clone(), row);
                manager.notify_table_change(Some(user_id.clone()), table_id, notification);
            }
        }

        Ok(row_key)
    }

    async fn insert_batch(
        &self,
        user_id: &UserId,
        rows: Vec<Row>,
        versions: &[VersionId],
    ) -> Result<Vec<StreamTableRowId>, KalamDbError> {
        if rows.is_empty() {
            return Ok(Vec::new());
        }

        let row_count = rows.len();
        let span = tracing::debug_span!(
            "stream.insert_batch",
            table_id = %self.core.table_id(),
            user_id = %user_id.as_str(),
            row_count
        );
        async move {
            let coerced_rows = coerce_rows(rows, &self.schema_ref()).map_err(|e| {
                KalamDbError::InvalidOperation(format!("Schema coercion failed: {}", e))
            })?;
            let row_count = coerced_rows.len();

            if versions.len() != row_count {
                return Err(KalamDbError::InvalidOperation(format!(
                    "versions length {} does not match rows length {}",
                    versions.len(),
                    row_count
                )));
            }

            let now_ms = Self::now_millis()?;
            let user_id_owned = user_id.clone();
            let versions_owned: Vec<VersionId> = versions.to_vec();
            let entries = tokio::task::spawn_blocking(move || {
                let mut entries: Vec<(StreamTableRowId, StreamTableRow)> =
                    Vec::with_capacity(row_count);
                for (row_data, version) in coerced_rows.into_iter().zip(versions_owned) {
                    let row_key = StreamTableRowId::new(user_id_owned.clone(), version);
                    entries.push((
                        row_key,
                        StreamTableRow {
                            user_id: user_id_owned.clone(),
                            _version: version,
                            _timestamp: now_ms as i64,
                            fields: row_data,
                        },
                    ));
                }
                entries
            })
            .await
            .map_err(|e| {
                KalamDbError::InvalidOperation(format!("spawn_blocking error: {}", e))
            })?;

            self.store.put_batch(&entries).map_err(|e| {
                KalamDbError::InvalidOperation(format!(
                    "Failed to batch insert stream events: {}",
                    e
                ))
            })?;

            let row_keys: Vec<StreamTableRowId> =
                entries.iter().map(|(row_key, _)| row_key.clone()).collect();

            let manager = self.core.services.notification_service.clone();
            let table_id = self.core.table_id().clone();
            let has_topics = self.core.has_topic_routes(&table_id);
            let has_live_subs = manager.has_subscribers(Some(user_id), &table_id);
            if has_topics || has_live_subs {
                let notification_rows: Vec<_> = entries
                    .iter()
                    .map(|(_row_key, entity)| Self::build_notification_row(entity))
                    .collect();

                if has_topics {
                    self.core
                        .publish_batch_to_topics(
                            &table_id,
                            kalamdb_commons::models::TopicOp::Insert,
                            &notification_rows,
                            Some(user_id),
                        )
                        .await;
                }

                if has_live_subs {
                    for row in notification_rows {
                        let notification = ChangeNotification::insert(table_id.clone(), row);
                        manager.notify_table_change(
                            Some(user_id.clone()),
                            table_id.clone(),
                            notification,
                        );
                    }
                }
            }

            Ok(row_keys)
        }
        .instrument(span)
        .await
    }

    async fn update(
        &self,
        _user_id: &UserId,
        _key: &StreamTableRowId,
        _updates: Row,
        _version: VersionId,
    ) -> Result<Option<StreamTableRowId>, KalamDbError> {
        Err(KalamDbError::InvalidOperation(
            "UPDATE is not supported for STREAM tables".to_string(),
        ))
    }

    async fn update_by_pk_value(
        &self,
        _user_id: &UserId,
        _pk_value: &str,
        _updates: Row,
        _version: VersionId,
    ) -> Result<Option<StreamTableRowId>, KalamDbError> {
        Err(KalamDbError::InvalidOperation(
            "UPDATE is not supported for STREAM tables".to_string(),
        ))
    }

    async fn delete(
        &self,
        user_id: &UserId,
        key: &StreamTableRowId,
        _version: VersionId,
    ) -> Result<(), KalamDbError> {
        // STREAM tables use hard delete: remove the hot-store row directly and
        // notify subscribers with a hard-delete event.
        self.store.delete(key).map_err(|e| {
            KalamDbError::InvalidOperation(format!("Failed to delete stream event: {}", e))
        })?;

        // Fire live query notification (DELETE hard)
        let notification_service = self.core.services.notification_service.clone();
        let table_id = self.core.table_id().clone();

        if notification_service.has_subscribers(Some(&user_id), &table_id) {
            let row_id_str = format!("{}:{}", key.user_id().as_str(), key.version().as_i64());
            let notification = ChangeNotification::delete_hard(table_id.clone(), row_id_str);
            notification_service.notify_table_change(Some(user_id.clone()), table_id, notification);
        }

        Ok(())
    }

    async fn delete_by_pk_value(
        &self,
        user_id: &UserId,
        pk_value: &str,
        _version: VersionId,
    ) -> Result<bool, KalamDbError> {
        // STREAM tables support DELETE by PK value for hard delete
        // PK column is typically an auto-generated ID (e.g., ULID(), event_id, etc.)

        // Scan all rows for this user to find matching PK values
        // Note: This is O(n) but STREAM tables are typically append-only with TTL eviction
        let rows = self.store.scan_user(user_id, None, usize::MAX).map_err(|e| {
            KalamDbError::InvalidOperation(format!("Failed to scan stream table keys: {}", e))
        })?;

        let pk_name = self.primary_key_field_name();
        let mut deleted_count = 0;

        for (key, entity) in rows {
            // Check if PK column matches the target value
            if let Some(row_pk_value) = entity.fields.get(pk_name) {
                let row_pk_str = match row_pk_value {
                    ScalarValue::Utf8(Some(s)) => s.clone(),
                    ScalarValue::Int64(Some(i)) => i.to_string(),
                    ScalarValue::Int32(Some(i)) => i.to_string(),
                    _ => continue,
                };

                if row_pk_str == pk_value {
                    // Delete this row
                    self.store.delete(&key).map_err(|e| {
                        KalamDbError::InvalidOperation(format!(
                            "Failed to delete stream event: {}",
                            e
                        ))
                    })?;

                    deleted_count += 1;

                    // Fire live query notification (DELETE hard)
                    let notification_service = self.core.services.notification_service.clone();
                    let table_id = self.core.table_id().clone();

                    if notification_service.has_subscribers(Some(&user_id), &table_id) {
                        let row_id_str =
                            format!("{}:{}", key.user_id().as_str(), key.version().as_i64());
                        let notification =
                            ChangeNotification::delete_hard(table_id.clone(), row_id_str);
                        notification_service.notify_table_change(
                            Some(user_id.clone()),
                            table_id,
                            notification,
                        );
                    }
                }
            }
        }

        Ok(deleted_count > 0)
    }

    async fn scan_rows(
        &self,
        state: &dyn Session,
        projection: Option<&Vec<usize>>,
        filter: Option<&Expr>,
        limit: Option<usize>,
    ) -> Result<RecordBatch, KalamDbError> {
        // Extract user_id from SessionState for RLS
        let (user_id, _role) = extract_user_context(state)?;

        // Extract sequence bounds from filter to optimize scan
        let (since_seq, _until_seq) = if let Some(expr) = filter {
            extract_seq_bounds_from_filter(expr)
        } else {
            (None, None)
        };

        // Perform KV scan (hot-only) and convert to batch
        let keep_deleted = false; // Stream tables don't support soft delete yet
        let kvs = self
            .scan_with_version_resolution_to_kvs_async(
                user_id,
                filter,
                since_seq,
                limit,
                keep_deleted,
                None,
                None,
            )
            .await?;
        let schema = self.schema_ref();
        crate::utils::base::rows_to_arrow_batch(&schema, kvs, projection, |row_values, row| {
            if self.core.schema_ref().field_with_name("user_id").is_ok() {
                row_values.values.insert(
                    "user_id".to_string(),
                    ScalarValue::Utf8(Some(row.user_id.as_str().to_string())),
                );
            }
        })
    }

    async fn scan_with_version_resolution_to_kvs_async(
        &self,
        user_id: &UserId,
        _filter: Option<&Expr>,
        since_seq: Option<VersionId>,
        limit: Option<usize>,
        _keep_deleted: bool,
        _cold_columns: Option<&[String]>,
        _snapshot_commit_seq: Option<VersionId>,
    ) -> Result<Vec<(StreamTableRowId, StreamTableRow)>, KalamDbError> {
        // since_seq is exclusive, so start at seq + 1
        let start_seq = since_seq.map(|seq| VersionId::try_from_i64(seq.as_i64().saturating_add(1)).unwrap_or(seq));

        let ttl_ms = self.ttl_seconds.map(|s| s * 1000);
        let now_ms = Self::now_millis()?;
        let scan_limit = limit.unwrap_or(100_000);

        let results = self
            .store
            .scan_user_streaming_async(user_id, start_seq, scan_limit, ttl_ms, now_ms)
            .await
            .map_err(|e| {
                KalamDbError::InvalidOperation(format!(
                    "Failed to scan stream table hot storage: {}",
                    e
                ))
            })?;

        // TODO(phase 13.6): Apply filter expression for simple predicates if provided
        Ok(results)
    }

    fn extract_row(row: &StreamTableRow) -> &Row {
        &row.fields
    }
}
