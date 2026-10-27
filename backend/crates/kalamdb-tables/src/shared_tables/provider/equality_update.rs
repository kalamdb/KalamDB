//! Secondary-index equality updates that skip DataFusion planning.
//!
//! `UPDATE t SET col = <literal> WHERE indexed_col = <literal>` seeks the
//! matching prefix index and returns the same staged mutations the SQL
//! `update` path would buffer. Cold segments, row-level security, and a seek
//! column assigned earlier in the transaction stay on the DataFusion path.

use std::collections::{BTreeMap, HashMap};

use datafusion::{
    logical_expr::{col, BinaryExpr, Expr, Operator},
    scalar::ScalarValue,
};
use kalamdb_commons::{
    conversions::{arrow_json_conversion::coerce_updates, scalar_to_pk_string},
    ids::VersionId,
    models::{rows::Row, OperationKind, UserId},
    TableType, TransactionId,
};
use kalamdb_transactions::{StagedMutation, TransactionOverlay};

use super::SharedTableProvider;
use crate::error::KalamDbError;

impl SharedTableProvider {
    /// Stage one literal equality update, or `Ok(None)` when this path cannot
    /// prove an index seek.
    ///
    /// The caller must drop `overlay` before staging the returned mutations.
    /// Holding that overlay clone across `stage_batch` copies the staged-row
    /// map on every following mutation.
    pub async fn plan_literal_equality_updates(
        &self,
        transaction_id: &TransactionId,
        user_id: &UserId,
        snapshot_commit_seq: Option<VersionId>,
        overlay: Option<&TransactionOverlay>,
        seek_column: &str,
        seek_value: &ScalarValue,
        assignments: &[(String, ScalarValue)],
    ) -> Result<Option<Vec<StagedMutation>>, KalamDbError> {
        if assignments.is_empty() || seek_value.is_null() {
            return Ok(None);
        }
        let pk_name = self.core.primary_key_field_name();
        if assignments.iter().any(|(column, _)| column == pk_name) {
            return Ok(None);
        }
        if self
            .vector_columns
            .iter()
            .any(|(column, _)| assignments.iter().any(|(assignment, _)| assignment == column))
        {
            return Ok(None);
        }
        let table_def = self.core.table_def();
        if table_def.foreign_keys.iter().any(|foreign_key| {
            foreign_key.columns.iter().any(|column| {
                assignments
                    .iter()
                    .any(|(assignment, _)| assignment.eq_ignore_ascii_case(column))
            })
        }) {
            return Ok(None);
        }

        let schema = self.core.schema_ref();
        if schema.field_with_name(seek_column).is_err() {
            return Ok(None);
        }
        for (column, _) in assignments {
            if schema.field_with_name(column).is_err() {
                return Ok(None);
            }
        }

        let table_id = self.core.table_id();
        if let Some(overlay) = overlay {
            if overlay.inserted_keys.get(table_id).is_some_and(|keys| !keys.is_empty()) {
                return Ok(None);
            }
            if overlay.column_was_updated(table_id, seek_column) {
                return Ok(None);
            }
        }

        if !self.hot_storage_holds_every_row().await? {
            return Ok(None);
        }

        let seek_value = coerced_column(&schema, seek_column, seek_value.clone())?;
        if seek_value.is_null() {
            return Ok(Some(Vec::new()));
        }
        let mut assigned = BTreeMap::new();
        for (column, value) in assignments {
            let value = coerced_column(&schema, column, value.clone())?;
            if value.is_null() && column_forbids_null(table_def.as_ref(), column) {
                return Err(KalamDbError::InvalidOperation(format!(
                    "null value in column \"{column}\" violates not-null constraint"
                )));
            }
            assigned.insert(column.clone(), value);
        }
        let assigned = Row::new(assigned);

        let filter = Expr::BinaryExpr(BinaryExpr {
            left:  Box::new(col(seek_column)),
            op:    Operator::Eq,
            right: Box::new(Expr::Literal(seek_value, None)),
        });
        let Some(plan) = self.store.plan_index_scan(None, &filter) else {
            return Ok(None);
        };
        if plan.has_range() || plan.equality_columns() == 0 {
            return Ok(None);
        }

        let hits = self
            .store
            .scan_by_index_plan_iter(&plan, None)
            .map_err(|error| {
                KalamDbError::InvalidOperation(format!("index equality scan failed: {error}"))
            })?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| {
                KalamDbError::InvalidOperation(format!("index equality scan failed: {error}"))
            })?;

