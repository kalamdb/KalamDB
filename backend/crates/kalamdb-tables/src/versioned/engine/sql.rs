//! DataFusion provider for both versioned scopes.

use std::{collections::HashSet, sync::Arc};

use async_trait::async_trait;
use datafusion::{
    arrow::datatypes::SchemaRef,
    catalog::Session,
    datasource::TableProvider,
    error::{DataFusionError, Result as DataFusionResult},
    execution::context::SessionState,
    logical_expr::{dml::InsertOp, Expr, TableProviderFilterPushDown},
    physical_plan::ExecutionPlan,
    scalar::ScalarValue,
};
use kalamdb_commons::{
    ids::VersionId,
    models::{rows::Row, Role, UserId},
    schemas::TableType,
    PolicyCommand,
};
use kalamdb_datafusion_sources::dml::PreparedUpdateAssignments;
use kalamdb_rls::BoundAuthorization;
use kalamdb_session_datafusion::{
    check_shared_table_access, check_shared_table_write_access, check_user_table_access,
    check_user_table_write_access, session_error_to_datafusion,
};
use kalamdb_transactions::extract_transaction_query_context;

use crate::{
    error::KalamDbError,
    utils::{
        base::BaseTableProvider, datafusion_dml::UpdateChange, dml_provider::TableRowDml,
        row_utils::extract_user_context,
    },
};

use super::{
    provider::VersionedTable,
    scope::{MutationGuard, SharedScope, TableScope, UserScope},
};

#[async_trait]
pub(crate) trait SqlHooks: Send + Sync {
    fn admit_read(&self, state: &dyn Session) -> DataFusionResult<()>;
    fn admit_write(&self, state: &dyn Session) -> DataFusionResult<()>;
    async fn ensure_sql_write_route(&self, _state: &dyn Session) -> DataFusionResult<()> {
        Ok(())
    }
    async fn authorize_sql_insert(
        &self,
        _state: &dyn Session,
        _user_id: &UserId,
        _role: Role,
        _rows: &[Row],
    ) -> DataFusionResult<()> {
        Ok(())
    }
    fn replicates_shared_writes(&self) -> bool {
        false
    }
    async fn before_direct_write(&self) -> Result<(), KalamDbError> {
        Ok(())
    }
    async fn update_gate(
        &self,
        _user_id: &UserId,
        _role: Role,
        pk_column: &str,
        _snapshot: Option<VersionId>,
    ) -> DataFusionResult<(Vec<String>, Option<BoundAuthorization>)> {
        Ok((vec![pk_column.to_string()], None))
    }
    fn rls_state(
        &self,
        _state: &dyn Session,
        _command: PolicyCommand,
    ) -> DataFusionResult<Option<SessionState>> {
        Ok(None)
    }
    async fn collect_delete_keys(
        &self,
        state: &dyn Session,
        role: Role,
        filters: &[Expr],
        pk_column: &str,
    ) -> DataFusionResult<Vec<String>>;
}

#[async_trait]
impl SqlHooks for VersionedTable<UserScope> {
    fn admit_read(&self, state: &dyn Session) -> DataFusionResult<()> {
        check_user_table_access(state, self.core.table_id()).map_err(session_error_to_datafusion)
    }

    fn admit_write(&self, state: &dyn Session) -> DataFusionResult<()> {
        check_user_table_write_access(state, self.core.table_id())
            .map_err(session_error_to_datafusion)
    }

    async fn collect_delete_keys(
        &self,
        state: &dyn Session,
        _role: Role,
        filters: &[Expr],
        pk_column: &str,
    ) -> DataFusionResult<Vec<String>> {
        let schema = self.schema_ref();
        let projection = kalamdb_datafusion_sources::dml::dml_scan_projection(
            &schema,
            filters,
            &[],
            &[pk_column],
        )?;
        crate::utils::datafusion_dml::collect_matching_primary_keys(
            self,
            state,
            filters,
            projection.as_ref(),
            pk_column,
        )
        .await
    }
}

