#[derive(Clone)]
pub struct SharedScanContext {
    snapshot_commit_seq: Option<VersionId>,
    policies: kalamdb_rls::BoundTablePolicies,
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

    async fn try_columnar_hot_scan(
        &self,
        scan_context: &Self::ScanContext,
        projection: Option<&Vec<usize>>,
        filter: Option<&Expr>,
        limit: Option<usize>,
        keep_deleted: bool,
    ) -> Result<Option<(RecordBatch, usize)>, KalamDbError> {
        self.columnar_hot_scan(
            scan_context.snapshot_commit_seq,
            projection,
            filter,
            limit,
            keep_deleted,
        )
        .await
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

    fn additional_stored_column_names(&self, scan_context: &Self::ScanContext) -> Vec<String> {
        let mut names = Vec::new();
        shared_table_authorization::push_missing_column_names(
            &mut names,
            &self.core.table_def().columns,
            &scan_context.policies.required_column_ids(),
        );
        names
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
    ) -> Result<Vec<(SharedTableRowId, SharedTableRow)>, KalamDbError> {
        let scan_limit = scan_context.policies.bypasses_rls().then_some(limit).flatten();
        let cold_columns = self.authorization_cold_columns(scan_context, cold_columns);
        self.scan_with_version_resolution_to_kvs_result_async(
            filter,
            since_seq,
            scan_limit,
            keep_deleted,
            cold_columns.as_deref(),
            scan_context.snapshot_commit_seq,
            false,
            storage_ordinals.map(|ordinals| ordinals.to_vec()),
        )
        .await
        .map(|result| result.rows)
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
            storage_ordinals.map(|ordinals| ordinals.to_vec()),
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
            fields: row_data.fields.clone(),
        };
        Ok(Some((row_key, row)))
    }

    /// Find a row key by primary-key value. A cold hit still returns `None`.
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
            ensure_manifest_ready(&self.core, None, "SharedTableProvider")?;

            crate::utils::datafusion_dml::validate_not_null_with_set(
                self.core.non_null_columns(),
                std::slice::from_ref(&row_data),
            )?;

            let predecessor = self.insert_predecessor(&row_data).await?;

            let seq_id = version;

            let entity = SharedTableRow {
                _version: seq_id,
                _deleted: false,
                fields: row_data,
            };

            // Key is just the SeqId (SharedTableRowId is type alias for SeqId)
            let row_key = seq_id;

            self.write_replaced_row(
                predecessor,
                row_key,
                entity.clone(),
                "Failed to insert shared table row",
            )
            .await?;

            log::debug!("Inserted shared table row with _seq {}", seq_id);

            if let Err(error) = self.stage_vector_upsert(seq_id, &entity.fields).await {
                crate::versioned::warn_vector_upsert(self.core.table_id(), None, seq_id, &error);
            }

            crate::versioned::mark_table_pending_write(&self.core, None);

            crate::versioned::fanout_row_change(
                &self.core,
                None,
                Some(_user_id),
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
        let pk_name = self.primary_key_field_name().to_string();

        // Load referenced prior version to derive PK value
        let prior = crate::versioned::hot_row_or_else(
            self.store.get(key).into_kalamdb_error("Failed to load prior version")?,
            load_row_from_parquet_by_seq(&self.core, None, *key, |row_data| SharedTableRow {
                _version: row_data.version,
                _deleted: row_data.deleted,
                fields: row_data.fields,
            }),
            "Row not found for update",
        )
        .await?;

        let pk_value_scalar = crate::versioned::required_pk_scalar(&prior.fields, &pk_name)?;

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

    async fn delete_by_pk_value(
        &self,
        _user_id: &UserId,
        pk_value: &str,
        version: VersionId,
    ) -> Result<bool, KalamDbError> {
        self.delete_by_pk_value_with_version(_user_id, pk_value, version).await
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
            None,
        )
        .await
        .map(|result| result.rows)
    }
}

impl SharedTableProvider {
    pub(crate) async fn columnar_hot_scan(
        &self,
        snapshot: Option<VersionId>,
        projection: Option<&Vec<usize>>,
        filter: Option<&Expr>,
        limit: Option<usize>,
        keep_deleted: bool,
    ) -> Result<Option<(RecordBatch, usize)>, KalamDbError> {
        crate::versioned::columnar::run_columnar_hot_scan(
            &self.core,
            &self.store,
            None,
            snapshot,
            projection,
            filter,
            limit,
            keep_deleted,
            crate::versioned::columnar::decode_shared_version,
        )
        .await
    }

