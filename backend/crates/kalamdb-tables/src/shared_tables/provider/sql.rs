// Manual Debug to satisfy DataFusion's TableProvider: Debug bound
impl std::fmt::Debug for SharedTableProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let table_id = self.core.table_id_arc();
        f.debug_struct("SharedTableProvider")
            .field("table_id", &table_id)
            .field("table_type", &self.core.table_type())
            .field("primary_key_field_name", &self.core.primary_key_field_name())
            .finish()
    }
}

// Implement DataFusion TableProvider trait
#[async_trait]
impl TableProvider for SharedTableProvider {
    fn schema(&self) -> SchemaRef {
        self.schema_ref()
    }

    fn table_type(&self) -> datafusion::logical_expr::TableType {
        datafusion::logical_expr::TableType::Base
    }

    fn get_column_default(&self, column: &str) -> Option<&Expr> {
        self.core.get_column_default(column)
    }

    async fn scan(
        &self,
        state: &dyn Session,
        projection: Option<&Vec<usize>>,
        filters: &[Expr],
        limit: Option<usize>,
    ) -> DataFusionResult<Arc<dyn ExecutionPlan>> {
        {
            let _span = kalamdb_observability::kdb_info_span_entered!("table.access");
            // SECURITY: Admit authenticated roles; FORCE RLS filters rows.
            check_shared_table_access(state, self.core.table_def())
                .map_err(session_error_to_datafusion)?;
        }

        // Leader routing and transaction table access live in `base_scan`.

        let table_overlay = extract_transaction_query_context(state)
            .and_then(|context| context.overlay_view.overlay_for_table(self.core.table_id()));

        kalamdb_observability::kdb_await_in_info_span!(
            <Self as BaseTableProvider<SharedTableRowId, SharedTableRow>>::base_scan_with_overlay(
                self,
                state,
                projection,
                filters,
                limit,
                table_overlay,
                None,
            ),
            "table.base_scan"
        )
    }

    async fn insert_into(
        &self,
        state: &dyn Session,
        input: Arc<dyn ExecutionPlan>,
        insert_op: InsertOp,
    ) -> DataFusionResult<Arc<dyn ExecutionPlan>> {
        check_shared_table_write_access(state, self.core.table_def())
            .map_err(session_error_to_datafusion)?;

        if insert_op != InsertOp::Append {
            return Err(DataFusionError::Plan(format!(
                "{} is not supported for shared tables",
                insert_op
            )));
        }

        self.ensure_shared_write_route(state).await?;

        let (user_id, role) =
            extract_user_context(state).map_err(|e| DataFusionError::Execution(e.to_string()))?;

        let rows = crate::utils::datafusion_dml::collect_input_rows(state, input).await?;
        let check_policies = self
            .bind_policies(user_id, role, PolicyCommand::Insert, true)
            .map_err(|error| DataFusionError::Execution(error.to_string()))?;
        let snapshot_commit_seq = extract_transaction_query_context(state).and_then(|context| {
            crate::utils::base::transaction_snapshot_bound(context.snapshot_commit_seq())
        });
        self.ensure_rows_authorized(&check_policies, &rows, snapshot_commit_seq, "WITH CHECK")
            .await?;
        if let Some(transaction_query_context) = extract_transaction_query_context(state) {
            let inserted = crate::utils::datafusion_dml::stage_insert_rows(
                transaction_query_context,
                self.core.table_id(),
                TableType::Shared,
                Some(user_id.clone()),
                self.primary_key_field_name(),
                rows,
            )?;

            return crate::utils::datafusion_dml::rows_affected_plan(state, inserted).await;
        }

        if self.core.services.cluster_coordinator.replicates_shared_writes() {
            let inserted = self
                .core
                .services
                .cluster_coordinator
                .propose_shared_insert(self.core.table_id(), &user_id, rows)
                .await
                .map_err(DataFusionError::Execution)?;
            return crate::utils::datafusion_dml::rows_affected_plan(state, inserted as u64).await;
        }

        let versions = crate::utils::base::direct_insert_versions(
            self.core.services.commit_sequence_source.allocate_next(),
            rows.len(),
        )
        .map_err(|error| DataFusionError::Execution(error.to_string()))?;
        let inserted = self
            .insert_batch_with_versions(Some(&user_id), rows, &versions)
            .await
            .map_err(|error| DataFusionError::Execution(error.to_string()))?;
        crate::utils::datafusion_dml::rows_affected_plan(state, inserted.len() as u64).await
    }

