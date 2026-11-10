impl UserTableProvider {
    async fn scan_with_version_resolution_to_kvs_result_async(
        &self,
        user_id: &UserId,
        filter: Option<&Expr>,
        since_seq: Option<VersionId>,
        limit: Option<usize>,
        keep_deleted: bool,
        cold_columns: Option<&[String]>,
        snapshot_commit_seq: Option<VersionId>,
        include_diagnostics: bool,
        storage_ordinals: Option<Vec<usize>>,
    ) -> Result<base::MvccScanResult<UserTableRowId, UserTableRow>, KalamDbError> {
        use kalamdb_store::EntityStoreAsync;

        let table_id = self.core.table_id();

        let user_prefix = UserTableRowId::user_prefix(user_id);
        let start_key_bytes = if let Some(seq) = since_seq {
            let start_seq = VersionId::try_from_i64(seq.as_i64().saturating_add(1)).unwrap_or(seq);
            let key = UserTableRowId::new(user_id.clone(), start_seq);
            Some(key.storage_key())
        } else {
            None
        };
        let scan_limit = base::calculate_scan_limit(limit);
        let pk_name = self.primary_key_field_name();

        let hot_future = async {
            if let Some(plan) = base::authoritative_hot_index_plan(
                self.store.as_ref(),
                filter,
                Some(user_id),
                keep_deleted,
                since_seq,
            ) {
                return base::scan_hot_index_resolved(
                    &self.store,
                    plan,
                    scan_limit,
                    pk_name,
                    Some(user_id),
                    storage_ordinals,
                )
                .await
                .and_then(|rows| {
                    base::reject_truncated_unbounded_scan(limit, rows.len())?;
                    Ok(rows)
                });
            }
            let scanned = if let Some(ordinals) = storage_ordinals {
                self.store
                    .scan_with_raw_prefix_selected_async(
                        &user_prefix,
                        start_key_bytes.as_deref(),
                        scan_limit,
                        ordinals,
                    )
                    .await
            } else {
                self.store
                    .scan_with_raw_prefix_async(
                        &user_prefix,
                        start_key_bytes.as_deref(),
                        scan_limit,
                    )
                    .await
            };
            scanned
                .map_err(|e| {
                    KalamDbError::InvalidOperation(format!(
                        "Failed to scan user table hot storage: {}",
                        e
                    ))
                })
                .and_then(|rows| {
                    base::reject_truncated_unbounded_scan(limit, rows.len())?;
                    Ok(rows)
                })
        };
        let cold_future = async {
            if include_diagnostics {
                base::scan_provider_parquet_stats(&self.core, Some(user_id), filter, cold_columns)
                    .await
            } else {
                base::scan_provider_parquet_result(&self.core, Some(user_id), filter, cold_columns)
                    .await
            }
        };
        let mask_secondary = base::masks_secondary_index_deletes(
            self.store.as_ref(),
            filter,
            Some(user_id),
            keep_deleted,
            since_seq,
        );
        let user_for_prefix = user_id.clone();
        let user_prefix_for_pk = |pk: &datafusion::scalar::ScalarValue| {
            self.pk_index.build_user_pk_prefix(&user_for_prefix, pk)
        };
        let mask = mask_secondary.then(|| base::SecondaryDeleteMask {
            store: self.store.as_ref(),
            schema: self.schema_ref(),
            pk_name,
            prefix_for_pk: &user_prefix_for_pk,
        });
        let resolved = base::resolve_latest_scan_from_futures(
            pk_name,
            limit,
            keep_deleted,
            snapshot_commit_seq,
            hot_future,
            cold_future,
            |row_data| self.construct_mvcc_row_from_parquet_data(user_id, row_data),
            mask,
        )
        .await?;

        let rows = resolved.rows;

        if log::log_enabled!(log::Level::Trace) {
            log::trace!(
                "[UserProvider] Final version-resolved (post-tombstone): {} rows (table={}; \
                 user={})",
                rows.len(),
                table_id,
                user_id.as_str()
            );
        }

        let diagnostics = if include_diagnostics {
            DeferredScanDiagnostics {
                hot_rows_scanned: Some(resolved.hot_rows_scanned),
                cold_rows_scanned: Some(resolved.cold_rows_scanned),
                cold_files_total: Some(resolved.cold_files_total),
                cold_files_skipped: Some(resolved.cold_files_skipped),
                cold_files_scanned: Some(resolved.cold_files_scanned),
                cold_files: resolved.cold_files,
                access_path: Some(ScanAccess::from_index_chosen(
                    base::authoritative_hot_index_plan(
                        self.store.as_ref(),
                        filter,
                        Some(user_id),
                        keep_deleted,
                        since_seq,
                    )
                    .is_some(),
                )),
            }
        } else {
            DeferredScanDiagnostics::default()
        };

        Ok(base::MvccScanResult { rows, diagnostics })
    }

    /// Count resolved rows without materializing full row data (single user).
    ///
    /// Used for COUNT(*) queries where projection is empty. Only decodes
    /// metadata (seq, deleted, pk) to perform version resolution.
    async fn count_resolved_rows_async(
        &self,
        user_id: &UserId,
        snapshot_commit_seq: Option<VersionId>,
    ) -> Result<usize, KalamDbError> {
        use kalamdb_commons::models::rows::RowMetadata;
        use kalamdb_store::EntityStoreAsync;

        let pk_name = self.primary_key_field_name().to_string();
        let store = Arc::clone(&self.store);
        let user_prefix = UserTableRowId::user_prefix(user_id);
        let pk_name_clone = pk_name.clone();

        // Hot storage: use the same typed user-prefix scan as the normal read path.
        let hot_future = async move {
            let hot_rows = store
                .scan_with_raw_prefix_async(
                    &user_prefix,
                    None,
                    crate::utils::base::UNBOUNDED_COUNT_SCAN_LIMIT,
                )
                .await
                .map_err(|e| {
                    KalamDbError::InvalidOperation(format!(
                        "Failed to scan user table hot storage for count: {}",
                        e
                    ))
                })?;
            crate::utils::base::reject_truncated_count_scan(hot_rows.len())?;

            let hot_metadata = hot_rows
                .into_iter()
                .map(|(_row_id, row)| RowMetadata {
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
            Some(user_id),
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
        user_id: &UserId,
        row_data: Row,
        version: VersionId,
        hot_predecessor: Option<(UserTableRowId, UserTableRow)>,
        planned_ops: Option<&mut Vec<kalamdb_store::Operation>>,
    ) -> Result<(UserTableRowId, Option<ChangeNotification>, Row), KalamDbError> {
        let span = tracing::debug_span!(
            "table.insert",
            table_id = %self.core.table_id(),
            user_id = %user_id.as_str(),
            column_count = row_data.values.len(),
            deferred_side_effects = true
        );
        async move {
            ensure_manifest_ready(&self.core, Some(user_id), "UserTableProvider")?;

            crate::utils::datafusion_dml::validate_not_null_with_set(
                self.core.non_null_columns(),
                std::slice::from_ref(&row_data),
            )?;

            // A row written earlier in this commit is still buffered, so the
            // storage lookup cannot see it. The caller passes that row.
            let predecessor = hot_predecessor;

            let seq_id = version;

            let entity = UserTableRow {
                user_id: user_id.clone(),
                _version: seq_id,
                _deleted: false,
                fields: row_data,
            };

            let row_key = UserTableRowId::new(user_id.clone(), seq_id);
            let stored_fields = entity.fields.clone();
            if let Some(operations) = planned_ops {
                crate::versioned::append_replaced_row_ops(
                    &self.store,
                    operations,
                    predecessor,
                    &row_key,
                    &entity,
                    "Failed to insert user table row",
                )?;
            } else {
                self.write_replaced_row(
                    predecessor,
                    row_key.clone(),
                    entity.clone(),
                    "Failed to insert user table row",
                )
                .await?;

                if let Err(error) = self.stage_vector_upsert(user_id, seq_id, &entity.fields).await
                {
                    crate::versioned::warn_vector_upsert(
                        self.core.table_id(),
                        Some(user_id),
                        seq_id,
                        &error,
                    );
                }

                crate::versioned::mark_table_pending_write(&self.core, Some(user_id));
            }

            let table_id = self.core.table_id().clone();
            let notification = crate::versioned::insert_notification_when_watched(
                crate::versioned::table_watch(&self.core, Some(user_id)).any(),
                table_id,
                || Self::build_notification_row(&entity),
            );
            Ok((row_key, notification, stored_fields))
        }
        .instrument(span)
        .await
    }

    /// Append one prevalidated insert without writing or staging vectors.
    pub(crate) async fn plan_insert_deferred(
        &self,
        user_id: &UserId,
        row_data: Row,
        version: VersionId,
        hot_predecessor: Option<(UserTableRowId, UserTableRow)>,
        operations: &mut Vec<kalamdb_store::Operation>,
    ) -> Result<(UserTableRowId, Option<ChangeNotification>, Row), KalamDbError> {
        self.insert_deferred_internal(user_id, row_data, version, hot_predecessor, Some(operations))
            .await
    }

    pub async fn insert_batch_with_versions(
        &self,
        user_id: &UserId,
        rows: Vec<Row>,
        versions: &[VersionId],
    ) -> Result<Vec<UserTableRowId>, KalamDbError> {
        let row_count = rows.len();
        let span = tracing::debug_span!(
            "table.insert_batch",
            table_id = %self.core.table_id(),
            user_id = %user_id.as_str(),
            row_count
        );
        async move {
            let entries = self.persist_insert_batch_rows(user_id, rows, versions, true).await?;
            let row_keys: Vec<UserTableRowId> =
                entries.iter().map(|(row_key, _)| row_key.clone()).collect();

            crate::versioned::fanout_insert_notifications(
                &self.core,
                Some(user_id),
                Some(user_id),
                || {
                    entries
                        .iter()
                        .map(|(_row_key, entity)| Self::build_notification_row(entity))
                        .collect()
                },
            )
            .await;

            Ok(row_keys)
        }
        .instrument(span)
        .await
    }

    pub async fn insert_batch_deferred_prevalidated_with_versions(
        &self,
        user_id: &UserId,
        rows: Vec<Row>,
        versions: &[VersionId],
    ) -> Result<Vec<(UserTableRowId, Option<ChangeNotification>)>, KalamDbError> {
        let row_count = rows.len();
        let span = tracing::debug_span!(
            "table.insert_batch",
            table_id = %self.core.table_id(),
            user_id = %user_id.as_str(),
            row_count,
            deferred_side_effects = true
        );
        async move {
            let entries = self.persist_insert_batch_rows(user_id, rows, versions, false).await?;

            let table_id = self.core.table_id().clone();
            let watched = crate::versioned::table_watch(&self.core, Some(user_id)).any();

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

    /// Validate and encode a prevalidated insert batch without writing it.
    pub(crate) async fn plan_insert_batch_prevalidated(
        &self,
        user_id: &UserId,
        rows: Vec<Row>,
        versions: &[VersionId],
    ) -> Result<
        (
            Vec<kalamdb_store::Operation>,
            Vec<(UserTableRowId, Option<ChangeNotification>)>,
            Vec<(UserTableRowId, UserTableRow)>,
        ),
        KalamDbError,
    > {
        let (entries, operations) =
            self.collect_insert_batch_rows(user_id, rows, versions, false).await?;
        let table_id = self.core.table_id().clone();
        let watched = crate::versioned::table_watch(&self.core, Some(user_id)).any();
        let applied = entries
            .iter()
            .map(|(row_key, entity)| {
                let notification = crate::versioned::insert_notification_when_watched(
                    watched,
                    table_id.clone(),
                    || Self::build_notification_row(entity),
                );
                (row_key.clone(), notification)
            })
            .collect();
        Ok((operations, applied, entries))
    }

    pub(crate) async fn plan_update_by_pk_value_deferred_with_hot(
        &self,
        user_id: &UserId,
        pk_value: &str,
        updates: Row,
        version: VersionId,
        known: Option<crate::versioned::AuthoritativeBaseline<UserTableRowId, UserTableRow>>,
        operations: &mut Vec<kalamdb_store::Operation>,
    ) -> Result<Option<(UserTableRowId, UserTableRow, Option<ChangeNotification>)>, KalamDbError>
    {
        self.update_by_pk_value_deferred_with_hot_ops(
            user_id,
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
        user_id: &UserId,
        pk_value: &str,
        updates: Row,
        version: VersionId,
        known: Option<crate::versioned::AuthoritativeBaseline<UserTableRowId, UserTableRow>>,
        planned_ops: Option<&mut Vec<kalamdb_store::Operation>>,
    ) -> Result<Option<(UserTableRowId, UserTableRow, Option<ChangeNotification>)>, KalamDbError>
    {
        let span = tracing::debug_span!(
            "table.update",
            table_id = %self.core.table_id(),
            user_id = %user_id.as_str(),
            pk = pk_value,
            deferred_side_effects = true
        );
        async move {
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
                        self.load_mutation_target(user_id, pk_value).await?,
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
                    user_id = %user_id.as_str(),
                    pk = pk_value,
                    "table.update_noop: row unchanged, skipping write"
                );
                return Ok(None);
            };

            let watched = crate::versioned::table_watch(&self.core, Some(user_id)).any();
            let old_notification = if watched {
                Some(Self::build_notification_row(&row))
            } else {
                None
            };
            let previous = hot_key.map(|key| (key, row));
            let seq_id = version;

            let entity = UserTableRow {
                user_id: user_id.clone(),
                _version: seq_id,
                _deleted: false,
                fields: new_fields,
            };
            let row_key = UserTableRowId::new(user_id.clone(), seq_id);
            if let Some(operations) = planned_ops {
                crate::versioned::append_replaced_row_ops(
                    &self.store,
                    operations,
                    previous,
                    &row_key,
                    &entity,
                    "Failed to update user table row",
                )?;
            } else {
                self.write_replaced_row(
                    previous,
                    row_key.clone(),
                    entity.clone(),
                    "Failed to update user table row",
                )
                .await?;

                if let Err(error) = self.stage_vector_upsert(user_id, seq_id, &entity.fields).await
                {
                    crate::versioned::warn_vector_upsert(
                        self.core.table_id(),
                        Some(user_id),
                        seq_id,
                        &error,
                    );
                }

                crate::versioned::mark_table_pending_write(&self.core, Some(user_id));
            }

            let notification = old_notification.map(|old_row| {
                crate::versioned::update_notification(
                    self.core.table_id().clone(),
                    old_row,
                    Self::build_notification_row(&entity),
                    self.primary_key_field_name().to_string(),
                )
            });

            Ok(Some((row_key, entity, notification)))
        }
        .instrument(span)
        .await
    }

    pub async fn delete_by_pk_value_deferred(
        &self,
        user_id: &UserId,
        pk_value: &str,
        version: VersionId,
    ) -> Result<Option<(UserTableRowId, Option<ChangeNotification>)>, KalamDbError> {
        Ok(self
            .delete_by_pk_value_deferred_with_hot(user_id, pk_value, version, None)
            .await?
            .map(|(row_key, _row, notification)| (row_key, notification)))
    }

    /// Delete `pk_value`, using `known_hot` when this commit already wrote that key.
    pub async fn delete_by_pk_value_deferred_with_hot(
        &self,
        user_id: &UserId,
        pk_value: &str,
        version: VersionId,
        known_hot: Option<(UserTableRowId, UserTableRow)>,
    ) -> Result<Option<(UserTableRowId, UserTableRow, Option<ChangeNotification>)>, KalamDbError>
    {
        self.delete_by_pk_value_deferred_with_hot_ops(
            user_id,
            pk_value,
            version,
            known_hot.map(|(key, row)| crate::versioned::AuthoritativeBaseline::Hot { key, row }),
            None,
        )
        .await
    }

    pub(crate) async fn plan_delete_by_pk_value_deferred_with_hot(
        &self,
        user_id: &UserId,
        pk_value: &str,
        version: VersionId,
        known: Option<crate::versioned::AuthoritativeBaseline<UserTableRowId, UserTableRow>>,
        operations: &mut Vec<kalamdb_store::Operation>,
    ) -> Result<Option<(UserTableRowId, UserTableRow, Option<ChangeNotification>)>, KalamDbError>
    {
        self.delete_by_pk_value_deferred_with_hot_ops(
            user_id,
            pk_value,
            version,
            known,
            Some(operations),
        )
        .await
    }

    async fn delete_by_pk_value_deferred_with_hot_ops(
        &self,
        user_id: &UserId,
        pk_value: &str,
        version: VersionId,
        known: Option<crate::versioned::AuthoritativeBaseline<UserTableRowId, UserTableRow>>,
        planned_ops: Option<&mut Vec<kalamdb_store::Operation>>,
    ) -> Result<Option<(UserTableRowId, UserTableRow, Option<ChangeNotification>)>, KalamDbError>
    {
        let span = tracing::debug_span!(
            "table.delete",
            table_id = %self.core.table_id(),
            user_id = %user_id.as_str(),
            pk = pk_value,
            deferred_side_effects = true
        );
        async move {
            let loaded = match known {
                Some(baseline) => {
                    crate::versioned::take_delete_baseline(baseline, |row| row._deleted)
                },
                None => crate::versioned::take_delete_baseline(
                    crate::versioned::authoritative_baseline(
                        self.load_mutation_target(user_id, pk_value).await?,
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

            let entity = UserTableRow {
                user_id: user_id.clone(),
                _version: seq_id,
                _deleted: true,
                fields: latest_row.fields,
            };
            let row_key = UserTableRowId::new(user_id.clone(), seq_id);
            if let Some(operations) = planned_ops {
                crate::versioned::append_replaced_row_ops(
                    &self.store,
                    operations,
                    previous,
                    &row_key,
                    &entity,
                    "Failed to delete user table row",
                )?;
            } else {
                self.write_replaced_row(
                    previous,
                    row_key.clone(),
                    entity.clone(),
                    "Failed to delete user table row",
                )
                .await?;

                if let Err(error) = self.stage_vector_delete(user_id, seq_id, pk_value).await {
                    crate::versioned::warn_vector_delete(
                        self.core.table_id(),
                        Some(user_id),
                        seq_id,
                        pk_value,
                        &error,
                    );
                }

                crate::versioned::mark_table_pending_write(&self.core, Some(user_id));
            }

            let table_id = self.core.table_id().clone();
            let notification = if crate::versioned::table_watch(&self.core, Some(user_id)).any() {
                Some(ChangeNotification::delete_soft(
                    table_id,
                    Self::build_notification_row(&entity),
                ))
            } else {
                None
            };

            Ok(Some((row_key, entity, notification)))
        }
        .instrument(span)
        .await
    }

    /// One `load_mutation_target` per key, then one `replace_batch`.
    ///
    /// That is the same authoritative decode the mixed commit uses. Update
    /// batches keep this load and do not add a second payload read to recheck
    /// a version.
    pub async fn delete_batch_by_pk_values_deferred(
        &self,
        user_id: &UserId,
        pk_values: &[String],
        versions: &[VersionId],
    ) -> Result<Vec<crate::versioned::DeferredMutation<UserTableRowId>>, KalamDbError> {
        if pk_values.is_empty() {
            return Ok(Vec::new());
        }
        let table_id = self.core.table_id().clone();
        let watched = crate::versioned::table_watch(&self.core, Some(user_id)).any();
        let mut items = Vec::with_capacity(pk_values.len());
        let mut applied = Vec::with_capacity(pk_values.len());
        for (pk_value, version) in pk_values.iter().zip(versions.iter().copied()) {
            let Some((previous, latest_row)) = crate::versioned::optional_live_row(
                self.load_mutation_target(user_id, pk_value).await?,
            ) else {
                continue;
            };
            let prior = latest_row.fields.clone();
            let row_key = UserTableRowId::new(user_id.clone(), version);
            let entity = UserTableRow {
                user_id: user_id.clone(),
                _version: version,
                _deleted: true,
                fields: latest_row.fields,
            };
            let notification = watched.then(|| {
                ChangeNotification::delete_soft(
                    table_id.clone(),
                    Self::build_notification_row(&entity),
                )
            });
            if let Err(error) = self.stage_vector_delete(user_id, version, pk_value).await {
                crate::versioned::warn_vector_delete(
                    self.core.table_id(),
                    Some(user_id),
                    version,
                    pk_value,
                    &error,
                );
            }
            items.push((previous, row_key.clone(), entity));
            applied.push(crate::versioned::DeferredMutation {
                key: row_key,
                notification,
                prior,
                payload: None,
            });
        }
        if !items.is_empty() {
            self.store.replace_batch_async(items).await.map_err(|e| {
                KalamDbError::InvalidOperation(format!("Failed to delete user table rows: {}", e))
            })?;
            crate::versioned::mark_table_pending_write(&self.core, Some(user_id));
        }
        Ok(applied)
    }

    pub async fn update_batch_by_pk_values_deferred(
        &self,
        user_id: &UserId,
        updates: &[(String, Row, VersionId)],
    ) -> Result<Vec<crate::versioned::DeferredMutation<UserTableRowId>>, KalamDbError> {
        if updates.is_empty() {
            return Ok(Vec::new());
        }
        let schema = self.schema();
        let table_id = self.core.table_id().clone();
        let watched = crate::versioned::table_watch(&self.core, Some(user_id)).any();
        let pk_column = self.primary_key_field_name().to_string();
        let mut items = Vec::with_capacity(updates.len());
        let mut applied = Vec::with_capacity(updates.len());
        for (pk_value, update_row, version) in updates {
            let updates = crate::versioned::coerce_update_row(update_row.clone(), &schema)?;
            let (previous, latest_row) = crate::versioned::require_live_row(
                self.load_mutation_target(user_id, pk_value).await?,
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
            let row_key = UserTableRowId::new(user_id.clone(), *version);
            let entity = UserTableRow {
                user_id: user_id.clone(),
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
            if let Err(error) = self.stage_vector_upsert(user_id, *version, &entity.fields).await {
                crate::versioned::warn_vector_upsert(
                    self.core.table_id(),
                    Some(user_id),
                    *version,
                    &error,
                );
            }
            items.push((previous, row_key.clone(), entity));
            applied.push(crate::versioned::DeferredMutation {
                key: row_key,
                notification,
                prior,
                payload: Some(update_row.clone()),
            });
        }
        if !items.is_empty() {
            self.store.replace_batch_async(items).await.map_err(|e| {
                KalamDbError::InvalidOperation(format!("Failed to update user table rows: {}", e))
            })?;
            crate::versioned::mark_table_pending_write(&self.core, Some(user_id));
        }
        Ok(applied)
    }
}
