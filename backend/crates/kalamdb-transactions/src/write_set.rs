use std::{
    collections::{BTreeMap, HashMap},
    fmt,
};

use datafusion::scalar::ScalarValue;

use kalamdb_commons::{
    models::{rows::Row, OperationKind, TableId, TransactionId, UserId},
    try_pk_bucket_key, PkBucketKey,
};

use crate::{
    overlay::{ScopedPkKey, ScopedTargetKey, TableScope},
    StagedMutation, TransactionOverlay,
};

fn scoped_table_key(user_id: Option<&UserId>, primary_key: &str) -> ScopedPkKey {
    ScopedPkKey::new(user_id, primary_key)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SavepointEntry {
    pub name: String,
    pub mutation_count: usize,
    pub next_order: u64,
}

/// Cold staged-write state allocated lazily on the first transaction write.
#[derive(Debug, Clone)]
pub struct TransactionWriteSet {
    pub transaction_id: TransactionId,
    pub next_mutation_order: u64,
    pub ordered_mutations: Vec<StagedMutation>,
    pub latest_by_table_key: HashMap<TableId, HashMap<ScopedPkKey, usize>>,
    pub overlay_cache: HashMap<TableId, TransactionOverlay>,
    pub buffer_bytes: usize,
    pub savepoint_marks: Vec<u64>,
    pub savepoints: Vec<SavepointEntry>,
}

impl TransactionWriteSet {
    pub fn new(transaction_id: TransactionId) -> Self {
        Self {
            transaction_id,
            next_mutation_order: 0,
            ordered_mutations: Vec::new(),
            latest_by_table_key: HashMap::new(),
            overlay_cache: HashMap::new(),
            buffer_bytes: 0,
            savepoint_marks: Vec::new(),
            savepoints: Vec::new(),
        }
    }

    pub fn stage(&mut self, mutation: StagedMutation) -> u64 {
        let estimated_bytes = mutation.approximate_size_bytes();
        self.stage_with_estimated_size(mutation, estimated_bytes)
    }

    pub fn stage_with_estimated_size(
        &mut self,
        mut mutation: StagedMutation,
        estimated_bytes: usize,
    ) -> u64 {
        let mutation_order = self.next_mutation_order;
        self.next_mutation_order += 1;
        mutation.mutation_order = mutation_order;

        let overlay_entry = mutation.overlay_entry();
        let table_id = mutation.table_id.clone();
        let table_key = scoped_table_key(mutation.user_id.as_ref(), mutation.primary_key.as_str());
        self.buffer_bytes += estimated_bytes;

        let mutation_index = self.ordered_mutations.len();
        self.ordered_mutations.push(mutation);
        self.latest_by_table_key
            .entry(table_id.clone())
            .or_default()
            .insert(table_key, mutation_index);
        self.overlay_cache
            .entry(table_id.clone())
            .or_insert_with(|| TransactionOverlay::new(self.transaction_id.clone()))
            .apply_entry(overlay_entry);

        mutation_order
    }

    #[inline]
    pub fn affected_rows(&self) -> usize {
        self.ordered_mutations.len()
    }

    #[inline]
    pub fn ordered_mutations(&self) -> &[StagedMutation] {
        &self.ordered_mutations
    }

    pub fn latest_mutation(
        &self,
        table_id: &TableId,
        primary_key: &str,
    ) -> Option<&StagedMutation> {
        self.latest_mutation_for_scope(table_id, None, primary_key)
    }

    pub fn latest_mutation_for_scope(
        &self,
        table_id: &TableId,
        user_id: Option<&UserId>,
        primary_key: &str,
    ) -> Option<&StagedMutation> {
        let table_key = scoped_table_key(user_id, primary_key);
        let mutation_index = *self.latest_by_table_key.get(table_id)?.get(&table_key)?;
        self.ordered_mutations.get(mutation_index)
    }

    pub fn overlay_for_table(&self, table_id: &TableId) -> Option<TransactionOverlay> {
        self.overlay_cache.get(table_id).cloned()
    }

    pub fn merged_overlay(&self) -> TransactionOverlay {
        let mut overlay = TransactionOverlay::new(self.transaction_id.clone());
        for table_overlay in self.overlay_cache.values() {
            overlay.merge_from(table_overlay);
        }
        overlay
    }

    pub fn push_savepoint(&mut self, name: &str) {
        let normalized = name.trim().to_ascii_lowercase();
        let entry = SavepointEntry {
            name: normalized,
            mutation_count: self.ordered_mutations.len(),
            next_order: self.next_mutation_order,
        };
        self.savepoint_marks.push(entry.next_order);
        self.savepoints.push(entry);
    }

    pub fn release_savepoint(&mut self, name: &str) -> bool {
        let normalized = name.trim().to_ascii_lowercase();
        let Some(pos) = self.savepoints.iter().rposition(|sp| sp.name == normalized) else {
            return false;
        };
        self.savepoints.truncate(pos);
        self.savepoint_marks.truncate(pos);
        true
    }

    pub fn rollback_to_savepoint(&mut self, name: &str) -> Option<(usize, usize)> {
        let normalized = name.trim().to_ascii_lowercase();
        let pos = self.savepoints.iter().rposition(|sp| sp.name == normalized)?;
        let target = self.savepoints[pos].clone();
        // Keep the savepoint itself active per PostgreSQL semantics; discard inner savepoints.
        self.savepoints.truncate(pos + 1);
        self.savepoint_marks.truncate(pos + 1);
        self.truncate_mutations(target.mutation_count, target.next_order);
        Some((self.ordered_mutations.len(), self.buffer_bytes))
    }

    fn truncate_mutations(&mut self, target_len: usize, next_order: u64) {
        self.ordered_mutations.truncate(target_len);
        self.next_mutation_order = next_order;
        self.latest_by_table_key.clear();
        self.overlay_cache.clear();
        self.buffer_bytes = 0;

        for (index, mutation) in self.ordered_mutations.iter().enumerate() {
            let estimated_bytes = mutation.approximate_size_bytes();
            self.buffer_bytes += estimated_bytes;
            let table_id = mutation.table_id.clone();
            let table_key =
                scoped_table_key(mutation.user_id.as_ref(), mutation.primary_key.as_str());
            self.latest_by_table_key
                .entry(table_id.clone())
                .or_default()
                .insert(table_key, index);
            self.overlay_cache
                .entry(table_id)
                .or_insert_with(|| TransactionOverlay::new(self.transaction_id.clone()))
                .apply_entry(mutation.overlay_entry());
        }
    }

    /// Fold repeated writes of one key into the final stored transition.
    ///
    /// `ordered_mutations` stays intact for savepoint rollback and statement counts.
    pub fn fold_commit(&self) -> Result<Vec<StagedMutation>, CommitFoldError> {
        self.fold_commit_with_primary_keys(&HashMap::new())
    }

    /// Same as [`Self::fold_commit`], expanding an update that assigns `pk_columns`
    /// to a delete of the old key and an insert of the new key.
    pub fn fold_commit_with_primary_keys(
        &self,
        pk_columns: &HashMap<TableId, String>,
    ) -> Result<Vec<StagedMutation>, CommitFoldError> {
        fold_mutations(self.ordered_mutations.iter().cloned(), pk_columns)
    }

    /// Consume this write set after the transaction is no longer writable.
    ///
    /// Overlay and latest-key indexes are released before folding. Payloads move
    /// into the folded result. When the fold stores nothing, `admission_history`
    /// keeps the original mutation identities and `accounted_bytes` is their size
    /// before those payloads were cleared, so group admission still runs.
    pub fn fold_commit_owned(mut self) -> Result<OwnedCommitFold, CommitFoldError> {
        self.overlay_cache.clear();
        self.latest_by_table_key.clear();
        self.savepoints.clear();
        self.savepoint_marks.clear();
        let mut history = std::mem::take(&mut self.ordered_mutations);
        let accounted_bytes = history.iter().map(StagedMutation::approximate_size_bytes).sum();
        self.buffer_bytes = 0;
        let mut folded_inputs = Vec::with_capacity(history.len());
        for mutation in &mut history {
            folded_inputs.push(take_payload_for_fold(mutation));
        }
        let folded = fold_mutations(folded_inputs, &HashMap::new())?;
        let admission_history = if folded.is_empty() {
            history
        } else {
            drop(history);
            Vec::new()
        };
        Ok(OwnedCommitFold {
            folded,
            admission_history,
            accounted_bytes,
        })
    }
}

/// Folded commit plus the net-no-op history required for group admission.
pub struct OwnedCommitFold {
    pub folded: Vec<StagedMutation>,
    pub admission_history: Vec<StagedMutation>,
    pub accounted_bytes: usize,
}

fn take_payload_for_fold(mutation: &mut StagedMutation) -> StagedMutation {
    let payload = std::mem::replace(&mut mutation.payload, Row::new(BTreeMap::new()));
    StagedMutation {
        transaction_id: mutation.transaction_id.clone(),
        mutation_order: mutation.mutation_order,
        table_id: mutation.table_id.clone(),
        table_type: mutation.table_type,
        user_id: mutation.user_id.clone(),
        operation_kind: mutation.operation_kind,
        primary_key: mutation.primary_key.clone(),
        payload,
        tombstone: mutation.tombstone,
    }
}

fn fold_mutations(
    mutations: impl IntoIterator<Item = StagedMutation>,
    pk_columns: &HashMap<TableId, String>,
) -> Result<Vec<StagedMutation>, CommitFoldError> {
    let mutations = mutations.into_iter();
    let mut slots: HashMap<ScopedTargetKey, FoldSlot> =
        HashMap::with_capacity(mutations.size_hint().0);
    for mutation in mutations {
        if let Some(pk_column) = pk_columns.get(&mutation.table_id) {
            if let Some(new_key) = renamed_primary_key(&mutation, pk_column) {
                let current_key = target_key(&mutation);
                let (delete_old, insert_new) =
                    expand_primary_key_rename(&mutation, &new_key, slots.get(&current_key));
                apply_folded_mutation(&mut slots, delete_old)?;
                apply_folded_mutation(&mut slots, insert_new)?;
                continue;
            }
        }
        apply_folded_mutation(&mut slots, mutation)?;
    }

    let mut folded: Vec<FoldSlot> = slots.into_values().collect();
    folded.sort_by_key(|slot| slot.order);
    Ok(folded.into_iter().filter_map(|slot| slot.into_mutation()).collect())
}

/// Duplicate primary key staged twice as an insert.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitFoldError {
    pub table_id: TableId,
    pub primary_key: String,
}

