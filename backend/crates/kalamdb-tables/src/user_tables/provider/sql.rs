// Manual Debug to satisfy DataFusion's TableProvider: Debug bound
impl std::fmt::Debug for UserTableProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UserTableProvider")
            .field("table_id", self.core.table_id())
            .field("table_type", &self.core.table_type())
            .field("primary_key_field_name", &self.core.primary_key_field_name())
            .finish()
    }
}

// Implement DataFusion TableProvider trait
#[async_trait]
impl TableProvider for UserTableProvider {
    fn schema(&self) -> SchemaRef {
        self.schema_ref()
    }

    fn table_type(&self) -> datafusion::logical_expr::TableType {
        datafusion::logical_expr::TableType::Base
    }

    fn get_column_default(&self, column: &str) -> Option<&Expr> {
        self.core.get_column_default(column)
    }

    fn supports_filters_pushdown(
        &self,
        filters: &[&Expr],
    ) -> DataFusionResult<Vec<TableProviderFilterPushDown>> {
        self.base_supports_filters_pushdown(filters)
    }

    async fn scan(
        &self,
        state: &dyn Session,
        projection: Option<&Vec<usize>>,
        filters: &[Expr],
        limit: Option<usize>,
    ) -> DataFusionResult<Arc<dyn ExecutionPlan>> {
        // SECURITY: Enforce user table access rules (namespace isolation)
        check_user_table_access(state, self.core.table_id())
            .map_err(session_error_to_datafusion)?;

        let table_overlay = extract_transaction_query_context(state)
            .and_then(|context| context.overlay_view.overlay_for_table(self.core.table_id()));
        let (user_id, _role) = extract_user_context(state)
            .map_err(crate::error::into_datafusion)?;

        self.base_scan_with_overlay(
            state,
            projection,
            filters,
            limit,
            table_overlay,
            Some(user_id.clone()),
        )
        .await
    }

    async fn insert_into(
        &self,
        state: &dyn Session,
        input: Arc<dyn ExecutionPlan>,
        insert_op: InsertOp,
    ) -> DataFusionResult<Arc<dyn ExecutionPlan>> {
        tracing::debug!(table_id = %self.core.table_id(), "table.insert_into");
        check_user_table_write_access(state, self.core.table_id())
            .map_err(session_error_to_datafusion)?;

        if insert_op != InsertOp::Append {
            return Err(DataFusionError::Plan(format!(
                "{} is not supported for user tables",
                insert_op
            )));
        }

        let (user_id, _role) =
            extract_user_context(state).map_err(|e| DataFusionError::Execution(e.to_string()))?;

        let rows = crate::utils::datafusion_dml::collect_input_rows(state, input).await?;
        if let Some(transaction_query_context) = extract_transaction_query_context(state) {
            let inserted = crate::utils::datafusion_dml::stage_insert_rows(
                transaction_query_context,
                self.core.table_id(),
                TableType::User,
                Some(user_id.clone()),
                self.primary_key_field_name(),
                rows,
            )?;

            return crate::utils::datafusion_dml::rows_affected_plan(state, inserted).await;
        }

        let versions = crate::utils::base::direct_insert_versions(
            self.core.services.commit_sequence_source.allocate_next(),
            rows.len(),
        )
        .map_err(crate::error::into_datafusion)?;
        let inserted = self
            .insert_batch_with_versions(&user_id, rows, &versions)
            .await
            .map_err(crate::error::into_datafusion)?;
        crate::utils::datafusion_dml::rows_affected_plan(state, inserted.len() as u64).await
    }

