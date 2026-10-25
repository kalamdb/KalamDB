impl SharedTableProvider {
    fn session_state_with_rls_command(
        state: &dyn Session,
        command: PolicyCommand,
    ) -> DataFusionResult<SessionState> {
        let mut state = state
            .as_any()
            .downcast_ref::<SessionState>()
            .ok_or_else(|| DataFusionError::Execution("Expected SessionState".to_string()))?
            .clone();
        state
            .config_mut()
            .options_mut()
            .extensions
            .insert(RlsCommandContext { command });
        Ok(state)
    }

    async fn bind_authorization(
        &self,
        policies: &kalamdb_rls::BoundTablePolicies,
        snapshot_commit_seq: Option<VersionId>,
    ) -> Result<kalamdb_rls::BoundAuthorization, KalamDbError> {
        self.authorization.bind_authorization(self, policies, snapshot_commit_seq).await
    }

    fn bind_policies(
        &self,
        user_id: &UserId,
        role: kalamdb_commons::Role,
        command: PolicyCommand,
        check: bool,
    ) -> Result<kalamdb_rls::BoundTablePolicies, KalamDbError> {
        self.authorization
            .bind_policies(self.core.as_ref(), user_id, role, command, check)
    }

    async fn ensure_rows_authorized(
        &self,
        policies: &kalamdb_rls::BoundTablePolicies,
        rows: &[Row],
        snapshot_commit_seq: Option<VersionId>,
        operation: &str,
    ) -> DataFusionResult<()> {
        self.authorization
            .ensure_rows_authorized(self, policies, rows, snapshot_commit_seq, operation)
            .await
    }

    pub async fn check_rows_authorized(
        &self,
        user_id: &UserId,
        role: kalamdb_commons::Role,
        command: PolicyCommand,
        check: bool,
        rows: &[Row],
        snapshot_commit_seq: Option<VersionId>,
    ) -> Result<(), KalamDbError> {
        self.authorization
            .check_rows_authorized(self, user_id, role, command, check, rows, snapshot_commit_seq)
            .await
    }

    pub async fn bind_live_authorization(
        &self,
        user_id: &UserId,
        role: kalamdb_commons::Role,
    ) -> Result<BoundLiveAuthorization, KalamDbError> {
        self.authorization.bind_live_authorization(self, user_id, role).await
    }

    fn authorization_cold_columns(
        &self,
        scan_context: &SharedScanContext,
        cold_columns: Option<&[String]>,
    ) -> Option<Vec<String>> {
        self.authorization.authorization_cold_columns(self, scan_context, cold_columns)
    }

    /// Create a new shared table provider
    ///
    /// # Arguments
    /// * `core` - Shared core with services, schema, pk_name, table_def, etc.
    /// * `store` - SharedTableIndexedStore for this table
    pub fn new(core: Arc<TableProviderCore>, store: Arc<SharedTableIndexedStore>) -> Self {
        let pk_index = SharedTablePkIndex::new(core.table_id(), core.primary_key_field_name());
        let vector_columns = base::embedding_columns(core.table_def());
        let vector_stores = crate::utils::vector_staging::build_vector_store_map(
            store.backend().clone(),
            core.table_id(),
            &vector_columns,
            new_indexed_shared_vector_hot_store,
        );

        Self {
            core,
            store,
            pk_index,
            vector_columns,
            vector_stores,
            authorization: SharedTableAuthorization::new(),
        }
    }

    pub fn authorization_generation(&self) -> u64 {
        self.authorization.authorization_generation()
    }

    pub fn has_active_authorization_mutations(&self) -> bool {
        self.authorization.has_active_authorization_mutations()
    }

    pub fn authorization_cache_metrics(&self) -> kalamdb_rls::AuthorizationCacheMetrics {
        self.authorization.cache_metrics()
    }

    fn begin_authorization_mutation(&self) -> kalamdb_rls::AuthorizationMutationGuard {
        self.authorization.begin_mutation()
    }

    pub async fn collect_live_string_primary_keys_before_async(
        &self,
        cutoff_exclusive: String,
        limit: usize,
    ) -> Result<Vec<String>, KalamDbError> {
        if limit == 0 {
            return Ok(Vec::new());
        }

        let store = Arc::clone(&self.store);
        let pk_name = self.primary_key_field_name().to_string();

        tokio::task::spawn_blocking(move || {
            collect_live_string_primary_keys_before_from_store(
                store.as_ref(),
                &pk_name,
                &cutoff_exclusive,
                limit,
            )
        })
        .await
        .map_err(|error| {
            KalamDbError::InvalidOperation(format!(
                "collect_live_string_primary_keys_before_async join error: {}",
                error
            ))
        })?
    }

    pub async fn hard_delete_string_primary_keys_async(
        &self,
        pk_values: Vec<String>,
    ) -> Result<usize, KalamDbError> {
        if pk_values.is_empty() {
            return Ok(0);
        }

        let store = Arc::clone(&self.store);
        let table_id = self.core.table_id().clone();
        let pk_name = self.primary_key_field_name().to_string();

        tokio::task::spawn_blocking(move || {
            let pk_index = SharedTablePkIndex::new(&table_id, &pk_name);
            hard_delete_string_primary_keys_from_store(store.as_ref(), &pk_index, &pk_values)
        })
        .await
        .map_err(|error| {
            KalamDbError::InvalidOperation(format!(
                "hard_delete_string_primary_keys_async join error: {}",
                error
            ))
        })?
    }

    /// Build a complete Row for live query/topic notifications including system columns (_seq,
    /// _deleted)
    ///
    /// This ensures notifications include all columns, not just user-defined fields.
    fn build_notification_row(entity: &SharedTableRow) -> Row {
        base::build_notification_row(&entity.fields, entity._version, entity._deleted)
    }

    fn build_delete_notification(&self, table_id: TableId, row: Row) -> ChangeNotification {
        let mut notification = ChangeNotification::delete_soft(table_id, row);
        notification.pk_columns = vec![self.primary_key_field_name().to_string()];
        notification
    }

    /// Access the underlying indexed store (used by flush jobs)
    pub fn store(&self) -> Arc<SharedTableIndexedStore> {
        Arc::clone(&self.store)
    }

    async fn ensure_shared_write_leader(&self) -> Result<(), KalamDbError> {
        if self.core.services.cluster_coordinator.is_cluster_mode().await {
            let is_leader = self.core.services.cluster_coordinator.is_leader_for_shared(self.table_id()).await;
            if !is_leader {
                let leader_addr =
                    self.core.services.cluster_coordinator.leader_addr_for_shared(self.table_id()).await;
                return Err(KalamDbError::NotLeader { leader_addr });
            }
        }

        Ok(())
    }

    async fn ensure_shared_write_route(&self, state: &dyn Session) -> DataFusionResult<()> {
        if extract_transaction_query_context(state).is_some() {
            self.validate_transaction_table_access(state)?;
            return Ok(());
        }

        self.ensure_shared_write_leader().await.map_err(|error| match error {
            KalamDbError::NotLeader { leader_addr } => {
                DataFusionError::External(Box::new(NotLeaderError::new(leader_addr)))
            },
            other => crate::error::into_datafusion(other),
        })
    }

    async fn stage_vector_upsert(
        &self,
        seq: SharedTableRowId,
        row: &Row,
    ) -> Result<(), KalamDbError> {
        if self.vector_columns.is_empty() {
            return Ok(());
        }

        let ops_by_column = crate::utils::vector_staging::build_vector_upsert_batch_ops(
            self.core.table_id(),
            self.primary_key_field_name(),
            &self.vector_columns,
            std::iter::once((seq, row)),
            |(_, row)| row,
            |(seq, _), pk| SharedVectorHotOpId::new(SeqId::from_i64(seq.as_i64()), pk.to_string()),
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
        entries: &[(SharedTableRowId, SharedTableRow)],
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
                SharedVectorHotOpId::new(SeqId::from_i64(row_key.as_i64()), pk.to_string())
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
        seq: SharedTableRowId,
        pk: &str,
    ) -> Result<(), KalamDbError> {
        if self.vector_columns.is_empty() {
            return Ok(());
        }

        let ops_by_column = crate::utils::vector_staging::build_vector_delete_ops(
            self.core.table_id(),
            &self.vector_columns,
            pk,
            |primary_key| {
                SharedVectorHotOpId::new(SeqId::from_i64(seq.as_i64()), primary_key.to_string())
            },
        );
        crate::utils::vector_staging::stage_vector_ops_by_column(
            &self.vector_stores,
            ops_by_column,
            "stage vector delete",
        )
        .await
    }

    /// Scan Parquet files from cold storage for shared table
    ///
    /// Lists all *.parquet files in the table's storage directory and merges them into a single
    /// RecordBatch. Returns an empty batch if no Parquet files exist.
    ///
    /// **Difference from user tables**: Shared tables have NO user_id partitioning,
    /// so all Parquet files are in the same directory (no subdirectories per user).
    ///
    /// **Phase 4 (US6, T082-T084)**: Integrated with ManifestService for manifest caching.
    /// Logs cache hits/misses and updates last_accessed timestamp. Full query optimization
    /// (batch file pruning based on manifest metadata) implemented in Phase 5 (US2, T119-T123).
    ///
    /// **Manifest-Driven Pruning**: Uses ManifestAccessPlanner to select files based on filter
    /// predicates, enabling row-group level pruning when row_group metadata is available.
    async fn scan_parquet_files_as_batch_async(
        &self,
        filter: Option<&Expr>,
        columns: Option<&[String]>,
    ) -> Result<RecordBatch, KalamDbError> {
        base::scan_parquet_files_as_batch_async(
            &self.core,
            self.core.table_id(),
            self.core.table_type(),
            None,
            self.schema_ref(),
            filter,
            columns,
        )
        .await
    }

    async fn scan_parquet_files_with_stats_async(
        &self,
        filter: Option<&Expr>,
        columns: Option<&[String]>,
    ) -> Result<base::ParquetScanResult, KalamDbError> {
        base::scan_parquet_files_with_stats_async(
            &self.core,
            self.core.table_id(),
            self.core.table_type(),
            None,
            self.schema_ref(),
            filter,
            columns,
        )
        .await
    }

    async fn scan_parquet_files_as_result_async(
        &self,
        filter: Option<&Expr>,
        columns: Option<&[String]>,
    ) -> Result<base::ParquetScanResult, KalamDbError> {
        base::scan_parquet_files_as_result_async(
            &self.core,
            self.core.table_id(),
            self.core.table_type(),
            None,
            self.schema_ref(),
            filter,
            columns,
        )
        .await
    }

    fn construct_shared_row_from_parquet_data(
        &self,
        row_data: crate::utils::version_resolution::ParquetRowData,
    ) -> DataFusionResult<(SharedTableRowId, SharedTableRow)> {
        self.construct_row_from_parquet_data(base::system_user_id(), &row_data)
            .map_err(crate::error::into_datafusion)?
            .ok_or_else(|| {
                DataFusionError::Execution("missing shared row from parquet data".to_string())
            })
    }

    /// Find a row by PK value using the PK index for efficient O(1) lookup.
    ///
    /// This method uses the PK index to find all versions of a row with the given PK value,
    /// then returns the latest non-deleted version.
    async fn latest_hot_pk_entry(
        &self,
        pk_value: &ScalarValue,
    ) -> Result<Option<(SharedTableRowId, SharedTableRow)>, KalamDbError> {
        self.latest_hot_pk_entry_maybe_selected(pk_value, None).await
    }

    async fn latest_hot_pk_entry_maybe_selected(
        &self,
        pk_value: &ScalarValue,
        storage_ordinals: Option<&[usize]>,
    ) -> Result<Option<(SharedTableRowId, SharedTableRow)>, KalamDbError> {
        let prefix = self.pk_index.build_prefix_for_pk(pk_value);
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

    async fn hot_pk_lookup(
        &self,
        pk_value: &ScalarValue,
    ) -> Result<crate::versioned::HotPkLookup<SharedTableRowId, SharedTableRow>, KalamDbError>
    {
        Ok(crate::versioned::classify_hot_pk(
            self.latest_hot_pk_entry(pk_value).await?,
            |row| row._deleted,
        ))
    }

    /// One hot lookup, then cold only when hot storage has no row for this key.
    pub(crate) async fn load_mutation_target(
        &self,
        pk_value: &str,
    ) -> Result<crate::versioned::MutationTarget<SharedTableRowId, SharedTableRow>, KalamDbError>
    {
        let pk_name = self.primary_key_field_name();
        let schema = self.schema_ref();
        let pk_field = schema.field_with_name(pk_name).map_err(|e| {
            KalamDbError::InvalidOperation(format!(
                "PK column '{}' not found in schema: {}",
                pk_name, e
            ))
        })?;
        let pk_scalar =
            kalamdb_commons::conversions::parse_string_as_scalar(pk_value, pk_field.data_type())
                .map_err(KalamDbError::InvalidOperation)?;

        match self.hot_pk_lookup(&pk_scalar).await? {
            crate::versioned::HotPkLookup::Live(key, row) => {
                Ok(crate::versioned::MutationTarget::Hot(key, row))
            },
            crate::versioned::HotPkLookup::Tombstoned(_, _) => {
                Ok(crate::versioned::MutationTarget::Tombstoned)
            },
            crate::versioned::HotPkLookup::Missing => {
                match base::find_row_by_pk(self, None, pk_value).await? {
                    Some((_, row)) => Ok(crate::versioned::MutationTarget::Cold(row)),
                    None => Ok(crate::versioned::MutationTarget::Missing),
                }
            },
        }
    }

    /// Hot tombstone to delete when re-inserting a primary key, if one exists.
    ///
    /// A live hot or cold row is a uniqueness error. A miss returns `None`.
    async fn insert_predecessor(
        &self,
        row_data: &Row,
    ) -> Result<Option<(SharedTableRowId, SharedTableRow)>, KalamDbError> {
        if self.core.is_auto_increment_pk() {
            return Ok(None);
        }
        let pk_name = self.primary_key_field_name();
        let Some(pk_value) = row_data.get(pk_name) else {
            return Ok(None);
        };
        if matches!(pk_value, ScalarValue::Null) {
            return Ok(None);
        }
        let pk_str = crate::utils::unified_dml::extract_user_pk_value(row_data, pk_name)?;
        let prefix = self.pk_index.build_prefix_for_pk(pk_value);
        match self
            .store
            .get_latest_deleted_flag_by_index_prefix(0, &prefix)
            .into_kalamdb_error("PK index scan failed")?
        {
            Some((_, false)) => Err(KalamDbError::AlreadyExists(format!(
                "Primary key violation: value '{}' already exists in column '{}' (hot storage)",
                pk_str, pk_name
            ))),
            Some((key, true)) => {
                let row = self.store.get(&key).into_kalamdb_error("PK tombstone load failed")?.ok_or_else(|| {
                    KalamDbError::NotFound(format!(
                        "tombstone for primary key '{pk_str}' disappeared before replacement"
                    ))
                })?;
                Ok(Some((key, row)))
            },
            None => {
                base::ensure_pk_absent_in_cold(self, None, &pk_str).await?;
                Ok(None)
            },
        }
    }

    async fn write_replaced_row(
        &self,
        previous: Option<(SharedTableRowId, SharedTableRow)>,
        row_key: SharedTableRowId,
        entity: SharedTableRow,
        error_context: &str,
    ) -> Result<(), KalamDbError> {
        let written = if previous.is_some() {
            self.store.replace_async(previous, row_key, entity).await
        } else {
            self.store.insert_async(row_key, entity).await
        };
        written.map_err(|e| KalamDbError::InvalidOperation(format!("{}: {}", error_context, e)))
    }

    pub async fn find_by_pk(
        &self,
        pk_value: &ScalarValue,
    ) -> Result<Option<(SharedTableRowId, SharedTableRow)>, KalamDbError> {
        Ok(self.latest_hot_pk_entry(pk_value).await?.and_then(|(row_id, row)| {
            if row._deleted {
                None
            } else {
                Some((row_id, row))
            }
        }))
    }

    /// Load the current visible row for a PK string, using the same hot-then-cold
    /// lookup as typed UPDATE/DELETE apply.
    pub async fn row_by_pk_value(&self, pk_value: &str) -> Result<Option<Row>, KalamDbError> {
        let pk_name = self.primary_key_field_name();
        let schema = self.schema_ref();
        let pk_field = schema
            .field_with_name(pk_name)
            .map_err(|e| KalamDbError::InvalidOperation(format!("PK column lookup failed: {e}")))?;
        let pk_scalar =
            kalamdb_commons::conversions::parse_string_as_scalar(pk_value, pk_field.data_type())
                .map_err(KalamDbError::InvalidOperation)?;

        match self.hot_pk_lookup(&pk_scalar).await? {
            crate::versioned::HotPkLookup::Live(_, row) => return Ok(Some(row.fields)),
            crate::versioned::HotPkLookup::Tombstoned(_, _) => return Ok(None),
            crate::versioned::HotPkLookup::Missing => {},
        }
        Ok(base::find_row_by_pk(self, None, pk_value).await?.map(|(_, row)| row.fields))
    }

    pub async fn patch_commit_seq_for_row_key(
        &self,
        row_key: &SharedTableRowId,
        version: VersionId,
    ) -> Result<(), KalamDbError> {
        let row = self
            .store
            .get(row_key)
            .into_kalamdb_error("Failed to load row for commit_seq patch")?
            .ok_or_else(|| {
                KalamDbError::NotFound(format!(
                    "row '{}' not found while patching commit_seq",
                    row_key.as_i64()
                ))
            })?;
        let _ = version; // version is assigned at insert; patch is a no-op
        self.store.insert_async(*row_key, row).await.map_err(|e| {
            KalamDbError::InvalidOperation(format!("Failed to patch commit_seq: {}", e))
        })
    }

    pub async fn patch_latest_commit_seq_by_pk(
        &self,
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

        let Some((row_key, _)) = self.latest_hot_pk_entry(&pk_scalar).await? else {
            return Ok(false);
        };

        self.patch_commit_seq_for_row_key(&row_key, version).await?;
        Ok(true)
    }

    /// Apply Raft ordinal field payloads without a second name-map encode.
    pub async fn insert_encoded_fields_with_versions(
        &self,
        actor_user_id: Option<&UserId>,
        payloads: &[Vec<u8>],
        versions: &[VersionId],
    ) -> Result<usize, KalamDbError> {
        if payloads.is_empty() {
            return Ok(0);
        }
        if self.core.table_def().columns.iter().any(|column| {
            matches!(
                column.data_type(),
                kalamdb_commons::models::datatypes::KalamDataType::File
            )
        }) {
            let rows = shared_rows_from_field_payloads(payloads, self.core.storage_schema().as_ref())?;
            return Ok(self.insert_batch_with_versions(actor_user_id, rows, versions).await?.len());
        }
        let entries = self.persist_encoded_field_rows(payloads, versions).await?;
        let row_count = entries.len();
        let notification_service = self.core.services.notification_service.clone();
        let table_id = self.core.table_id().clone();
        let has_topics = self.core.has_topic_routes(&table_id);
        let has_live_subs = notification_service.has_subscribers(None, &table_id);
        if has_topics || has_live_subs {
            let rows: Vec<_> = entries
                .iter()
                .map(|(_, entity)| Self::build_notification_row(entity))
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
                    notification_service.notify_table_change(
                        None,
                        table_id.clone(),
                        ChangeNotification::insert(table_id.clone(), row),
                    );
                }
            }
        }
        Ok(row_count)
    }

    async fn persist_encoded_field_rows(
        &self,
        payloads: &[Vec<u8>],
        versions: &[VersionId],
    ) -> Result<Vec<(SharedTableRowId, SharedTableRow)>, KalamDbError> {
        ensure_manifest_ready(&self.core, self.core.table_type(), None, "SharedTableProvider")?;
        let storage = Arc::clone(self.core.storage_schema());
        let arrow = self.schema_ref();
        let mut decoded = Vec::with_capacity(payloads.len());
        for payload in payloads {
            let mut columns =
                kalamdb_serialization::decode_row_fields_columns(payload, storage.as_ref())
                    .map_err(|error| {
                        KalamDbError::InvalidOperation(format!(
                            "ordinal insert decode failed: {error}"
                        ))
                    })?;
            shared_coerce_storage_columns(&mut columns, storage.as_ref(), &arrow)?;
            decoded.push(columns);
        }
        shared_validate_encoded_not_null(&decoded, storage.as_ref(), self.core.non_null_columns())?;
        if versions.len() != decoded.len() {
            return Err(KalamDbError::InvalidOperation(format!(
                "version count {} does not match row count {}",
                versions.len(),
                decoded.len()
            )));
        }
        let names: Vec<String> = storage.fields.iter().map(|field| field.name.clone()).collect();
        let rows = decoded
            .into_iter()
            .map(|columns| Row::from_aligned(&names, columns))
            .collect();
        self.persist_insert_batch_rows(rows, versions, true).await
    }

}