impl fmt::Display for CommitFoldError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "duplicate primary key '{}' for table '{}'",
            self.primary_key,
            self.table_id.full_name()
        )
    }
}

impl std::error::Error for CommitFoldError {}

enum FoldedTargetState {
    Insert {
        fields: Row,
        from_tombstone: bool,
    },
    Update {
        patch_fields: BTreeMap<String, ScalarValue>,
    },
    Delete {
        prior_known_fields: Option<Row>,
    },
    NoOpAfterInsertDelete,
}

struct FoldSlot {
    state: FoldedTargetState,
    order: u64,
    template: StagedMutation,
}

impl FoldSlot {
    fn into_mutation(self) -> Option<StagedMutation> {
        let mut mutation = self.template;
        match self.state {
            FoldedTargetState::Insert { fields, .. } => {
                mutation.operation_kind = OperationKind::Insert;
                mutation.payload = fields;
                mutation.tombstone = false;
                Some(mutation)
            },
            FoldedTargetState::Update { patch_fields } => {
                mutation.operation_kind = OperationKind::Update;
                mutation.payload = Row::new(patch_fields);
                mutation.tombstone = false;
                Some(mutation)
            },
            FoldedTargetState::Delete {
                prior_known_fields, ..
            } => {
                mutation.operation_kind = OperationKind::Delete;
                mutation.payload = prior_known_fields.unwrap_or_else(|| Row::new(BTreeMap::new()));
                mutation.tombstone = true;
                Some(mutation)
            },
            FoldedTargetState::NoOpAfterInsertDelete => None,
        }
    }
}

