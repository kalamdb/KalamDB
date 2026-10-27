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
            let hot_rows = store
                .scan_all_async(Some(crate::utils::base::UNBOUNDED_COUNT_SCAN_LIMIT), None, None)
                .await
                .map_err(|e| {
                    KalamDbError::InvalidOperation(format!(
                        "Failed to scan shared table hot storage for count: {}",
                        e
                    ))
                })?;
            crate::utils::base::reject_truncated_count_scan(hot_rows.len())?;

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
        hot_predecessor: Option<(SharedTableRowId, SharedTableRow)>,
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

            // A row written earlier in this commit is still buffered, so the
            // storage lookup cannot see it. The caller passes that row.
            let predecessor = if hot_predecessor.is_some() {
                hot_predecessor
            } else if validate_unique_pk {
                self.insert_predecessor(&row_data).await?
            } else {
                None
            };

            let seq_id = version;

            let entity = SharedTableRow {
                _version: seq_id,
                _deleted: false,
                fields:   row_data,
            };
            let row_key = seq_id;

            if let Err(e) = self.stage_vector_upsert(seq_id, &entity.fields).await {
                log::warn!(
                    "Failed to stage vector upsert for table={}, seq={}: {}",
                    self.core.table_id(),
                    seq_id.as_i64(),
                    e
                );
            }

            let notification_service = self.core.services.notification_service.clone();
            let table_id = self.core.table_id().clone();
            let watched = self.core.has_topic_routes(&table_id)
                || notification_service.has_subscribers(None, &table_id);
            let notification = watched.then(|| {
                ChangeNotification::insert(table_id, Self::build_notification_row(&entity))
            });

            self.write_replaced_row(
                predecessor,
                row_key,
                entity,
                "Failed to insert shared table row",
            )
            .await?;

            let manifest_service = self.core.services.manifest_service.clone();
            if let Err(e) = manifest_service.mark_pending_write(self.core.table_id(), None) {
                log::warn!(
                    "Failed to mark manifest as pending_write for {}: {}",
                    self.core.table_id(),
                    e
                );
            }

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
        self.insert_deferred_internal(row_data, true, version, None).await
    }

    pub async fn insert_deferred_prevalidated(
        &self,
        row_data: Row,
        version: VersionId,
    ) -> Result<(SharedTableRowId, Option<ChangeNotification>), KalamDbError> {
        self.insert_deferred_internal(row_data, false, version, None).await
    }

    /// Insert over a hot row already staged in this commit.
    pub async fn insert_over_hot_deferred(
        &self,
        row_data: Row,
        version: VersionId,
        previous: (SharedTableRowId, SharedTableRow),
    ) -> Result<(SharedTableRowId, Option<ChangeNotification>), KalamDbError> {
        self.insert_deferred_internal(row_data, false, version, Some(previous)).await
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
        let mut replaced_tombstones: HashMap<String, (SharedTableRowId, SharedTableRow)> =
            HashMap::new();

        if validate_unique_pk {
            replaced_tombstones = kalamdb_observability::kdb_await_in_info_span!(
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
                        let hot_scan = if pk_prefixes.len() <= 1 {
                            scan_hot_pk_insert(&store, &pk_prefixes)?
                        } else {
                            tokio::task::spawn_blocking(
                        move || -> Result<
                            crate::versioned::HotInsertScan<SharedTableRowId, SharedTableRow>,
                            KalamDbError,
                        > {
                            scan_hot_pk_insert(&store, &pk_prefixes)
                        },
                    )
                    .await
                    .map_err(|e| {
                        KalamDbError::InvalidOperation(format!("spawn_blocking error: {}", e))
                    })??
                        };

                        if let Some(dup_pk) = hot_scan.duplicate {
                            return Err(KalamDbError::AlreadyExists(format!(
                                "Primary key violation: value '{}' already exists in column '{}'",
                                dup_pk, pk_name
                            )));
                        }

                        let pk_column_id = self.core.primary_key_column_id();
                        let mut pk_values_for_cold_check: Vec<String> =
                            Vec::with_capacity(pk_values_to_check.len());
                        for (pk_str, _pk_value) in &pk_values_to_check {
                            if !hot_scan.tombstones.contains_key(pk_str) {
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
                        return Ok(hot_scan.tombstones);
                    }
                    Ok(HashMap::new())
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
        if replaced_tombstones.is_empty() {
            store.insert_batch(&entries).map_err(|e| {
                KalamDbError::InvalidOperation(format!("Failed to batch insert table rows: {}", e))
            })?;
        } else {
            let pk_name = self.primary_key_field_name();
            let items = entries
                .iter()
                .map(|(key, entity)| {
                    let previous = crate::utils::unified_dml::extract_user_pk_value(
                        &entity.fields,
                        pk_name,
                    )
                    .ok()
                    .and_then(|pk| replaced_tombstones.remove(&pk));
                    (previous, *key, entity.clone())
                })
                .collect::<Vec<_>>();
            store.replace_batch(&items).map_err(|e| {
                KalamDbError::InvalidOperation(format!("Failed to batch insert table rows: {}", e))
            })?;
        }

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
        Ok(self
            .update_by_pk_value_deferred_with_hot(pk_value, updates, version, None)
            .await?
            .map(|(row_key, _row, notification)| (row_key, notification)))
    }

    /// Update `pk_value`, using `known_hot` when this commit already wrote that key.
    pub async fn update_by_pk_value_deferred_with_hot(
        &self,
        pk_value: &str,
        updates: Row,
        version: VersionId,
        known_hot: Option<(SharedTableRowId, SharedTableRow)>,
    ) -> Result<Option<(SharedTableRowId, SharedTableRow, Option<ChangeNotification>)>, KalamDbError> {
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
            let (previous, latest_row) = if let Some((key, row)) = known_hot {
                if row._deleted {
                    return Err(KalamDbError::NotFound(format!(
                        "Row with {}={} was deleted",
                        pk_name, pk_value
                    )));
                }
                (Some((key, row.clone())), row)
            } else {
                match self.load_mutation_target(pk_value).await? {
                    crate::versioned::MutationTarget::Hot(key, row) => {
                        (Some((key, row.clone())), row)
                    },
                    crate::versioned::MutationTarget::Cold(row) => (None, row),
                    crate::versioned::MutationTarget::Tombstoned => {
                        return Err(KalamDbError::NotFound(format!(
                            "Row with {}={} was deleted",
                            pk_name, pk_value
                        )));
                    },
                    crate::versioned::MutationTarget::Missing => {
                        return Err(KalamDbError::NotFound(format!(
                            "Row with {}={} not found (checked both hot and cold storage)",
                            pk_name, pk_value
                        )));
                    },
                }
            };

            let mut merged = latest_row.fields.values.clone();
            for (key, value) in updates.values {
                merged.insert(key, value);
            }
            let new_fields = Row::new(merged);

            crate::utils::datafusion_dml::validate_not_null_with_set(
                self.core.non_null_columns(),
                std::slice::from_ref(&new_fields),
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

            if let Err(e) = self.stage_vector_upsert(seq_id, &entity.fields).await {
                log::warn!(
                    "Failed to stage vector upsert for table={}, seq={}: {}",
                    self.core.table_id(),
                    seq_id.as_i64(),
                    e
                );
            }

            let notification_service = self.core.services.notification_service.clone();
            let table_id = self.core.table_id().clone();
            let watched = self.core.has_topic_routes(&table_id)
                || notification_service.has_subscribers(None, &table_id);
            let notification = if watched {
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

            self.write_replaced_row(
                previous,
                row_key,
                entity.clone(),
                "Failed to update shared table row",
            )
            .await?;

            let manifest_service = self.core.services.manifest_service.clone();
            if let Err(e) = manifest_service.mark_pending_write(self.core.table_id(), None) {
                log::warn!(
                    "Failed to mark manifest as pending_write for {}: {}",
                    self.core.table_id(),
                    e
                );
            }

            Ok(Some((row_key, entity, notification)))
        }
        .instrument(span)
        .await
    }

    pub async fn delete_by_pk_value_deferred(
        &self,
        pk_value: &str,
        version: VersionId,
    ) -> Result<Option<(SharedTableRowId, Option<ChangeNotification>)>, KalamDbError> {
        Ok(self
            .delete_by_pk_value_deferred_with_hot(pk_value, version, None)
            .await?
            .map(|(row_key, _row, notification)| (row_key, notification)))
    }

    /// Delete `pk_value`, using `known_hot` when this commit already wrote that key.
    pub async fn delete_by_pk_value_deferred_with_hot(
        &self,
        pk_value: &str,
        version: VersionId,
        known_hot: Option<(SharedTableRowId, SharedTableRow)>,
    ) -> Result<Option<(SharedTableRowId, SharedTableRow, Option<ChangeNotification>)>, KalamDbError> {
        let span = tracing::debug_span!(
            "table.delete",
            table_id = %self.core.table_id(),
            scope = "shared",
            pk = pk_value,
            deferred_side_effects = true
        );
        async move {
            let _authorization_mutation = self.begin_authorization_mutation();
            let (previous, latest_row) = if let Some((key, row)) = known_hot {
                if row._deleted {
                    return Ok(None);
                }
                (Some((key, row.clone())), row)
            } else {
                match self.load_mutation_target(pk_value).await? {
                    crate::versioned::MutationTarget::Hot(key, row) => {
                        (Some((key, row.clone())), row)
                    },
                    crate::versioned::MutationTarget::Cold(row) => (None, row),
                    crate::versioned::MutationTarget::Tombstoned
                    | crate::versioned::MutationTarget::Missing => return Ok(None),
                }
            };

            let seq_id = version;

            let entity = SharedTableRow {
                _version: seq_id,
                _deleted: true,
                fields:   latest_row.fields,
            };
            let row_key = seq_id;

            if let Err(e) = self.stage_vector_delete(seq_id, pk_value).await {
                log::warn!(
                    "Failed to stage vector delete for table={}, seq={}, pk={}: {}",
                    self.core.table_id(),
                    seq_id.as_i64(),
                    pk_value,
                    e
                );
            }

            let notification_service = self.core.services.notification_service.clone();
            let table_id = self.core.table_id().clone();
            let watched = self.core.has_topic_routes(&table_id)
                || notification_service.has_subscribers(None, &table_id);
            let notification = if watched {
                Some(self.build_delete_notification(table_id, Self::build_notification_row(&entity)))
            } else {
                None
            };

            self.write_replaced_row(
                previous,
                row_key,
                entity.clone(),
                "Failed to delete shared table row",
            )
            .await?;

            let manifest_service = self.core.services.manifest_service.clone();
            if let Err(e) = manifest_service.mark_pending_write(self.core.table_id(), None) {
                log::warn!(
                    "Failed to mark manifest as pending_write for {}: {}",
                    self.core.table_id(),
                    e
                );
            }

            Ok(Some((row_key, entity, notification)))
        }
        .instrument(span)
        .await
    }

    /// Delete many primary keys in one RocksDB batch and one pending-write mark.
    pub async fn delete_batch_by_pk_values_deferred(
        &self,
        pk_values: &[String],
        versions: &[VersionId],
    ) -> Result<Vec<crate::versioned::DeferredMutation<SharedTableRowId>>, KalamDbError> {
        if pk_values.is_empty() {
            return Ok(Vec::new());
        }
        let notification_service = self.core.services.notification_service.clone();
        let table_id = self.core.table_id().clone();
        let watched = self.core.has_topic_routes(&table_id)
            || notification_service.has_subscribers(None, &table_id);

        let mut items = Vec::with_capacity(pk_values.len());
        let mut applied = Vec::with_capacity(pk_values.len());
        for (pk_value, version) in pk_values.iter().zip(versions.iter().copied()) {
            let (previous, latest_row) = match self.load_mutation_target(pk_value).await? {
                crate::versioned::MutationTarget::Hot(key, row) => (Some((key, row.clone())), row),
                crate::versioned::MutationTarget::Cold(row) => (None, row),
                crate::versioned::MutationTarget::Tombstoned
                | crate::versioned::MutationTarget::Missing => continue,
            };
            let prior = latest_row.fields.clone();
            let entity = SharedTableRow {
                _version: version,
                _deleted: true,
                fields:   latest_row.fields,
            };
            let notification = watched.then(|| {
                self.build_delete_notification(
                    table_id.clone(),
                    Self::build_notification_row(&entity),
                )
            });
            if let Err(e) = self.stage_vector_delete(version, pk_value).await {
                log::warn!(
                    "Failed to stage vector delete for table={}, seq={}, pk={}: {}",
                    self.core.table_id(),
                    version.as_i64(),
                    pk_value,
                    e
                );
            }
            items.push((previous, version, entity));
            applied.push(crate::versioned::DeferredMutation {
                key: version,
                notification,
                prior,
                payload: None,
            });
        }

        if items.is_empty() {
            return Ok(applied);
        }
        self.store.replace_batch_async(items).await.map_err(|e| {
            KalamDbError::InvalidOperation(format!("Failed to delete shared table rows: {}", e))
        })?;
        if let Err(e) = self
            .core
            .services
            .manifest_service
            .mark_pending_write(self.core.table_id(), None)
        {
            log::warn!(
                "Failed to mark manifest as pending_write for {}: {}",
                self.core.table_id(),
                e
            );
        }
        Ok(applied)
    }

    /// Update many primary keys in one RocksDB batch and one pending-write mark.
    pub async fn update_batch_by_pk_values_deferred(
        &self,
        updates: &[(String, Row, VersionId)],
    ) -> Result<Vec<crate::versioned::DeferredMutation<SharedTableRowId>>, KalamDbError> {
        if updates.is_empty() {
            return Ok(Vec::new());
        }
        let schema = self.schema();
        let notification_service = self.core.services.notification_service.clone();
        let table_id = self.core.table_id().clone();
        let watched = self.core.has_topic_routes(&table_id)
            || notification_service.has_subscribers(None, &table_id);
        let pk_column = self.primary_key_field_name().to_string();

        let mut items = Vec::with_capacity(updates.len());
        let mut applied = Vec::with_capacity(updates.len());
        for (pk_value, update_row, version) in updates {
            let updates = coerce_updates(update_row.clone(), &schema).map_err(|e| {
                KalamDbError::InvalidOperation(format!("Schema coercion failed: {}", e))
            })?;
            let (previous, latest_row) = match self.load_mutation_target(pk_value).await? {
                crate::versioned::MutationTarget::Hot(key, row) => (Some((key, row.clone())), row),
                crate::versioned::MutationTarget::Cold(row) => (None, row),
                crate::versioned::MutationTarget::Tombstoned => {
                    return Err(KalamDbError::NotFound(format!(
                        "Row with {}={} was deleted",
                        pk_column, pk_value
                    )));
                },
                crate::versioned::MutationTarget::Missing => {
                    return Err(KalamDbError::NotFound(format!(
                        "Row with {}={} not found (checked both hot and cold storage)",
                        pk_column, pk_value
                    )));
                },
            };
            let mut merged = latest_row.fields.values.clone();
            for (key, value) in updates.values {
                merged.insert(key, value);
            }
            let new_fields = Row::new(merged);
            crate::utils::datafusion_dml::validate_not_null_with_set(
                self.core.non_null_columns(),
                std::slice::from_ref(&new_fields),
            )?;
            if new_fields == latest_row.fields {
                continue;
            }
            let prior = latest_row.fields.clone();
            let entity = SharedTableRow {
                _version: *version,
                _deleted: false,
                fields:   new_fields,
            };
            let notification = if watched {
                Some(ChangeNotification::update(
                    table_id.clone(),
                    Self::build_notification_row(&latest_row),
                    Self::build_notification_row(&entity),
                    vec![pk_column.clone()],
                ))
            } else {
                None
            };
            if let Err(e) = self.stage_vector_upsert(*version, &entity.fields).await {
                log::warn!(
                    "Failed to stage vector upsert for table={}, seq={}: {}",
                    self.core.table_id(),
                    version.as_i64(),
                    e
                );
            }
            items.push((previous, *version, entity));
            applied.push(crate::versioned::DeferredMutation {
                key: *version,
                notification,
                prior,
                payload: Some(update_row.clone()),
            });
        }

        if items.is_empty() {
            return Ok(applied);
        }
        self.store.replace_batch_async(items).await.map_err(|e| {
            KalamDbError::InvalidOperation(format!("Failed to update shared table rows: {}", e))
        })?;
        if let Err(e) = self
            .core
            .services
            .manifest_service
            .mark_pending_write(self.core.table_id(), None)
        {
            log::warn!(
                "Failed to mark manifest as pending_write for {}: {}",
                self.core.table_id(),
                e
            );
        }
        Ok(applied)
    }
}
