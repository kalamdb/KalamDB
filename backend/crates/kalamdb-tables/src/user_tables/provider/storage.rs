impl UserTableProvider {
    /// Create a new user table provider
    ///
    /// # Arguments
    /// * `core` - Shared core with services, schema, pk_name, etc.
    /// * `store` - IndexedEntityStore with PK index for this table
    pub fn new(core: Arc<TableProviderCore>, store: Arc<UserTableIndexedStore>) -> Self {
        let pk_index = UserTablePkIndex::new(core.table_id(), core.primary_key_field_name());
        let vector_columns = base::embedding_columns(core.table_def());
        let vector_stores = crate::utils::vector_staging::build_vector_store_map(
            store.backend().clone(),
            core.table_id(),
            &vector_columns,
            new_indexed_user_vector_hot_store,
        );

        if log::log_enabled!(log::Level::Debug) {
            let field_names: Vec<_> = core.schema().fields().iter().map(|f| f.name()).collect();
            log::debug!(
                "UserTableProvider: Created for {} with schema fields: {:?}",
                core.table_id(),
                field_names
            );
        }

        Self {
            core,
            store,
            pk_index,
            vector_columns,
            vector_stores,
        }
    }

    /// Get the primary key field name
    pub fn primary_key_field_name(&self) -> &str {
        self.core.primary_key_field_name()
    }

    /// Access the underlying indexed store (used by flush jobs)
    pub fn store(&self) -> Arc<UserTableIndexedStore> {
        Arc::clone(&self.store)
    }

    async fn stage_vector_upsert(
        &self,
        user_id: &UserId,
        version: VersionId,
        row: &Row,
    ) -> Result<(), KalamDbError> {
        if self.vector_columns.is_empty() {
            return Ok(());
        }

        let seq = SeqId::from_i64(version.as_i64());
        let ops_by_column = crate::utils::vector_staging::build_vector_upsert_batch_ops(
            self.core.table_id(),
            self.primary_key_field_name(),
            &self.vector_columns,
            std::iter::once((seq, row)),
            |(_, row)| row,
            |(seq, _), pk| UserVectorHotOpId::new(user_id.clone(), *seq, pk.to_string()),
        )?;
        crate::utils::vector_staging::stage_vector_ops_by_column(
            &self.vector_stores,
            ops_by_column,
            "stage vector upsert",
        )
        .await
    }

    async fn stage_vector_upsert_batch(
        &self,
        user_id: &UserId,
        entries: &[(UserTableRowId, UserTableRow)],
    ) -> Result<(), KalamDbError> {
        if self.vector_columns.is_empty() || entries.is_empty() {
            return Ok(());
        }

        let ops_by_column = crate::utils::vector_staging::build_vector_upsert_batch_ops(
            self.core.table_id(),
            self.primary_key_field_name(),
            &self.vector_columns,
            entries.iter(),
            |(_, entity)| &entity.fields,
            |(row_key, _), pk| {
                UserVectorHotOpId::new(
                    user_id.clone(),
                    SeqId::from_i64(row_key.version().as_i64()),
                    pk.to_string(),
                )
            },
        )?;
        crate::utils::vector_staging::stage_vector_ops_by_column(
            &self.vector_stores,
            ops_by_column,
            "batch stage vector upsert",
        )
        .await
    }

    async fn stage_vector_delete(
        &self,
        user_id: &UserId,
        version: VersionId,
        pk: &str,
    ) -> Result<(), KalamDbError> {
        if self.vector_columns.is_empty() {
            return Ok(());
        }

        let seq = SeqId::from_i64(version.as_i64());
        let ops_by_column = crate::utils::vector_staging::build_vector_delete_ops(
            self.core.table_id(),
            &self.vector_columns,
            pk,
            |primary_key| UserVectorHotOpId::new(user_id.clone(), seq, primary_key.to_string()),
        );
        crate::utils::vector_staging::stage_vector_ops_by_column(
            &self.vector_stores,
            ops_by_column,
            "stage vector delete",
        )
        .await
    }

    async fn append_hot_row(
        &self,
        row_key: &UserTableRowId,
        entity: &UserTableRow,
        error_context: &str,
    ) -> Result<(), KalamDbError> {
        let store = self.store.clone();
        let row_key = row_key.clone();
        let entity = entity.clone();
        let error_context = error_context.to_string();

        tokio::task::spawn_blocking(move || -> Result<(), KalamDbError> {
            store
                .insert(&row_key, &entity)
                .map_err(|e| KalamDbError::InvalidOperation(format!("{}: {}", error_context, e)))
        })
        .await
        .map_err(|e| KalamDbError::InvalidOperation(format!("spawn_blocking error: {}", e)))??;

        Ok(())
    }

    pub async fn validate_insert_batch_rows<'a, I>(
        &self,
        user_id: &UserId,
        rows: I,
    ) -> Result<(), KalamDbError>
    where
        I: IntoIterator<Item = &'a Row>,
    {
        let pk_name = self.primary_key_field_name();
        let rows = rows.into_iter();
        let (lower_bound, upper_bound) = rows.size_hint();
        let capacity = upper_bound.unwrap_or(lower_bound);
        let mut pk_values_to_check: Vec<(String, ScalarValue)> = Vec::with_capacity(capacity);
        let mut seen_batch_pks = HashSet::with_capacity(capacity);

        for row_data in rows {
            if let Some(pk_value) = row_data.get(pk_name) {
                if !matches!(pk_value, ScalarValue::Null) {
                    let pk_str =
                        crate::utils::unified_dml::extract_user_pk_value(row_data, pk_name)?;
                    if !seen_batch_pks.insert(pk_str.clone()) {
                        return Err(KalamDbError::AlreadyExists(format!(
                            "Primary key violation: value '{}' appears multiple times in the \
                             insert batch for column '{}'",
                            pk_str, pk_name
                        )));
                    }
                    pk_values_to_check.push((pk_str, pk_value.clone()));
                }
            }
        }

        if pk_values_to_check.is_empty() {
            return Ok(());
        }

        let mut pk_prefixes: Vec<(String, Vec<u8>)> = Vec::with_capacity(pk_values_to_check.len());
        for (pk_str, pk_value) in &pk_values_to_check {
            pk_prefixes
                .push((pk_str.clone(), self.pk_index.build_prefix_for_pk(user_id, pk_value)));
        }

        let store = self.store.clone();
        let (hot_duplicate, tombstoned_pks) = if pk_prefixes.len() <= 1 {
            scan_user_hot_pk_insert(&store, &pk_prefixes)?
        } else {
            tokio::task::spawn_blocking(
                move || -> Result<(Option<String>, HashSet<String>), KalamDbError> {
                    scan_user_hot_pk_insert(&store, &pk_prefixes)
                },
            )
            .await
            .map_err(|e| KalamDbError::InvalidOperation(format!("spawn_blocking error: {}", e)))??
        };

        if let Some(dup_pk) = hot_duplicate {
            return Err(KalamDbError::AlreadyExists(format!(
                "Primary key violation: value '{}' already exists in column '{}' (hot)",
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
                Some(user_id),
                pk_name,
                pk_column_id,
                &pk_values_for_cold_check,
            )
            .await?
            {
                return Err(KalamDbError::AlreadyExists(format!(
                    "Primary key violation: value '{}' already exists in column '{}' (cold)",
                    found_pk, pk_name
                )));
            }
        }

        Ok(())
    }

    async fn persist_insert_batch_rows(
        &self,
        user_id: &UserId,
        rows: Vec<Row>,
        versions: &[VersionId],
        validate_unique_pk: bool,
    ) -> Result<Vec<(UserTableRowId, UserTableRow)>, KalamDbError> {
        if rows.is_empty() {
            return Ok(Vec::new());
        }

        ensure_manifest_ready(
            &self.core,
            self.core.table_type(),
            Some(user_id),
            "UserTableProvider",
        )?;

        let coerced_rows = coerce_rows(rows, &self.schema_ref()).map_err(|e| {
            KalamDbError::InvalidOperation(format!("Schema coercion failed: {}", e))
        })?;

        crate::utils::datafusion_dml::validate_not_null_with_set(
            self.core.non_null_columns(),
            &coerced_rows,
        )
        .map_err(|e| KalamDbError::ConstraintViolation(e.to_string()))?;

        let row_count = coerced_rows.len();

        if validate_unique_pk {
            self.validate_insert_batch_rows(user_id, coerced_rows.iter()).await?;
        }

        if versions.len() != row_count {
            return Err(KalamDbError::InvalidOperation(format!(
                "version count {} does not match row count {}",
                versions.len(),
                row_count
            )));
        }

        let mut entries: Vec<(UserTableRowId, UserTableRow)> = Vec::with_capacity(row_count);

        for (row_data, version) in coerced_rows.into_iter().zip(versions.iter().copied()) {
            let row_key = UserTableRowId::new(user_id.clone(), version);
            entries.push((
                row_key,
                UserTableRow {
                    user_id:  user_id.clone(),
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

        if let Err(e) = self.stage_vector_upsert_batch(user_id, &entries).await {
            log::warn!(
                "Failed to batch stage vector upserts for table={}, user={}: {}",
                self.core.table_id(),
                user_id.as_str(),
                e
            );
        }

        let manifest_service = self.core.services.manifest_service.clone();
        if let Err(e) = manifest_service.mark_pending_write(self.core.table_id(), Some(user_id)) {
            log::warn!(
                "Failed to mark manifest as pending_write for {}: {}",
                self.core.table_id(),
                e
            );
        }

        log::debug!(
            "Batch inserted {} user table rows for user {} with _seq range [{}, {}]",
            row_count,
            user_id.as_str(),
            entries.first().map(|(k, _)| k.version.as_i64()).unwrap_or(0),
            entries.last().map(|(k, _)| k.version.as_i64()).unwrap_or(0)
        );

        Ok(entries)
    }

    /// Build a complete Row from UserTableRow including system columns (_seq, _deleted)
    ///
    /// This ensures live query notifications include all columns, not just user-defined fields.
    fn build_notification_row(entity: &UserTableRow) -> Row {
        base::build_notification_row(&entity.fields, entity._version, entity._deleted)
    }

    /// Find a row by primary key value using the PK index
    ///
    /// Returns the latest non-deleted version of the row with the given PK.
    /// This is more efficient than scanning all rows.
    ///
    /// # Arguments
    /// * `user_id` - User scope for RLS
    /// * `pk_value` - Primary key value to search for
    ///
    /// # Returns
    /// Option<(UserTableRowId, UserTableRow)> if found
    async fn latest_hot_pk_entry(
        &self,
        user_id: &UserId,
        pk_value: &ScalarValue,
    ) -> Result<Option<(UserTableRowId, UserTableRow)>, KalamDbError> {
        self.latest_hot_pk_entry_maybe_selected(user_id, pk_value, None).await
    }

    async fn latest_hot_pk_entry_maybe_selected(
        &self,
        user_id: &UserId,
        pk_value: &ScalarValue,
        storage_ordinals: Option<&[usize]>,
    ) -> Result<Option<(UserTableRowId, UserTableRow)>, KalamDbError> {
        let prefix = self.pk_index.build_prefix_for_pk(user_id, pk_value);
        match storage_ordinals {
            Some(ordinals) => self
                .store
                .get_latest_by_index_prefix_selected_async(0, prefix, ordinals.to_vec())
                .await
                .into_kalamdb_error("PK index scan failed"),
            None => self
                .store
                .get_latest_by_index_prefix_async(0, prefix)
                .await
                .into_kalamdb_error("PK index scan failed"),
        }
    }

    pub async fn find_by_pk(
        &self,
        user_id: &UserId,
        pk_value: &ScalarValue,
    ) -> Result<Option<(UserTableRowId, UserTableRow)>, KalamDbError> {
        Ok(self.latest_hot_pk_entry(user_id, pk_value).await?.and_then(|(row_id, row)| {
            if row._deleted {
                None
            } else {
                Some((row_id, row))
            }
        }))
    }

    pub async fn patch_commit_seq_for_row_key(
        &self,
        row_key: &UserTableRowId,
        version: VersionId,
    ) -> Result<(), KalamDbError> {
        let mut row = self
            .store
            .get(row_key)
            .into_kalamdb_error("Failed to load row for commit_seq patch")?
            .ok_or_else(|| {
                KalamDbError::NotFound(format!(
                    "row '{}' not found while patching commit_seq",
                    row_key.version()
                ))
            })?;
        let _ = version; // version is assigned at insert; patch is a no-op
        self.store.insert_async(row_key.clone(), row).await.map_err(|e| {
            KalamDbError::InvalidOperation(format!("Failed to patch commit_seq: {}", e))
        })
    }

    pub async fn patch_latest_commit_seq_by_pk(
        &self,
        user_id: &UserId,
        pk_value: &str,
        version: VersionId,
    ) -> Result<bool, KalamDbError> {
        let schema = self.schema_ref();
        let pk_field = schema.field_with_name(self.primary_key_field_name()).map_err(|e| {
            KalamDbError::InvalidOperation(format!("PK column lookup failed: {}", e))
        })?;
        let pk_scalar =
            kalamdb_commons::conversions::parse_string_as_scalar(pk_value, pk_field.data_type())
                .map_err(KalamDbError::InvalidOperation)?;

        let Some((row_key, _)) = self.latest_hot_pk_entry(user_id, &pk_scalar).await? else {
            return Ok(false);
        };

        self.patch_commit_seq_for_row_key(&row_key, version).await?;
        Ok(true)
    }

    /// Returns true if the latest hot-storage version of this PK is a tombstone
    /// (`_deleted = true`).  Returns false if the PK is absent from hot storage
    /// or if the latest version is active.
    ///
    /// Used in the PK fast-path of `scan_rows` to prevent cold storage (Parquet)
    /// from surfacing a row that has already been deleted in hot storage.
    async fn pk_tombstoned_in_hot(
        &self,
        user_id: &UserId,
        pk_value: &ScalarValue,
    ) -> Result<bool, KalamDbError> {
        Ok(self
            .latest_hot_pk_entry(user_id, pk_value)
            .await?
            .map(|(_, row)| row._deleted)
            .unwrap_or(false))
    }

    /// Scan Parquet files from cold storage for a specific user (async version).
    ///
    /// Lists all *.parquet files in the user's storage directory and merges them into a single
    /// RecordBatch. Returns an empty batch if no Parquet files exist.
    ///
    /// **Phase 4 (US6, T082-T084)**: Integrated with ManifestService for manifest caching.
    /// Logs cache hits/misses and updates last_accessed timestamp. Full query optimization
    /// (batch file pruning based on manifest metadata) implemented in Phase 5 (US2, T119-T123).
    async fn scan_parquet_files_as_batch_async(
        &self,
        user_id: &UserId,
        filter: Option<&Expr>,
        columns: Option<&[String]>,
    ) -> Result<RecordBatch, KalamDbError> {
        base::scan_parquet_files_as_batch_async(
            &self.core,
            self.core.table_id(),
            self.core.table_type(),
            Some(user_id),
            self.schema_ref(),
            filter,
            columns,
        )
        .await
    }

    async fn scan_parquet_files_with_stats_async(
        &self,
        user_id: &UserId,
        filter: Option<&Expr>,
        columns: Option<&[String]>,
    ) -> Result<base::ParquetScanResult, KalamDbError> {
        base::scan_parquet_files_with_stats_async(
            &self.core,
            self.core.table_id(),
            self.core.table_type(),
            Some(user_id),
            self.schema_ref(),
            filter,
            columns,
        )
        .await
    }

    async fn scan_parquet_files_as_result_async(
        &self,
        user_id: &UserId,
        filter: Option<&Expr>,
        columns: Option<&[String]>,
    ) -> Result<base::ParquetScanResult, KalamDbError> {
        base::scan_parquet_files_as_result_async(
            &self.core,
            self.core.table_id(),
            self.core.table_type(),
            Some(user_id),
            self.schema_ref(),
            filter,
            columns,
        )
        .await
    }

    fn construct_mvcc_row_from_parquet_data(
        &self,
        user_id: &UserId,
        row_data: crate::utils::version_resolution::ParquetRowData,
    ) -> DataFusionResult<(UserTableRowId, UserMvccRow)> {
        self.construct_row_from_parquet_data(user_id, &row_data)
            .map_err(|error| DataFusionError::Execution(error.to_string()))?
            .map(|(row_id, row)| (row_id, UserMvccRow(row)))
            .ok_or_else(|| {
                DataFusionError::Execution("missing user row from parquet data".to_string())
            })
    }

    /// Async version of scan_all_users_with_version_resolution to avoid blocking the async runtime.
    async fn scan_all_users_with_version_resolution_async(
        &self,
        filter: Option<&Expr>,
        limit: Option<usize>,
        keep_deleted: bool,
        snapshot_commit_seq: Option<VersionId>,
        fallback_user_id: Option<&UserId>,
        include_diagnostics: bool,
    ) -> Result<base::MvccScanResult<UserTableRowId, UserTableRow>, KalamDbError> {
        use kalamdb_store::EntityStoreAsync;

        let table_id = self.core.table_id();
        base::warn_if_unfiltered_scan(table_id, filter, limit, self.core.table_type());

        let scan_limit = base::calculate_scan_limit(limit);
        // Use async version to avoid blocking the runtime
        let hot_rows = self
            .store
            .scan_typed_with_prefix_and_start_async(None, None, scan_limit)
            .await
            .map_err(|e| {
                KalamDbError::InvalidOperation(format!(
                    "Failed to scan user table hot storage: {}",
                    e
                ))
            })?;

        let hot_rows_scanned = hot_rows.len();
        let mut hot_rows_by_user: HashMap<UserId, Vec<(UserTableRowId, UserTableRow)>> =
            HashMap::with_capacity(hot_rows_scanned.min(64));
        for (row_id, row) in hot_rows {
            hot_rows_by_user.entry(row.user_id.clone()).or_default().push((row_id, row));
        }
        let mut user_ids: HashSet<UserId> = hot_rows_by_user.keys().cloned().collect();

        if let Ok(scopes) = self.core.services.manifest_service.get_manifest_user_ids(table_id) {
            user_ids.extend(scopes);
        }

        if let Some(user_id) = fallback_user_id {
            user_ids.insert(user_id.clone());
        }

        let pk_name = self.primary_key_field_name().to_string();
        let mut result = Vec::new();
        let mut diagnostics = if include_diagnostics {
            DeferredScanDiagnostics {
                hot_rows_scanned:   Some(hot_rows_scanned),
                cold_rows_scanned:  Some(0),
                cold_files_total:   Some(0),
                cold_files_skipped: Some(0),
                cold_files_scanned: Some(0),
                cold_files:         Vec::new(),
            }
        } else {
            DeferredScanDiagnostics::default()
        };

        for user_id in user_ids {
            let cold_result = if include_diagnostics {
                self.scan_parquet_files_with_stats_async(&user_id, filter, None).await?
            } else {
                self.scan_parquet_files_as_result_async(&user_id, filter, None).await?
            };
            if include_diagnostics {
                diagnostics.cold_rows_scanned =
                    Some(diagnostics.cold_rows_scanned.unwrap_or(0) + cold_result.batch.num_rows());
                diagnostics.cold_files_total =
                    Some(diagnostics.cold_files_total.unwrap_or(0) + cold_result.stats.total_files);
                diagnostics.cold_files_skipped = Some(
                    diagnostics.cold_files_skipped.unwrap_or(0) + cold_result.stats.skipped_files,
                );
                diagnostics.cold_files_scanned = Some(
                    diagnostics.cold_files_scanned.unwrap_or(0) + cold_result.stats.scanned_files,
                );
                diagnostics.cold_files.extend(cold_result.stats.visited_files);
            }
            let hot_rows = hot_rows_by_user.remove(&user_id).unwrap_or_default();
            let resolved: Vec<(UserTableRowId, UserMvccRow)> = resolve_latest_kvs_from_cold_batch(
                &pk_name,
                hot_rows.into_iter().map(|(row_id, row)| (row_id, UserMvccRow(row))),
                &cold_result.batch,
                keep_deleted,
                snapshot_commit_seq,
                |row_data| self.construct_mvcc_row_from_parquet_data(&user_id, row_data),
            )
            .map_err(|error| KalamDbError::DataFusion(error.to_string()))?;
            result.extend(resolved.into_iter().map(|(row_id, row)| (row_id, row.0)));
        }

        base::apply_limit(&mut result, limit);

        Ok(base::MvccScanResult {
            rows: result,
            diagnostics,
        })
    }

    async fn collect_matching_rows_for_subject(
        &self,
        state: &dyn Session,
        filters: &[Expr],
        projection: Option<&Vec<usize>>,
    ) -> DataFusionResult<Vec<Row>> {
        crate::utils::datafusion_dml::collect_matching_rows_with_projection(
            self, state, filters, projection,
        )
        .await
    }
}