#[async_trait]
impl SqlHooks for VersionedTable<SharedScope> {
    fn admit_read(&self, state: &dyn Session) -> DataFusionResult<()> {
        let _span = kalamdb_observability::kdb_info_span_entered!("table.access");
        check_shared_table_access(state, self.core.table_def()).map_err(session_error_to_datafusion)
    }

    fn admit_write(&self, state: &dyn Session) -> DataFusionResult<()> {
        check_shared_table_write_access(state, self.core.table_def())
            .map_err(session_error_to_datafusion)
    }

    async fn ensure_sql_write_route(&self, state: &dyn Session) -> DataFusionResult<()> {
        self.ensure_shared_write_route(state).await
    }

    async fn authorize_sql_insert(
        &self,
        state: &dyn Session,
        user_id: &UserId,
        role: Role,
        rows: &[Row],
    ) -> DataFusionResult<()> {
        let check_policies = self
            .bind_policies(user_id, role, PolicyCommand::Insert, true)
            .map_err(crate::error::into_datafusion)?;
        let snapshot_commit_seq = extract_transaction_query_context(state).and_then(|context| {
            crate::utils::base::transaction_snapshot_bound(context.snapshot_commit_seq())
        });
        self.ensure_rows_authorized(&check_policies, rows, snapshot_commit_seq, "WITH CHECK")
            .await
    }

    fn replicates_shared_writes(&self) -> bool {
        VersionedTable::replicates_shared_writes(self)
    }

    async fn before_direct_write(&self) -> Result<(), KalamDbError> {
        self.ensure_shared_write_leader().await
    }

    async fn update_gate(
        &self,
        user_id: &UserId,
        role: Role,
        pk_column: &str,
        snapshot: Option<VersionId>,
    ) -> DataFusionResult<(Vec<String>, Option<BoundAuthorization>)> {
        let check_policies = self
            .bind_policies(user_id, role, PolicyCommand::Update, true)
            .map_err(crate::error::into_datafusion)?;
        let mut required_columns = vec![pk_column.to_string()];
        crate::shared_tables::shared_table_authorization::push_missing_column_names(
            &mut required_columns,
            &self.core.table_def().columns,
            &check_policies.required_column_ids(),
        );
        let check = self
            .bind_authorization(&check_policies, snapshot)
            .await
            .map_err(crate::error::into_datafusion)?;
        Ok((required_columns, Some(check)))
    }

    fn rls_state(
        &self,
        state: &dyn Session,
        command: PolicyCommand,
    ) -> DataFusionResult<Option<SessionState>> {
        Ok(Some(Self::session_state_with_rls_command(state, command)?))
    }

    async fn collect_delete_keys(
        &self,
        state: &dyn Session,
        role: Role,
        filters: &[Expr],
        pk_column: &str,
    ) -> DataFusionResult<Vec<String>> {
        if matches!(role, Role::System | Role::Dba) {
            if let Some(pk_values) =
                crate::utils::datafusion_dml::pk_membership_literals(filters, pk_column)
            {
                return Ok(pk_values);
            }
        }
        let schema = self.schema_ref();
        let projection = kalamdb_datafusion_sources::dml::dml_scan_projection(
            &schema,
            filters,
            &[],
            &[pk_column],
        )?;
        let rls_state = Self::session_state_with_rls_command(state, PolicyCommand::Delete)?;
        crate::utils::datafusion_dml::collect_matching_primary_keys(
            self,
            &rls_state,
            filters,
            projection.as_ref(),
            pk_column,
        )
        .await
    }
}

impl<S: TableScope> std::fmt::Debug for VersionedTable<S>
where
    Self: MutationGuard,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct(S::PROVIDER_NAME)
            .field("table_id", self.core.table_id())
            .field("table_type", &self.core.table_type())
            .field("primary_key_field_name", &self.core.primary_key_field_name())
            .finish()
    }
}