    async fn delete_from(
        &self,
        state: &dyn Session,
        filters: Vec<Expr>,
    ) -> DataFusionResult<Arc<dyn ExecutionPlan>> {
        check_shared_table_write_access(state, self.core.table_def())
            .map_err(session_error_to_datafusion)?;
        crate::utils::datafusion_dml::validate_where_clause(&filters, "DELETE")?;

        self.ensure_shared_write_route(state).await?;

        let (user_id, _role) =
            extract_user_context(state).map_err(|e| DataFusionError::Execution(e.to_string()))?;

        let pk_column = self.primary_key_field_name().to_string();
        let schema = self.schema_ref();
        let projection = crate::utils::datafusion_dml::dml_scan_projection(
            &schema,
            &filters,
            &[],
            &[&pk_column],
        )?;
        let rls_state = Self::session_state_with_rls_command(state, PolicyCommand::Delete)?;
        let rows = crate::utils::datafusion_dml::collect_matching_rows_with_projection(
            self,
            &rls_state,
            &filters,
            projection.as_ref(),
        )
        .await?;
        if rows.is_empty() {
            return crate::utils::datafusion_dml::rows_affected_plan(state, 0).await;
        }

        let mut seen = HashSet::new();
        let mut deleted: u64 = 0;
        let transaction_query_context = extract_transaction_query_context(state);
        let mut ordinal = 0u32;
        let mut staged_mutations =
            transaction_query_context.map(|_| Vec::with_capacity(rows.len()));
        let mut direct_pks = Vec::new();

        for row in rows {
            let pk_value = crate::utils::datafusion_dml::extract_pk_value(&row, &pk_column)?;
            if !seen.insert(pk_value.clone()) {
                continue;
            }

            if let Some(staged_mutations) = staged_mutations.as_mut() {
                staged_mutations.push(StagedMutation::new(
                    transaction_query_context
                        .expect("transaction_query_context must exist when staging DELETE")
                        .transaction_id
                        .clone(),
                    self.core.table_id().clone(),
                    TableType::Shared,
                    Some(user_id.clone()),
                    OperationKind::Delete,
                    pk_value,
                    Row::new(std::collections::BTreeMap::new()),
                    true,
                ));
                deleted += 1;
                continue;
            }

            direct_pks.push(pk_value);
        }

        if !direct_pks.is_empty() {
            if self.core.services.cluster_coordinator.replicates_shared_writes() {
                let removed = self
                    .core
                    .services
                    .cluster_coordinator
                    .propose_shared_delete(self.core.table_id(), &user_id, direct_pks)
                    .await
                    .map_err(DataFusionError::Execution)?;
                deleted += removed as u64;
            } else {
                let commit_seq = self.core.services.commit_sequence_source.allocate_next();
                for pk_value in direct_pks {
                    let version = version_from_commit_seq(commit_seq, ordinal)
                        .map_err(|error| DataFusionError::Execution(error.to_string()))?;
                    ordinal = ordinal.saturating_add(1);
                    if self
                        .delete_by_pk_value_with_version(user_id, &pk_value, version)
                        .await
                        .map_err(|e| DataFusionError::Execution(e.to_string()))?
                    {
                        deleted += 1;
                    }
                }
            }
        }

        if let (Some(transaction_query_context), Some(staged_mutations)) =
            (transaction_query_context, staged_mutations)
        {
            crate::utils::datafusion_dml::stage_transaction_mutations(
                transaction_query_context,
                staged_mutations,
            )?;
        }

        crate::utils::datafusion_dml::rows_affected_plan(state, deleted).await
    }