fn target_key(mutation: &StagedMutation) -> ScopedTargetKey {
    ScopedTargetKey::new(
        mutation.table_id.clone(),
        TableScope::for_mutation(mutation.table_type, mutation.user_id.as_ref()),
        PkBucketKey::Text(mutation.primary_key.clone()),
    )
}

fn apply_folded_mutation(
    slots: &mut HashMap<ScopedTargetKey, FoldSlot>,
    mut mutation: StagedMutation,
) -> Result<(), CommitFoldError> {
    let key = target_key(&mutation);
    let payload = std::mem::replace(&mut mutation.payload, Row::new(BTreeMap::new()));
    let kind = mutation.operation_kind;
    let order = mutation.mutation_order;
    let previous = slots.remove(&key);
    let (state, template, slot_order) = match previous {
        None => {
            let (state, _) =
                folded_state(kind, payload, None, &mutation.table_id, &mutation.primary_key)?;
            (state, mutation, order)
        },
        Some(slot) => {
            let FoldSlot {
                state,
                order: prior_order,
                template,
            } = slot;
            let (next, keep_prior) = folded_state(
                kind,
                payload,
                Some(state),
                &mutation.table_id,
                &mutation.primary_key,
            )?;
            if keep_prior {
                (next, template, prior_order)
            } else {
                (next, mutation, order)
            }
        },
    };
    slots.insert(
        key,
        FoldSlot {
            state,
            order: slot_order,
            template,
        },
    );
    Ok(())
}

/// `keep_prior` is true when this mutation does not change the stored target.
fn folded_state(
    kind: OperationKind,
    payload: Row,
    previous: Option<FoldedTargetState>,
    table_id: &TableId,
    primary_key: &str,
) -> Result<(FoldedTargetState, bool), CommitFoldError> {
    let next = match previous {
        None => match kind {
            OperationKind::Insert => FoldedTargetState::Insert {
                fields: payload,
                from_tombstone: false,
            },
            OperationKind::Update => FoldedTargetState::Update {
                patch_fields: payload.values,
            },
            OperationKind::Delete => FoldedTargetState::Delete {
                prior_known_fields: non_empty_row(payload),
            },
        },
        Some(FoldedTargetState::Insert {
            fields,
            from_tombstone,
        }) => match kind {
            OperationKind::Insert => {
                return Err(CommitFoldError {
                    table_id: table_id.clone(),
                    primary_key: primary_key.to_string(),
                });
            },
            OperationKind::Update => FoldedTargetState::Insert {
                fields: merge_row_fields(fields, payload.values),
                from_tombstone,
            },
            OperationKind::Delete => {
                if from_tombstone {
                    FoldedTargetState::Delete {
                        prior_known_fields: Some(fields),
                    }
                } else {
                    FoldedTargetState::NoOpAfterInsertDelete
                }
            },
        },
        Some(FoldedTargetState::Update { patch_fields }) => match kind {
            OperationKind::Insert => {
                return Err(CommitFoldError {
                    table_id: table_id.clone(),
                    primary_key: primary_key.to_string(),
                });
            },
            OperationKind::Update => FoldedTargetState::Update {
                patch_fields: merge_patches(patch_fields, payload.values),
            },
            OperationKind::Delete => FoldedTargetState::Delete {
                prior_known_fields: Some(Row::new(patch_fields)),
            },
        },
        Some(FoldedTargetState::Delete { prior_known_fields }) => match kind {
            OperationKind::Insert => FoldedTargetState::Insert {
                fields: payload,
                from_tombstone: true,
            },
            OperationKind::Update | OperationKind::Delete => {
                return Ok((FoldedTargetState::Delete { prior_known_fields }, true));
            },
        },
        Some(FoldedTargetState::NoOpAfterInsertDelete) => match kind {
            OperationKind::Insert => FoldedTargetState::Insert {
                fields: payload,
                from_tombstone: false,
            },
            OperationKind::Update | OperationKind::Delete => {
                return Ok((FoldedTargetState::NoOpAfterInsertDelete, true));
            },
        },
    };
    Ok((next, false))
}

