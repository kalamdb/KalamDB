#[derive(Clone)]
pub struct UserScanContext {
    user_id:             UserId,
    allow_all_users:     bool,
    snapshot_commit_seq: Option<VersionId>,
}

#[async_trait]
impl DeferredMvccScanProvider<UserTableRowId, UserTableRow> for UserTableProvider {
    type ScanContext = UserScanContext;

    fn scan_source_name(&self) -> &'static str {
        "user_table_scan"
    }

    fn build_scan_context(&self, state: &dyn Session) -> Result<Self::ScanContext, KalamDbError> {
        let (user_id, role) = extract_user_context(state)?;
        Ok(UserScanContext {
            user_id:             user_id.clone(),
            allow_all_users:     can_read_all_users(role),
            snapshot_commit_seq: extract_transaction_query_context(state).and_then(|context| {
                crate::utils::base::transaction_snapshot_bound(context.snapshot_commit_seq())
            }),
        })
    }

    fn scan_snapshot_commit_seq(&self, scan_context: &Self::ScanContext) -> Option<VersionId> {
        scan_context.snapshot_commit_seq
    }

    fn allow_pk_fast_path(&self, scan_context: &Self::ScanContext) -> bool {
        !scan_context.allow_all_users && scan_context.snapshot_commit_seq.is_none()
    }

    fn allow_count_only_fast_path(&self, scan_context: &Self::ScanContext) -> bool {
        !scan_context.allow_all_users
    }

    fn scan_scope_label(&self, scan_context: &Self::ScanContext) -> &'static str {
        if scan_context.allow_all_users {
            "all-users"
        } else {
            "subject"
        }
    }

    fn scan_cold_scope<'a>(&self, scan_context: &'a Self::ScanContext) -> Option<&'a UserId> {
        Some(&scan_context.user_id)
    }

    async fn scan_latest_hot_pk_entry(
        &self,
        scan_context: &Self::ScanContext,
        pk_value: &ScalarValue,
        storage_ordinals: Option<&[usize]>,
    ) -> Result<Option<(UserTableRowId, UserTableRow)>, KalamDbError> {
        self.latest_hot_pk_entry_maybe_selected(&scan_context.user_id, pk_value, storage_ordinals)
            .await
    }

    async fn count_rows_with_context(
        &self,
        scan_context: &Self::ScanContext,
    ) -> Result<usize, KalamDbError> {
        self.count_resolved_rows_async(&scan_context.user_id, scan_context.snapshot_commit_seq)
            .await
    }

    async fn scan_kvs_with_context(
        &self,
        scan_context: &Self::ScanContext,
        filter: Option<&Expr>,
        since_seq: Option<VersionId>,
        limit: Option<usize>,
        keep_deleted: bool,
        cold_columns: Option<&[String]>,
    ) -> Result<Vec<(UserTableRowId, UserTableRow)>, KalamDbError> {
        let user_id = &scan_context.user_id;
        if scan_context.allow_all_users {
            self.scan_all_users_with_version_resolution_async(
                filter,
                limit,
                keep_deleted,
                scan_context.snapshot_commit_seq,
                Some(user_id),
                false,
            )
            .await
            .map(|result| result.rows)
        } else {
            self.scan_with_version_resolution_to_kvs_async(
                user_id,
                filter,
                since_seq,
                limit,
                keep_deleted,
                cold_columns,
                scan_context.snapshot_commit_seq,
            )
            .await
        }
    }

    async fn scan_kvs_with_diagnostics(
        &self,
        scan_context: &Self::ScanContext,
        filter: Option<&Expr>,
        since_seq: Option<VersionId>,
        limit: Option<usize>,
        keep_deleted: bool,
        cold_columns: Option<&[String]>,
    ) -> Result<base::MvccScanResult<UserTableRowId, UserTableRow>, KalamDbError> {
        let user_id = &scan_context.user_id;
        if scan_context.allow_all_users {
            self.scan_all_users_with_version_resolution_async(
                filter,
                limit,
                keep_deleted,
                scan_context.snapshot_commit_seq,
                Some(user_id),
                true,
            )
            .await
        } else {
            self.scan_with_version_resolution_to_kvs_with_diagnostics_async(
                user_id,
                filter,
                since_seq,
                limit,
                keep_deleted,
                cold_columns,
                scan_context.snapshot_commit_seq,
            )
            .await
        }
    }
}