    async fn update(
        &self,
        state: &dyn Session,
        assignments: Vec<(String, Expr)>,
        filters: Vec<Expr>,
    ) -> DataFusionResult<Arc<dyn ExecutionPlan>> {
        check_shared_table_write_access(state, self.core.table_def())
            .map_err(session_error_to_datafusion)?;
        crate::utils::datafusion_dml::validate_where_clause(&filters, "UPDATE")?;

        self.ensure_shared_write_route(state).await?;

        let (user_id, role) =
            extract_user_context(state).map_err(|e| DataFusionError::Execution(e.to_string()))?;

        let pk_column = self.primary_key_field_name().to_string();
        crate::utils::datafusion_dml::validate_update_assignments(&assignments, &pk_column)?;

        let check_policies = self
            .bind_policies(user_id, role, PolicyCommand::Update, true)
            .map_err(|error| DataFusionError::Execution(error.to_string()))?;
        let mut required_columns = vec![pk_column.clone()];
        for column_id in check_policies.required_column_ids() {
            if let Some(column) = self
                .core
                .table_def()
                .columns
                .iter()
                .find(|column| column.column_id == column_id)
            {
                if !required_columns.iter().any(|name| name == &column.column_name) {
                    required_columns.push(column.column_name.clone());
                }
            }
        }
        let required_column_refs = required_columns.iter().map(String::as_str).collect::<Vec<_>>();

        let schema = self.schema_ref();
        let projection = crate::utils::datafusion_dml::dml_scan_projection(
            &schema,
            &filters,
            &assignments,
            &required_column_refs,
        )?;
        let rls_state = Self::session_state_with_rls_command(state, PolicyCommand::Update)?;
        let rows = crate::utils::datafusion_dml::collect_matching_rows_with_projection(
            self,
            &rls_state,
            &filters,
            projection.as_ref(),
        )
        .await?;
        if rows.is_empty() {
            return crate::utils::datafusion_dml::rows_affected_plan(state, 0).await;
        }

        let mut seen = HashSet::new();
        let mut updated: u64 = 0;
        let transaction_query_context = extract_transaction_query_context(state);
        let snapshot_commit_seq = transaction_query_context.and_then(|context| {
            crate::utils::base::transaction_snapshot_bound(context.snapshot_commit_seq())
        });
        let check_authorization = self
            .bind_authorization(&check_policies, snapshot_commit_seq)
            .await
            .map_err(|error| DataFusionError::Execution(error.to_string()))?;
        let mut ordinal = 0u32;
        let mut staged_mutations =
            transaction_query_context.map(|_| Vec::with_capacity(rows.len()));
        let mut direct_updates = Vec::new();

        let evaluator = PreparedUpdateAssignments::new(state, &schema, &assignments)?;
        let batch_size = state.config().batch_size().max(1);
        let mut remaining = rows.into_iter();
        loop {
            let batch = remaining.by_ref().take(batch_size).collect::<Vec<_>>();
            if batch.is_empty() {
                break;
            }
            let evaluated = evaluator.evaluate(&batch)?;
            for (row, evaluated_updates) in batch.into_iter().zip(evaluated) {
                let pk_value = crate::utils::datafusion_dml::extract_pk_value(&row, &pk_column)?;
                if !seen.insert(pk_value.clone()) {
                    continue;
                }

                let mut new_row = row.clone();
                for (column, value) in &evaluated_updates.values {
                    new_row.values.insert(column.clone(), value.clone());
                }
                if !check_authorization.authorizes(&new_row) {
                    return Err(DataFusionError::Plan(format!(
                        "row-level security WITH CHECK policy denied UPDATE on {}",
                        self.core.table_id()
                    )));
                }
                if crate::utils::datafusion_dml::update_assignments_noop(
                    &schema,
                    &row,
                    &evaluated_updates,
                )? {
                    continue;
                }

                if let Some(staged_mutations) = staged_mutations.as_mut() {
                    staged_mutations.push(StagedMutation::new(
                        transaction_query_context
                            .expect("transaction_query_context must exist when staging UPDATE")
                            .transaction_id
                            .clone(),
                        self.core.table_id().clone(),
                        TableType::Shared,
                        Some(user_id.clone()),
                        OperationKind::Update,
                        pk_value,
                        evaluated_updates,
                        false,
                    ));
                    updated += 1;
                    continue;
                }

                direct_updates.push((pk_value, evaluated_updates));
            }
        }

        if !direct_updates.is_empty() {
            if self.core.services.cluster_coordinator.replicates_shared_writes() {
                let (pk_values, updates) = direct_updates.into_iter().unzip();
                let changed = self
                    .core
                    .services
                    .cluster_coordinator
                    .propose_shared_update(self.core.table_id(), &user_id, pk_values, updates)
                    .await
                    .map_err(DataFusionError::Execution)?;
                updated += changed as u64;
            } else {
                let commit_seq = self.core.services.commit_sequence_source.allocate_next();
                for (pk_value, evaluated_updates) in direct_updates {
                    let version = version_from_commit_seq(commit_seq, ordinal)
                        .map_err(|error| DataFusionError::Execution(error.to_string()))?;
                    ordinal = ordinal.saturating_add(1);
                    let result = self
                        .update_by_pk_value_with_version(
                            user_id,
                            &pk_value,
                            evaluated_updates,
                            version,
                        )
                        .await
                        .map_err(|e| DataFusionError::Execution(e.to_string()))?;
                    if result.is_some() {
                        updated += 1;
                    }
                }
            }
        }

        if let (Some(transaction_query_context), Some(staged_mutations)) =
            (transaction_query_context, staged_mutations)
        {
            crate::utils::datafusion_dml::stage_transaction_mutations(
                transaction_query_context,
                staged_mutations,
            )?;
        }

        crate::utils::datafusion_dml::rows_affected_plan(state, updated).await
    }