fn non_empty_row(row: Row) -> Option<Row> {
    if row.values.is_empty() {
        None
    } else {
        Some(row)
    }
}

fn merge_patches(
    mut current: BTreeMap<String, ScalarValue>,
    patch: BTreeMap<String, ScalarValue>,
) -> BTreeMap<String, ScalarValue> {
    for (column, value) in patch {
        current.insert(column, value);
    }
    current
}

fn merge_row_fields(current: Row, patch: BTreeMap<String, ScalarValue>) -> Row {
    Row::new(merge_patches(current.values, patch))
}

fn renamed_primary_key(mutation: &StagedMutation, pk_column: &str) -> Option<String> {
    if mutation.operation_kind != OperationKind::Update {
        return None;
    }
    let value = mutation.payload.values.get(pk_column)?;
    if value.is_null() {
        return None;
    }
    let new_key = try_pk_bucket_key(value).ok()?.to_string();
    if new_key == mutation.primary_key {
        None
    } else {
        Some(new_key)
    }
}

fn expand_primary_key_rename(
    mutation: &StagedMutation,
    new_key: &str,
    current: Option<&FoldSlot>,
) -> (StagedMutation, StagedMutation) {
    let mut fields = match current.map(|slot| &slot.state) {
        Some(FoldedTargetState::Insert { fields, .. }) => fields.values.clone(),
        Some(FoldedTargetState::Update { patch_fields }) => patch_fields.clone(),
        _ => mutation.payload.values.clone(),
    };
    for (column, value) in &mutation.payload.values {
        fields.insert(column.clone(), value.clone());
    }

    let mut delete_old = mutation.clone();
    delete_old.operation_kind = OperationKind::Delete;
    delete_old.tombstone = true;
    delete_old.payload = Row::new(BTreeMap::new());

    let mut insert_new = mutation.clone();
    insert_new.operation_kind = OperationKind::Insert;
    insert_new.primary_key = new_key.to_string();
    insert_new.payload = Row::new(fields);
    insert_new.tombstone = false;
    (delete_old, insert_new)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use datafusion::scalar::ScalarValue;
    use kalamdb_commons::{
        models::{rows::Row, OperationKind, SchemaId, TableId, TableName},
        TableType,
    };

    use super::*;

    fn row(values: &[(&'static str, ScalarValue)]) -> Row {
        let mut fields = BTreeMap::new();
        for (name, value) in values {
            fields.insert((*name).to_string(), value.clone());
        }
        Row::new(fields)
    }

    #[test]
    fn merged_overlay_keeps_latest_state_per_primary_key() {
        let transaction_id = TransactionId::new("01960f7b-3d15-7d6d-b26c-7e4db6f25f8d");
        let table_id = TableId::new(SchemaId::new("app"), TableName::new("items"));
        let mut write_set = TransactionWriteSet::new(transaction_id.clone());

        let mut first_values = BTreeMap::new();
        first_values.insert("id".to_string(), ScalarValue::Int64(Some(1)));
        first_values.insert("name".to_string(), ScalarValue::Utf8(Some("first".to_string())));
        write_set.stage(StagedMutation::new(
            transaction_id.clone(),
            table_id.clone(),
            TableType::Shared,
            None,
            OperationKind::Insert,
            "1",
            Row::new(first_values),
            false,
        ));

        let mut second_values = BTreeMap::new();
        second_values.insert("id".to_string(), ScalarValue::Int64(Some(1)));
        second_values.insert("name".to_string(), ScalarValue::Utf8(Some("second".to_string())));
        write_set.stage(StagedMutation::new(
            transaction_id,
            table_id.clone(),
            TableType::Shared,
            None,
            OperationKind::Update,
            "1",
            Row::new(second_values),
            false,
        ));

        let overlay = write_set.merged_overlay();
        let entry = overlay.latest_visible_entry(&table_id, "1").unwrap();
        assert_eq!(entry.mutation_order, 1);
        assert_eq!(entry.operation_kind, OperationKind::Insert);
        assert_eq!(
            entry.payload.values.get("name"),
            Some(&ScalarValue::Utf8(Some("second".to_string())))
        );
    }

    #[test]
    fn stage_updates_overlay_incrementally() {
        let transaction_id = TransactionId::new("01960f7b-3d15-7d6d-b26c-7e4db6f25f8d");
        let table_id = TableId::new(SchemaId::new("app"), TableName::new("items"));
        let mut write_set = TransactionWriteSet::new(transaction_id.clone());

        let mut inserted_values = BTreeMap::new();
        inserted_values.insert("id".to_string(), ScalarValue::Int64(Some(1)));
        inserted_values.insert("name".to_string(), ScalarValue::Utf8(Some("before".to_string())));
        inserted_values.insert("color".to_string(), ScalarValue::Utf8(Some("red".to_string())));
        write_set.stage(StagedMutation::new(
            transaction_id.clone(),
            table_id.clone(),
            TableType::Shared,
            None,
            OperationKind::Insert,
            "1",
            Row::new(inserted_values),
            false,
        ));

        let mut updated_values = BTreeMap::new();
        updated_values.insert("name".to_string(), ScalarValue::Utf8(Some("after".to_string())));
        write_set.stage(StagedMutation::new(
            transaction_id,
            table_id.clone(),
            TableType::Shared,
            None,
            OperationKind::Update,
            "1",
            Row::new(updated_values),
            false,
        ));

        let overlay = write_set.overlay_for_table(&table_id).expect("table overlay");
        let entry = overlay.latest_visible_entry(&table_id, "1").expect("overlay entry");
        assert_eq!(entry.operation_kind, OperationKind::Insert);
        assert_eq!(entry.payload.values.get("id"), Some(&ScalarValue::Int64(Some(1))));
        assert_eq!(
            entry.payload.values.get("name"),
            Some(&ScalarValue::Utf8(Some("after".to_string())))
        );
        assert_eq!(
            entry.payload.values.get("color"),
            Some(&ScalarValue::Utf8(Some("red".to_string())))
        );
    }

    #[test]
    fn merged_overlay_preserves_distinct_user_scope_for_same_primary_key() {
        let transaction_id = TransactionId::new("01960f7b-3d15-7d6d-b26c-7e4db6f25f8d");
        let table_id = TableId::new(SchemaId::new("app"), TableName::new("items"));
        let first_user = UserId::new("user-a");
        let second_user = UserId::new("user-b");
        let mut write_set = TransactionWriteSet::new(transaction_id.clone());

        write_set.stage(StagedMutation::new(
            transaction_id.clone(),
            table_id.clone(),
            TableType::User,
            Some(first_user.clone()),
            OperationKind::Insert,
            "1",
            row(&[
                ("id", ScalarValue::Int64(Some(1))),
                ("name", ScalarValue::Utf8(Some("alice".to_string()))),
            ]),
            false,
        ));
        write_set.stage(StagedMutation::new(
            transaction_id,
            table_id.clone(),
            TableType::User,
            Some(second_user.clone()),
            OperationKind::Insert,
            "1",
            row(&[
                ("id", ScalarValue::Int64(Some(1))),
                ("name", ScalarValue::Utf8(Some("bob".to_string()))),
            ]),
            false,
        ));

        let first = write_set
            .latest_mutation_for_scope(&table_id, Some(&first_user), "1")
            .expect("first user mutation");
        let second = write_set
            .latest_mutation_for_scope(&table_id, Some(&second_user), "1")
            .expect("second user mutation");
        assert_eq!(
            first.payload.values.get("name"),
            Some(&ScalarValue::Utf8(Some("alice".to_string())))
        );
        assert_eq!(
            second.payload.values.get("name"),
            Some(&ScalarValue::Utf8(Some("bob".to_string())))
        );
    }

    fn stage(
        write_set: &mut TransactionWriteSet,
        transaction_id: &TransactionId,
        table_id: &TableId,
        table_type: TableType,
        user_id: Option<&UserId>,
        kind: OperationKind,
        primary_key: &str,
        values: &[(&'static str, ScalarValue)],
    ) {
        write_set.stage(StagedMutation::new(
            transaction_id.clone(),
            table_id.clone(),
            table_type,
            user_id.cloned(),
            kind,
            primary_key,
            row(values),
            matches!(kind, OperationKind::Delete),
        ));
    }

    fn assert_fold_for_scope(table_type: TableType, user_id: Option<UserId>) {
        let transaction_id = TransactionId::new("01960f7b-3d15-7d6d-b26c-7e4db6f25f8d");
        let table_id = TableId::new(SchemaId::new("app"), TableName::new("items"));
        let owner = user_id.as_ref();

        let mut missing_insert = TransactionWriteSet::new(transaction_id.clone());
        stage(
            &mut missing_insert,
            &transaction_id,
            &table_id,
            table_type,
            owner,
            OperationKind::Insert,
            "1",
            &[
                ("id", ScalarValue::Int64(Some(1))),
                ("name", ScalarValue::Utf8(Some("a".into()))),
            ],
        );
        let folded = missing_insert.fold_commit().expect("missing insert");
        assert_eq!(folded.len(), 1);
        assert_eq!(folded[0].operation_kind, OperationKind::Insert);

        let mut insert_update = TransactionWriteSet::new(transaction_id.clone());
        stage(
            &mut insert_update,
            &transaction_id,
            &table_id,
            table_type,
            owner,
            OperationKind::Insert,
            "1",
            &[
                ("id", ScalarValue::Int64(Some(1))),
                ("name", ScalarValue::Utf8(Some("a".into()))),
                ("color", ScalarValue::Utf8(Some("red".into()))),
            ],
        );
        stage(
            &mut insert_update,
            &transaction_id,
            &table_id,
            table_type,
            owner,
            OperationKind::Update,
            "1",
            &[("name", ScalarValue::Utf8(None))],
        );
        let folded = insert_update.fold_commit().expect("insert update");
        assert_eq!(folded.len(), 1);
        assert_eq!(folded[0].operation_kind, OperationKind::Insert);
        assert_eq!(folded[0].payload.values.get("name"), Some(&ScalarValue::Utf8(None)));
        assert_eq!(
            folded[0].payload.values.get("color"),
            Some(&ScalarValue::Utf8(Some("red".into())))
        );
        assert_eq!(insert_update.ordered_mutations().len(), 2);

        let mut update_update = TransactionWriteSet::new(transaction_id.clone());
        stage(
            &mut update_update,
            &transaction_id,
            &table_id,
            table_type,
            owner,
            OperationKind::Update,
            "1",
            &[("name", ScalarValue::Utf8(Some("a".into())))],
        );
        stage(
            &mut update_update,
            &transaction_id,
            &table_id,
            table_type,
            owner,
            OperationKind::Update,
            "1",
            &[("name", ScalarValue::Utf8(Some("b".into())))],
        );
        let folded = update_update.fold_commit().expect("update update");
        assert_eq!(folded.len(), 1);
        assert_eq!(folded[0].operation_kind, OperationKind::Update);
        assert_eq!(
            folded[0].payload.values.get("name"),
            Some(&ScalarValue::Utf8(Some("b".into())))
        );

        let mut insert_delete = TransactionWriteSet::new(transaction_id.clone());
        stage(
            &mut insert_delete,
            &transaction_id,
            &table_id,
            table_type,
            owner,
            OperationKind::Insert,
            "1",
            &[("id", ScalarValue::Int64(Some(1)))],
        );
        stage(
            &mut insert_delete,
            &transaction_id,
            &table_id,
            table_type,
            owner,
            OperationKind::Delete,
            "1",
            &[],
        );
        assert!(insert_delete.fold_commit().expect("insert delete").is_empty());
        assert_eq!(insert_delete.affected_rows(), 2);

        let mut update_delete = TransactionWriteSet::new(transaction_id.clone());
        stage(
            &mut update_delete,
            &transaction_id,
            &table_id,
            table_type,
            owner,
            OperationKind::Update,
            "1",
            &[("name", ScalarValue::Utf8(Some("gone".into())))],
        );
        stage(
            &mut update_delete,
            &transaction_id,
            &table_id,
            table_type,
            owner,
            OperationKind::Delete,
            "1",
            &[],
        );
        let folded = update_delete.fold_commit().expect("update delete");
        assert_eq!(folded.len(), 1);
        assert_eq!(folded[0].operation_kind, OperationKind::Delete);
        assert!(folded[0].tombstone);
        assert_eq!(
            folded[0].payload.values.get("name"),
            Some(&ScalarValue::Utf8(Some("gone".into())))
        );

        let mut delete_insert = TransactionWriteSet::new(transaction_id.clone());
        stage(
            &mut delete_insert,
            &transaction_id,
            &table_id,
            table_type,
            owner,
            OperationKind::Delete,
            "1",
            &[],
        );
        stage(
            &mut delete_insert,
            &transaction_id,
            &table_id,
            table_type,
            owner,
            OperationKind::Insert,
            "1",
            &[
                ("id", ScalarValue::Int64(Some(1))),
                ("name", ScalarValue::Utf8(Some("back".into()))),
            ],
        );
        let folded = delete_insert.fold_commit().expect("delete insert");
        assert_eq!(folded.len(), 1);
        assert_eq!(folded[0].operation_kind, OperationKind::Insert);
        assert_eq!(
            folded[0].payload.values.get("name"),
            Some(&ScalarValue::Utf8(Some("back".into())))
        );

        let mut insert_insert = TransactionWriteSet::new(transaction_id.clone());
        stage(
            &mut insert_insert,
            &transaction_id,
            &table_id,
            table_type,
            owner,
            OperationKind::Insert,
            "1",
            &[("id", ScalarValue::Int64(Some(1)))],
        );
        stage(
            &mut insert_insert,
            &transaction_id,
            &table_id,
            table_type,
            owner,
            OperationKind::Insert,
            "1",
            &[("id", ScalarValue::Int64(Some(1)))],
        );
        assert!(insert_insert.fold_commit().is_err());

        let mut delete_update = TransactionWriteSet::new(transaction_id.clone());
        stage(
            &mut delete_update,
            &transaction_id,
            &table_id,
            table_type,
            owner,
            OperationKind::Delete,
            "1",
            &[("name", ScalarValue::Utf8(Some("was".into())))],
        );
        stage(
            &mut delete_update,
            &transaction_id,
            &table_id,
            table_type,
            owner,
            OperationKind::Update,
            "1",
            &[("name", ScalarValue::Utf8(Some("nope".into())))],
        );
        let folded = delete_update.fold_commit().expect("delete update");
        assert_eq!(folded.len(), 1);
        assert_eq!(folded[0].operation_kind, OperationKind::Delete);
        assert_eq!(
            folded[0].payload.values.get("name"),
            Some(&ScalarValue::Utf8(Some("was".into())))
        );

        let mut delete_delete = TransactionWriteSet::new(transaction_id.clone());
        stage(
            &mut delete_delete,
            &transaction_id,
            &table_id,
            table_type,
            owner,
            OperationKind::Delete,
            "1",
            &[("name", ScalarValue::Utf8(Some("once".into())))],
        );
        stage(
            &mut delete_delete,
            &transaction_id,
            &table_id,
            table_type,
            owner,
            OperationKind::Delete,
            "1",
            &[],
        );
        let folded = delete_delete.fold_commit().expect("delete delete");
        assert_eq!(folded.len(), 1);
        assert_eq!(folded[0].operation_kind, OperationKind::Delete);

        let mut reinsert_delete = TransactionWriteSet::new(transaction_id.clone());
        stage(
            &mut reinsert_delete,
            &transaction_id,
            &table_id,
            table_type,
            owner,
            OperationKind::Delete,
            "1",
            &[("name", ScalarValue::Utf8(Some("was".into())))],
        );
        stage(
            &mut reinsert_delete,
            &transaction_id,
            &table_id,
            table_type,
            owner,
            OperationKind::Insert,
            "1",
            &[
                ("id", ScalarValue::Int64(Some(1))),
                ("name", ScalarValue::Utf8(Some("back".into()))),
            ],
        );
        stage(
            &mut reinsert_delete,
            &transaction_id,
            &table_id,
            table_type,
            owner,
            OperationKind::Delete,
            "1",
            &[],
        );
        let folded = reinsert_delete.fold_commit().expect("reinsert delete");
        assert_eq!(folded.len(), 1);
        assert_eq!(folded[0].operation_kind, OperationKind::Delete);
        assert!(folded[0].tombstone);
        assert_eq!(
            folded[0].payload.values.get("name"),
            Some(&ScalarValue::Utf8(Some("back".into())))
        );

        let mut noop_insert = TransactionWriteSet::new(transaction_id.clone());
        stage(
            &mut noop_insert,
            &transaction_id,
            &table_id,
            table_type,
            owner,
            OperationKind::Insert,
            "1",
            &[("id", ScalarValue::Int64(Some(1)))],
        );
        stage(
            &mut noop_insert,
            &transaction_id,
            &table_id,
            table_type,
            owner,
            OperationKind::Delete,
            "1",
            &[],
        );
        stage(
            &mut noop_insert,
            &transaction_id,
            &table_id,
            table_type,
            owner,
            OperationKind::Insert,
            "1",
            &[
                ("id", ScalarValue::Int64(Some(1))),
                ("name", ScalarValue::Utf8(Some("again".into()))),
            ],
        );
        let folded = noop_insert.fold_commit().expect("noop insert");
        assert_eq!(folded.len(), 1);
        assert_eq!(folded[0].operation_kind, OperationKind::Insert);
        assert_eq!(
            folded[0].payload.values.get("name"),
            Some(&ScalarValue::Utf8(Some("again".into())))
        );

        let mut noop_update = TransactionWriteSet::new(transaction_id.clone());
        stage(
            &mut noop_update,
            &transaction_id,
            &table_id,
            table_type,
            owner,
            OperationKind::Insert,
            "1",
            &[("id", ScalarValue::Int64(Some(1)))],
        );
        stage(
            &mut noop_update,
            &transaction_id,
            &table_id,
            table_type,
            owner,
            OperationKind::Delete,
            "1",
            &[],
        );
        stage(
            &mut noop_update,
            &transaction_id,
            &table_id,
            table_type,
            owner,
            OperationKind::Update,
            "1",
            &[("name", ScalarValue::Utf8(Some("invented".into())))],
        );
        assert!(noop_update.fold_commit().expect("noop update").is_empty());
    }

    #[test]
    fn fold_transitions_cover_shared_and_user_scopes() {
        assert_fold_for_scope(TableType::Shared, None);
        assert_fold_for_scope(TableType::User, Some(UserId::new("user-a")));
    }

    #[test]
    fn fold_keeps_distinct_user_scopes_and_savepoint_rewind() {
        let transaction_id = TransactionId::new("01960f7b-3d15-7d6d-b26c-7e4db6f25f8d");
        let table_id = TableId::new(SchemaId::new("app"), TableName::new("items"));
        let first_user = UserId::new("user-a");
        let second_user = UserId::new("user-b");
        let mut write_set = TransactionWriteSet::new(transaction_id.clone());
        stage(
            &mut write_set,
            &transaction_id,
            &table_id,
            TableType::User,
            Some(&first_user),
            OperationKind::Insert,
            "1",
            &[("name", ScalarValue::Utf8(Some("alice".into())))],
        );
        write_set.push_savepoint("sp");
        stage(
            &mut write_set,
            &transaction_id,
            &table_id,
            TableType::User,
            Some(&first_user),
            OperationKind::Update,
            "1",
            &[("name", ScalarValue::Utf8(Some("changed".into())))],
        );
        stage(
            &mut write_set,
            &transaction_id,
            &table_id,
            TableType::User,
            Some(&second_user),
            OperationKind::Insert,
            "1",
            &[("name", ScalarValue::Utf8(Some("bob".into())))],
        );
        write_set.rollback_to_savepoint("sp").expect("savepoint");
        let folded = write_set.fold_commit().expect("folded after rewind");
        assert_eq!(write_set.ordered_mutations().len(), 1);
        assert_eq!(folded.len(), 1);
        assert_eq!(folded[0].user_id.as_ref(), Some(&first_user));
        assert_eq!(
            folded[0].payload.values.get("name"),
            Some(&ScalarValue::Utf8(Some("alice".into())))
        );
    }

    #[test]
    fn fold_expands_primary_key_rename_and_collapses_one_key() {
        let transaction_id = TransactionId::new("01960f7b-3d15-7d6d-b26c-7e4db6f25f8d");
        let table_id = TableId::new(SchemaId::new("app"), TableName::new("items"));
        let mut write_set = TransactionWriteSet::new(transaction_id.clone());
        stage(
            &mut write_set,
            &transaction_id,
            &table_id,
            TableType::Shared,
            None,
            OperationKind::Insert,
            "1",
            &[
                ("id", ScalarValue::Int64(Some(1))),
                ("name", ScalarValue::Utf8(Some("a".into()))),
            ],
        );
        stage(
            &mut write_set,
            &transaction_id,
            &table_id,
            TableType::Shared,
            None,
            OperationKind::Update,
            "1",
            &[
                ("id", ScalarValue::Int64(Some(2))),
                ("name", ScalarValue::Utf8(Some("b".into()))),
            ],
        );
        let mut pk_columns = HashMap::new();
        pk_columns.insert(table_id.clone(), "id".to_string());
        let folded = write_set.fold_commit_with_primary_keys(&pk_columns).expect("rename");
        let new_row = folded.iter().find(|mutation| mutation.primary_key == "2").expect("new key");
        assert_eq!(new_row.operation_kind, OperationKind::Insert);
        assert_eq!(new_row.payload.values.get("name"), Some(&ScalarValue::Utf8(Some("b".into()))));
        assert!(folded.iter().all(|mutation| mutation.primary_key != "1"));

        let mut repeated = TransactionWriteSet::new(transaction_id.clone());
        for index in 0..25_000 {
            stage(
                &mut repeated,
                &transaction_id,
                &table_id,
                TableType::Shared,
                None,
                OperationKind::Update,
                "9",
                &[("n", ScalarValue::Int64(Some(index)))],
            );
        }
        let folded = repeated.fold_commit().expect("one key");
        assert_eq!(repeated.ordered_mutations().len(), 25_000);
        assert_eq!(folded.len(), 1);
        assert_eq!(folded[0].payload.values.get("n"), Some(&ScalarValue::Int64(Some(24_999))));
    }

    #[test]
    fn owned_fold_drops_history_payloads_and_keeps_noop_admission() {
        let transaction_id = TransactionId::new("01960f7b-3d15-7d6d-b26c-7e4db6f25f8d");
        let table_id = TableId::new(SchemaId::new("app"), TableName::new("items"));

        let mut distinct = TransactionWriteSet::new(transaction_id.clone());
        for index in 0..25_000 {
            stage(
                &mut distinct,
                &transaction_id,
                &table_id,
                TableType::Shared,
                None,
                OperationKind::Insert,
                &index.to_string(),
                &[("id", ScalarValue::Int64(Some(index)))],
            );
        }
        let history_bytes = distinct
            .ordered_mutations()
            .iter()
            .map(StagedMutation::approximate_size_bytes)
            .sum::<usize>();
        assert!(!distinct.overlay_cache.is_empty());
        let borrowed = distinct.fold_commit().expect("borrowed fold");
        let borrowed_bytes =
            borrowed.iter().map(StagedMutation::approximate_size_bytes).sum::<usize>();
        let retained_with_history = history_bytes + borrowed_bytes;
        let owned = distinct.fold_commit_owned().expect("owned fold");
        let owned_bytes =
            owned.folded.iter().map(StagedMutation::approximate_size_bytes).sum::<usize>();
        assert!(owned.admission_history.is_empty());
        assert_eq!(owned.folded.len(), borrowed.len());
        assert!(owned_bytes < retained_with_history);

        let mut one_key = TransactionWriteSet::new(transaction_id.clone());
        for index in 0..25_000 {
            stage(
                &mut one_key,
                &transaction_id,
                &table_id,
                TableType::Shared,
                None,
                OperationKind::Update,
                "9",
                &[("n", ScalarValue::Int64(Some(index)))],
            );
        }
        let one_key_history = one_key
            .ordered_mutations()
            .iter()
            .map(StagedMutation::approximate_size_bytes)
            .sum::<usize>();
        let owned_one_key = one_key.fold_commit_owned().expect("owned one key");
        let one_key_folded = owned_one_key
            .folded
            .iter()
            .map(StagedMutation::approximate_size_bytes)
            .sum::<usize>();
        assert_eq!(owned_one_key.folded.len(), 1);
        assert!(owned_one_key.admission_history.is_empty());
        assert!(one_key_folded < one_key_history);

        let mut cancelled = TransactionWriteSet::new(transaction_id.clone());
        stage(
            &mut cancelled,
            &transaction_id,
            &table_id,
            TableType::Shared,
            None,
            OperationKind::Insert,
            "1",
            &[("name", ScalarValue::Utf8(Some("row".into())))],
        );
        stage(
            &mut cancelled,
            &transaction_id,
            &table_id,
            TableType::Shared,
            None,
            OperationKind::Delete,
            "1",
            &[("name", ScalarValue::Utf8(Some("row".into())))],
        );
        let cancelled_bytes = cancelled
            .ordered_mutations()
            .iter()
            .map(StagedMutation::approximate_size_bytes)
            .sum::<usize>();
        let owned_cancel = cancelled.fold_commit_owned().expect("owned cancel");
        assert!(owned_cancel.folded.is_empty());
        assert_eq!(owned_cancel.admission_history.len(), 2);
        assert_eq!(owned_cancel.accounted_bytes, cancelled_bytes);
        assert!(owned_cancel
            .admission_history
            .iter()
            .all(|mutation| mutation.payload.values.is_empty()));
    }
}
