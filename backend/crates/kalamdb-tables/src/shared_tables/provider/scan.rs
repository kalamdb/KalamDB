#[derive(Clone)]
pub struct SharedScanContext {
    snapshot_commit_seq: Option<VersionId>,
    policies:            kalamdb_rls::BoundTablePolicies,
}

#[async_trait]
impl DeferredMvccScanProvider<SharedTableRowId, SharedTableRow> for SharedTableProvider {
    type ScanContext = SharedScanContext;

    fn scan_source_name(&self) -> &'static str {
        "shared_table_scan"
    }

    fn build_scan_context(&self, state: &dyn Session) -> Result<Self::ScanContext, KalamDbError> {
        let (user_id, role, _) = extract_full_user_context(state)?;
        let command = state
            .config()
            .options()
            .extensions
            .get::<RlsCommandContext>()
            .map(|context| context.command)
            .unwrap_or(PolicyCommand::Select);
        let policies = self.bind_policies(user_id, role, command, false)?;
        Ok(SharedScanContext {
            snapshot_commit_seq: extract_transaction_query_context(state).and_then(|context| {
                crate::utils::base::transaction_snapshot_bound(context.snapshot_commit_seq())
            }),
            policies,
        })
    }

    fn scan_snapshot_commit_seq(&self, scan_context: &Self::ScanContext) -> Option<VersionId> {
        scan_context.snapshot_commit_seq
    }

    fn allow_count_only_fast_path(&self, scan_context: &Self::ScanContext) -> bool {
        scan_context.policies.bypasses_rls()
    }

    fn requires_row_authorization(&self, scan_context: &Self::ScanContext) -> bool {
        !scan_context.policies.bypasses_rls()
    }

    fn authorization_plan_details(
        &self,
        scan_context: &Self::ScanContext,
        filter: Option<&Expr>,
    ) -> Option<String> {
        self.authorization.authorization_plan_details(self, scan_context, filter)
    }

    async fn pre_authorize_scan(
        &self,
        scan_context: &Self::ScanContext,
        filter: Option<&Expr>,
    ) -> Result<bool, KalamDbError> {
        self.authorization.pre_authorize_scan(self, scan_context, filter).await
    }

    async fn authorize_resolved_rows(
        &self,
        scan_context: &Self::ScanContext,
        rows: Vec<(SharedTableRowId, SharedTableRow)>,
    ) -> Result<Vec<(SharedTableRowId, SharedTableRow)>, KalamDbError> {
        self.authorization.authorize_resolved_rows(self, scan_context, rows).await
    }

    fn scan_scope_label(&self, _scan_context: &Self::ScanContext) -> &'static str {
        "shared"
    }

    fn scan_cold_scope<'a>(&self, _scan_context: &'a Self::ScanContext) -> Option<&'a UserId> {
        None
    }

    async fn scan_latest_hot_pk_entry(
        &self,
        _scan_context: &Self::ScanContext,
        pk_value: &ScalarValue,
        storage_ordinals: Option<&[usize]>,
    ) -> Result<Option<(SharedTableRowId, SharedTableRow)>, KalamDbError> {
        self.latest_hot_pk_entry_maybe_selected(pk_value, storage_ordinals).await
    }

    async fn count_rows_with_context(
        &self,
        scan_context: &Self::ScanContext,
    ) -> Result<usize, KalamDbError> {
        self.count_resolved_rows_async(scan_context.snapshot_commit_seq).await
    }

    async fn scan_kvs_with_context(
        &self,
        scan_context: &Self::ScanContext,
        filter: Option<&Expr>,
        since_seq: Option<VersionId>,
        limit: Option<usize>,
        keep_deleted: bool,
        cold_columns: Option<&[String]>,
    ) -> Result<Vec<(SharedTableRowId, SharedTableRow)>, KalamDbError> {
        let scan_limit = scan_context.policies.bypasses_rls().then_some(limit).flatten();
        let cold_columns = self.authorization_cold_columns(scan_context, cold_columns);
        self.scan_with_version_resolution_to_kvs_async(
            base::system_user_id(),
            filter,
            since_seq,
            scan_limit,
            keep_deleted,
            cold_columns.as_deref(),
            scan_context.snapshot_commit_seq,
        )
        .await
    }

    async fn scan_kvs_with_diagnostics(
        &self,
        scan_context: &Self::ScanContext,
        filter: Option<&Expr>,
        since_seq: Option<VersionId>,
        limit: Option<usize>,
        keep_deleted: bool,
        cold_columns: Option<&[String]>,
    ) -> Result<base::MvccScanResult<SharedTableRowId, SharedTableRow>, KalamDbError> {
        let scan_limit = scan_context.policies.bypasses_rls().then_some(limit).flatten();
        let cold_columns = self.authorization_cold_columns(scan_context, cold_columns);
        self.scan_with_version_resolution_to_kvs_result_async(
            filter,
            since_seq,
            scan_limit,
            keep_deleted,
            cold_columns.as_deref(),
            scan_context.snapshot_commit_seq,
            true,
        )
        .await
    }
}