    fn supports_filters_pushdown(
        &self,
        filters: &[&Expr],
    ) -> DataFusionResult<Vec<TableProviderFilterPushDown>> {
        self.base_supports_filters_pushdown(filters)
    }
}

// KalamTableProvider: extends TableProvider with KalamDB-specific DML
#[async_trait]
impl crate::utils::dml_provider::KalamTableProvider for SharedTableProvider {
    async fn insert_rows(
        &self,
        user_id: &UserId,
        rows: Vec<Row>,
        versions: &[VersionId],
    ) -> Result<usize, KalamDbError> {
        self.ensure_shared_write_leader().await?;
        let keys = self.insert_batch(user_id, rows, versions).await?;
        Ok(keys.len())
    }

    async fn update_row_by_pk(
        &self,
        user_id: &UserId,
        pk_value: &str,
        updates: Row,
        version: VersionId,
    ) -> Result<bool, KalamDbError> {
        self.ensure_shared_write_leader().await?;

        match self.update_by_pk_value(user_id, pk_value, updates, version).await {
            Ok(Some(_)) => Ok(true),
            Ok(None) => Ok(false), // no-op: row unchanged
            Err(KalamDbError::NotFound(_)) => Ok(false),
            Err(e) => Err(e),
        }
    }

    async fn delete_row_by_pk(
        &self,
        user_id: &UserId,
        pk_value: &str,
        version: VersionId,
    ) -> Result<bool, KalamDbError> {
        self.ensure_shared_write_leader().await?;

        self.delete_by_pk_value(user_id, pk_value, version).await
    }

