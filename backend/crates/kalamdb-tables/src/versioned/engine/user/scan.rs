#[derive(Clone)]
pub struct UserScanContext {
    user_id: UserId,
    allow_all_users: bool,
    snapshot_commit_seq: Option<VersionId>,
}

impl UserTableProvider {
    pub async fn sum_hot_covering_integers(
        &self,
        user_id: &UserId,
        filters: &[Expr],
        value_column: &str,
    ) -> Result<Option<(u64, i128)>, KalamDbError> {
        crate::versioned::covering::sum_hot_covering_integers(
            &self.core,
            &self.store,
            Some(user_id),
            filters,
            value_column,
        )
        .await
    }

    pub async fn plan_literal_equality_updates(
        &self,
        transaction_id: &kalamdb_commons::TransactionId,
        user_id: &UserId,
        snapshot_commit_seq: Option<VersionId>,
        overlay: Option<&kalamdb_transactions::TransactionOverlay>,
        seek_column: &str,
        seek_value: &ScalarValue,
        assignments: &[(String, ScalarValue)],
    ) -> Result<Option<Vec<kalamdb_transactions::StagedMutation>>, KalamDbError> {
        crate::versioned::equality::plan_literal_equality_updates(
            &self.core,
            &self.store,
            &self.vector_columns,
            kalamdb_commons::TableType::User,
            Some(user_id),
            transaction_id,
            user_id,
            snapshot_commit_seq,
            overlay,
            seek_column,
            seek_value,
            assignments,
        )
        .await
    }

