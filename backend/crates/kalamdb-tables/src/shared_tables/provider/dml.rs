impl SharedTableProvider {
    /// Count resolved live rows for COUNT(*) without a filter.
    ///
    /// Hot rows are decoded through the store codec so KOBJ payloads reconstruct
    /// `_seq`, `_deleted`, and the PK used for MVCC winner selection.
    async fn count_resolved_rows_async(
        &self,
        snapshot_commit_seq: Option<VersionId>,
    ) -> Result<usize, KalamDbError> {
        use kalamdb_commons::models::rows::RowMetadata;

        let pk_name = self.primary_key_field_name().to_string();
        let store = Arc::clone(&self.store);
        let pk_name_clone = pk_name.clone();

        // Hot storage: typed scan so KOBJ values reconstruct seq/deleted/PK from the
        // store codec instead of the retired FlatBuffers metadata decoder.
        let hot_future = async move {
            let hot_rows =
                store.scan_all_async(Some(1_000_000), None, None).await.map_err(|e| {
                    KalamDbError::InvalidOperation(format!(
                        "Failed to scan shared table hot storage for count: {}",
                        e
                    ))
                })?;

            let hot_metadata = hot_rows
                .into_iter()
                .map(|(_key, row)| RowMetadata {
                    version:   row._version,
                    deleted:   row._deleted,
                    pk_bucket: pk_bucket_key_from_row(&row.fields, &pk_name_clone, row._version),
                })
                .collect();
            Ok::<_, KalamDbError>(hot_metadata)
        };

        // Cold storage: project only the PK + MVCC metadata needed for counting.
        let cold_columns = base::compute_metadata_only_cold_columns(&pk_name);
        let cold_future =
            self.scan_parquet_files_as_batch_async(None, Some(cold_columns.as_slice()));

        base::count_resolved_rows_from_futures(
            &pk_name,
            snapshot_commit_seq,
            hot_future,
            cold_future,
        )
        .await
    }

    async fn insert_deferred_internal(
        &self,
        row_data: Row,
        validate_unique_pk: bool,
        version: VersionId,
    ) -> Result<(SharedTableRowId, Option<ChangeNotification>), KalamDbError> {
        let span = tracing::debug_span!(
            "table.insert",
            table_id = %self.core.table_id(),
            scope = "shared",
            column_count = row_data.values.len(),
            deferred_side_effects = true
        );
        async move {
            let _authorization_mutation = self.begin_authorization_mutation();
            ensure_manifest_ready(&self.core, self.core.table_type(), None, "SharedTableProvider")?;

            crate::utils::datafusion_dml::validate_not_null_with_set(
                self.core.non_null_columns(),
                std::slice::from_ref(&row_data),
            )?;

            if validate_unique_pk {
                base::ensure_unique_pk_value(self, None, &row_data).await?;
            }

            let seq_id = version;

            let entity = SharedTableRow {
                _version: seq_id,
                _deleted: false,
                fields:   row_data,
            };
            let row_key = seq_id;

            self.store.insert_async(row_key, entity.clone()).await.map_err(|e| {
                KalamDbError::InvalidOperation(format!("Failed to insert shared table row: {}", e))
            })?;

            if let Err(e) = self.stage_vector_upsert(seq_id, &entity.fields).await {
                log::warn!(
                    "Failed to stage vector upsert for table={}, seq={}: {}",
                    self.core.table_id(),
                    seq_id.as_i64(),
                    e
                );
            }

            let manifest_service = self.core.services.manifest_service.clone();
            if let Err(e) = manifest_service.mark_pending_write(self.core.table_id(), None) {
                log::warn!(
                    "Failed to mark manifest as pending_write for {}: {}",
                    self.core.table_id(),
                    e
                );
            }

            let notification_service = self.core.services.notification_service.clone();
            let table_id = self.core.table_id().clone();
            let has_topics = self.core.has_topic_routes(&table_id);
            let has_live_subs = notification_service.has_subscribers(None, &table_id);
            let notification = crate::versioned::insert_notification_when_watched(
                has_topics || has_live_subs,
                table_id,
                Self::build_notification_row(&entity),
            );

            Ok((row_key, notification))
        }
        .instrument(span)
        .await
    }

    pub async fn insert_deferred(
        &self,
        row_data: Row,
        version: VersionId,
    ) -> Result<(SharedTableRowId, Option<ChangeNotification>), KalamDbError> {
        self.insert_deferred_internal(row_data, true, version).await
    }

    pub async fn insert_deferred_prevalidated(
        &self,
        row_data: Row,
        version: VersionId,
    ) -> Result<(SharedTableRowId, Option<ChangeNotification>), KalamDbError> {
        self.insert_deferred_internal(row_data, false, version).await
    }

    async fn persist_insert_batch_rows(
        &self,
        rows: Vec<Row>,
        versions: &[VersionId],
        validate_unique_pk: bool,
    ) -> Result<Vec<(SharedTableRowId, SharedTableRow)>, KalamDbError> {
        if rows.is_empty() {
            return Ok(Vec::new());
        }

        let _authorization_mutation = self.begin_authorization_mutation();

        ensure_manifest_ready(&self.core, self.core.table_type(), None, "SharedTableProvider")?;

        let coerced_rows = coerce_rows(rows, &self.schema_ref()).map_err(|e| {
            KalamDbError::InvalidOperation(format!("Schema coercion failed: {}", e))
        })?;

        crate::utils::datafusion_dml::validate_not_null_with_set(
            self.core.non_null_columns(),
            &coerced_rows,
        )?;

        let row_count = coerced_rows.len();

        if validate_unique_pk {
            kalamdb_observability::kdb_await_in_info_span!(
                async {
                    let pk_name = self.primary_key_field_name();
                    let mut pk_values_to_check: Vec<(String, ScalarValue)> =
                        Vec::with_capacity(row_count);
                    let mut seen_batch_pks = HashSet::with_capacity(row_count);
                    for row_data in &coerced_rows {
                        if let Some(pk_value) = row_data.get(pk_name) {
                            if !matches!(pk_value, ScalarValue::Null) {
                                let pk_str = crate::utils::unified_dml::extract_user_pk_value(
                                    row_data, pk_name,
                                )?;
                                if !seen_batch_pks.insert(pk_str.clone()) {
                                    return Err(KalamDbError::AlreadyExists(format!(
                                        "Primary key violation: value '{}' appears multiple times \
                                         in the insert batch for column '{}'",
                                        pk_str, pk_name
                                    )));
                                }
                                pk_values_to_check.push((pk_str, pk_value.clone()));
                            }
                        }
                    }

                    if !pk_values_to_check.is_empty() {
                        let mut pk_prefixes: Vec<(String, Vec<u8>)> =
                            Vec::with_capacity(pk_values_to_check.len());
                        for (pk_str, pk_value) in &pk_values_to_check {
                            pk_prefixes.push((
                                pk_str.clone(),
                                self.pk_index.build_prefix_for_pk(pk_value),
                            ));
                        }

                        let store = self.store.clone();
                        let (hot_duplicate, tombstoned_pks) = if pk_prefixes.len() <= 1 {
                            scan_hot_pk_insert(&store, &pk_prefixes)?
                        } else {
                            tokio::task::spawn_blocking(
                        move || -> Result<(Option<String>, HashSet<String>), KalamDbError> {
                            scan_hot_pk_insert(&store, &pk_prefixes)
                        },
                    )
                    .await
                    .map_err(|e| {
                        KalamDbError::InvalidOperation(format!("spawn_blocking error: {}", e))
                    })??
                        };

                        if let Some(dup_pk) = hot_duplicate {
                            return Err(KalamDbError::AlreadyExists(format!(
                                "Primary key violation: value '{}' already exists in column '{}'",
                                dup_pk, pk_name
                            )));
                        }

                        let pk_column_id = self.core.primary_key_column_id();
                        let mut pk_values_for_cold_check: Vec<String> =
                            Vec::with_capacity(pk_values_to_check.len());
                        for (pk_str, _pk_value) in &pk_values_to_check {
                            if !tombstoned_pks.contains(pk_str) {
                                pk_values_for_cold_check.push(pk_str.clone());
                            }
                        }

                        if !pk_values_for_cold_check.is_empty() {
                            if let Some(found_pk) = base::pk_exists_batch_in_cold(
                                &self.core,
                                self.core.table_id(),
                                self.core.table_type(),
                                None,
                                pk_name,
                                pk_column_id,
                                &pk_values_for_cold_check,
                            )
                            .await?
                            {
                                return Err(KalamDbError::AlreadyExists(format!(
                                    "Primary key violation: value '{}' already exists in column \
                                     '{}'",
                                    found_pk, pk_name
                                )));
                            }
                        }
                    }
                    Ok(())
                },
                "table.pk_unique"
            )?;
        }

        if versions.len() != row_count {
            return Err(KalamDbError::InvalidOperation(format!(
                "version count {} does not match row count {}",
                versions.len(),
                row_count
            )));
        }

        let mut entries: Vec<(SharedTableRowId, SharedTableRow)> = Vec::with_capacity(row_count);

        for (row_data, version) in coerced_rows.into_iter().zip(versions.iter().copied()) {
            entries.push((
                version,
                SharedTableRow {
                    _version: version,
                    _deleted: false,
                    fields:   row_data,
                },
            ));
        }

        let store = self.store.clone();
        store.insert_batch(&entries).map_err(|e| {
            KalamDbError::InvalidOperation(format!("Failed to batch insert table rows: {}", e))
        })?;

        if let Err(e) = self.stage_vector_upsert_batch(&entries).await {
            log::warn!(
                "Failed to batch stage vector upserts for table={}: {}",
                self.core.table_id(),
                e
            );
        }

        let manifest_service = self.core.services.manifest_service.clone();
        if let Err(e) = manifest_service.mark_pending_write(self.core.table_id(), None) {
            log::warn!(
                "Failed to mark manifest as pending_write for {}: {}",
                self.core.table_id(),
                e
            );
        }

        log::debug!(
            "Batch inserted {} shared table rows with _seq range [{}, {}]",
            row_count,
            entries.first().map(|(k, _)| k.as_i64()).unwrap_or(0),
            entries.last().map(|(k, _)| k.as_i64()).unwrap_or(0)
        );

        Ok(entries)
    }

    pub async fn insert_batch_with_versions(
        &self,
        actor_user_id: Option<&UserId>,
        rows: Vec<Row>,
        versions: &[VersionId],
    ) -> Result<Vec<SharedTableRowId>, KalamDbError> {
        let row_count = rows.len();
        let span = tracing::debug_span!(
            "table.insert_batch",
            table_id = %self.core.table_id(),
            scope = "shared",
            row_count
        );
        async move {
            let entries = self.persist_insert_batch_rows(rows, versions, true).await?;
            let row_keys: Vec<SharedTableRowId> =
                entries.iter().map(|(row_key, _)| *row_key).collect();

            let notification_service = self.core.services.notification_service.clone();
            let table_id = self.core.table_id().clone();

            let has_topics = self.core.has_topic_routes(&table_id);
            let has_live_subs = notification_service.has_subscribers(None, &table_id);
            if has_topics || has_live_subs {
                let rows: Vec<_> = entries
                    .iter()
                    .map(|(_row_key, entity)| Self::build_notification_row(entity))
                    .collect();

                if has_topics {
                    self.core
                        .publish_batch_to_topics(
                            &table_id,
                            kalamdb_commons::models::TopicOp::Insert,
                            &rows,
                            actor_user_id,
                        )
                        .await;
                }
                if has_live_subs {
                    for row in rows {
                        let notification = ChangeNotification::insert(table_id.clone(), row);
                        notification_service.notify_table_change(
                            None,
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

    pub async fn insert_batch_deferred_prevalidated(
        &self,
        rows: Vec<Row>,
        versions: &[VersionId],
    ) -> Result<Vec<(SharedTableRowId, Option<ChangeNotification>)>, KalamDbError> {
        self.insert_batch_deferred_prevalidated_with_versions(rows, versions).await
    }

    pub async fn insert_batch_deferred_prevalidated_with_versions(
        &self,
        rows: Vec<Row>,
        versions: &[VersionId],
    ) -> Result<Vec<(SharedTableRowId, Option<ChangeNotification>)>, KalamDbError> {
        let row_count = rows.len();
        let span = tracing::debug_span!(
            "table.insert_batch",
            table_id = %self.core.table_id(),
            scope = "shared",
            row_count,
            deferred_side_effects = true
        );
        async move {
            let entries = self.persist_insert_batch_rows(rows, versions, false).await?;

            let notification_service = self.core.services.notification_service.clone();
            let table_id = self.core.table_id().clone();
            let has_topics = self.core.has_topic_routes(&table_id);
            let has_live_subs = notification_service.has_subscribers(None, &table_id);

            Ok(entries
                .into_iter()
                .map(|(row_key, entity)| {
                    let notification = crate::versioned::insert_notification_when_watched(
                        has_topics || has_live_subs,
                        table_id.clone(),
                        Self::build_notification_row(&entity),
                    );
                    (row_key, notification)
                })
                .collect())
        }
        .instrument(span)
        .await
    }

    pub async fn update_by_pk_value_deferred(
        &self,
        pk_value: &str,
        updates: Row,
        version: VersionId,
    ) -> Result<Option<(SharedTableRowId, Option<ChangeNotification>)>, KalamDbError> {
        let span = tracing::debug_span!(
            "table.update",
            table_id = %self.core.table_id(),
            scope = "shared",
            pk = pk_value,
            deferred_side_effects = true
        );
        async move {
            let _authorization_mutation = self.begin_authorization_mutation();
            let schema = self.schema();
            let updates = coerce_updates(updates, &schema).map_err(|e| {
                KalamDbError::InvalidOperation(format!("Schema coercion failed: {}", e))
            })?;

            let pk_name = self.primary_key_field_name().to_string();
            let pk_field = schema.field_with_name(&pk_name).map_err(|e| {
                KalamDbError::InvalidOperation(format!(
                    "PK column '{}' not found in schema: {}",
                    pk_name, e
                ))
            })?;
            let pk_column_type = pk_field.data_type();
            let pk_value_scalar =
                kalamdb_commons::conversions::parse_string_as_scalar(pk_value, pk_column_type)
                    .map_err(KalamDbError::InvalidOperation)?;

            let (_latest_key, latest_row) =
                if let Some(result) = self.find_by_pk(&pk_value_scalar).await? {
                    result
                } else if self.pk_tombstoned_in_hot(&pk_value_scalar).await? {
                    return Err(KalamDbError::NotFound(format!(
                        "Row with {}={} was deleted",
                        pk_name, pk_value
                    )));
                } else {
                    base::find_row_by_pk(self, None, pk_value).await?.ok_or_else(|| {
                        KalamDbError::NotFound(format!(
                            "Row with {}={} not found (checked both hot and cold storage)",
                            pk_name, pk_value
                        ))
                    })?
                };

            let mut merged = latest_row.fields.values.clone();
            for (key, value) in updates.values {
                merged.insert(key, value);
            }
            let new_fields = Row::new(merged);

            crate::utils::datafusion_dml::validate_not_null_with_set(
                self.core.non_null_columns(),
                &[new_fields.clone()],
            )?;

            if new_fields == latest_row.fields {
                tracing::debug!(
                    table_id = %self.core.table_id(),
                    pk = pk_value,
                    "table.update_noop: row unchanged, skipping write"
                );
                return Ok(None);
            }

            let seq_id = version;

            let entity = SharedTableRow {
                _version: seq_id,
                _deleted: false,
                fields:   new_fields,
            };
            let row_key = seq_id;
            self.store.insert_async(row_key, entity.clone()).await.map_err(|e| {
                KalamDbError::InvalidOperation(format!("Failed to update shared table row: {}", e))
            })?;

            if let Err(e) = self.stage_vector_upsert(seq_id, &entity.fields).await {
                log::warn!(
                    "Failed to stage vector upsert for table={}, seq={}: {}",
                    self.core.table_id(),
                    seq_id.as_i64(),
                    e
                );
            }

            let manifest_service = self.core.services.manifest_service.clone();
            if let Err(e) = manifest_service.mark_pending_write(self.core.table_id(), None) {
                log::warn!(
                    "Failed to mark manifest as pending_write for {}: {}",
                    self.core.table_id(),
                    e
                );
            }

            let notification_service = self.core.services.notification_service.clone();
            let table_id = self.core.table_id().clone();
            let has_topics = self.core.has_topic_routes(&table_id);
            let has_live_subs = notification_service.has_subscribers(None, &table_id);
            let notification = if has_topics || has_live_subs {
                let old_row = Self::build_notification_row(&latest_row);
                let new_row = Self::build_notification_row(&entity);
                Some(ChangeNotification::update(
                    table_id,
                    old_row,
                    new_row,
                    vec![self.primary_key_field_name().to_string()],
                ))
            } else {
                None
            };

            Ok(Some((row_key, notification)))
        }
        .instrument(span)
        .await
    }

    pub async fn delete_by_pk_value_deferred(
        &self,
        pk_value: &str,
        version: VersionId,
    ) -> Result<Option<(SharedTableRowId, Option<ChangeNotification>)>, KalamDbError> {
        let span = tracing::debug_span!(
            "table.delete",
            table_id = %self.core.table_id(),
            scope = "shared",
            pk = pk_value,
            deferred_side_effects = true
        );
        async move {
            let _authorization_mutation = self.begin_authorization_mutation();
            let pk_name = self.primary_key_field_name().to_string();
            let schema = self.schema();
            let pk_field = schema.field_with_name(&pk_name).map_err(|e| {
                KalamDbError::InvalidOperation(format!(
                    "PK column '{}' not found in schema: {}",
                    pk_name, e
                ))
            })?;
            let pk_column_type = pk_field.data_type();
            let pk_value_scalar =
                kalamdb_commons::conversions::parse_string_as_scalar(pk_value, pk_column_type)
                    .map_err(KalamDbError::InvalidOperation)?;

            let latest_row = if let Some((_key, row)) = self.find_by_pk(&pk_value_scalar).await? {
                row
            } else if self.pk_tombstoned_in_hot(&pk_value_scalar).await? {
                return Ok(None);
            } else {
                match base::find_row_by_pk(self, None, pk_value).await? {
                    Some((_key, row)) => row,
                    None => return Ok(None),
                }
            };

            let seq_id = version;

            let entity = SharedTableRow {
                _version: seq_id,
                _deleted: true,
                fields:   Row::new(latest_row.fields.values.clone()),
            };
            let row_key = seq_id;
            self.store.insert_async(row_key, entity.clone()).await.map_err(|e| {
                KalamDbError::InvalidOperation(format!("Failed to delete shared table row: {}", e))
            })?;

            if let Err(e) = self.stage_vector_delete(seq_id, pk_value).await {
                log::warn!(
                    "Failed to stage vector delete for table={}, seq={}, pk={}: {}",
                    self.core.table_id(),
                    seq_id.as_i64(),
                    pk_value,
                    e
                );
            }

            let manifest_service = self.core.services.manifest_service.clone();
            if let Err(e) = manifest_service.mark_pending_write(self.core.table_id(), None) {
                log::warn!(
                    "Failed to mark manifest as pending_write for {}: {}",
                    self.core.table_id(),
                    e
                );
            }

            let notification_service = self.core.services.notification_service.clone();
            let table_id = self.core.table_id().clone();
            let has_topics = self.core.has_topic_routes(&table_id);
            let has_live_subs = notification_service.has_subscribers(None, &table_id);
            let notification = if has_topics || has_live_subs {
                Some(
                    self.build_delete_notification(table_id, Self::build_notification_row(&entity)),
                )
            } else {
                None
            };

            Ok(Some((row_key, notification)))
        }
        .instrument(span)
        .await
    }
}
