struct StreamScanSource {
    core: Arc<TableProviderCore>,
    store: Arc<StreamTableStore>,
    ttl_seconds: Option<u64>,
    user_id: UserId,
    descriptor: ScanDescriptor,
    filter: Option<Expr>,
    physical_filter: Option<Arc<dyn PhysicalExpr>>,
    output_projection: Option<Vec<usize>>,
    output_schema: SchemaRef,
}

impl std::fmt::Debug for StreamScanSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StreamScanSource")
            .field("table_id", &self.core.table_id())
            .field("user_id", &self.user_id)
            .field("ttl_seconds", &self.ttl_seconds)
            .field("projected_columns", &self.descriptor.projection.as_ref().map(|p| p.len()))
            .field("filter_count", &self.descriptor.filters.len())
            .finish()
    }
}

impl StreamScanSource {
    async fn produce_output(
        &self,
        include_diagnostics: bool,
    ) -> DataFusionResult<DeferredBatchOutput> {
        let since_seq = self
            .filter
            .as_ref()
            .map(extract_seq_bounds_from_filter)
            .map(|(since_seq, _until_seq)| since_seq)
            .unwrap_or(None);

        // LIMIT pushdown remains disabled here because scan-time ordering is not
        // enough to prove semantic safety when ORDER BY may execute above the scan.
        let ttl_ms = self.ttl_seconds.map(|seconds| seconds * 1000);
        let now_ms = StreamTableEngine::now_millis().map_err(|error| {
            DataFusionError::Execution(format!("stream scan clock failed: {error}"))
        })?;

        let start_seq = since_seq
            .map(|seq| VersionId::try_from_i64(seq.as_i64().saturating_add(1)).unwrap_or(seq));
        let store = Arc::clone(&self.store);
        let user_id = self.user_id.clone();
        let schema = self.core.schema_ref();
        let projection = self.descriptor.projection.as_ref().map(|indices| indices.to_vec());
        let include_user_id = schema.field_with_name("user_id").is_ok();
        let batches = tokio::task::spawn_blocking(move || {
            let mut batches = Vec::new();
            let mut scanned = 0usize;
            store.for_each_user_streaming_page(
                &user_id,
                start_seq,
                ttl_ms,
                now_ms,
                8_192,
                None,
                |page| {
                    scanned += page.len();
                    let batch = crate::utils::base::rows_to_arrow_batch(
                        &schema,
                        page,
                        projection.as_ref(),
                        |row_values, row| {
                            if include_user_id {
                                row_values.values.insert(
                                    "user_id".to_string(),
                                    ScalarValue::Utf8(Some(row.user_id.as_str().to_string())),
                                );
                            }
                        },
                    )
                    .map_err(|error| {
                        kalamdb_store::storage_trait::StorageError::IoError(error.to_string())
                    })?;
                    batches.push(batch);
                    Ok(true)
                },
            )?;
            if batches.is_empty() {
                batches.push(
                    crate::utils::base::rows_to_arrow_batch(
                        &schema,
                        Vec::<(
                            kalamdb_commons::ids::StreamTableRowId,
                            kalamdb_commons::models::StreamTableRow,
                        )>::new(),
                        projection.as_ref(),
                        |_: &mut kalamdb_commons::models::rows::Row, _| {},
                    )
                    .map_err(|error| {
                        kalamdb_store::storage_trait::StorageError::IoError(error.to_string())
                    })?,
                );
            }
            Ok::<_, kalamdb_store::storage_trait::StorageError>((batches, scanned))
        })
        .await
        .map_err(|error| {
            DataFusionError::Execution(format!("stream scan task failed: {error}"))
        })?
        .map_err(|error| {
            DataFusionError::Execution(format!(
                "failed to scan stream table hot storage: {error}"
            ))
        })?;
        let hot_rows_scanned = batches.1;
        let mut batches = batches.0;
        let batch = batches.remove(0);
        let follow = batches;

        let batch = finalize_deferred_batch(
            batch,
            self.physical_filter.as_ref(),
            self.output_projection.as_deref(),
            None,
            self.source_name(),
        )?;
        let follow = follow
            .into_iter()
            .map(|chunk| {
                finalize_deferred_batch(
                    chunk,
                    self.physical_filter.as_ref(),
                    self.output_projection.as_deref(),
                    None,
                    self.source_name(),
                )
            })
            .collect::<DataFusionResult<Vec<_>>>()?;

        let mut output = DeferredBatchOutput::new(batch).with_follow(follow);
        if include_diagnostics {
            output = output.with_diagnostics(DeferredScanDiagnostics {
                hot_rows_scanned: Some(hot_rows_scanned),
                cold_rows_scanned: Some(0),
                cold_files_total: Some(0),
                cold_files_skipped: Some(0),
                cold_files_scanned: Some(0),
                cold_files: Vec::new(),
                access_path: Some(ScanAccess::Sequential),
            });
        }
        Ok(output)
    }
}

#[async_trait]
impl DeferredBatchSource for StreamScanSource {
    fn source_name(&self) -> &'static str {
        "stream_table_scan"
    }

    fn plan_details(&self) -> Option<String> {
        Some("storage_tiers=[hot=rocksdb], stream=true".to_string())
    }

    fn schema(&self) -> SchemaRef {
        Arc::clone(&self.output_schema)
    }

    async fn produce_batch(&self) -> DataFusionResult<RecordBatch> {
        kalamdb_datafusion_sources::exec::flatten_deferred_output(self.produce_output(false).await?)
    }

    async fn load_batch_output(&self) -> DataFusionResult<DeferredBatchOutput> {
        self.produce_output(false).await
    }

    async fn produce_batch_with_diagnostics(&self) -> DataFusionResult<DeferredBatchOutput> {
        self.produce_output(true).await
    }
}