impl SourceProvider for UserTableProvider {
    fn filter_capability(&self, filter: &Expr) -> FilterCapability {
        mvcc_filter_capability(filter, self.primary_key_field_name())
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
impl BaseTableProvider<UserTableRowId, UserTableRow> for UserTableProvider {
    fn core(&self) -> &base::TableProviderCore {
        &self.core
    }

    fn construct_row_from_parquet_data(
        &self,
        user_id: &UserId,
        row_data: &crate::utils::version_resolution::ParquetRowData,
    ) -> Result<Option<(UserTableRowId, UserTableRow)>, KalamDbError> {
        let row_key = UserTableRowId::new(user_id.clone(), row_data.version);
        let row = UserTableRow {
            user_id:  user_id.clone(),
            _version: row_data.version,
            _deleted: row_data.deleted,
            fields:   row_data.fields.clone(),
        };
        Ok(Some((row_key, row)))
    }

    /// Override find_row_key_by_id_field to use PK index for efficient lookup
    ///
    /// This avoids scanning all rows and instead uses the secondary index.
    /// For hot storage (RocksDB), uses fast existence check. If not found in hot storage,
    /// falls back to checking cold storage using manifest-based pruning.
    ///
    /// OPTIMIZED: Uses `pk_exists_in_hot` for fast hot-path check (single index lookup + 1 entity
    /// fetch max). OPTIMIZED: Uses `pk_exists_in_cold` with manifest-based segment pruning for
    /// cold storage.
    async fn find_row_key_by_id_field(
        &self,
        user_id: &UserId,
        id_value: &str,
    ) -> Result<Option<UserTableRowId>, KalamDbError> {
        // Use shared helper to parse PK value
        let pk_value = crate::utils::pk::parse_pk_value(id_value);

        if let Some((row_id, row)) = self.latest_hot_pk_entry(user_id, &pk_value).await? {
            if row._deleted {
                log::trace!("[UserTableProvider] PK {} latest hot version is tombstoned", id_value);
                return Ok(None);
            }
            log::trace!("[UserTableProvider] PK collision in hot storage: id={}", id_value);
            return Ok(Some(row_id));
        }

        log::trace!("[UserTableProvider] PK {} not in hot storage, checking cold", id_value);

        // Not found in hot storage - check cold storage using optimized manifest-based lookup
        // This uses column_stats to prune segments that can't contain the PK
        let pk_name = self.primary_key_field_name();
        let pk_column_id = self.core.primary_key_column_id();
        let exists_in_cold = base::pk_exists_in_cold(
            &self.core,
            self.core.table_id(),
            self.core.table_type(),
            Some(user_id),
            pk_name,
            pk_column_id,
            id_value,
        )
        .await?;

        if exists_in_cold {
            log::trace!("[UserTableProvider] PK {} exists in cold storage", id_value);
            // Return a sentinel key to signal existence in cold storage.
            // Callers that only check `is_some()` (PK uniqueness guards) will reject duplicates.
            return Ok(Some(UserTableRowId::new(
                user_id.clone(),
                VersionId::try_from_i64(1).expect("test version"),
            )));
        }

        Ok(None)
    }

    async fn insert(
        &self,
        user_id: &UserId,
        row_data: Row,
        version: VersionId,
    ) -> Result<UserTableRowId, KalamDbError> {
        let span = tracing::debug_span!(
            "table.insert",
            table_id = %self.core.table_id(),
            user_id = %user_id.as_str(),
            column_count = row_data.values.len()
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
            )
            .map_err(|e| KalamDbError::ConstraintViolation(e.to_string()))?;

            // Validate PRIMARY KEY uniqueness if user provided PK value
            base::ensure_unique_pk_value(self, Some(user_id), &row_data).await?;

            // Generate new SeqId via SystemColumnsService
            let seq_id = version;

            // Create UserTableRow directly
            let entity = UserTableRow {
                user_id:  user_id.clone(),
                _version: seq_id,
                _deleted: false,
                fields:   row_data,
            };

            // Create composite key
            let row_key = UserTableRowId::new(user_id.clone(), seq_id);

            // log::info!("🔍 [AS_USER_DEBUG] Inserting row for user_id='{}' _seq={}",
            //            user_id.as_str(), seq_id);

            // Use the same preencoded append path as batch inserts so single-row
            // MVCC writes and batch writes stay consistent.
            self.append_hot_row(&row_key, &entity, "Failed to insert user table row")
                .await?;

            if log::log_enabled!(log::Level::Debug) {
                log::debug!(
                    "Inserted user table row for user {} with _seq {}",
                    user_id.as_str(),
                    seq_id
                );
            }

            if let Err(e) = self.stage_vector_upsert(user_id, seq_id, &entity.fields).await {
                log::warn!(
                    "Failed to stage vector upsert for table={}, user={}, seq={}: {}",
                    self.core.table_id(),
                    user_id.as_str(),
                    seq_id.as_i64(),
                    e
                );
            }

            // Mark manifest as having pending writes (hot data needs to be flushed)
            let manifest_service = self.core.services.manifest_service.clone();
            if let Err(e) = manifest_service.mark_pending_write(self.core.table_id(), Some(user_id))
            {
                log::warn!(
                    "Failed to mark manifest as pending_write for {}: {}",
                    self.core.table_id(),
                    e
                );
            }

            // Fire live query + topic notification (INSERT)
            let notification_service = self.core.services.notification_service.clone();
            let table_id = self.core.table_id().clone();

            let has_topics = self.core.has_topic_routes(&table_id);
            let has_live_subs = notification_service.has_subscribers(Some(&user_id), &table_id);
            if has_topics || has_live_subs {
                // Build complete row including system columns (_seq, _deleted)
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
                    notification_service.notify_table_change(
                        Some(user_id.clone()),
                        table_id,
                        notification,
                    );
                }
            }

            Ok(row_key)
        }
        .instrument(span)
        .await
    }

