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
                    version: row._version,
                    deleted: row._deleted,
                    pk_bucket: pk_bucket_key_from_row(&row.fields, &pk_name_clone, row._version),
                })
                .collect();
            Ok::<_, KalamDbError>(hot_metadata)
        };

        // Cold storage: project only the PK + MVCC metadata needed for counting.
        let cold_columns = base::compute_metadata_only_cold_columns(&pk_name);
        let cold_future = base::scan_provider_parquet_result(
            &self.core,
            None,
            None,
            Some(cold_columns.as_slice()),
        );

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
        version: VersionId,
        hot_predecessor: Option<(SharedTableRowId, SharedTableRow)>,
        planned_ops: Option<&mut Vec<kalamdb_store::Operation>>,
    ) -> Result<(SharedTableRowId, Option<ChangeNotification>, Row), KalamDbError> {
        let span = tracing::debug_span!(
            "table.insert",
            table_id = %self.core.table_id(),
            scope = "shared",
            column_count = row_data.values.len(),
            deferred_side_effects = true
        );
        async move {
            let _authorization_mutation = self.begin_authorization_mutation();
            ensure_manifest_ready(&self.core, None, "SharedTableProvider")?;

            crate::utils::datafusion_dml::validate_not_null_with_set(
                self.core.non_null_columns(),
                std::slice::from_ref(&row_data),
            )?;

            // A row written earlier in this commit is still buffered, so the
            // storage lookup cannot see it. The caller passes that row.
            let predecessor = hot_predecessor;

            let seq_id = version;

            let entity = SharedTableRow {
                _version: seq_id,
                _deleted: false,
                fields: row_data,
            };
            let row_key = seq_id;
            let stored_fields = entity.fields.clone();
            let table_id = self.core.table_id().clone();
            let watched = crate::versioned::table_watch(&self.core, None).any();
            let notification = watched.then(|| {
                ChangeNotification::insert(table_id, Self::build_notification_row(&entity))
            });

            if let Some(operations) = planned_ops {
                crate::versioned::append_replaced_row_ops(
                    &self.store,
                    operations,
                    predecessor,
                    &row_key,
                    &entity,
                    "Failed to insert shared table row",
                )?;
            } else {
                if let Err(error) = self.stage_vector_upsert(seq_id, &entity.fields).await {
                    crate::versioned::warn_vector_upsert(
                        self.core.table_id(),
                        None,
                        seq_id,
                        &error,
                    );
                }

                self.write_replaced_row(
                    predecessor,
                    row_key,
                    entity,
                    "Failed to insert shared table row",
                )
                .await?;

                crate::versioned::mark_table_pending_write(&self.core, None);
            }

            Ok((row_key, notification, stored_fields))
        }
        .instrument(span)
        .await
    }

    pub(crate) async fn plan_insert_deferred(
        &self,
        row_data: Row,
        version: VersionId,
        hot_predecessor: Option<(SharedTableRowId, SharedTableRow)>,
        operations: &mut Vec<kalamdb_store::Operation>,
    ) -> Result<(SharedTableRowId, Option<ChangeNotification>, Row), KalamDbError> {
        self.insert_deferred_internal(row_data, version, hot_predecessor, Some(operations))
            .await
    }

    async fn persist_insert_batch_rows(
        &self,
        rows: Vec<Row>,
        versions: &[VersionId],
        validate_unique_pk: bool,
    ) -> Result<Vec<(SharedTableRowId, SharedTableRow)>, KalamDbError> {
        let (entries, operations) =
            self.collect_insert_batch_rows(rows, versions, validate_unique_pk).await?;
        if entries.is_empty() {
            return Ok(entries);
        }
        self.store.commit_operations(operations).map_err(|error| {
            KalamDbError::InvalidOperation(format!("Failed to batch insert table rows: {error}"))
        })?;
        self.finish_planned_shared_insert(&entries).await;
        let row_count = entries.len();
        log::debug!(
            "Batch inserted {} shared table rows with _seq range [{}, {}]",
            row_count,
            entries.first().map(|(k, _)| k.as_i64()).unwrap_or(0),
            entries.last().map(|(k, _)| k.as_i64()).unwrap_or(0)
        );
        Ok(entries)
    }

    /// Build shared insert rows and storage operations without writing or staging effects.
    pub(crate) async fn collect_insert_batch_rows(
        &self,
        rows: Vec<Row>,
        versions: &[VersionId],
        validate_unique_pk: bool,
    ) -> Result<
        (Vec<(SharedTableRowId, SharedTableRow)>, Vec<kalamdb_store::Operation>),
        KalamDbError,
    > {
        if rows.is_empty() {
            return Ok((Vec::new(), Vec::new()));
        }

        let _authorization_mutation = self.begin_authorization_mutation();

        ensure_manifest_ready(&self.core, None, "SharedTableProvider")?;

        let coerced_rows = crate::versioned::coerce_row_batch(rows, &self.schema_ref())?;

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
                    let pk_values =
                        crate::versioned::batch_primary_keys(pk_name, coerced_rows.iter())?;
                    if pk_values.is_empty() {
                        return Ok(HashMap::new());
                    }
                    let prefixes = pk_values
                        .iter()
                        .map(|(pk_str, pk_value)| {
                            (pk_str.clone(), self.pk_index.build_shared_pk_prefix(pk_value))
                        })
                        .collect();
                    crate::versioned::tombstones_replacing_insert_keys(
                        Arc::clone(&self.store),
                        &self.core,
                        None,
                        pk_name,
                        prefixes,
                        None,
                        None,
                    )
                    .await
                },
                "table.pk_unique"
            )?;
        }

        crate::versioned::require_version_count(versions.len(), row_count)?;

        let mut entries: Vec<(SharedTableRowId, SharedTableRow)> = Vec::with_capacity(row_count);

        for (row_data, version) in coerced_rows.into_iter().zip(versions.iter().copied()) {
            entries.push((
                version,
                SharedTableRow {
                    _version: version,
                    _deleted: false,
                    fields: row_data,
                },
            ));
        }

        let mut operations = Vec::new();
        crate::versioned::append_insert_batch_ops(
            &self.store,
            &entries,
            replaced_tombstones,
            self.primary_key_field_name(),
            |entity| &entity.fields,
            &mut operations,
        )?;
        Ok((entries, operations))
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

            crate::versioned::fanout_insert_notifications(&self.core, None, actor_user_id, || {
                entries
                    .iter()
                    .map(|(_row_key, entity)| Self::build_notification_row(entity))
                    .collect()
            })
            .await;

            Ok(row_keys)
        }
        .instrument(span)
        .await
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

            let table_id = self.core.table_id().clone();
            let watched = crate::versioned::table_watch(&self.core, None).any();

            Ok(entries
                .into_iter()
                .map(|(row_key, entity)| {
                    let notification = crate::versioned::insert_notification_when_watched(
                        watched,
                        table_id.clone(),
                        || Self::build_notification_row(&entity),
                    );
                    (row_key, notification)
                })
                .collect())
        }
        .instrument(span)
        .await
    }

    /// Validate and encode a prevalidated shared insert batch without writing it.
    pub(crate) async fn plan_insert_batch_prevalidated(
        &self,
        rows: Vec<Row>,
        versions: &[VersionId],
    ) -> Result<
        (
            Vec<kalamdb_store::Operation>,
            Vec<(SharedTableRowId, Option<ChangeNotification>)>,
            Vec<(SharedTableRowId, SharedTableRow)>,
        ),
        KalamDbError,
    > {
        let (entries, operations) = self.collect_insert_batch_rows(rows, versions, false).await?;
        let table_id = self.core.table_id().clone();
        let watched = crate::versioned::table_watch(&self.core, None).any();
        let applied = entries
            .iter()
            .map(|(row_key, entity)| {
                let notification = crate::versioned::insert_notification_when_watched(
                    watched,
                    table_id.clone(),
                    || Self::build_notification_row(entity),
                );
                (*row_key, notification)
            })
            .collect();
        Ok((operations, applied, entries))
    }

    pub(crate) async fn plan_update_by_pk_value_deferred_with_hot(
        &self,
        pk_value: &str,
        updates: Row,
        version: VersionId,
        known: Option<crate::versioned::AuthoritativeBaseline<SharedTableRowId, SharedTableRow>>,
        operations: &mut Vec<kalamdb_store::Operation>,
    ) -> Result<Option<(SharedTableRowId, SharedTableRow, Option<ChangeNotification>)>, KalamDbError>
    {
        self.update_by_pk_value_deferred_with_hot_ops(
            pk_value,
            updates,
            version,
            known,
            Some(operations),
        )
        .await
    }

    async fn update_by_pk_value_deferred_with_hot_ops(
        &self,
        pk_value: &str,
        updates: Row,
        version: VersionId,
        known: Option<crate::versioned::AuthoritativeBaseline<SharedTableRowId, SharedTableRow>>,
        planned_ops: Option<&mut Vec<kalamdb_store::Operation>>,
    ) -> Result<Option<(SharedTableRowId, SharedTableRow, Option<ChangeNotification>)>, KalamDbError>
    {
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
            let updates = crate::versioned::coerce_update_row(updates, &schema)?;

            let pk_name = self.primary_key_field_name().to_string();
            let loaded = match known {
                Some(baseline) => crate::versioned::take_update_baseline(
                    baseline,
                    |row| row._deleted,
                    &pk_name,
                    pk_value,
                )?,
                None => crate::versioned::take_update_baseline(
                    crate::versioned::authoritative_baseline(
                        self.load_mutation_target(pk_value).await?,
                    ),
                    |row| row._deleted,
                    &pk_name,
                    pk_value,
                )?,
            };
            let (hot_key, row) = match loaded {
                crate::versioned::LoadedBaseline::Hot(key, row) => (Some(key), row),
                crate::versioned::LoadedBaseline::Cold(row) => (None, row),
            };

            let Some(new_fields) = crate::versioned::changed_update_fields(
                &row.fields,
                updates,
                self.core.non_null_columns(),
            )?
            else {
                tracing::debug!(
                    table_id = %self.core.table_id(),
                    pk = pk_value,
                    "table.update_noop: row unchanged, skipping write"
                );
                return Ok(None);
            };

            let watched = crate::versioned::table_watch(&self.core, None).any();
            let old_notification = if watched {
                Some(Self::build_notification_row(&row))
            } else {
                None
            };
            let previous = hot_key.map(|key| (key, row));
            let seq_id = version;

            let entity = SharedTableRow {
                _version: seq_id,
                _deleted: false,
                fields: new_fields,
            };
            let row_key = seq_id;

            let notification = old_notification.map(|old_row| {
                crate::versioned::update_notification(
                    self.core.table_id().clone(),
                    old_row,
                    Self::build_notification_row(&entity),
                    self.primary_key_field_name().to_string(),
                )
            });

            if let Some(operations) = planned_ops {
                crate::versioned::append_replaced_row_ops(
                    &self.store,
                    operations,
                    previous,
                    &row_key,
                    &entity,
                    "Failed to update shared table row",
                )?;
            } else {
                if let Err(error) = self.stage_vector_upsert(seq_id, &entity.fields).await {
                    crate::versioned::warn_vector_upsert(
                        self.core.table_id(),
                        None,
                        seq_id,
                        &error,
                    );
                }

                self.write_replaced_row(
                    previous,
                    row_key,
                    entity.clone(),
                    "Failed to update shared table row",
                )
                .await?;

                crate::versioned::mark_table_pending_write(&self.core, None);
            }

            Ok(Some((row_key, entity, notification)))
        }
        .instrument(span)
        .await
    }

    pub(crate) async fn plan_delete_by_pk_value_deferred_with_hot(
        &self,
        pk_value: &str,
        version: VersionId,
        known: Option<crate::versioned::AuthoritativeBaseline<SharedTableRowId, SharedTableRow>>,
        operations: &mut Vec<kalamdb_store::Operation>,
    ) -> Result<Option<(SharedTableRowId, SharedTableRow, Option<ChangeNotification>)>, KalamDbError>
    {
        self.delete_by_pk_value_deferred_with_hot_ops(pk_value, version, known, Some(operations))
            .await
    }

    async fn delete_by_pk_value_deferred_with_hot_ops(
        &self,
        pk_value: &str,
        version: VersionId,
        known: Option<crate::versioned::AuthoritativeBaseline<SharedTableRowId, SharedTableRow>>,
        planned_ops: Option<&mut Vec<kalamdb_store::Operation>>,
    ) -> Result<Option<(SharedTableRowId, SharedTableRow, Option<ChangeNotification>)>, KalamDbError>
    {
        let span = tracing::debug_span!(
            "table.delete",
            table_id = %self.core.table_id(),
            scope = "shared",
            pk = pk_value,
            deferred_side_effects = true
        );
        async move {
            let _authorization_mutation = self.begin_authorization_mutation();
            let loaded = match known {
                Some(baseline) => {
                    crate::versioned::take_delete_baseline(baseline, |row| row._deleted)
                },
                None => crate::versioned::take_delete_baseline(
                    crate::versioned::authoritative_baseline(
                        self.load_mutation_target(pk_value).await?,
                    ),
                    |row| row._deleted,
                ),
            };
            let Some(loaded) = loaded else {
                return Ok(None);
            };
            let (previous, latest_row) = match loaded {
                crate::versioned::LoadedBaseline::Hot(key, row) => (Some((key, row.clone())), row),
                crate::versioned::LoadedBaseline::Cold(row) => (None, row),
            };

            let seq_id = version;

            let entity = SharedTableRow {
                _version: seq_id,
                _deleted: true,
                fields: latest_row.fields,
            };
            let row_key = seq_id;

            let table_id = self.core.table_id().clone();
            let watched = crate::versioned::table_watch(&self.core, None).any();
            let notification = if watched {
                Some(
                    self.build_delete_notification(table_id, Self::build_notification_row(&entity)),
                )
            } else {
                None
            };

            if let Some(operations) = planned_ops {
                crate::versioned::append_replaced_row_ops(
                    &self.store,
                    operations,
                    previous,
                    &row_key,
                    &entity,
                    "Failed to delete shared table row",
                )?;
            } else {
                if let Err(error) = self.stage_vector_delete(seq_id, pk_value).await {
                    crate::versioned::warn_vector_delete(
                        self.core.table_id(),
                        None,
                        seq_id,
                        pk_value,
                        &error,
                    );
                }

                self.write_replaced_row(
                    previous,
                    row_key,
                    entity.clone(),
                    "Failed to delete shared table row",
                )
                .await?;

                crate::versioned::mark_table_pending_write(&self.core, None);
            }

            Ok(Some((row_key, entity, notification)))
        }
        .instrument(span)
        .await
    }

    /// One `load_mutation_target` per key, then one replace batch.
    ///
    /// Same decode budget as the user range delete. Updates use that load and
    /// do not decode the payload again to recheck a version. One RocksDB batch
    /// and one pending-write mark follow the loads.
    pub async fn delete_batch_by_pk_values_deferred(
        &self,
        pk_values: &[String],
        versions: &[VersionId],
    ) -> Result<Vec<crate::versioned::DeferredMutation<SharedTableRowId>>, KalamDbError> {
        if pk_values.is_empty() {
            return Ok(Vec::new());
        }
        let table_id = self.core.table_id().clone();
        let watched = crate::versioned::table_watch(&self.core, None).any();

        let mut items = Vec::with_capacity(pk_values.len());
        let mut applied = Vec::with_capacity(pk_values.len());
        for (pk_value, version) in pk_values.iter().zip(versions.iter().copied()) {
            let Some((previous, latest_row)) =
                crate::versioned::optional_live_row(self.load_mutation_target(pk_value).await?)
            else {
                continue;
            };
            let prior = latest_row.fields.clone();
            let entity = SharedTableRow {
                _version: version,
                _deleted: true,
                fields: latest_row.fields,
            };
            let notification = watched.then(|| {
                self.build_delete_notification(
                    table_id.clone(),
                    Self::build_notification_row(&entity),
                )
            });
            if let Err(error) = self.stage_vector_delete(version, pk_value).await {
                crate::versioned::warn_vector_delete(
                    self.core.table_id(),
                    None,
                    version,
                    pk_value,
                    &error,
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
        crate::versioned::mark_table_pending_write(&self.core, None);
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
        let table_id = self.core.table_id().clone();
        let watched = crate::versioned::table_watch(&self.core, None).any();
        let pk_column = self.primary_key_field_name().to_string();

        let mut items = Vec::with_capacity(updates.len());
        let mut applied = Vec::with_capacity(updates.len());
        for (pk_value, update_row, version) in updates {
            let updates = crate::versioned::coerce_update_row(update_row.clone(), &schema)?;
            let (previous, latest_row) = crate::versioned::require_live_row(
                self.load_mutation_target(pk_value).await?,
                &pk_column,
                pk_value,
            )?;
            let Some(new_fields) = crate::versioned::changed_update_fields(
                &latest_row.fields,
                updates,
                self.core.non_null_columns(),
            )?
            else {
                continue;
            };
            let prior = latest_row.fields.clone();
            let entity = SharedTableRow {
                _version: *version,
                _deleted: false,
                fields: new_fields,
            };
            let notification = crate::versioned::update_notification_when_watched(
                watched,
                table_id.clone(),
                || Self::build_notification_row(&latest_row),
                || Self::build_notification_row(&entity),
                pk_column.clone(),
            );
            if let Err(error) = self.stage_vector_upsert(*version, &entity.fields).await {
                crate::versioned::warn_vector_upsert(self.core.table_id(), None, *version, &error);
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
        crate::versioned::mark_table_pending_write(&self.core, None);
        Ok(applied)
    }
}
