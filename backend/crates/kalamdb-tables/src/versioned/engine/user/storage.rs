impl UserTableProvider {
    /// Create a new user table provider
    ///
    /// # Arguments
    /// * `core` - Shared core with services, schema, pk_name, etc.
    /// * `store` - IndexedEntityStore with PK index for this table
    pub fn new(core: Arc<TableProviderCore>, store: Arc<UserTableIndexedStore>) -> Self {
        let pk_index = new_user_pk_index(core.table_id(), core.primary_key_field_name());
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

    pub async fn validate_insert_batch_rows<'a, I>(
        &self,
        user_id: &UserId,
        rows: I,
    ) -> Result<HashMap<String, (UserTableRowId, UserTableRow)>, KalamDbError>
    where
        I: IntoIterator<Item = &'a Row>,
    {
        let pk_name = self.primary_key_field_name();
        let pk_values = crate::versioned::batch_primary_keys(pk_name, rows)?;
        if pk_values.is_empty() {
            return Ok(HashMap::new());
        }
        let prefixes = pk_values
            .iter()
            .map(|(pk_str, pk_value)| {
                (pk_str.clone(), self.pk_index.build_user_pk_prefix(user_id, pk_value))
            })
            .collect();
        crate::versioned::tombstones_replacing_insert_keys(
            Arc::clone(&self.store),
            &self.core,
            Some(user_id),
            pk_name,
            prefixes,
            Some("hot"),
            Some("cold"),
        )
        .await
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

        ensure_manifest_ready(&self.core, Some(user_id), "UserTableProvider")?;

        let coerced_rows = crate::versioned::coerce_row_batch(rows, &self.schema_ref())?;

        crate::utils::datafusion_dml::validate_not_null_with_set(
            self.core.non_null_columns(),
            &coerced_rows,
        )?;

        let row_count = coerced_rows.len();

        let replaced_tombstones = if validate_unique_pk {
            self.validate_insert_batch_rows(user_id, coerced_rows.iter()).await?
        } else {
            HashMap::new()
        };

        crate::versioned::require_version_count(versions.len(), row_count)?;

        let mut entries: Vec<(UserTableRowId, UserTableRow)> = Vec::with_capacity(row_count);

        for (row_data, version) in coerced_rows.into_iter().zip(versions.iter().copied()) {
            let row_key = UserTableRowId::new(user_id.clone(), version);
            entries.push((
                row_key,
                UserTableRow {
                    user_id: user_id.clone(),
                    _version: version,
                    _deleted: false,
                    fields: row_data,
                },
            ));
        }

        crate::versioned::write_insert_batch(
            &self.store,
            &entries,
            replaced_tombstones,
            self.primary_key_field_name(),
            |entity| &entity.fields,
        )?;

        if let Err(error) = self.stage_vector_upsert_batch(user_id, &entries).await {
            crate::versioned::warn_vector_upsert_batch(self.core.table_id(), Some(user_id), &error);
        }

        crate::versioned::mark_table_pending_write(&self.core, Some(user_id));

        log::debug!(
            "Batch inserted {} user table rows for user {} with _seq range [{}, {}]",
            row_count,
            user_id.as_str(),
            entries.first().map(|(k, _)| k.version.as_i64()).unwrap_or(0),
            entries.last().map(|(k, _)| k.version.as_i64()).unwrap_or(0)
        );

        Ok(entries)
    }

    /// Apply Raft ordinal field payloads without decoding them back into name maps
    /// before the storage encode.
    pub async fn insert_encoded_fields_with_versions(
        &self,
        user_id: &UserId,
        payloads: &[Vec<u8>],
        versions: &[VersionId],
    ) -> Result<usize, KalamDbError> {
        if payloads.is_empty() {
            return Ok(0);
        }
        if crate::versioned::table_has_file_columns(self.core.table_def()) {
            let rows = crate::versioned::rows_from_field_payloads(
                payloads,
                self.core.storage_schema().as_ref(),
            )?;
            return Ok(self.insert_batch_with_versions(user_id, rows, versions).await?.len());
        }

        let entries = self.persist_encoded_field_rows(user_id, payloads, versions).await?;
        let row_count = entries.len();
        crate::versioned::fanout_insert_notifications(
            &self.core,
            Some(user_id),
            Some(user_id),
            || entries.iter().map(|(_, entity)| Self::build_notification_row(entity)).collect(),
        )
        .await;
        Ok(row_count)
    }

    async fn persist_encoded_field_rows(
        &self,
        user_id: &UserId,
        payloads: &[Vec<u8>],
        versions: &[VersionId],
    ) -> Result<Vec<(UserTableRowId, UserTableRow)>, KalamDbError> {
        ensure_manifest_ready(&self.core, Some(user_id), "UserTableProvider")?;
        let storage = Arc::clone(self.core.storage_schema());
        let arrow = self.schema_ref();
        let decoded = crate::versioned::decode_ordinal_insert(
            payloads,
            storage.as_ref(),
            &arrow,
            self.core.non_null_columns(),
            versions,
        )?;
        let names = decoded.names;
        let decoded = decoded.columns;
        let pk_name = self.primary_key_field_name();
        let pk_ordinal = storage.fields.iter().position(|field| field.name == pk_name);
        let pk_rows: Vec<Row> = decoded
            .iter()
            .map(|columns| {
                let mut values = std::collections::BTreeMap::new();
                if let Some(ordinal) = pk_ordinal {
                    if let Some(value) = columns.get(ordinal) {
                        values.insert(pk_name.to_string(), value.clone());
                    }
                }
                Row::new(values)
            })
            .collect();
        let replaced_tombstones = self.validate_insert_batch_rows(user_id, pk_rows.iter()).await?;
        if !replaced_tombstones.is_empty() {
            let rows =
                decoded.into_iter().map(|columns| Row::from_aligned(&names, columns)).collect();
            return self.persist_insert_batch_rows(user_id, rows, versions, false).await;
        }

        let mut entries = Vec::with_capacity(decoded.len());
        let mut encoded_values = Vec::with_capacity(decoded.len());
        for (columns, version) in decoded.into_iter().zip(versions.iter().copied()) {
            let bytes = kalamdb_serialization::encode_user_row_from_columns(
                false,
                &columns,
                storage.as_ref(),
            )
            .map_err(|error| {
                KalamDbError::InvalidOperation(format!("ordinal insert encode failed: {error}"))
            })?
            .into_bytes();
            encoded_values.push(bytes);
            entries.push((
                UserTableRowId::new(user_id.clone(), version),
                UserTableRow {
                    user_id: user_id.clone(),
                    _version: version,
                    _deleted: false,
                    fields: Row::from_aligned(&names, columns),
                },
            ));
        }
        self.store.insert_batch_preencoded(&entries, encoded_values).map_err(|error| {
            KalamDbError::InvalidOperation(format!("Failed to batch insert table rows: {error}"))
        })?;
        if let Err(error) = self.stage_vector_upsert_batch(user_id, &entries).await {
            crate::versioned::warn_vector_upsert_batch(self.core.table_id(), Some(user_id), &error);
        }
        crate::versioned::mark_table_pending_write(&self.core, Some(user_id));
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
        let prefix = self.pk_index.build_user_pk_prefix(user_id, pk_value);
        crate::versioned::latest_hot_by_prefix(&self.store, prefix, storage_ordinals).await
    }

    async fn hot_pk_lookup(
        &self,
        user_id: &UserId,
        pk_value: &ScalarValue,
    ) -> Result<crate::versioned::HotPkLookup<UserTableRowId, UserTableRow>, KalamDbError> {
        Ok(crate::versioned::classify_hot_pk(
            self.latest_hot_pk_entry(user_id, pk_value).await?,
            |row| row._deleted,
        ))
    }

    pub(crate) async fn load_mutation_target(
        &self,
        user_id: &UserId,
        pk_value: &str,
    ) -> Result<crate::versioned::MutationTarget<UserTableRowId, UserTableRow>, KalamDbError> {
        let pk_name = self.primary_key_field_name();
        let pk_scalar = crate::versioned::pk_text_to_scalar(&self.schema_ref(), pk_name, pk_value)?;
        let lookup = self.hot_pk_lookup(user_id, &pk_scalar).await?;
        let cold = if matches!(lookup, crate::versioned::HotPkLookup::Missing) {
            base::find_row_by_pk(self, Some(user_id), pk_value).await?.map(|(_, row)| row)
        } else {
            None
        };
        Ok(crate::versioned::mutation_target_from_hot(lookup, cold))
    }

    async fn insert_predecessor(
        &self,
        user_id: &UserId,
        row_data: &Row,
    ) -> Result<Option<(UserTableRowId, UserTableRow)>, KalamDbError> {
        let pk_name = self.primary_key_field_name();
        let Some((pk_value, pk_str)) =
            crate::versioned::insert_pk_parts(self.core.is_auto_increment_pk(), row_data, pk_name)?
        else {
            return Ok(None);
        };
        let prefix = self.pk_index.build_user_pk_prefix(user_id, pk_value);
        match crate::versioned::hot_insert_predecessor(&self.store, &prefix, &pk_str, pk_name)? {
            crate::versioned::InsertPredecessor::Tombstone(key, row) => Ok(Some((key, row))),
            crate::versioned::InsertPredecessor::Missing => {
                base::ensure_pk_absent_in_cold(self, Some(user_id), &pk_str).await?;
                Ok(None)
            },
        }
    }

    async fn write_replaced_row(
        &self,
        previous: Option<(UserTableRowId, UserTableRow)>,
        row_key: UserTableRowId,
        entity: UserTableRow,
        error_context: &str,
    ) -> Result<(), KalamDbError> {
        crate::versioned::write_replaced_row(&self.store, previous, row_key, entity, error_context)
            .await
    }

    pub async fn find_by_pk(
        &self,
        user_id: &UserId,
        pk_value: &ScalarValue,
    ) -> Result<Option<(UserTableRowId, UserTableRow)>, KalamDbError> {
        Ok(crate::versioned::live_indexed_row(
            self.latest_hot_pk_entry(user_id, pk_value).await?,
            |row| row._deleted,
        ))
    }

    fn construct_mvcc_row_from_parquet_data(
        &self,
        user_id: &UserId,
        row_data: crate::utils::version_resolution::ParquetRowData,
    ) -> DataFusionResult<(UserTableRowId, UserTableRow)> {
        self.construct_row_from_parquet_data(user_id, &row_data)
            .map_err(crate::error::into_datafusion)?
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
        base::reject_truncated_unbounded_scan(limit, hot_rows.len())?;

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
                hot_rows_scanned: Some(hot_rows_scanned),
                cold_rows_scanned: Some(0),
                cold_files_total: Some(0),
                cold_files_skipped: Some(0),
                cold_files_scanned: Some(0),
                cold_files: Vec::new(),
            }
        } else {
            DeferredScanDiagnostics::default()
        };

        for user_id in user_ids {
            let cold_result = if include_diagnostics {
                base::scan_provider_parquet_stats(&self.core, Some(&user_id), filter, None).await?
            } else {
                base::scan_provider_parquet_result(&self.core, Some(&user_id), filter, None).await?
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
            let resolved = resolve_latest_kvs_from_cold_batch(
                &pk_name,
                hot_rows,
                &cold_result.batch,
                keep_deleted,
                snapshot_commit_seq,
                |row_data| self.construct_mvcc_row_from_parquet_data(&user_id, row_data),
            )
            .map_err(|error| KalamDbError::DataFusion(error.to_string()))?;
            result.extend(resolved);
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