#[async_trait]
impl<S> TableProvider for VersionedTable<S>
where
    S: TableScope,
    Self: MutationGuard
        + SqlHooks
        + super::scan::ScopeScan<RowId = S::RowId, Row = S::Row, Context = S::ScanContext>
        + BaseTableProvider<S::RowId, S::Row>
        + crate::utils::base::DeferredMvccScanProvider<S::RowId, S::Row>,
    S::RowId: Clone + Send + Sync + 'static,
    S::Row: Clone + Send + Sync + 'static,
{
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
        self.admit_read(state)?;
        let overlay = if S::USER_SCOPED_ROWS {
            let (user_id, _) =
                extract_user_context(state).map_err(crate::error::into_datafusion)?;
            Some(user_id.clone())
        } else {
            None
        };
        kalamdb_observability::kdb_await_in_info_span!(
            <Self as BaseTableProvider<S::RowId, S::Row>>::base_scan_with_overlay(
                self, state, projection, filters, limit, overlay,
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
        tracing::debug!(table_id = %self.core.table_id(), "table.insert_into");
        self.admit_write(state)?;
        if insert_op != InsertOp::Append {
            return Err(DataFusionError::Plan(format!(
                "{} is not supported for {} tables",
                insert_op,
                S::LABEL
            )));
        }
        self.ensure_sql_write_route(state).await?;
        let (user_id, role) = extract_user_context(state)
            .map_err(|error| DataFusionError::Execution(error.to_string()))?;
        let rows = kalamdb_datafusion_sources::dml::collect_input_rows(state, input).await?;
        crate::utils::datafusion_dml::reject_missing_foreign_keys(
            self.schema_registry().as_ref(),
            state,
            self.core.table_id(),
            &self.core.table_def().foreign_keys,
            &rows,
        )
        .await?;
        self.authorize_sql_insert(state, user_id, role, &rows).await?;
        if let Some(transaction_query_context) = extract_transaction_query_context(state) {
            let inserted = crate::utils::datafusion_dml::stage_insert_rows(
                transaction_query_context,
                self.core.table_id(),
                S::TABLE_TYPE,
                Some(user_id.clone()),
                self.primary_key_field_name(),
                rows,
            )?;
            return kalamdb_datafusion_sources::dml::rows_affected_plan(state, inserted).await;
        }
        if self.replicates_shared_writes() {
            let row_count = rows.len();
            let table_id = self.core.table_id().clone();
            let inserted = kalamdb_observability::kdb_await_in_info_span!(
                self.core.services.cluster_coordinator.propose_shared_insert(
                    &table_id,
                    user_id,
                    rows,
                ),
                "table.shared_autocommit",
                op = "insert",
                table_id = %table_id,
                rows = row_count
            )
            .map_err(DataFusionError::Execution)?;
            let _ = row_count;
            return kalamdb_datafusion_sources::dml::rows_affected_plan(state, inserted as u64)
                .await;
        }
        let versions = crate::utils::base::direct_insert_versions(
            self.core.services.commit_sequence_source.allocate_next(),
            rows.len(),
        )
        .map_err(crate::error::into_datafusion)?;
        let row_count = versions.len();
        let inserted = kalamdb_observability::kdb_await_in_info_span!(
            self.insert_batch_with_versions(Some(user_id), rows, &versions),
            "table.autocommit",
            op = "insert",
            table_id = %self.core.table_id(),
            rows = row_count
        )
        .map_err(crate::error::into_datafusion)?;
        let _ = row_count;
        kalamdb_datafusion_sources::dml::rows_affected_plan(state, inserted.len() as u64).await
    }

    async fn delete_from(
        &self,
        state: &dyn Session,
        filters: Vec<Expr>,
    ) -> DataFusionResult<Arc<dyn ExecutionPlan>> {
        self.admit_write(state)?;
        self.ensure_sql_write_route(state).await?;
        let (user_id, role) = extract_user_context(state)
            .map_err(|error| DataFusionError::Execution(error.to_string()))?;
        let pk_column = self.primary_key_field_name().to_string();
        let pk_values = self.collect_delete_keys(state, role, &filters, &pk_column).await?;
        if pk_values.is_empty() {
            return kalamdb_datafusion_sources::dml::rows_affected_plan(state, 0).await;
        }
        let transaction_query_context = extract_transaction_query_context(state);
        self.delete_statement(state, user_id, pk_values, transaction_query_context)
            .await
    }

    async fn update(
        &self,
        state: &dyn Session,
        assignments: Vec<(String, Expr)>,
        filters: Vec<Expr>,
    ) -> DataFusionResult<Arc<dyn ExecutionPlan>> {
        self.admit_write(state)?;
        self.ensure_sql_write_route(state).await?;
        let (user_id, role) = extract_user_context(state)
            .map_err(|error| DataFusionError::Execution(error.to_string()))?;
        let pk_column = self.primary_key_field_name().to_string();
        crate::utils::datafusion_dml::validate_update_assignments(&assignments)?;
        let relocates_pk =
            assignments.iter().any(|(column, _)| column.eq_ignore_ascii_case(&pk_column));
        let transaction_query_context = extract_transaction_query_context(state);
        let snapshot_commit_seq = transaction_query_context.and_then(|context| {
            crate::utils::base::transaction_snapshot_bound(context.snapshot_commit_seq())
        });
        let (required_columns, check) =
            self.update_gate(user_id, role, &pk_column, snapshot_commit_seq).await?;
        let schema = self.schema_ref();
        let required_refs = required_columns.iter().map(String::as_str).collect::<Vec<_>>();
        let projection = if relocates_pk {
            None
        } else {
            kalamdb_datafusion_sources::dml::dml_scan_projection(
                &schema,
                &filters,
                &assignments,
                &required_refs,
            )?
        };
        let rls_state = self.rls_state(state, PolicyCommand::Update)?;
        let visit_state: &dyn Session = rls_state.as_ref().map(|state| state as _).unwrap_or(state);
        let mut seen = HashSet::new();
        let mut updated: u64 = 0;
        let mut ordinal = 0u32;
        let mut staged_mutations = transaction_query_context.map(|_| Vec::new());
        let mut direct_moves = Vec::new();
        let mut direct_updates = Vec::new();
        let mut matched = false;
        let eval_schema =
            kalamdb_datafusion_sources::dml::projected_schema(&schema, projection.as_deref())?;
        let evaluator = PreparedUpdateAssignments::new(state, &eval_schema, &assignments)?;
        crate::utils::datafusion_dml::visit_matching_batches(
            self,
            visit_state,
            &filters,
            projection.as_ref(),
            |batch| {
                if batch.num_rows() == 0 {
                    return Ok(());
                }
                matched = true;
                let evaluated = evaluator.evaluate_batch(&batch)?;
                let project_fields = !relocates_pk
                    && check
                        .as_ref()
                        .is_none_or(kalamdb_rls::BoundAuthorization::allows_all_current_rows);
                if project_fields {
                    for (row_idx, evaluated_updates) in evaluated.into_iter().enumerate() {
                        let (pk_value, updates) =
                            crate::utils::datafusion_dml::projected_field_change(
                                &batch,
                                row_idx,
                                &pk_column,
                                &schema,
                                evaluated_updates,
                            )?;
                        if !seen.insert(pk_value.clone()) {
                            continue;
                        }
                        let Some(updates) = updates else {
                            continue;
                        };
                        if let Some(staged_mutations) = staged_mutations.as_mut() {
                            let context = transaction_query_context
                                .expect("transaction_query_context must exist when staging UPDATE");
                            if crate::utils::datafusion_dml::stage_update_change(
                                staged_mutations,
                                context,
                                self.core.table_id(),
                                S::TABLE_TYPE,
                                user_id,
                                &pk_column,
                                pk_value,
                                UpdateChange::FieldUpdate(updates),
                            )? {
                                updated += 1;
                            }
                            continue;
                        }
                        direct_updates.push((pk_value, updates));
                    }
                    return Ok(());
                }
                let source_rows = kalamdb_datafusion_sources::dml::record_batch_to_rows(&batch)?;
                for (row, evaluated_updates) in source_rows.into_iter().zip(evaluated) {
                    let pk_value =
                        crate::utils::datafusion_dml::extract_pk_value(&row, &pk_column)?;
                    if !seen.insert(pk_value.clone()) {
                        continue;
                    }
                    if let Some(check) = check.as_ref() {
                        let mut new_row = row.clone();
                        for (column, value) in &evaluated_updates.values {
                            new_row.values.insert(column.clone(), value.clone());
                        }
                        if !check.authorizes(&new_row) {
                            return Err(DataFusionError::Plan(format!(
                                "row-level security WITH CHECK policy denied UPDATE on {}",
                                self.core.table_id()
                            )));
                        }
                    }
                    let change = crate::utils::datafusion_dml::classify_update(
                        &schema,
                        &pk_column,
                        &pk_value,
                        self.core.is_auto_increment_pk(),
                        &row,
                        evaluated_updates,
                    )?;
                    if let Some(staged_mutations) = staged_mutations.as_mut() {
                        let context = transaction_query_context
                            .expect("transaction_query_context must exist when staging UPDATE");
                        if crate::utils::datafusion_dml::stage_update_change(
                            staged_mutations,
                            context,
                            self.core.table_id(),
                            S::TABLE_TYPE,
                            user_id,
                            &pk_column,
                            pk_value,
                            change,
                        )? {
                            updated += 1;
                        }
                        continue;
                    }
                    match change {
                        UpdateChange::Unchanged => {},
                        UpdateChange::PrimaryKeyMove(inserted_row) => {
                            direct_moves.push((pk_value, inserted_row));
                            updated += 1;
                        },
                        UpdateChange::FieldUpdate(updates) => {
                            direct_updates.push((pk_value, updates));
                        },
                    }
                }
                Ok(())
            },
        )
        .await?;
        if !matched {
            return kalamdb_datafusion_sources::dml::rows_affected_plan(state, 0).await;
        }
        self.apply_statement_updates(
            user_id,
            &mut ordinal,
            direct_moves,
            direct_updates,
            &mut updated,
        )
        .await?;
        if let (Some(transaction_query_context), Some(staged_mutations)) =
            (transaction_query_context, staged_mutations)
        {
            crate::utils::datafusion_dml::stage_transaction_mutations(
                transaction_query_context,
                staged_mutations,
            )?;
        }
        kalamdb_datafusion_sources::dml::rows_affected_plan(state, updated).await
    }
}

impl<S> VersionedTable<S>
where
    S: TableScope,
    Self: MutationGuard
        + SqlHooks
        + super::scan::ScopeScan<RowId = S::RowId, Row = S::Row, Context = S::ScanContext>
        + BaseTableProvider<S::RowId, S::Row>,
{
    async fn delete_statement(
        &self,
        state: &dyn Session,
        user_id: &UserId,
        pk_values: Vec<String>,
        transaction_query_context: Option<&kalamdb_transactions::TransactionQueryContext>,
    ) -> DataFusionResult<Arc<dyn ExecutionPlan>> {
        let mut seen = HashSet::new();
        let mut deleted = 0u64;
        let mut ordinal = 0u32;
        let mut staged_mutations =
            transaction_query_context.as_ref().map(|_| Vec::with_capacity(pk_values.len()));
        let mut direct_pks = Vec::with_capacity(pk_values.len());
        for pk_value in pk_values {
            if seen.insert(pk_value.clone()) {
                direct_pks.push(pk_value);
            }
        }
        if let (Some(context), Some(staged)) =
            (transaction_query_context, staged_mutations.as_mut())
        {
            // Claim keys in canonical order to avoid lock-order inversions.
            // Once held, recheck committed liveness with bounded MultiGet batches
            // instead of initiating one blocking lookup per deleted row.
            let scope_user = (S::TABLE_TYPE == TableType::User).then_some(user_id);
            if let Some(claims) = context.pk_claims() {
                let mut sorted: Vec<&str> = direct_pks.iter().map(String::as_str).collect();
                sorted.sort_unstable();
                for pk in sorted {
                    claims
                        .acquire_scoped(self.core.table_id(), scope_user, pk, &context.transaction_id)
                        .await;
                }
            }
            const DELETE_LOOKUP_BATCH: usize = 512;
            for chunk in direct_pks.chunks(DELETE_LOOKUP_BATCH) {
                let targets = self
                    .load_mutation_targets(Some(user_id), chunk)
                    .await
                    .map_err(crate::error::into_datafusion)?;
                for (pk, target) in chunk.iter().zip(targets) {
                    if matches!(
                        target,
                        crate::versioned::MutationTarget::Hot(_, _)
                            | crate::versioned::MutationTarget::Cold(_)
                    ) {
                        crate::utils::datafusion_dml::stage_delete(
                            staged,
                            context.transaction_id.clone(),
                            self.core.table_id(),
                            S::TABLE_TYPE,
                            user_id.clone(),
                            pk.clone(),
                        );
                        deleted += 1;
                    } else if let Some(claims) = context.pk_claims() {
                        claims.release_key_scoped(
                            self.core.table_id(),
                            scope_user,
                            pk,
                            &context.transaction_id,
                        );
                    }
                }
            }
        } else if !direct_pks.is_empty() {
            if self.replicates_shared_writes() {
                let key_count = direct_pks.len();
                let table_id = self.core.table_id().clone();
                let removed = kalamdb_observability::kdb_await_in_info_span!(
                    self.core.services.cluster_coordinator.propose_shared_delete(
                        &table_id,
                        user_id,
                        direct_pks,
                    ),
                    "table.shared_autocommit",
                    op = "delete",
                    table_id = %table_id,
                    rows = key_count
                )
                .map_err(DataFusionError::Execution)?;
                let _ = key_count;
                deleted += removed as u64;
            } else {
                let commit_seq = self.core.services.commit_sequence_source.allocate_next();
                let key_count = direct_pks.len();
                deleted += kalamdb_observability::kdb_await_in_info_span!(
                    self.commit_direct_deletes(user_id, &direct_pks, commit_seq, &mut ordinal),
                    "table.autocommit",
                    op = "delete",
                    table_id = %self.core.table_id(),
                    rows = key_count
                )
                .map_err(crate::error::into_datafusion)?;
                let _ = key_count;
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
        kalamdb_datafusion_sources::dml::rows_affected_plan(state, deleted).await
    }

    async fn apply_statement_updates(
        &self,
        user_id: &UserId,
        ordinal: &mut u32,
        direct_moves: Vec<(String, Row)>,
        direct_updates: Vec<(String, Row)>,
        updated: &mut u64,
    ) -> DataFusionResult<()> {
        if self.replicates_shared_writes() {
            if !direct_moves.is_empty() {
                let transaction_id = kalamdb_commons::models::TransactionId::fresh();
                let table_id = self.core.table_id().clone();
                let pk_column = self.primary_key_field_name().to_string();
                let update_count = direct_updates.len() as u64;
                let mut staged = Vec::new();
                for (old_pk, row) in direct_moves {
                    crate::utils::datafusion_dml::stage_primary_key_move(
                        &mut staged,
                        transaction_id.clone(),
                        &table_id,
                        S::TABLE_TYPE,
                        user_id.clone(),
                        &pk_column,
                        old_pk,
                        row,
                    )?;
                }
                for (pk_value, updates) in direct_updates {
                    crate::utils::datafusion_dml::stage_field_update(
                        &mut staged,
                        transaction_id.clone(),
                        &table_id,
                        S::TABLE_TYPE,
                        user_id.clone(),
                        pk_value,
                        updates,
                    );
                }
                for (order, mutation) in staged.iter_mut().enumerate() {
                    mutation.mutation_order = order as u64;
                }
                kalamdb_observability::kdb_await_in_info_span!(
                    self.core.services.cluster_coordinator.propose_shared_statement(staged),
                    "table.shared_autocommit",
                    op = "update",
                    table_id = %table_id
                )
                .map_err(DataFusionError::Execution)?;
                *updated += update_count;
            } else if !direct_updates.is_empty() {
                let (pk_values, updates) = direct_updates.into_iter().unzip();
                let changed = kalamdb_observability::kdb_await_in_info_span!(
                    self.core.services.cluster_coordinator.propose_shared_update(
                        self.core.table_id(),
                        user_id,
                        pk_values,
                        updates,
                    ),
                    "table.shared_autocommit",
                    op = "update",
                    table_id = %self.core.table_id()
                )
                .map_err(DataFusionError::Execution)?;
                *updated += changed as u64;
            }
            return Ok(());
        }
        if direct_moves.is_empty() && direct_updates.is_empty() {
            return Ok(());
        }
        let commit_seq = self.core.services.commit_sequence_source.allocate_next();
        let row_count = direct_moves.len() + direct_updates.len();
        *updated += kalamdb_observability::kdb_await_in_info_span!(
            self.commit_direct_updates(user_id, commit_seq, ordinal, direct_moves, direct_updates),
            "table.autocommit",
            op = "update",
            table_id = %self.core.table_id(),
            rows = row_count
        )
        .map_err(crate::error::into_datafusion)?;
        let _ = row_count;
        Ok(())
    }
}

#[async_trait]
impl<S> TableRowDml for VersionedTable<S>
where
    S: TableScope,
    Self: MutationGuard + SqlHooks + BaseTableProvider<S::RowId, S::Row> + TableProvider,
    S::RowId: Clone + Send + Sync + 'static,
    S::Row: Clone + Send + Sync + 'static,
{
    async fn insert_rows(
        &self,
        user_id: &UserId,
        rows: Vec<Row>,
        versions: &[VersionId],
    ) -> Result<usize, KalamDbError> {
        self.before_direct_write().await?;
        let keys = self.insert_batch(user_id, rows, versions).await?;
        Ok(keys.len())
    }

    async fn insert_rows_returning(
        &self,
        user_id: &UserId,
        rows: Vec<Row>,
        versions: &[VersionId],
    ) -> Result<Vec<ScalarValue>, KalamDbError> {
        self.before_direct_write().await?;
        let keys = self.insert_batch(user_id, rows, versions).await?;
        Ok(keys
            .into_iter()
            .map(|key| ScalarValue::Int64(Some(S::row_id_version(&key).as_i64())))
            .collect())
    }

    async fn update_row_by_pk(
        &self,
        user_id: &UserId,
        pk_value: &str,
        updates: Row,
        version: VersionId,
    ) -> Result<bool, KalamDbError> {
        self.before_direct_write().await?;
        crate::versioned::mutation_applied(
            self.update_by_pk_value(user_id, pk_value, updates, version).await,
        )
    }

    async fn delete_row_by_pk(
        &self,
        user_id: &UserId,
        pk_value: &str,
        version: VersionId,
    ) -> Result<bool, KalamDbError> {
        self.before_direct_write().await?;
        self.delete_by_pk_value(user_id, pk_value, version).await
    }
}