impl SourceProvider for SharedTableProvider {
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
impl BaseTableProvider<SharedTableRowId, SharedTableRow> for SharedTableProvider {
    fn core(&self) -> &base::TableProviderCore {
        &self.core
    }

    fn construct_row_from_parquet_data(
        &self,
        _user_id: &UserId,
        row_data: &crate::utils::version_resolution::ParquetRowData,
    ) -> Result<Option<(SharedTableRowId, SharedTableRow)>, KalamDbError> {
        // Shared tables use SeqId as the key (no user_id scoping)
        let row_key = row_data.version;
        let row = SharedTableRow {
            _version: row_data.version,
            _deleted: row_data.deleted,
            fields:   row_data.fields.clone(),
        };
        Ok(Some((row_key, row)))
    }

    /// Find row by PK value using the PK index for O(1) lookup.
    ///
    /// OPTIMIZED: Uses `pk_exists_in_hot` for fast hot-path check.
    /// OPTIMIZED: Uses `pk_exists_in_cold` with manifest-based segment pruning for cold storage.
    /// For shared tables, user_id is ignored (no RLS).
    async fn find_row_key_by_id_field(
        &self,
        _user_id: &UserId,
        id_value: &str,
    ) -> Result<Option<SharedTableRowId>, KalamDbError> {
        // Use shared helper to parse PK value
        let pk_value = crate::utils::pk::parse_pk_value(id_value);

        if let Some((row_id, row)) = self.latest_hot_pk_entry(&pk_value).await? {
            if row._deleted {
                return Ok(None);
            }
            return Ok(Some(row_id));
        }

        // Not found in hot storage - check cold storage using optimized manifest-based lookup
        // This uses column_stats to prune segments that can't contain the PK
        let pk_name = self.primary_key_field_name();
        let pk_column_id = self.core.primary_key_column_id();
        let exists_in_cold = base::pk_exists_in_cold(
            &self.core,
            self.core.table_id(),
            self.core.table_type(),
            None, // No user scoping for shared tables
            pk_name,
            pk_column_id,
            id_value,
        )
        .await?;

        if exists_in_cold {
            log::trace!("[SharedTableProvider] PK {} exists in cold storage", id_value);
            // For PK uniqueness check, we just need to know it exists
            // Return None to indicate "exists but key not available synchronously"
            return Ok(None);
        }

        Ok(None)
    }