fn shared_rows_from_field_payloads(
    payloads: &[Vec<u8>],
    storage: &kalamdb_serialization::StorageSchema,
) -> Result<Vec<Row>, KalamDbError> {
    let names: Vec<String> = storage.fields.iter().map(|field| field.name.clone()).collect();
    payloads
        .iter()
        .map(|payload| {
            let columns = kalamdb_serialization::decode_row_fields_columns(payload, storage)
                .map_err(|error| {
                    KalamDbError::InvalidOperation(format!("ordinal insert decode failed: {error}"))
                })?;
            Ok(Row::from_aligned(&names, columns))
        })
        .collect()
}

fn shared_coerce_storage_columns(
    columns: &mut [ScalarValue],
    storage: &kalamdb_serialization::StorageSchema,
    arrow: &SchemaRef,
) -> Result<(), KalamDbError> {
    for (value, field) in columns.iter_mut().zip(storage.fields.iter()) {
        if field.dropped {
            continue;
        }
        let Ok(arrow_field) = arrow.field_with_name(&field.name) else {
            continue;
        };
        let owned = std::mem::replace(value, ScalarValue::Null);
        *value = kalamdb_commons::conversions::coerce_scalar_to_field(owned, arrow_field)
            .map_err(|error| {
                KalamDbError::InvalidOperation(format!("Schema coercion failed: {error}"))
            })?;
    }
    Ok(())
}

fn shared_validate_encoded_not_null(
    rows: &[Vec<ScalarValue>],
    storage: &kalamdb_serialization::StorageSchema,
    required: &HashSet<String>,
) -> Result<(), KalamDbError> {
    if required.is_empty() {
        return Ok(());
    }
    for (row_index, row) in rows.iter().enumerate() {
        for name in required {
            let Some(ordinal) = storage.fields.iter().position(|field| &field.name == name) else {
                continue;
            };
            if row.get(ordinal).is_none_or(ScalarValue::is_null) {
                return Err(KalamDbError::Coded(kalamdb_commons::CodedError::not_null(
                    name.clone(),
                    (row_index + 1) as i64,
                )));
            }
        }
    }
    Ok(())
}