        let mut newest: HashMap<String, (VersionId, Row, bool)> =
            HashMap::with_capacity(hits.len());
        for (_key, row) in hits {
            if snapshot_commit_seq.is_some_and(|snapshot| row._version > snapshot) {
                return Ok(None);
            }
            let pk_value = row.fields.get(pk_name).ok_or_else(|| {
                KalamDbError::InvalidOperation(format!(
                    "primary key column '{pk_name}' missing from indexed row"
                ))
            })?;
            let pk_value = scalar_to_pk_string(pk_value).map_err(|error| {
                KalamDbError::InvalidOperation(format!("invalid primary key value: {error}"))
            })?;
            match newest.get(&pk_value) {
                Some((version, _, _)) if *version >= row._version => {},
                _ => {
                    newest.insert(pk_value, (row._version, row.fields, row._deleted));
                },
            }
        }

        let mut mutations = Vec::with_capacity(newest.len());
        for (pk_value, (_version, fields, deleted)) in newest {
            if deleted {
                continue;
            }
            if let Some(entry) =
                overlay.and_then(|overlay| overlay.latest_visible_entry(table_id, &pk_value))
            {
                if entry.is_deleted() {
                    continue;
                }
            }

            let mut current_values = BTreeMap::new();
            for column in assigned.values.keys() {
                if let Some(entry) =
                    overlay.and_then(|overlay| overlay.latest_visible_entry(table_id, &pk_value))
                {
                    if let Some(value) = entry.payload.values.get(column) {
                        current_values.insert(column.clone(), value.clone());
                        continue;
                    }
                }
                if let Some(value) = fields.get(column) {
                    current_values.insert(column.clone(), value.clone());
                }
            }
            let current = Row::new(current_values);
            let unchanged =
                crate::utils::datafusion_dml::update_assignments_noop(&schema, &current, &assigned)
                    .map_err(|error| KalamDbError::InvalidOperation(error.to_string()))?;
            if unchanged {
                continue;
            }

            mutations.push(StagedMutation::new(
                transaction_id.clone(),
                table_id.clone(),
                TableType::Shared,
                Some(user_id.clone()),
                OperationKind::Update,
                pk_value,
                assigned.clone(),
                false,
            ));
        }

        Ok(Some(mutations))
    }

    pub(crate) async fn hot_storage_holds_every_row(&self) -> Result<bool, KalamDbError> {
        match self.core.manifest_service().get_or_load_async(self.core.table_id(), None).await {
            Ok(None) => Ok(true),
            Ok(Some(entry)) => Ok(entry.manifest.segments.is_empty()),
            Err(_) => Ok(false),
        }
    }
}

fn coerced_column(
    schema: &datafusion::arrow::datatypes::SchemaRef,
    column: &str,
    value: ScalarValue,
) -> Result<ScalarValue, KalamDbError> {
    let row = coerce_updates(Row::new(BTreeMap::from([(column.to_string(), value)])), schema)
        .map_err(|error| {
            KalamDbError::InvalidOperation(format!("schema coercion failed: {error}"))
        })?;
    row.values.get(column).cloned().ok_or_else(|| {
        KalamDbError::InvalidOperation(format!("column '{column}' missing after coercion"))
    })
}

fn column_forbids_null(
    table_def: &kalamdb_commons::models::schemas::TableDefinition,
    column: &str,
) -> bool {
    table_def
        .columns
        .iter()
        .find(|definition| definition.column_name == column)
        .is_some_and(|definition| !definition.is_nullable)
}
