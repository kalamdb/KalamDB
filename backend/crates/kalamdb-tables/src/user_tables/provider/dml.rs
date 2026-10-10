fn scan_user_hot_pk_insert(
    store: &UserTableIndexedStore,
    pk_prefixes: &[(String, Vec<u8>)],
) -> Result<(Option<String>, HashSet<String>), KalamDbError> {
    crate::versioned::first_live_pk(pk_prefixes, |prefix| {
        Ok(store
            .get_latest_by_index_prefix(0, prefix)
            .map_err(|e| KalamDbError::InvalidOperation(format!("PK index scan failed: {}", e)))?
            .map(|(_, row)| row._deleted))
    })
}

impl UserTableProvider {
    async fn scan_with_version_resolution_to_kvs_with_diagnostics_async(
        &self,
        user_id: &UserId,
        filter: Option<&Expr>,
        since_seq: Option<VersionId>,
        limit: Option<usize>,
        keep_deleted: bool,
        cold_columns: Option<&[String]>,
        snapshot_commit_seq: Option<VersionId>,
    ) -> Result<base::MvccScanResult<UserTableRowId, UserTableRow>, KalamDbError> {
        self.scan_with_version_resolution_to_kvs_result_async(
            user_id,
            filter,
            since_seq,
            limit,
            keep_deleted,
            cold_columns,
            snapshot_commit_seq,
            true,
        )
        .await
    }

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
    ) -> Result<base::MvccScanResult<UserTableRowId, UserTableRow>, KalamDbError> {
        use kalamdb_store::EntityStoreAsync;

        let table_id = self.core.table_id();
        base::warn_if_unfiltered_scan(table_id, filter, limit, self.core.table_type());

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
            if since_seq.is_none() {
                if let Some(plan) = base::hot_index_seek(self.store.as_ref(), filter, Some(user_id))
                {
                    return base::scan_hot_index_resolved(
                        &self.store,
                        plan,
                        scan_limit,
                        pk_name,
                        Some(user_id),
                    )
                    .await
                    .map(|rows| {
                        rows.into_iter()
                            .map(|(row_id, row)| (row_id, UserMvccRow(row)))
                            .collect::<Vec<_>>()
                    });
                }
            }
            self.store
                .scan_with_raw_prefix_async(&user_prefix, start_key_bytes.as_deref(), scan_limit)
                .await
                .map_err(|e| {
                    KalamDbError::InvalidOperation(format!(
                        "Failed to scan user table hot storage: {}",
                        e
                    ))
                })
                .map(|rows| {
                    rows.into_iter()
                        .map(|(row_id, row)| (row_id, UserMvccRow(row)))
                        .collect::<Vec<_>>()
                })
        };
        let cold_future = async {
            if include_diagnostics {
                self.scan_parquet_files_with_stats_async(user_id, filter, cold_columns).await
            } else {
                self.scan_parquet_files_as_result_async(user_id, filter, cold_columns).await
            }
        };
        let resolved = base::resolve_latest_scan_from_futures(
            pk_name,
            limit,
            keep_deleted,
            snapshot_commit_seq,
            hot_future,
            cold_future,
            |row_data| self.construct_mvcc_row_from_parquet_data(user_id, row_data),
        )
        .await?;

        let rows: Vec<(UserTableRowId, UserTableRow)> =
            resolved.rows.into_iter().map(|(row_id, row)| (row_id, row.0)).collect();

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
                hot_rows_scanned:   Some(resolved.hot_rows_scanned),
                cold_rows_scanned:  Some(resolved.cold_rows_scanned),
                cold_files_total:   Some(resolved.cold_files_total),
                cold_files_skipped: Some(resolved.cold_files_skipped),
                cold_files_scanned: Some(resolved.cold_files_scanned),
                cold_files:         resolved.cold_files,
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
                .scan_with_raw_prefix_async(&user_prefix, None, 1_000_000)
                .await
                .map_err(|e| {
                    KalamDbError::InvalidOperation(format!(
                        "Failed to scan user table hot storage for count: {}",
                        e
                    ))
                })?;

            let hot_metadata = hot_rows
                .into_iter()
                .map(|(_row_id, row)| RowMetadata {
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
            self.scan_parquet_files_as_batch_async(user_id, None, Some(cold_columns.as_slice()));

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
        validate_unique_pk: bool,
        version: VersionId,
    ) -> Result<(UserTableRowId, Option<ChangeNotification>), KalamDbError> {
        let span = tracing::debug_span!(
            "table.insert",
            table_id = %self.core.table_id(),
            user_id = %user_id.as_str(),
            column_count = row_data.values.len(),
            deferred_side_effects = true
        );
        async move {
            ensure_manifest_ready(
                &self.core,
                self.core.table_type(),
                Some(user_id),
                "UserTableProvider",
            )?;

            crate::utils::datafusion_dml::validate_not_null_with_set(
                self.core.non_null_columns(),
                std::slice::from_ref(&row_data),
            )?;

            if validate_unique_pk {
                base::ensure_unique_pk_value(self, Some(user_id), &row_data).await?;
            }

            let seq_id = version;

            let entity = UserTableRow {
                user_id:  user_id.clone(),
                _version: seq_id,
                _deleted: false,
                fields:   row_data,
            };

            let row_key = UserTableRowId::new(user_id.clone(), seq_id);
            self.append_hot_row(&row_key, &entity, "Failed to insert user table row")
                .await?;

            if let Err(e) = self.stage_vector_upsert(user_id, seq_id, &entity.fields).await {
                log::warn!(
                    "Failed to stage vector upsert for table={}, user={}, seq={}: {}",
                    self.core.table_id(),
                    user_id.as_str(),
                    seq_id.as_i64(),
                    e
                );
            }

            let manifest_service = self.core.services.manifest_service.clone();
            if let Err(e) = manifest_service.mark_pending_write(self.core.table_id(), Some(user_id))
            {
                log::warn!(
                    "Failed to mark manifest as pending_write for {}: {}",
                    self.core.table_id(),
                    e
                );
            }

            let notification_service = self.core.services.notification_service.clone();
            let table_id = self.core.table_id().clone();
            let has_topics = self.core.has_topic_routes(&table_id);
            let has_live_subs = notification_service.has_subscribers(Some(user_id), &table_id);
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
        user_id: &UserId,
        row_data: Row,
        version: VersionId,
    ) -> Result<(UserTableRowId, Option<ChangeNotification>), KalamDbError> {
        self.insert_deferred_internal(user_id, row_data, true, version).await
    }

    pub async fn insert_deferred_prevalidated(
        &self,
        user_id: &UserId,
        row_data: Row,
        version: VersionId,
    ) -> Result<(UserTableRowId, Option<ChangeNotification>), KalamDbError> {
        self.insert_deferred_internal(user_id, row_data, false, version).await
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

            let notification_service = self.core.services.notification_service.clone();
            let table_id = self.core.table_id().clone();

            let has_topics = self.core.has_topic_routes(&table_id);
            let has_live_subs = notification_service.has_subscribers(Some(user_id), &table_id);
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
                            Some(user_id),
                        )
                        .await;
                }
                if has_live_subs {
                    for row in rows {
                        let notification = ChangeNotification::insert(table_id.clone(), row);
                        notification_service.notify_table_change(
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

    pub async fn insert_batch_deferred_prevalidated(
        &self,
        user_id: &UserId,
        rows: Vec<Row>,
        versions: &[VersionId],
    ) -> Result<Vec<(UserTableRowId, Option<ChangeNotification>)>, KalamDbError> {
        self.insert_batch_deferred_prevalidated_with_versions(user_id, rows, versions)
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

            let notification_service = self.core.services.notification_service.clone();
            let table_id = self.core.table_id().clone();
            let has_topics = self.core.has_topic_routes(&table_id);
            let has_live_subs = notification_service.has_subscribers(Some(user_id), &table_id);

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
        user_id: &UserId,
        pk_value: &str,
        updates: Row,
        version: VersionId,
    ) -> Result<Option<(UserTableRowId, Option<ChangeNotification>)>, KalamDbError> {
        let span = tracing::debug_span!(
            "table.update",
            table_id = %self.core.table_id(),
            user_id = %user_id.as_str(),
            pk = pk_value,
            deferred_side_effects = true
        );
        async move {
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
                if let Some(result) = self.find_by_pk(user_id, &pk_value_scalar).await? {
                    result
                } else if self.pk_tombstoned_in_hot(user_id, &pk_value_scalar).await? {
                    return Err(KalamDbError::NotFound(format!(
                        "Row with {}={} was deleted",
                        pk_name, pk_value
                    )));
                } else {
                    base::find_row_by_pk(self, Some(user_id), pk_value).await?.ok_or_else(|| {
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
                    user_id = %user_id.as_str(),
                    pk = pk_value,
                    "table.update_noop: row unchanged, skipping write"
                );
                return Ok(None);
            }

            let seq_id = version;

            let entity = UserTableRow {
                user_id:  user_id.clone(),
                _version: seq_id,
                _deleted: false,
                fields:   new_fields,
            };
            let row_key = UserTableRowId::new(user_id.clone(), seq_id);
            self.append_hot_row(&row_key, &entity, "Failed to update user table row")
                .await?;

            if let Err(e) = self.stage_vector_upsert(user_id, seq_id, &entity.fields).await {
                log::warn!(
                    "Failed to stage vector upsert for table={}, user={}, seq={}: {}",
                    self.core.table_id(),
                    user_id.as_str(),
                    seq_id.as_i64(),
                    e
                );
            }

            let manifest_service = self.core.services.manifest_service.clone();
            if let Err(e) = manifest_service.mark_pending_write(self.core.table_id(), Some(user_id))
            {
                log::warn!(
                    "Failed to mark manifest as pending_write for {}: {}",
                    self.core.table_id(),
                    e
                );
            }

            let notification_service = self.core.services.notification_service.clone();
            let table_id = self.core.table_id().clone();
            let has_topics = self.core.has_topic_routes(&table_id);
            let has_live_subs = notification_service.has_subscribers(Some(user_id), &table_id);
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
        user_id: &UserId,
        pk_value: &str,
        version: VersionId,
    ) -> Result<Option<(UserTableRowId, Option<ChangeNotification>)>, KalamDbError> {
        let span = tracing::debug_span!(
            "table.delete",
            table_id = %self.core.table_id(),
            user_id = %user_id.as_str(),
            pk = pk_value,
            deferred_side_effects = true
        );
        async move {
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

            let latest_row =
                if let Some((_key, row)) = self.find_by_pk(user_id, &pk_value_scalar).await? {
                    row
                } else if self.pk_tombstoned_in_hot(user_id, &pk_value_scalar).await? {
                    return Ok(None);
                } else {
                    match base::find_row_by_pk(self, Some(user_id), pk_value).await? {
                        Some((_key, row)) => row,
                        None => return Ok(None),
                    }
                };

            let seq_id = version;

            let values = latest_row.fields.values.clone();
            let entity = UserTableRow {
                user_id:  user_id.clone(),
                _version: seq_id,
                _deleted: true,
                fields:   Row::new(values),
            };
            let row_key = UserTableRowId::new(user_id.clone(), seq_id);
            self.append_hot_row(&row_key, &entity, "Failed to delete user table row")
                .await?;

            if let Err(e) = self.stage_vector_delete(user_id, seq_id, pk_value).await {
                log::warn!(
                    "Failed to stage vector delete for table={}, user={}, seq={}, pk={}: {}",
                    self.core.table_id(),
                    user_id.as_str(),
                    seq_id.as_i64(),
                    pk_value,
                    e
                );
            }

            let manifest_service = self.core.services.manifest_service.clone();
            if let Err(e) = manifest_service.mark_pending_write(self.core.table_id(), Some(user_id))
            {
                log::warn!(
                    "Failed to mark manifest as pending_write for {}: {}",
                    self.core.table_id(),
                    e
                );
            }

            let notification_service = self.core.services.notification_service.clone();
            let table_id = self.core.table_id().clone();
            let has_topics = self.core.has_topic_routes(&table_id);
            let has_live_subs = notification_service.has_subscribers(Some(user_id), &table_id);
            let notification = if has_topics || has_live_subs {
                Some(ChangeNotification::delete_soft(
                    table_id,
                    Self::build_notification_row(&entity),
                ))
            } else {
                None
            };

            Ok(Some((row_key, notification)))
        }
        .instrument(span)
        .await
    }
}