    async fn insert_rows_returning(
        &self,
        user_id: &UserId,
        rows: Vec<Row>,
        versions: &[VersionId],
    ) -> Result<Vec<ScalarValue>, KalamDbError> {
        self.ensure_shared_write_leader().await?;
        let keys = self.insert_batch(user_id, rows, versions).await?;
        Ok(keys.into_iter().map(|k| ScalarValue::Int64(Some(k.as_i64()))).collect())
    }
}

impl SharedTableProvider {
    async fn update_by_pk_value_with_version(
        &self,
        _user_id: &UserId,
        pk_value: &str,
        updates: Row,
        version: VersionId,
    ) -> Result<Option<SharedTableRowId>, KalamDbError> {
        let span = tracing::debug_span!(
            "table.update",
            table_id = %self.core.table_id(),
            scope = "shared",
            pk = pk_value,
            update_columns = updates.values.len()
        );
        async move {
            let _authorization_mutation = self.begin_authorization_mutation();
            // IGNORE user_id parameter - no RLS for shared tables
            let pk_name = self.primary_key_field_name().to_string();

            // Get PK column data type from schema for proper type coercion
            let schema = self.schema();
            let pk_field = schema.field_with_name(&pk_name).map_err(|e| {
                KalamDbError::InvalidOperation(format!(
                    "PK column '{}' not found in schema: {}",
                    pk_name, e
                ))
            })?;
            let pk_column_type = pk_field.data_type();

            // Convert string PK value to proper ScalarValue based on column type
            let pk_value_scalar = parse_string_as_scalar(pk_value, pk_column_type)
                .map_err(|e| KalamDbError::InvalidOperation(e))?;

            // Resolve latest per PK - first try hot storage (O(1) via PK index),
            // then fall back to cold storage (Parquet scan)
            let (_latest_key, latest_row) =
                if let Some(result) = self.find_by_pk(&pk_value_scalar).await? {
                    result
                } else if self.pk_tombstoned_in_hot(&pk_value_scalar).await? {
                    return Err(KalamDbError::NotFound(format!(
                        "Row with {}={} was deleted",
                        pk_name, pk_value
                    )));
                } else {
                    // Not in hot storage, check cold storage
                    log::debug!(
                        "[UPDATE] PK {} not found in hot storage, querying cold storage for pk={}",
                        pk_name,
                        pk_value
                    );
                    base::find_row_by_pk(self, None, pk_value).await?.ok_or_else(|| {
                        KalamDbError::NotFound(format!(
                            "Row with {}={} not found (checked both hot and cold storage)",
                            pk_name, pk_value
                        ))
                    })?
                };

            // Coerce update values to match schema types (e.g., Utf8 → TimestampMicrosecond).
            // Without this, the no-op comparison would fail for any column where the
            // SQL literal type differs from the stored Arrow type (TIMESTAMP, INT, etc.).
            let coerced = coerce_updates(updates, &self.schema_ref()).map_err(|e| {
                KalamDbError::InvalidOperation(format!("Schema coercion failed: {}", e))
            })?;

            // Merge coerced updates onto latest
            let mut merged = latest_row.fields.values.clone();
            for (k, v) in coerced.values {
                merged.insert(k, v);
            }
            let new_fields = Row::new(merged);

            // Skip write if the merged row is identical to the existing row.
            // Like PostgreSQL / MySQL, a no-op UPDATE should not create a new
            // MVCC version, fire notifications, or count as a row affected.
            if new_fields == latest_row.fields {
                tracing::debug!(
                    table_id = %self.core.table_id(),
                    pk = pk_value,
                    "table.update_noop: row unchanged, skipping write"
                );
                return Ok(None);
            }

            let seq_id = version;

            let entity = SharedTableRow {
                _version: seq_id,
                _deleted: false,
                fields:   new_fields,
            };
            let row_key = seq_id;
            // Use insert() to update PK index for the new MVCC version
            self.store.insert_async(row_key, entity.clone()).await.map_err(|e| {
                KalamDbError::InvalidOperation(format!("Failed to update shared table row: {}", e))
            })?;

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

            // Fire topic/CDC notification (UPDATE). User is actor metadata only;
            // live fanout remains shared-scoped.
            let notification_service = self.core.services.notification_service.clone();
            let table_id = self.core.table_id().clone();

            let has_topics = self.core.has_topic_routes(&table_id);
            let has_live_subs = notification_service.has_subscribers(None, &table_id);
            if has_topics || has_live_subs {
                let new_row = Self::build_notification_row(&entity);
                if has_topics {
                    self.core
                        .publish_to_topics(
                            &table_id,
                            kalamdb_commons::models::TopicOp::Update,
                            &new_row,
                            Some(_user_id),
                        )
                        .await;
                }
                if has_live_subs {
                    let old_row = Self::build_notification_row(&latest_row);
                    let pk_col = self.primary_key_field_name().to_string();
                    let notification = ChangeNotification::update(
                        table_id.clone(),
                        old_row,
                        new_row,
                        vec![pk_col],
                    );
                    notification_service.notify_table_change(None, table_id, notification);
                }
            }

            Ok(Some(row_key))
        }
        .instrument(span)
        .await
    }