    pub async fn columnar_hot_scan(
        &self,
        user_id: &UserId,
        snapshot: Option<VersionId>,
        projection: Option<&Vec<usize>>,
        filter: Option<&Expr>,
        limit: Option<usize>,
        keep_deleted: bool,
    ) -> Result<Option<(RecordBatch, usize)>, KalamDbError> {
        crate::versioned::columnar::run_columnar_hot_scan(
            &self.core,
            &self.store,
            Some(user_id),
            snapshot,
            projection,
            filter,
            limit,
            keep_deleted,
            crate::versioned::columnar::decode_user_version,
        )
        .await
    }
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
            user_id: user_id.clone(),
            allow_all_users: can_read_all_users(role),
            snapshot_commit_seq: extract_transaction_query_context(state).and_then(|context| {
                crate::utils::base::transaction_snapshot_bound(context.snapshot_commit_seq())
            }),
        })
    }

    fn scan_snapshot_commit_seq(&self, scan_context: &Self::ScanContext) -> Option<VersionId> {
        scan_context.snapshot_commit_seq
    }

    fn can_try_pk_fast_path(&self, scan_context: &Self::ScanContext) -> bool {
        !scan_context.allow_all_users
    }

    fn allow_count_only_fast_path(&self, scan_context: &Self::ScanContext) -> bool {
        !scan_context.allow_all_users
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

    async fn try_columnar_hot_scan(
        &self,
        scan_context: &Self::ScanContext,
        projection: Option<&Vec<usize>>,
        filter: Option<&Expr>,
        limit: Option<usize>,
        keep_deleted: bool,
    ) -> Result<Option<(RecordBatch, usize)>, KalamDbError> {
        if scan_context.allow_all_users {
            return Ok(None);
        }
        crate::versioned::columnar::run_columnar_hot_scan(
            &self.core,
            &self.store,
            Some(&scan_context.user_id),
            scan_context.snapshot_commit_seq,
            projection,
            filter,
            limit,
            keep_deleted,
            crate::versioned::columnar::decode_user_version,
        )
        .await
    }

    fn requires_row_authorization(&self, _scan_context: &Self::ScanContext) -> bool {
        false
    }

    async fn scan_kvs_with_context_projected(
        &self,
        scan_context: &Self::ScanContext,
        filter: Option<&Expr>,
        since_seq: Option<VersionId>,
        limit: Option<usize>,
        keep_deleted: bool,
        cold_columns: Option<&[String]>,
        storage_ordinals: Option<&[usize]>,
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
            self.scan_with_version_resolution_to_kvs_result_async(
                user_id,
                filter,
                since_seq,
                limit,
                keep_deleted,
                cold_columns,
                scan_context.snapshot_commit_seq,
                false,
                storage_ordinals.map(|ordinals| ordinals.to_vec()),
            )
            .await
            .map(|result| result.rows)
        }
    }

    async fn scan_kvs_with_diagnostics_projected(
        &self,
        scan_context: &Self::ScanContext,
        filter: Option<&Expr>,
        since_seq: Option<VersionId>,
        limit: Option<usize>,
        keep_deleted: bool,
        cold_columns: Option<&[String]>,
        storage_ordinals: Option<&[usize]>,
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
            self.scan_with_version_resolution_to_kvs_result_async(
                user_id,
                filter,
                since_seq,
                limit,
                keep_deleted,
                cold_columns,
                scan_context.snapshot_commit_seq,
                true,
                storage_ordinals.map(|ordinals| ordinals.to_vec()),
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
            user_id: user_id.clone(),
            _version: row_data.version,
            _deleted: row_data.deleted,
            fields: row_data.fields.clone(),
        };
        Ok(Some((row_key, row)))
    }

    /// Find a row key by primary-key value through the hot index, then cold storage.
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
        if base::pk_exists_in_cold(&self.core, Some(user_id), id_value).await?.is_some() {
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
            ensure_manifest_ready(&self.core, Some(user_id), "UserTableProvider")?;

            crate::utils::datafusion_dml::validate_not_null_with_set(
                self.core.non_null_columns(),
                std::slice::from_ref(&row_data),
            )?;

            let predecessor = self.insert_predecessor(user_id, &row_data).await?;

            // Generate new SeqId via SystemColumnsService
            let seq_id = version;

            // Create UserTableRow directly
            let entity = UserTableRow {
                user_id: user_id.clone(),
                _version: seq_id,
                _deleted: false,
                fields: row_data,
            };

            // Create composite key
            let row_key = UserTableRowId::new(user_id.clone(), seq_id);

            // Use the same preencoded append path as batch inserts so single-row
            // MVCC writes and batch writes stay consistent.
            self.write_replaced_row(
                predecessor,
                row_key.clone(),
                entity.clone(),
                "Failed to insert user table row",
            )
            .await?;

            if log::log_enabled!(log::Level::Debug) {
                log::debug!(
                    "Inserted user table row for user {} with _seq {}",
                    user_id.as_str(),
                    seq_id
                );
            }

            if let Err(error) = self.stage_vector_upsert(user_id, seq_id, &entity.fields).await {
                crate::versioned::warn_vector_upsert(
                    self.core.table_id(),
                    Some(user_id),
                    seq_id,
                    &error,
                );
            }

            crate::versioned::mark_table_pending_write(&self.core, Some(user_id));

            crate::versioned::fanout_row_change(
                &self.core,
                Some(user_id),
                Some(user_id),
                kalamdb_commons::models::TopicOp::Insert,
                || Self::build_notification_row(&entity),
                |table_id, row| ChangeNotification::insert(table_id, row),
            )
            .await;

            Ok(row_key)
        }
        .instrument(span)
        .await
    }

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
        let prior = crate::versioned::hot_row_or_else(
            self.store.get(key).into_kalamdb_error("Failed to load prior version")?,
            load_row_from_parquet_by_seq(&self.core, Some(user_id), key.version(), |row_data| {
                UserTableRow {
                    user_id: user_id.clone(),
                    _version: row_data.version,
                    _deleted: row_data.deleted,
                    fields: row_data.fields,
                }
            }),
            "Row not found for update",
        )
        .await?;

        let pk_name = self.primary_key_field_name().to_string();
        let pk_value_scalar = crate::versioned::required_pk_scalar(&prior.fields, &pk_name)?;

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

    async fn delete_by_pk_value(
        &self,
        user_id: &UserId,
        pk_value: &str,
        version: VersionId,
    ) -> Result<bool, KalamDbError> {
        self.delete_by_pk_value_with_version(user_id, pk_value, version).await
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
            None,
        )
        .await
        .map(|result| result.rows)
    }
}
