struct StreamScanSource {
    core:              Arc<TableProviderCore>,
    store:             Arc<StreamTableStore>,
    ttl_seconds:       Option<u64>,
    user_id:           UserId,
    descriptor:        ScanDescriptor,
    filter:            Option<Expr>,
    physical_filter:   Option<Arc<dyn PhysicalExpr>>,
    output_projection: Option<Vec<usize>>,
    output_schema:     SchemaRef,
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
        let scan_limit = None;
        let ttl_ms = self.ttl_seconds.map(|seconds| seconds * 1000);
        let now_ms = StreamTableProvider::now_millis().map_err(|error| {
            DataFusionError::Execution(format!("stream scan clock failed: {error}"))
        })?;

        let start_seq = since_seq.map(|seq| VersionId::try_from_i64(seq.as_i64().saturating_add(1)).unwrap_or(seq));
        let results = self
            .store
            .scan_user_streaming_async(
                &self.user_id,
                start_seq,
                scan_limit.unwrap_or(100_000),
                ttl_ms,
                now_ms,
            )
            .await
            .map_err(|error| {
                DataFusionError::Execution(format!(
                    "failed to scan stream table hot storage: {error}"
                ))
            })?;
        let hot_rows_scanned = results.len();

        let batch = crate::utils::base::rows_to_arrow_batch(
            &self.core.schema_ref(),
            results,
            self.descriptor.projection.as_ref().map(|indices| indices.to_vec()).as_ref(),
            |row_values, row| {
                if self.core.schema_ref().field_with_name("user_id").is_ok() {
                    row_values.values.insert(
                        "user_id".to_string(),
                        ScalarValue::Utf8(Some(row.user_id.as_str().to_string())),
                    );
                }
            },
        )
        .map_err(crate::error::into_datafusion)?;

        let batch = finalize_deferred_batch(
            batch,
            self.physical_filter.as_ref(),
            self.output_projection.as_deref(),
            None,
            self.source_name(),
        )?;

        let mut output = DeferredBatchOutput::new(batch);
        if include_diagnostics {
            output = output.with_diagnostics(DeferredScanDiagnostics {
                hot_rows_scanned:   Some(hot_rows_scanned),
                cold_rows_scanned:  Some(0),
                cold_files_total:   Some(0),
                cold_files_skipped: Some(0),
                cold_files_scanned: Some(0),
                cold_files:         Vec::new(),
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
        Ok(self.produce_output(false).await?.batch)
    }

    async fn produce_batch_with_diagnostics(&self) -> DataFusionResult<DeferredBatchOutput> {
        self.produce_output(true).await
    }
}