    /// Optimized batch insert using single RocksDB WriteBatch
    ///
    /// **Performance**: This method is significantly faster than calling insert() N times:
    /// - Single mutex acquisition for all SeqId generation
    /// - Single RocksDB WriteBatch for all rows (one disk write vs N)
    /// - Batch PK validation (single scan + HashSet lookup instead of N individual checks)
    ///
    /// # Arguments
    /// * `user_id` - Subject user ID for RLS
    /// * `rows` - Vector of Row objects to insert
    ///
    /// # Returns
    /// Vector of generated UserTableRowIds
    async fn insert_batch(
        &self,
        user_id: &UserId,
        rows: Vec<Row>,
        versions: &[VersionId],
    ) -> Result<Vec<UserTableRowId>, KalamDbError> {
        self.insert_batch_with_versions(user_id, rows, versions).await
    }

    async fn update(
        &self,
        user_id: &UserId,
        key: &UserTableRowId,
        updates: Row,
        version: VersionId,
    ) -> Result<Option<UserTableRowId>, KalamDbError> {
        // Load referenced version to extract PK, then delegate to update_by_pk_value
        let prior_opt = self.store.get(key).into_kalamdb_error("Failed to load prior version")?;

        let prior = if let Some(p) = prior_opt {
            p
        } else {
            load_row_from_parquet_by_seq(
                &self.core,
                self.core.table_type(),
                self.core.schema(),
                Some(user_id),
                key.version(),
                |row_data| UserTableRow {
                    user_id:  user_id.clone(),
                    _version: row_data.version,
                    _deleted: row_data.deleted,
                    fields:   row_data.fields,
                },
            )
            .await?
            .ok_or_else(|| KalamDbError::NotFound("Row not found for update".to_string()))?
        };

        let pk_name = self.primary_key_field_name().to_string();
        let pk_value_scalar = prior.fields.get(&pk_name).cloned().ok_or_else(|| {
            KalamDbError::InvalidOperation(format!("Prior row missing PK {}", pk_name))
        })?;

        // Validate PK update (check if new PK value already exists) — only needed when updating by
        // key
        base::validate_pk_update(self, Some(user_id), &updates, &pk_value_scalar).await?;

        // Delegate to the canonical implementation
        let pk_value_str = pk_value_scalar.to_string();
        self.update_by_pk_value(user_id, &pk_value_str, updates, version).await
    }