    pub async fn sum_hot_covering_integers(
        &self,
        filters: &[Expr],
        value_column: &str,
    ) -> Result<Option<(u64, i128)>, KalamDbError> {
        crate::versioned::covering::sum_hot_covering_integers(
            &self.core,
            &self.store,
            None,
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
            TableType::Shared,
            None,
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

    /// SELECT visibility for one shared FILE path.
    ///
    /// DBA and System bypass. An empty policy set denies. Otherwise a current
    /// row must contain this `{subfolder}/{stored_name}` in a FILE column and
    /// survive `authorize_resolved_rows`.
    pub async fn select_authorizes_file_path(
        &self,
        user_id: &UserId,
        role: kalamdb_commons::Role,
        subfolder: &str,
        stored_name: &str,
    ) -> Result<bool, KalamDbError> {
        let policies = self.bind_policies(user_id, role, PolicyCommand::Select, false)?;
        if policies.bypasses_rls() {
            return Ok(true);
        }
        if policies.is_default_deny() {
            return Ok(false);
        }

        let file_columns = file_column_names(self.core.table_def());
        if file_columns.is_empty() {
            return Ok(false);
        }

        let scan_context = SharedScanContext {
            snapshot_commit_seq: None,
            policies,
        };
        if !self.authorization.pre_authorize_scan(self, &scan_context, None).await? {
            return Ok(false);
        }

        let rows = DeferredMvccScanProvider::scan_kvs_with_context(
            self,
            &scan_context,
            None,
            None,
            None,
            false,
            Some(file_columns.as_slice()),
        )
        .await?;

        for (row_id, row) in rows {
            if !row_contains_file_path(&row.fields, &file_columns, subfolder, stored_name) {
                continue;
            }
            let authorized =
                self.authorize_resolved_rows(&scan_context, vec![(row_id, row)]).await?;
            if !authorized.is_empty() {
                return Ok(true);
            }
        }
        Ok(false)
    }
}

fn file_column_names(table_def: &kalamdb_commons::models::schemas::TableDefinition) -> Vec<String> {
    table_def
        .columns
        .iter()
        .filter(|column| column.data_type() == kalamdb_commons::KalamDataType::File)
        .map(|column| column.column_name.clone())
        .collect()
}

fn row_contains_file_path(
    row: &Row,
    file_columns: &[String],
    subfolder: &str,
    stored_name: &str,
) -> bool {
    file_columns.iter().any(|column| {
        let Some(value) = row.values.get(column) else {
            return false;
        };
        let json = match value {
            ScalarValue::Utf8(Some(json)) | ScalarValue::LargeUtf8(Some(json)) => json.as_str(),
            _ => return false,
        };
        kalamdb_commons::FileRef::try_from_json(json).is_ok_and(|file_ref| {
            file_ref.sub == subfolder && file_ref.stored_name() == stored_name
        })
    })
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
        storage_ordinals: Option<Vec<usize>>,
    ) -> Result<base::MvccScanResult<SharedTableRowId, SharedTableRow>, KalamDbError> {
        use kalamdb_store::EntityStoreAsync;

        let start_key =
            since_seq.and_then(|seq| VersionId::try_from_i64(seq.as_i64().saturating_add(1)).ok());
        let scan_limit = base::calculate_scan_limit(limit);
        let pk_name = self.primary_key_field_name();
        let hot_future = async {
            if since_seq.is_none() {
                if let Some(plan) = base::hot_index_seek(self.store.as_ref(), filter, None) {
                    let rows = base::scan_hot_index_resolved(
                        &self.store,
                        plan,
                        scan_limit,
                        pk_name,
                        None,
                        storage_ordinals,
                    )
                    .await?;
                    base::reject_truncated_unbounded_scan(limit, rows.len())?;
                    return Ok(rows);
                }
            }
            let scanned = if let Some(ordinals) = storage_ordinals {
                self.store
                    .scan_typed_with_prefix_and_start_selected_async(
                        None,
                        start_key.as_ref(),
                        scan_limit,
                        ordinals,
                    )
                    .await
            } else {
                self.store
                    .scan_typed_with_prefix_and_start_async(None, start_key.as_ref(), scan_limit)
                    .await
            };
            scanned
                .map_err(|e| {
                    KalamDbError::InvalidOperation(format!(
                        "Failed to scan shared table hot storage: {}",
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
                base::scan_provider_parquet_stats(&self.core, None, filter, cold_columns).await
            } else {
                base::scan_provider_parquet_result(&self.core, None, filter, cold_columns).await
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
                hot_rows_scanned: Some(resolved.hot_rows_scanned),
                cold_rows_scanned: Some(resolved.cold_rows_scanned),
                cold_files_total: Some(resolved.cold_files_total),
                cold_files_skipped: Some(resolved.cold_files_skipped),
                cold_files_scanned: Some(resolved.cold_files_scanned),
                cold_files: resolved.cold_files,
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