    async fn insert(
        &self,
        _user_id: &UserId,
        row_data: Row,
        version: VersionId,
    ) -> Result<SharedTableRowId, KalamDbError> {
        let span = tracing::debug_span!(
            "table.insert",
            table_id = %self.core.table_id(),
            scope = "shared",
            column_count = row_data.values.len()
        );
        async move {
            let _authorization_mutation = self.begin_authorization_mutation();
            ensure_manifest_ready(&self.core, self.core.table_type(), None, "SharedTableProvider")?;

            crate::utils::datafusion_dml::validate_not_null_with_set(
                self.core.non_null_columns(),
                std::slice::from_ref(&row_data),
            )?;

            // IGNORE user_id parameter - no RLS for shared tables
            base::ensure_unique_pk_value(self, None, &row_data).await?;

            // Generate new SeqId via SystemColumnsService
            let seq_id = version;

            // Create SharedTableRow directly
            let entity = SharedTableRow {
                _version: seq_id,
                _deleted: false,
                fields:   row_data,
            };

            // Key is just the SeqId (SharedTableRowId is type alias for SeqId)
            let row_key = seq_id;

            // Store the entity in RocksDB (hot storage) using insert() to update PK index
            self.store.insert_async(row_key, entity.clone()).await.map_err(|e| {
                KalamDbError::InvalidOperation(format!("Failed to insert shared table row: {}", e))
            })?;

            log::debug!("Inserted shared table row with _seq {}", seq_id);

            if let Err(e) = self.stage_vector_upsert(seq_id, &entity.fields).await {
                log::warn!(
                    "Failed to stage vector upsert for table={}, seq={}: {}",
                    self.core.table_id(),
                    seq_id.as_i64(),
                    e
                );
            }

            // Mark manifest as having pending writes (hot data needs to be flushed)
            let manifest_service = self.core.services.manifest_service.clone();
            if let Err(e) = manifest_service.mark_pending_write(self.core.table_id(), None) {
                log::warn!(
                    "Failed to mark manifest as pending_write for {}: {}",
                    self.core.table_id(),
                    e
                );
            }

            // Fire topic/CDC notification (INSERT). User is actor metadata only;
            // live fanout remains shared-scoped.
            let notification_service = self.core.services.notification_service.clone();
            let table_id = self.core.table_id().clone();

            let has_topics = self.core.has_topic_routes(&table_id);
            let has_live_subs = notification_service.has_subscribers(None, &table_id);
            if has_topics || has_live_subs {
                let row = Self::build_notification_row(&entity);
                if has_topics {
                    self.core
                        .publish_to_topics(
                            &table_id,
                            kalamdb_commons::models::TopicOp::Insert,
                            &row,
                            Some(_user_id),
                        )
                        .await;
                }
                if has_live_subs {
                    let notification = ChangeNotification::insert(table_id.clone(), row);
                    notification_service.notify_table_change(None, table_id, notification);
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
    /// - Batch PK validation (single scan for large batches, O(1) lookups for small batches)
    ///
    /// # Arguments
    /// * `_user_id` - Ignored for shared tables (no RLS)
    /// * `rows` - Vector of Row objects to insert
    ///
    /// # Returns
    /// Vector of generated SharedTableRowIds (SeqIds)
    async fn insert_batch(
        &self,
        _user_id: &UserId,
        rows: Vec<Row>,
        versions: &[VersionId],
    ) -> Result<Vec<SharedTableRowId>, KalamDbError> {
        self.insert_batch_with_versions(Some(_user_id), rows, versions).await
    }

    async fn update(
        &self,
        _user_id: &UserId,
        key: &SharedTableRowId,
        updates: Row,
        version: VersionId,
    ) -> Result<Option<SharedTableRowId>, KalamDbError> {
        // IGNORE user_id parameter - no RLS for shared tables
        // Extract PK from prior row, then delegate to update_by_pk_value
        let pk_name = self.primary_key_field_name().to_string();

        // Load referenced prior version to derive PK value
        let prior_opt = self.store.get(key).into_kalamdb_error("Failed to load prior version")?;

        let prior = if let Some(p) = prior_opt {
            p
        } else {
            load_row_from_parquet_by_seq(
                &self.core,
                self.core.table_type(),
                &self.core.schema_ref(),
                None,
                *key,
                |row_data| SharedTableRow {
                    _version: row_data.version,
                    _deleted: row_data.deleted,
                    fields:   row_data.fields,
                },
            )
            .await?
            .ok_or_else(|| KalamDbError::NotFound("Row not found for update".to_string()))?
        };

        let pk_value_scalar = prior.fields.get(&pk_name).cloned().ok_or_else(|| {
            KalamDbError::InvalidOperation(format!("Prior row missing PK {}", pk_name))
        })?;

        // Validate PK is not being changed to a value that already exists
        base::validate_pk_update(self, None, &updates, &pk_value_scalar).await?;

        let pk_value_str = pk_value_scalar.to_string();
        self.update_by_pk_value(_user_id, &pk_value_str, updates, version).await
    }

    async fn update_by_pk_value(
        &self,
        _user_id: &UserId,
        pk_value: &str,
        updates: Row,
        version: VersionId,
    ) -> Result<Option<SharedTableRowId>, KalamDbError> {
        self.update_by_pk_value_with_version(_user_id, pk_value, updates, version).await
    }

    async fn delete(
        &self,
        _user_id: &UserId,
        key: &SharedTableRowId,
        version: VersionId,
    ) -> Result<(), KalamDbError> {
        // IGNORE user_id parameter - no RLS for shared tables
        // Extract PK from prior row, then delegate to delete_by_pk_value
        let pk_name = self.primary_key_field_name().to_string();

        let prior_opt = self.store.get(key).into_kalamdb_error("Failed to load prior version")?;

        let prior = if let Some(p) = prior_opt {
            p
        } else {
            load_row_from_parquet_by_seq(
                &self.core,
                self.core.table_type(),
                &self.core.schema_ref(),
                None,
                *key,
                |row_data| SharedTableRow {
                    _version: row_data.version,
                    _deleted: row_data.deleted,
                    fields:   row_data.fields,
                },
            )
            .await?
            .ok_or_else(|| KalamDbError::NotFound("Row not found for delete".to_string()))?
        };

        let pk_value_scalar = prior.fields.get(&pk_name).cloned().ok_or_else(|| {
            KalamDbError::InvalidOperation(format!("Prior row missing PK {}", pk_name))
        })?;
        let pk_value_str = pk_value_scalar.to_string();

        self.delete_by_pk_value(_user_id, &pk_value_str, version).await?;
        Ok(())
    }

    async fn delete_by_pk_value(
        &self,
        _user_id: &UserId,
        pk_value: &str,
        version: VersionId,
    ) -> Result<bool, KalamDbError> {
        self.delete_by_pk_value_with_version(_user_id, pk_value, version).await
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
        _user_id: &UserId,
        filter: Option<&Expr>,
        since_seq: Option<VersionId>,
        limit: Option<usize>,
        keep_deleted: bool,
        cold_columns: Option<&[String]>,
        snapshot_commit_seq: Option<VersionId>,
    ) -> Result<Vec<(SharedTableRowId, SharedTableRow)>, KalamDbError> {
        self.scan_with_version_resolution_to_kvs_result_async(
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

    fn extract_row(row: &SharedTableRow) -> &Row {
        &row.fields
    }
}

impl SharedTableProvider {
    async fn scan_with_version_resolution_to_kvs_result_async(
        &self,
        filter: Option<&Expr>,
        since_seq: Option<VersionId>,
        limit: Option<usize>,
        keep_deleted: bool,
        cold_columns: Option<&[String]>,
        snapshot_commit_seq: Option<VersionId>,
        include_diagnostics: bool,
    ) -> Result<base::MvccScanResult<SharedTableRowId, SharedTableRow>, KalamDbError> {
        use kalamdb_store::EntityStoreAsync;

        base::warn_if_unfiltered_scan(self.core.table_id(), filter, limit, self.core.table_type());

        let start_key =
            since_seq.and_then(|seq| VersionId::try_from_i64(seq.as_i64().saturating_add(1)).ok());
        let scan_limit = base::calculate_scan_limit(limit);
        let pk_name = self.primary_key_field_name();
        let hot_future = async {
            if since_seq.is_none() {
                if let Some(plan) = base::hot_index_seek(self.store.as_ref(), filter, None) {
                    return base::scan_hot_index_resolved(
                        &self.store,
                        plan,
                        scan_limit,
                        pk_name,
                        None,
                    )
                    .await;
                }
            }
            self.store
                .scan_typed_with_prefix_and_start_async(None, start_key.as_ref(), scan_limit)
                .await
                .map_err(|e| {
                    KalamDbError::InvalidOperation(format!(
                        "Failed to scan shared table hot storage: {}",
                        e
                    ))
                })
        };
        let cold_future = async {
            if include_diagnostics {
                self.scan_parquet_files_with_stats_async(filter, cold_columns).await
            } else {
                self.scan_parquet_files_as_result_async(filter, cold_columns).await
            }
        };
        let resolved = base::resolve_latest_scan_from_futures(
            pk_name,
            limit,
            keep_deleted,
            snapshot_commit_seq,
            hot_future,
            cold_future,
            |row_data| self.construct_shared_row_from_parquet_data(row_data),
        )
        .await?;

        log::trace!("[SharedProvider] RocksDB scan returned {} rows", resolved.hot_rows_scanned);
        log::trace!(
            "[SharedProvider] Cold scan returned {} Parquet rows",
            resolved.cold_rows_scanned
        );

        log::trace!(
            "[SharedProvider] Version-resolved rows (post-tombstone filter): {}",
            resolved.rows.len()
        );
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

        Ok(base::MvccScanResult {
            diagnostics,
            rows: resolved.rows,
        })
    }
}