    async fn update_by_pk_value(
        &self,
        user_id: &UserId,
        pk_value: &str,
        updates: Row,
        version: VersionId,
    ) -> Result<Option<UserTableRowId>, KalamDbError> {
        self.update_by_pk_value_with_version(user_id, pk_value, updates, version).await
    }

    async fn delete(
        &self,
        user_id: &UserId,
        key: &UserTableRowId,
        version: VersionId,
    ) -> Result<(), KalamDbError> {
        // Load referenced version to extract PK, then delegate to delete_by_pk_value
        let prior_opt = self.store.get(key).into_kalamdb_error("Failed to load prior version")?;

        let prior = if let Some(p) = prior_opt {
            p
        } else {
            load_row_from_parquet_by_seq(
                &self.core,
                self.core.table_type(),
                self.core.schema(),
                Some(user_id),
                key.version(),
                |row_data| UserTableRow {
                    user_id:  user_id.clone(),
                    _version: row_data.version,
                    _deleted: row_data.deleted,
                    fields:   row_data.fields,
                },
            )
            .await?
            .ok_or_else(|| KalamDbError::NotFound("Row not found for delete".to_string()))?
        };

        let pk_name = self.primary_key_field_name().to_string();
        let pk_value_scalar = prior.fields.get(&pk_name).cloned().ok_or_else(|| {
            KalamDbError::InvalidOperation(format!("Prior row missing PK {}", pk_name))
        })?;
        let pk_value_str = pk_value_scalar.to_string();

        self.delete_by_pk_value(user_id, &pk_value_str, version).await?;
        Ok(())
    }

    async fn delete_by_pk_value(
        &self,
        user_id: &UserId,
        pk_value: &str,
        version: VersionId,
    ) -> Result<bool, KalamDbError> {
        self.delete_by_pk_value_with_version(user_id, pk_value, version).await
    }

    async fn scan_rows(
        &self,
        state: &dyn Session,
        projection: Option<&Vec<usize>>,
        filter: Option<&Expr>,
        limit: Option<usize>,
    ) -> Result<RecordBatch, KalamDbError> {
        let scan_context = self.build_scan_context(state)?;
        self.scan_rows_with_context(&scan_context, projection, filter, limit).await
    }

    async fn scan_with_version_resolution_to_kvs_async(
        &self,
        user_id: &UserId,
        filter: Option<&Expr>,
        since_seq: Option<VersionId>,
        limit: Option<usize>,
        keep_deleted: bool,
        cold_columns: Option<&[String]>,
        snapshot_commit_seq: Option<VersionId>,
    ) -> Result<Vec<(UserTableRowId, UserTableRow)>, KalamDbError> {
        self.scan_with_version_resolution_to_kvs_result_async(
            user_id,
            filter,
            since_seq,
            limit,
            keep_deleted,
            cold_columns,
            snapshot_commit_seq,
            false,
        )
        .await
        .map(|result| result.rows)
    }

    fn extract_row(row: &UserTableRow) -> &Row {
        &row.fields
    }
}