    async fn delete_by_pk_value_with_version(
        &self,
        _user_id: &UserId,
        pk_value: &str,
        version: VersionId,
    ) -> Result<bool, KalamDbError> {
        let span = tracing::debug_span!(
            "table.delete",
            table_id = %self.core.table_id(),
            scope = "shared",
            pk = pk_value
        );
        async move {
            let _authorization_mutation = self.begin_authorization_mutation();
            // IGNORE user_id parameter - no RLS for shared tables
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

            // Find latest resolved row for this PK
            // First try hot storage (O(1) via PK index), then fall back to cold storage (Parquet
            // scan)
            let latest_row = if let Some((_key, row)) = self.find_by_pk(&pk_value_scalar).await? {
                row
            } else if self.pk_tombstoned_in_hot(&pk_value_scalar).await? {
                return Ok(false);
            } else {
                // Not in hot storage, check cold storage
                match base::find_row_by_pk(self, None, pk_value).await? {
                    Some((_key, row)) => row,
                    None => {
                        log::trace!(
                            "[SharedProvider DELETE_BY_PK] Row with {}={} not found",
                            pk_name,
                            pk_value
                        );
                        return Ok(false);
                    },
                }
            };

            let seq_id = version;

            // Preserve ALL fields in the tombstone
            let values = latest_row.fields.values.clone();

            let entity = SharedTableRow {
                _version: seq_id,
                _deleted: true,
                fields:   Row::new(values),
            };
            let row_key = seq_id;
            log::debug!(
                "[SharedProvider DELETE_BY_PK] Writing tombstone: pk={}, _seq={}",
                pk_value,
                seq_id.as_i64()
            );
            // Use insert() to update PK index for the tombstone record
            self.store.insert_async(row_key, entity.clone()).await.map_err(|e| {
                KalamDbError::InvalidOperation(format!("Failed to delete shared table row: {}", e))
            })?;

            if let Err(e) = self.stage_vector_delete(seq_id, pk_value).await {
                log::warn!(
                    "Failed to stage vector delete for table={}, seq={}, pk={}: {}",
                    self.core.table_id(),
                    seq_id.as_i64(),
                    pk_value,
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

            // Fire topic/CDC notification (DELETE). User is actor metadata only;
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
                            kalamdb_commons::models::TopicOp::Delete,
                            &row,
                            Some(_user_id),
                        )
                        .await;
                }
                if has_live_subs {
                    let notification = self.build_delete_notification(table_id.clone(), row);
                    notification_service.notify_table_change(None, table_id, notification);
                }
            }

            Ok(true)
        }
        .instrument(span)
        .await
    }
}
