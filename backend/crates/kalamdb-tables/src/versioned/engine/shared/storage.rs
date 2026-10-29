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
        let pk_index = new_shared_pk_index(core.table_id(), core.primary_key_field_name());
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

    pub fn authorization_cache_metrics(&self) -> kalamdb_rls::AuthorizationCacheMetrics {
        self.authorization.cache_metrics()
    }

    fn begin_authorization_mutation(&self) -> kalamdb_rls::AuthorizationMutationGuard {
        self.authorization.begin_mutation()
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
            let is_leader = self
                .core
                .services
                .cluster_coordinator
                .is_leader_for_shared(self.table_id())
                .await;
            if !is_leader {
                let leader_addr = self
                    .core
                    .services
                    .cluster_coordinator
                    .leader_addr_for_shared(self.table_id())
                    .await;
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
        let prefix = self.pk_index.build_shared_pk_prefix(pk_value);
        crate::versioned::latest_hot_by_prefix(&self.store, prefix, storage_ordinals).await
    }

    async fn hot_pk_lookup(
        &self,
        pk_value: &ScalarValue,
    ) -> Result<crate::versioned::HotPkLookup<SharedTableRowId, SharedTableRow>, KalamDbError> {
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
        let pk_scalar = crate::versioned::pk_text_to_scalar(&self.schema_ref(), pk_name, pk_value)?;
        let lookup = self.hot_pk_lookup(&pk_scalar).await?;
        let cold = if matches!(lookup, crate::versioned::HotPkLookup::Missing) {
            base::find_row_by_pk(self, None, pk_value).await?.map(|(_, row)| row)
        } else {
            None
        };
        Ok(crate::versioned::mutation_target_from_hot(lookup, cold))
    }

    /// Hot tombstone to delete when re-inserting a primary key, if one exists.
    ///
    /// A live hot or cold row is a uniqueness error. A miss returns `None`.
    async fn insert_predecessor(
        &self,
        row_data: &Row,
    ) -> Result<Option<(SharedTableRowId, SharedTableRow)>, KalamDbError> {
        let pk_name = self.primary_key_field_name();
        let Some((pk_value, pk_str)) =
            crate::versioned::insert_pk_parts(self.core.is_auto_increment_pk(), row_data, pk_name)?
        else {
            return Ok(None);
        };
        let prefix = self.pk_index.build_shared_pk_prefix(pk_value);
        match crate::versioned::hot_insert_predecessor(&self.store, &prefix, &pk_str, pk_name)? {
            crate::versioned::InsertPredecessor::Tombstone(key, row) => Ok(Some((key, row))),
            crate::versioned::InsertPredecessor::Missing => {
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
        crate::versioned::write_replaced_row(&self.store, previous, row_key, entity, error_context)
            .await
    }

    pub async fn find_by_pk(
        &self,
        pk_value: &ScalarValue,
    ) -> Result<Option<(SharedTableRowId, SharedTableRow)>, KalamDbError> {
        Ok(crate::versioned::live_indexed_row(
            self.latest_hot_pk_entry(pk_value).await?,
            |row| row._deleted,
        ))
    }

    /// Load the current visible row for a PK string, using the same hot-then-cold
    /// lookup as typed UPDATE/DELETE apply.
    pub async fn row_by_pk_value(&self, pk_value: &str) -> Result<Option<Row>, KalamDbError> {
        let pk_name = self.primary_key_field_name();
        let pk_scalar = crate::versioned::pk_column_scalar(&self.schema_ref(), pk_name, pk_value)?;

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
        let _ = version;
        crate::versioned::patch_stored_row(&self.store, row_key, row_key.as_i64()).await
    }

    pub async fn patch_latest_commit_seq_by_pk(
        &self,
        pk_value: &str,
        version: VersionId,
    ) -> Result<bool, KalamDbError> {
        let pk_scalar = crate::versioned::pk_column_scalar(
            &self.schema_ref(),
            self.primary_key_field_name(),
            pk_value,
        )?;

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
        if crate::versioned::table_has_file_columns(self.core.table_def()) {
            let rows = crate::versioned::rows_from_field_payloads(
                payloads,
                self.core.storage_schema().as_ref(),
            )?;
            return Ok(self.insert_batch_with_versions(actor_user_id, rows, versions).await?.len());
        }
        let entries = self.persist_encoded_field_rows(payloads, versions).await?;
        let row_count = entries.len();
        crate::versioned::fanout_insert_notifications(&self.core, None, actor_user_id, || {
            entries.iter().map(|(_, entity)| Self::build_notification_row(entity)).collect()
        })
        .await;
        Ok(row_count)
    }

    async fn persist_encoded_field_rows(
        &self,
        payloads: &[Vec<u8>],
        versions: &[VersionId],
    ) -> Result<Vec<(SharedTableRowId, SharedTableRow)>, KalamDbError> {
        ensure_manifest_ready(&self.core, None, "SharedTableProvider")?;
        let storage = self.core.storage_schema();
        let decoded = crate::versioned::decode_ordinal_insert(
            payloads,
            storage.as_ref(),
            &self.schema_ref(),
            self.core.non_null_columns(),
            versions,
        )?;
        let rows = decoded
            .columns
            .into_iter()
            .map(|columns| Row::from_aligned(&decoded.names, columns))
            .collect();
        self.persist_insert_batch_rows(rows, versions, true).await
    }
}