    async fn delete_from(
        &self,
        state: &dyn Session,
        filters: Vec<Expr>,
    ) -> DataFusionResult<Arc<dyn ExecutionPlan>> {
        check_user_table_write_access(state, self.core.table_id())
            .map_err(session_error_to_datafusion)?;
        crate::utils::datafusion_dml::validate_where_clause(&filters, "DELETE")?;

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
        let rows = self
            .collect_matching_rows_for_subject(state, &filters, projection.as_ref())
            .await?;
        if rows.is_empty() {
            return crate::utils::datafusion_dml::rows_affected_plan(state, 0).await;
        }

        let transaction_query_context = extract_transaction_query_context(state);

        let mut seen = HashSet::new();
        let mut deleted: u64 = 0;
        let commit_seq = transaction_query_context
            .is_none()
            .then(|| self.core.services.commit_sequence_source.allocate_next());
        let mut ordinal = 0u32;
        let mut staged_mutations =
            transaction_query_context.map(|_| Vec::with_capacity(rows.len()));

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
                    TableType::User,
                    Some(user_id.clone()),
                    OperationKind::Delete,
                    pk_value,
                    Row::new(std::collections::BTreeMap::new()),
                    true,
                ));
                deleted += 1;
                continue;
            }

            let version = version_from_commit_seq(
                commit_seq.expect("commit_seq must exist for direct DELETE"),
                ordinal,
            )
            .map_err(crate::error::into_datafusion)?;
            ordinal = ordinal.saturating_add(1);
            if self
                .delete_by_pk_value_with_version(user_id, &pk_value, version)
                .await
                .map_err(crate::error::into_datafusion)?
            {
                deleted += 1;
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
        check_user_table_write_access(state, self.core.table_id())
            .map_err(session_error_to_datafusion)?;
        crate::utils::datafusion_dml::validate_where_clause(&filters, "UPDATE")?;

        let pk_column = self.primary_key_field_name().to_string();
        crate::utils::datafusion_dml::validate_update_assignments(&assignments, &pk_column)?;

        let (user_id, _role) =
            extract_user_context(state).map_err(|e| DataFusionError::Execution(e.to_string()))?;

        let schema = self.schema_ref();
        let projection = crate::utils::datafusion_dml::dml_scan_projection(
            &schema,
            &filters,
            &assignments,
            &[&pk_column],
        )?;
        let rows = self
            .collect_matching_rows_for_subject(state, &filters, projection.as_ref())
            .await?;
        if rows.is_empty() {
            return crate::utils::datafusion_dml::rows_affected_plan(state, 0).await;
        }

        let transaction_query_context = extract_transaction_query_context(state);

        let mut seen = HashSet::new();
        let mut updated: u64 = 0;
        let commit_seq = transaction_query_context
            .is_none()
            .then(|| self.core.services.commit_sequence_source.allocate_next());
        let mut ordinal = 0u32;
        let mut staged_mutations =
            transaction_query_context.map(|_| Vec::with_capacity(rows.len()));

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
                        TableType::User,
                        Some(user_id.clone()),
                        OperationKind::Update,
                        pk_value,
                        evaluated_updates,
                        false,
                    ));
                    updated += 1;
                    continue;
                }

                let version = version_from_commit_seq(
                    commit_seq.expect("commit_seq must exist for direct UPDATE"),
                    ordinal,
                )
                .map_err(crate::error::into_datafusion)?;
                ordinal = ordinal.saturating_add(1);
                let result = self
                    .update_by_pk_value_with_version(user_id, &pk_value, evaluated_updates, version)
                    .await
                    .map_err(crate::error::into_datafusion)?;
                if result.is_some() {
                    updated += 1;
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
}

// KalamTableProvider: extends TableProvider with KalamDB-specific DML
#[async_trait]
impl crate::utils::dml_provider::KalamTableProvider for UserTableProvider {
    async fn insert_rows(
        &self,
        user_id: &UserId,
        rows: Vec<Row>,
        versions: &[VersionId],
    ) -> Result<usize, KalamDbError> {
        let keys = self.insert_batch(user_id, rows, versions).await?;
        Ok(keys.len())
    }

    async fn insert_rows_returning(
        &self,
        user_id: &UserId,
        rows: Vec<Row>,
        versions: &[VersionId],
    ) -> Result<Vec<ScalarValue>, KalamDbError> {
        let keys = self.insert_batch(user_id, rows, versions).await?;
        Ok(keys.into_iter().map(|k| ScalarValue::Int64(Some(k.version.as_i64()))).collect())
    }

    async fn update_row_by_pk(
        &self,
        user_id: &UserId,
        pk_value: &str,
        updates: Row,
        version: VersionId,
    ) -> Result<bool, KalamDbError> {
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
        self.delete_by_pk_value(user_id, pk_value, version).await
    }
}

