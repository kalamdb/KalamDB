// Manual Debug to satisfy DataFusion's TableProvider: Debug bound
impl std::fmt::Debug for StreamTableProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let table_id = self.core.table_id_arc();
        f.debug_struct("StreamTableProvider")
            .field("table_id", &table_id)
            .field("table_type", &self.core.table_type())
            .field("ttl_seconds", &self.ttl_seconds)
            .field("primary_key_field_name", &self.core.primary_key_field_name())
            .finish()
    }
}

// Implement DataFusion TableProvider trait
#[async_trait]
impl TableProvider for StreamTableProvider {
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
        self.validate_transaction_table_access(state)?;
        self.ensure_leader_read(state)
            .await
            .map_err(|error| DataFusionError::Execution(error.to_string()))?;

        let (user_id, _role) = extract_user_context(state)
            .map_err(|error| DataFusionError::Execution(error.to_string()))?;

        let descriptor = self.scan_descriptor(projection, filters, limit);
        let combined_filter = combined_filter(filters);
        let merged_schema = match descriptor.projection.as_ref() {
            Some(indices) => descriptor
                .schema
                .project(indices)
                .map(Arc::new)
                .map_err(|error| DataFusionError::ArrowError(Box::new(error), None))?,
            None => Arc::clone(&descriptor.schema),
        };

        let output_projection = if !filters.is_empty() {
            projection.map(|indices| {
                remap_projection_indices(&descriptor.schema, &merged_schema, indices)
            })
        } else {
            None
        };
        let output_schema = match projection {
            Some(indices) => descriptor
                .schema
                .project(indices)
                .map(Arc::new)
                .map_err(|error| DataFusionError::ArrowError(Box::new(error), None))?,
            None => Arc::clone(&descriptor.schema),
        };
        let physical_filter = if let Some(filter) = combined_filter.clone() {
            let df_schema = DFSchema::try_from(Arc::clone(&merged_schema))?;
            Some(state.create_physical_expr(filter, &df_schema)?)
        } else {
            None
        };

        let source = Arc::new(StreamScanSource {
            core: Arc::clone(&self.core),
            store: Arc::clone(&self.store),
            ttl_seconds: self.ttl_seconds,
            user_id: user_id.clone(),
            descriptor,
            filter: combined_filter,
            physical_filter,
            output_projection,
            output_schema,
        });

        if scan_diagnostics_enabled(state) {
            Ok(Arc::new(DeferredBatchExec::new_with_scan_diagnostics(source)))
        } else {
            Ok(Arc::new(DeferredBatchExec::new(source)))
        }
    }

    fn supports_filters_pushdown(
        &self,
        filters: &[&Expr],
    ) -> DataFusionResult<Vec<TableProviderFilterPushDown>> {
        Ok(pushdown_results_for_filters(filters, |filter| self.filter_capability(filter)))
    }

    async fn insert_into(
        &self,
        state: &dyn Session,
        input: Arc<dyn ExecutionPlan>,
        insert_op: InsertOp,
    ) -> DataFusionResult<Arc<dyn ExecutionPlan>> {
        check_user_table_write_access(state, self.core.table_id())
            .map_err(session_error_to_datafusion)?;

        if insert_op != InsertOp::Append {
            return Err(DataFusionError::Plan(format!(
                "{} is not supported for stream tables",
                insert_op
            )));
        }

        let (user_id, _role) =
            extract_user_context(state).map_err(|e| DataFusionError::Execution(e.to_string()))?;

        let rows = crate::utils::datafusion_dml::collect_input_rows(state, input).await?;
        let mut versions = Vec::with_capacity(rows.len());
        for _ in 0..rows.len() {
            let sequence = self.core.services.commit_sequence_source.allocate_next();
            let version = VersionId::try_from_local_sequence(sequence)
                .map_err(|error| DataFusionError::Execution(error.to_string()))?;
            versions.push(version);
        }
        let inserted = self
            .insert_batch(&user_id, rows, &versions)
            .await
            .map_err(|error| DataFusionError::Execution(error.to_string()))?;
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
        let rows = crate::utils::datafusion_dml::collect_matching_rows_with_projection(
            self,
            state,
            &filters,
            projection.as_ref(),
        )
        .await?;
        if rows.is_empty() {
            return crate::utils::datafusion_dml::rows_affected_plan(state, 0).await;
        }

        let mut seen = HashSet::new();
        let mut deleted: u64 = 0;

        for row in rows {
            let pk_value = crate::utils::datafusion_dml::extract_pk_value(&row, &pk_column)?;
            if !seen.insert(pk_value.clone()) {
                continue;
            }

            if self
                .delete_by_pk_value(user_id, &pk_value, VersionId::try_from_i64(1).expect("placeholder"))
                .await
                .map_err(|e| DataFusionError::Execution(e.to_string()))?
            {
                deleted += 1;
            }
        }

        crate::utils::datafusion_dml::rows_affected_plan(state, deleted).await
    }

    async fn update(
        &self,
        _state: &dyn Session,
        _assignments: Vec<(String, Expr)>,
        _filters: Vec<Expr>,
    ) -> DataFusionResult<Arc<dyn ExecutionPlan>> {
        Err(DataFusionError::Plan("UPDATE not supported for STREAM tables".to_string()))
    }
}

// KalamTableProvider: extends TableProvider with KalamDB-specific DML
#[async_trait]
impl crate::utils::dml_provider::KalamTableProvider for StreamTableProvider {
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
}