impl UserTableProvider {
    async fn update_by_pk_value_with_version(
        &self,
        user_id: &UserId,
        pk_value: &str,
        updates: Row,
        version: VersionId,
    ) -> Result<Option<UserTableRowId>, KalamDbError> {
        let span = tracing::debug_span!(
            "table.update",
            table_id = %self.core.table_id(),
            user_id = %user_id.as_str(),
            pk = pk_value,
            update_columns = updates.values.len()
        );
        async move {
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
            // Find latest resolved row for this PK under same user
            // First try hot storage (O(1) via PK index), then fall back to cold storage (Parquet
            // scan)
            let (_latest_key, latest_row) =
                if let Some(result) = self.find_by_pk(user_id, &pk_value_scalar).await? {
                    result
                } else if self.pk_tombstoned_in_hot(user_id, &pk_value_scalar).await? {
                    return Err(KalamDbError::NotFound(format!(
                        "Row with {}={} was deleted",
                        pk_name, pk_value
                    )));
                } else {
                    // Not in hot storage, check cold storage
                    if log::log_enabled!(log::Level::Debug) {
                        log::debug!(
                            "[UPDATE] PK {} not found in hot storage, querying cold storage for \
                             user={}, pk={}",
                            pk_name,
                            user_id.as_str(),
                            pk_value
                        );
                    }
                    base::find_row_by_pk(self, Some(user_id), pk_value).await?.ok_or_else(|| {
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
                // if log::log_enabled!(log::Level::Debug) {
                //     log::debug!(
                //         table_id = %self.core.table_id(),
                //         pk = pk_value,
                //         "table.update_noop: row unchanged, skipping write"
                //     );
                // }
                return Ok(None);
            }

            // VALIDATE NOT NULL CONSTRAINTS on the merged row (per ADR-016)
            crate::utils::datafusion_dml::validate_not_null_with_set(
                self.core.non_null_columns(),
                &[new_fields.clone()],
            )?;

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

            // Fire live query + topic notification (UPDATE)
            let notification_service = self.core.services.notification_service.clone();
            let table_id = self.core.table_id().clone();

            let has_topics = self.core.has_topic_routes(&table_id);
            let has_live_subs = notification_service.has_subscribers(Some(&user_id), &table_id);
            if has_topics || has_live_subs {
                let new_row = Self::build_notification_row(&entity);
                if has_topics {
                    self.core
                        .publish_to_topics(
                            &table_id,
                            kalamdb_commons::models::TopicOp::Update,
                            &new_row,
                            Some(&user_id),
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
                    notification_service.notify_table_change(
                        Some(user_id.clone()),
                        table_id,
                        notification,
                    );
                }
            }
            Ok(Some(row_key))
        }
        .instrument(span)
        .await
    }

    async fn delete_by_pk_value_with_version(
        &self,
        user_id: &UserId,
        pk_value: &str,
        version: VersionId,
    ) -> Result<bool, KalamDbError> {
        let span = tracing::debug_span!(
            "table.delete",
            table_id = %self.core.table_id(),
            user_id = %user_id.as_str(),
            pk = pk_value
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

            // Find latest resolved row for this PK under same user
            // First try hot storage (O(1) via PK index), then fall back to cold storage (Parquet
            // scan)
            let latest_row =
                if let Some((_key, row)) = self.find_by_pk(user_id, &pk_value_scalar).await? {
                    row
                } else if self.pk_tombstoned_in_hot(user_id, &pk_value_scalar).await? {
                    return Ok(false);
                } else {
                    // Not in hot storage, check cold storage
                    match base::find_row_by_pk(self, Some(user_id), pk_value).await? {
                        Some((_key, row)) => row,
                        None => {
                            log::trace!(
                                "[UserProvider DELETE_BY_PK] Row with {}={} not found",
                                pk_name,
                                pk_value
                            );
                            return Ok(false);
                        },
                    }
                };

            let seq_id = version;

            // Preserve ALL fields in the tombstone so they can be queried if _deleted=true
            // This allows "undo" functionality and auditing of deleted records
            let values = latest_row.fields.values.clone();

            let entity = UserTableRow {
                user_id:  user_id.clone(),
                _version: seq_id,
                _deleted: true,
                fields:   Row::new(values),
            };
            let row_key = UserTableRowId::new(user_id.clone(), seq_id);

            if log::log_enabled!(log::Level::Debug) {
                log::debug!(
                    "[UserProvider DELETE_BY_PK] Writing tombstone: user={}, pk={}, _seq={}",
                    user_id.as_str(),
                    pk_value,
                    seq_id.as_i64()
                );
            }
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

            // Fire live query + topic notification (DELETE soft)
            let notification_service = self.core.services.notification_service.clone();
            let table_id = self.core.table_id().clone();

            let has_topics = self.core.has_topic_routes(&table_id);
            let has_live_subs = notification_service.has_subscribers(Some(&user_id), &table_id);
            if has_topics || has_live_subs {
                // Provide tombstone entity with system columns for filter matching
                let row = Self::build_notification_row(&entity);
                if has_topics {
                    self.core
                        .publish_to_topics(
                            &table_id,
                            kalamdb_commons::models::TopicOp::Delete,
                            &row,
                            Some(&user_id),
                        )
                        .await;
                }
                if has_live_subs {
                    let notification = ChangeNotification::delete_soft(table_id.clone(), row);
                    notification_service.notify_table_change(
                        Some(user_id.clone()),
                        table_id,
                        notification,
                    );
                }
            }
            Ok(true)
        }
        .instrument(span)
        .await
    }
}
