//! Hot-only projected scan shared by user and shared tables.
//!
//! The row walk is the shared-table body. User scans pass a user prefix and
//! decode `(user_id, version)` keys. Shared scans pass no prefix and decode a
//! version key. `Ok(None)` means the caller keeps the row-map scan.

use std::{cmp::Ordering, collections::HashMap, mem, sync::Arc};

use datafusion::{
    arrow::{
        datatypes::{DataType, Field, Schema, SchemaRef},
        record_batch::RecordBatch,
    },
    logical_expr::{Expr, Operator},
    scalar::ScalarValue,
};
use kalamdb_commons::{
    constants::SystemColumnNames,
    conversions::arrow_json_conversion::{
        coerce_scalar_for_arrow_field, scalar_columns_to_record_batch,
    },
    ids::{SharedTableRowId, UserTableRowId, VersionId},
    pk_bucket_key_from_scalar, KSerializable, PkBucketKey, StorageKey, UserId,
};
use kalamdb_datafusion_sources::exec::prefers_version;
use kalamdb_serialization::{project_row_fields, StorageSchema};
use kalamdb_store::{EntityStore, IndexedEntityStore};

use crate::{
    error::KalamDbError,
    utils::base::{
        authoritative_hot_index_plan, calculate_scan_limit, reject_truncated_unbounded_scan,
        TableProviderCore,
    },
};

#[derive(Clone, Copy)]
enum OutputColumn {
    Version,
    Deleted,
    Stored(usize),
}

struct PreparedScan {
    fields: Vec<Field>,
    outputs: Vec<OutputColumn>,
    decode_ordinals: Vec<usize>,
    pk_slot: usize,
    /// Drop a resolved row only when this proves the predicate false.
    ///
    /// Unknown predicates stay in the batch. The physical filter still runs.
    predicate: Option<HotPredicate>,
}

struct WinnerSlot {
    version: VersionId,
    row: Option<usize>,
}

pub(crate) fn decode_shared_version(bytes: &[u8]) -> Option<VersionId> {
    SharedTableRowId::from_storage_key(bytes).ok()
}

pub(crate) fn decode_user_version(bytes: &[u8]) -> Option<VersionId> {
    UserTableRowId::from_storage_key(bytes).ok().map(|id| id.version())
}

pub(crate) async fn run_columnar_hot_scan<K, V>(
    core: &TableProviderCore,
    store: &Arc<IndexedEntityStore<K, V>>,
    user_id: Option<&UserId>,
    snapshot: Option<VersionId>,
    projection: Option<&Vec<usize>>,
    filter: Option<&Expr>,
    limit: Option<usize>,
    keep_deleted: bool,
    chunk_rows: usize,
    decode_version: fn(&[u8]) -> Option<VersionId>,
) -> Result<Option<(Vec<RecordBatch>, usize)>, KalamDbError>
where
    K: StorageKey + Clone + Send + Sync + 'static,
    V: KSerializable + Clone + Send + Sync + 'static,
{
    if authoritative_hot_index_plan(store.as_ref(), filter, user_id, keep_deleted, None).is_some() {
        return Ok(None);
    }
    let manifest = core.manifest_service().get_or_load_async(core.table_id(), user_id).await;
    let hot_only = match &manifest {
        Ok(None) => true,
        Ok(Some(entry)) => entry.manifest.segments.is_empty(),
        Err(_) => false,
    };
    if !hot_only {
        return Ok(None);
    }
    let Some(prepared) = prepare_scan(
        Arc::clone(core.schema()),
        core.storage_schema(),
        projection,
        core.primary_key_field_name(),
        filter,
    ) else {
        return Ok(None);
    };

    let store = Arc::clone(store);
    let storage = Arc::clone(core.storage_schema());
    let prefix = user_id.map(UserTableRowId::user_prefix);
    tokio::task::spawn_blocking(move || {
        scan_hot_columnar(
            store.as_ref(),
            storage.as_ref(),
            &prepared,
            prefix.as_deref(),
            snapshot,
            limit,
            keep_deleted,
            chunk_rows,
            decode_version,
        )
    })
    .await
    .map_err(|error| {
        KalamDbError::InvalidOperation(format!("columnar hot scan task failed: {error}"))
    })?
}

fn prepare_scan(
    schema: SchemaRef,
    storage: &StorageSchema,
    projection: Option<&Vec<usize>>,
    pk_name: &str,
    filter: Option<&Expr>,
) -> Option<PreparedScan> {
    let pk_ordinal = storage
        .fields
        .iter()
        .position(|field| !field.dropped && field.name == pk_name)?;
    let indices: Vec<usize> = match projection {
        Some(indices) if !indices.is_empty() => indices.clone(),
        Some(_) => return None,
        None => (0..schema.fields().len()).collect(),
    };
    let mut fields = Vec::with_capacity(indices.len());
    let mut outputs = Vec::with_capacity(indices.len());
    let mut decode_ordinals = Vec::new();
    let mut slot_of: HashMap<usize, usize> = HashMap::new();
    let pk_slot = intern_ordinal(pk_ordinal, &mut decode_ordinals, &mut slot_of);
    for index in indices {
        let field = schema.field(index).clone();
        let kind = if field.name() == SystemColumnNames::VERSION {
            OutputColumn::Version
        } else if field.name() == SystemColumnNames::DELETED {
            OutputColumn::Deleted
        } else {
            let ordinal = storage
                .fields
                .iter()
                .position(|stored| !stored.dropped && stored.name == *field.name())?;
            OutputColumn::Stored(intern_ordinal(ordinal, &mut decode_ordinals, &mut slot_of))
        };
        fields.push(field);
        outputs.push(kind);
    }
    if let Some(filter) = filter {
        collect_filter_columns(filter, storage, &mut decode_ordinals, &mut slot_of);
    }
    let mut column_slots = HashMap::new();
    let mut column_fields = HashMap::new();
    for (slot, ordinal) in decode_ordinals.iter().copied().enumerate() {
        let Some(stored) = storage.fields.get(ordinal) else {
            continue;
        };
        let key = stored.name.to_ascii_lowercase();
        column_slots.insert(key.clone(), slot);
        if let Some(field) = schema
            .fields()
            .iter()
            .find(|field| field.name().eq_ignore_ascii_case(&stored.name))
        {
            column_fields.insert(key, field.as_ref().clone());
        }
    }
    // A lone primary-key equality has no physical filter behind this scan.
    // Leave that shape untouched. Every other predicate is checked again
    // after the batch is built, so dropping only proven misses is safe.
    let predicate = filter
        .filter(|expr| !pk_equality_only(expr, pk_name))
        .and_then(|expr| compile_hot_predicate(expr, &column_slots, &column_fields));
    Some(PreparedScan {
        fields,
        outputs,
        decode_ordinals,
        pk_slot,
        predicate,
    })
}

fn intern_ordinal(
    ordinal: usize,
    decode_ordinals: &mut Vec<usize>,
    slot_of: &mut HashMap<usize, usize>,
) -> usize {
    if let Some(slot) = slot_of.get(&ordinal) {
        return *slot;
    }
    let slot = decode_ordinals.len();
    decode_ordinals.push(ordinal);
    slot_of.insert(ordinal, slot);
    slot
}

fn scan_hot_columnar<K, V>(
    store: &IndexedEntityStore<K, V>,
    storage: &StorageSchema,
    prepared: &PreparedScan,
    prefix: Option<&[u8]>,
    snapshot: Option<VersionId>,
    limit: Option<usize>,
    keep_deleted: bool,
    chunk_rows: usize,
    decode_version: fn(&[u8]) -> Option<VersionId>,
) -> Result<Option<(Vec<RecordBatch>, usize)>, KalamDbError>
where
    K: StorageKey + Clone + Send + Sync + 'static,
    V: KSerializable + Clone + Send + Sync + 'static,
{
    let scan_limit = calculate_scan_limit(limit);
    let partition = store.partition();
    let iter =
        store
            .backend()
            .scan(&partition, prefix, None, Some(scan_limit))
            .map_err(|error| {
                KalamDbError::InvalidOperation(format!("columnar hot scan failed: {error}"))
            })?;

    let mut columns: Vec<Vec<ScalarValue>> =
        (0..prepared.outputs.len()).map(|_| Vec::new()).collect();
    let mut dead = Vec::new();
    let mut winners: HashMap<PkBucketKey, WinnerSlot> = HashMap::new();
    let mut decoded = vec![ScalarValue::Null; prepared.decode_ordinals.len()];
    let mut decoded_any = false;
    let mut visited = 0usize;

    for item in iter {
        let (key_bytes, value_bytes) = item.map_err(|error| {
            KalamDbError::InvalidOperation(format!("columnar hot scan failed: {error}"))
        })?;
        visited += 1;
        let Some(version) = decode_version(&key_bytes) else {
            log::warn!("Skipping hot row with a malformed key");
            continue;
        };
        if let Some(snapshot) = snapshot {
            if version > snapshot {
                continue;
            }
        }
        let deleted = match project_row_fields(
            &value_bytes,
            storage,
            &prepared.decode_ordinals,
            |index, value| {
                decoded[index] = value;
                Ok(())
            },
        ) {
            Ok(deleted) => deleted,
            Err(error) => {
                if !decoded_any {
                    log::warn!("columnar hot scan falling back: {error}");
                    return Ok(None);
                }
                log::warn!("Skipping hot row with a malformed value: {error}");
                continue;
            },
        };
        decoded_any = true;
        let pk_key = pk_bucket_key_from_scalar(&decoded[prepared.pk_slot], version);
        if let Some(existing) = winners.get(&pk_key) {
            if !prefers_version(version, existing.version) {
                continue;
            }
        }

        if (deleted && !keep_deleted)
            || prepared
                .predicate
                .as_ref()
                .is_some_and(|predicate| predicate.evaluate(&decoded) == PredicateTruth::False)
        {
            suppress_winner(&mut winners, &mut dead, pk_key, version);
            continue;
        }

        let mut values = Vec::with_capacity(prepared.outputs.len());
        for (offset, output) in prepared.outputs.iter().enumerate() {
            let raw = match output {
                OutputColumn::Version => ScalarValue::Int64(Some(version.as_i64())),
                OutputColumn::Deleted => ScalarValue::Boolean(Some(deleted)),
                OutputColumn::Stored(slot) => mem::replace(&mut decoded[*slot], ScalarValue::Null),
            };
            let coerced =
                coerce_scalar_for_arrow_field(raw, &prepared.fields[offset]).map_err(|error| {
                    KalamDbError::InvalidOperation(format!(
                        "columnar hot scan failed to coerce {}: {error}",
                        prepared.fields[offset].name()
                    ))
                })?;
            values.push(coerced);
        }

        if let Some(existing) = winners.get_mut(&pk_key) {
            if let Some(row) = existing.row {
                for (offset, value) in values.into_iter().enumerate() {
                    columns[offset][row] = value;
                }
                dead[row] = false;
                existing.version = version;
            } else {
                let row = dead.len();
                push_row(&mut columns, values);
                dead.push(false);
                existing.version = version;
                existing.row = Some(row);
            }
        } else {
            let row = dead.len();
            push_row(&mut columns, values);
            dead.push(false);
            winners.insert(
                pk_key,
                WinnerSlot {
                    version,
                    row: Some(row),
                },
            );
        }
    }

    reject_truncated_unbounded_scan(limit, visited)?;
    if dead.iter().any(|flag| *flag) {
        compact_dead_rows(&mut columns, &dead);
    }
    if let Some(limit) = limit {
        for column in &mut columns {
            column.truncate(limit);
        }
    }

    let batch_schema = Arc::new(Schema::new(prepared.fields.clone()));
    let batches = record_batches_from_columns(batch_schema, columns, chunk_rows)?;
    Ok(Some((batches, visited)))
}

/// Drain resolved scalar columns into owned batches of at most `chunk_rows`.
///
/// Each batch is a separate allocation. The caller can drop a batch without
/// keeping the rest of the scan. Version resolution has already finished, so
/// a later chunk cannot change an earlier row.
pub(crate) fn record_batches_from_columns(
    schema: SchemaRef,
    mut columns: Vec<Vec<ScalarValue>>,
    chunk_rows: usize,
) -> Result<Vec<RecordBatch>, KalamDbError> {
    let chunk_rows = chunk_rows.max(1);
    let total = columns.first().map(Vec::len).unwrap_or(0);
    if total == 0 {
        let batch = scalar_columns_to_record_batch(&schema, columns).map_err(|error| {
            KalamDbError::InvalidOperation(format!(
                "columnar hot scan failed to build arrow: {error}"
            ))
        })?;
        return Ok(vec![batch]);
    }
    let mut batches = Vec::with_capacity(total.div_ceil(chunk_rows));
    while columns.first().is_some_and(|column| !column.is_empty()) {
        let width = chunk_rows.min(columns[0].len());
        let mut piece = Vec::with_capacity(columns.len());
        for column in &mut columns {
            let tail = column.split_off(width);
            piece.push(mem::replace(column, tail));
        }
        let batch = scalar_columns_to_record_batch(&schema, piece).map_err(|error| {
            KalamDbError::InvalidOperation(format!(
                "columnar hot scan failed to build arrow: {error}"
            ))
        })?;
        batches.push(batch);
    }
    Ok(batches)
}

fn suppress_winner(
    winners: &mut HashMap<PkBucketKey, WinnerSlot>,
    dead: &mut Vec<bool>,
    pk_key: PkBucketKey,
    version: VersionId,
) {
    if let Some(existing) = winners.get_mut(&pk_key) {
        if let Some(row) = existing.row {
            dead[row] = true;
        }
        existing.version = version;
        existing.row = None;
    } else {
        winners.insert(pk_key, WinnerSlot { version, row: None });
    }
}

fn push_row(columns: &mut [Vec<ScalarValue>], values: Vec<ScalarValue>) {
    for (column, value) in columns.iter_mut().zip(values) {
        column.push(value);
    }
}

fn collect_filter_columns(
    expr: &Expr,
    storage: &StorageSchema,
    decode_ordinals: &mut Vec<usize>,
    slot_of: &mut HashMap<usize, usize>,
) {
    match expr {
        Expr::Alias(alias) => {
            collect_filter_columns(&alias.expr, storage, decode_ordinals, slot_of)
        },
        Expr::BinaryExpr(binary) => {
            collect_filter_columns(&binary.left, storage, decode_ordinals, slot_of);
            collect_filter_columns(&binary.right, storage, decode_ordinals, slot_of);
        },
        Expr::Not(inner)
        | Expr::IsNull(inner)
        | Expr::IsNotNull(inner)
        | Expr::IsTrue(inner)
        | Expr::IsFalse(inner)
        | Expr::IsUnknown(inner)
        | Expr::IsNotTrue(inner)
        | Expr::IsNotFalse(inner)
        | Expr::IsNotUnknown(inner)
        | Expr::Negative(inner)
        | Expr::Cast(datafusion::logical_expr::Cast { expr: inner, .. })
        | Expr::TryCast(datafusion::logical_expr::TryCast { expr: inner, .. }) => {
            collect_filter_columns(inner, storage, decode_ordinals, slot_of);
        },
        Expr::Like(like) | Expr::SimilarTo(like) => {
            collect_filter_columns(&like.expr, storage, decode_ordinals, slot_of);
            collect_filter_columns(&like.pattern, storage, decode_ordinals, slot_of);
        },
        Expr::Between(between) => {
            collect_filter_columns(&between.expr, storage, decode_ordinals, slot_of);
            collect_filter_columns(&between.low, storage, decode_ordinals, slot_of);
            collect_filter_columns(&between.high, storage, decode_ordinals, slot_of);
        },
        Expr::InList(list) => {
            collect_filter_columns(&list.expr, storage, decode_ordinals, slot_of);
            for item in &list.list {
                collect_filter_columns(item, storage, decode_ordinals, slot_of);
            }
        },
        Expr::Column(column) => {
            if let Some(ordinal) = storage
                .fields
                .iter()
                .position(|field| !field.dropped && field.name.eq_ignore_ascii_case(&column.name))
            {
                intern_ordinal(ordinal, decode_ordinals, slot_of);
            }
        },
        _ => {},
    }
}

/// Three-valued result. Only [`PredicateTruth::False`] drops a row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PredicateTruth {
    True,
    False,
    Unknown,
}

enum HotPredicate {
    Unknown,
    And(Box<HotPredicate>, Box<HotPredicate>),
    Or(Box<HotPredicate>, Box<HotPredicate>),
    Not(Box<HotPredicate>),
    Compare {
        slot: usize,
        op: Operator,
        literal: ScalarValue,
    },
    Like {
        slot: usize,
        kind: LikeShape,
        negated: bool,
    },
    IsNull {
        slot: usize,
        negated: bool,
    },
}

enum LikeShape {
    Contains(String),
    Prefix(String),
    Suffix(String),
    Exact(String),
}

impl HotPredicate {
    fn is_unknown(&self) -> bool {
        matches!(self, Self::Unknown)
    }

    fn evaluate(&self, decoded: &[ScalarValue]) -> PredicateTruth {
        match self {
            Self::Unknown => PredicateTruth::Unknown,
            Self::And(left, right) => match (left.evaluate(decoded), right.evaluate(decoded)) {
                (PredicateTruth::False, _) | (_, PredicateTruth::False) => PredicateTruth::False,
                (PredicateTruth::True, PredicateTruth::True) => PredicateTruth::True,
                _ => PredicateTruth::Unknown,
            },
            Self::Or(left, right) => match (left.evaluate(decoded), right.evaluate(decoded)) {
                (PredicateTruth::True, _) | (_, PredicateTruth::True) => PredicateTruth::True,
                (PredicateTruth::False, PredicateTruth::False) => PredicateTruth::False,
                _ => PredicateTruth::Unknown,
            },
            Self::Not(inner) => match inner.evaluate(decoded) {
                PredicateTruth::True => PredicateTruth::False,
                PredicateTruth::False => PredicateTruth::True,
                PredicateTruth::Unknown => PredicateTruth::Unknown,
            },
            Self::Compare { slot, op, literal } => decoded
                .get(*slot)
                .map(|value| compare_scalars(value, *op, literal))
                .unwrap_or(PredicateTruth::Unknown),
            Self::Like {
                slot,
                kind,
                negated,
            } => {
                let Some(value) = decoded.get(*slot).and_then(scalar_text) else {
                    return PredicateTruth::Unknown;
                };
                let matched = kind.matches(value);
                truth(if *negated { !matched } else { matched })
            },
            Self::IsNull { slot, negated } => {
                let Some(value) = decoded.get(*slot) else {
                    return PredicateTruth::Unknown;
                };
                let is_null = value.is_null();
                truth(if *negated { !is_null } else { is_null })
            },
        }
    }
}

impl LikeShape {
    fn matches(&self, value: &str) -> bool {
        match self {
            Self::Contains(needle) => value.contains(needle),
            Self::Prefix(needle) => value.starts_with(needle),
            Self::Suffix(needle) => value.ends_with(needle),
            Self::Exact(needle) => value == needle,
        }
    }
}

fn pk_equality_only(expr: &Expr, pk_name: &str) -> bool {
    let Expr::BinaryExpr(binary) = expr else {
        return false;
    };
    if binary.op != Operator::Eq {
        return false;
    }
    let column = match (binary.left.as_ref(), binary.right.as_ref()) {
        (Expr::Column(column), Expr::Literal(_, _))
        | (Expr::Literal(_, _), Expr::Column(column)) => column,
        _ => return false,
    };
    column.name.eq_ignore_ascii_case(pk_name)
}

fn compile_hot_predicate(
    expr: &Expr,
    slots: &HashMap<String, usize>,
    fields: &HashMap<String, Field>,
) -> Option<HotPredicate> {
    let node = compile_predicate_node(expr, slots, fields);
    if node.is_unknown() {
        None
    } else {
        Some(node)
    }
}

fn compile_predicate_node(
    expr: &Expr,
    slots: &HashMap<String, usize>,
    fields: &HashMap<String, Field>,
) -> HotPredicate {
    match expr {
        Expr::Alias(alias) => compile_predicate_node(&alias.expr, slots, fields),
        Expr::BinaryExpr(binary) => match binary.op {
            Operator::And => fold_and(
                compile_predicate_node(&binary.left, slots, fields),
                compile_predicate_node(&binary.right, slots, fields),
            ),
            Operator::Or => fold_or(
                compile_predicate_node(&binary.left, slots, fields),
                compile_predicate_node(&binary.right, slots, fields),
            ),
            Operator::Eq
            | Operator::NotEq
            | Operator::Lt
            | Operator::LtEq
            | Operator::Gt
            | Operator::GtEq => {
                compile_comparison(&binary.left, binary.op, &binary.right, slots, fields)
            },
            _ => HotPredicate::Unknown,
        },
        Expr::Not(inner) => match compile_predicate_node(inner, slots, fields) {
            HotPredicate::Unknown => HotPredicate::Unknown,
            node => HotPredicate::Not(Box::new(node)),
        },
        Expr::IsNull(inner) => compile_null_test(inner, false, slots),
        Expr::IsNotNull(inner) => compile_null_test(inner, true, slots),
        Expr::Like(like) => compile_like(like, slots, fields),
        Expr::Between(between) => {
            let low =
                compile_comparison(&between.expr, Operator::GtEq, &between.low, slots, fields);
            let high =
                compile_comparison(&between.expr, Operator::LtEq, &between.high, slots, fields);
            let range = fold_and(low, high);
            if between.negated {
                match range {
                    HotPredicate::Unknown => HotPredicate::Unknown,
                    node => HotPredicate::Not(Box::new(node)),
                }
            } else {
                range
            }
        },
        _ => HotPredicate::Unknown,
    }
}

fn fold_and(left: HotPredicate, right: HotPredicate) -> HotPredicate {
    match (left, right) {
        (HotPredicate::Unknown, other) | (other, HotPredicate::Unknown) => other,
        (left, right) => HotPredicate::And(Box::new(left), Box::new(right)),
    }
}

fn fold_or(left: HotPredicate, right: HotPredicate) -> HotPredicate {
    match (left, right) {
        (HotPredicate::Unknown, _) | (_, HotPredicate::Unknown) => HotPredicate::Unknown,
        (left, right) => HotPredicate::Or(Box::new(left), Box::new(right)),
    }
}

fn compile_null_test(expr: &Expr, negated: bool, slots: &HashMap<String, usize>) -> HotPredicate {
    let Some(name) = bare_column_name(expr) else {
        return HotPredicate::Unknown;
    };
    let Some(&slot) = slots.get(&name.to_ascii_lowercase()) else {
        return HotPredicate::Unknown;
    };
    HotPredicate::IsNull { slot, negated }
}

fn compile_like(
    like: &datafusion::logical_expr::Like,
    slots: &HashMap<String, usize>,
    fields: &HashMap<String, Field>,
) -> HotPredicate {
    if like.case_insensitive || like.escape_char.is_some() {
        return HotPredicate::Unknown;
    }
    let Some(name) = bare_column_name(&like.expr) else {
        return HotPredicate::Unknown;
    };
    let key = name.to_ascii_lowercase();
    let Some(&slot) = slots.get(&key) else {
        return HotPredicate::Unknown;
    };
    let Some(field) = fields.get(&key) else {
        return HotPredicate::Unknown;
    };
    if !matches!(field.data_type(), DataType::Utf8 | DataType::LargeUtf8 | DataType::Utf8View) {
        return HotPredicate::Unknown;
    }
    let Some(pattern) = plain_literal(&like.pattern, field.data_type()) else {
        return HotPredicate::Unknown;
    };
    let Some(pattern) = scalar_text(&pattern) else {
        return HotPredicate::Unknown;
    };
    let Some(kind) = like_shape(pattern) else {
        return HotPredicate::Unknown;
    };
    HotPredicate::Like {
        slot,
        kind,
        negated: like.negated,
    }
}

fn like_shape(pattern: &str) -> Option<LikeShape> {
    if pattern.contains('_') || pattern.contains('\\') {
        return None;
    }
    let starts = pattern.starts_with('%');
    let ends = pattern.ends_with('%');
    let core = pattern.trim_matches('%');
    if core.contains('%') {
        return None;
    }
    let core = core.to_string();
    Some(match (starts, ends) {
        (true, true) => LikeShape::Contains(core),
        (false, true) => LikeShape::Prefix(core),
        (true, false) => LikeShape::Suffix(core),
        (false, false) => LikeShape::Exact(core),
    })
}

fn compile_comparison(
    left: &Expr,
    op: Operator,
    right: &Expr,
    slots: &HashMap<String, usize>,
    fields: &HashMap<String, Field>,
) -> HotPredicate {
    if let Some(node) = compile_column_literal(left, op, right, slots, fields) {
        return node;
    }
    if let Some(flipped) = flip_comparison(op) {
        if let Some(node) = compile_column_literal(right, flipped, left, slots, fields) {
            return node;
        }
    }
    HotPredicate::Unknown
}

fn compile_column_literal(
    column_expr: &Expr,
    op: Operator,
    literal_expr: &Expr,
    slots: &HashMap<String, usize>,
    fields: &HashMap<String, Field>,
) -> Option<HotPredicate> {
    let name = bare_column_name(column_expr)?;
    let key = name.to_ascii_lowercase();
    let slot = *slots.get(&key)?;
    let field = fields.get(&key)?;
    if cast_type(column_expr).is_some_and(|casted| casted != field.data_type()) {
        return None;
    }
    let literal = plain_literal(literal_expr, field.data_type())?;
    let literal = coerce_scalar_for_arrow_field(literal, field).ok()?;
    Some(HotPredicate::Compare { slot, op, literal })
}

fn flip_comparison(op: Operator) -> Option<Operator> {
    Some(match op {
        Operator::Eq | Operator::NotEq => op,
        Operator::Lt => Operator::Gt,
        Operator::LtEq => Operator::GtEq,
        Operator::Gt => Operator::Lt,
        Operator::GtEq => Operator::LtEq,
        _ => return None,
    })
}

fn bare_column_name(expr: &Expr) -> Option<&str> {
    match expr {
        Expr::Column(column) => Some(column.name.as_str()),
        Expr::Alias(alias) => bare_column_name(&alias.expr),
        Expr::Cast(cast) => bare_column_name(&cast.expr),
        Expr::TryCast(cast) => bare_column_name(&cast.expr),
        _ => None,
    }
}

fn cast_type(expr: &Expr) -> Option<&DataType> {
    match expr {
        Expr::Cast(cast) => Some(cast.field.data_type()),
        Expr::TryCast(cast) => Some(cast.field.data_type()),
        Expr::Alias(alias) => cast_type(&alias.expr),
        _ => None,
    }
}

fn plain_literal(expr: &Expr, expected: &DataType) -> Option<ScalarValue> {
    match expr {
        Expr::Literal(value, _) => Some(value.clone()),
        Expr::Cast(cast) if cast.field.data_type() == expected => {
            plain_literal(&cast.expr, expected)
        },
        Expr::Alias(alias) => plain_literal(&alias.expr, expected),
        _ => None,
    }
}

fn truth(holds: bool) -> PredicateTruth {
    if holds {
        PredicateTruth::True
    } else {
        PredicateTruth::False
    }
}

fn compare_scalars(left: &ScalarValue, op: Operator, right: &ScalarValue) -> PredicateTruth {
    if left.is_null() || right.is_null() {
        return PredicateTruth::Unknown;
    }
    let Some(ordering) = left.partial_cmp(right) else {
        return PredicateTruth::Unknown;
    };
    let matched = match op {
        Operator::Eq => ordering == Ordering::Equal,
        Operator::NotEq => ordering != Ordering::Equal,
        Operator::Lt => ordering == Ordering::Less,
        Operator::LtEq => ordering != Ordering::Greater,
        Operator::Gt => ordering == Ordering::Greater,
        Operator::GtEq => ordering != Ordering::Less,
        _ => return PredicateTruth::Unknown,
    };
    if matched {
        PredicateTruth::True
    } else {
        PredicateTruth::False
    }
}

fn scalar_text(value: &ScalarValue) -> Option<&str> {
    match value {
        ScalarValue::Utf8(text) | ScalarValue::LargeUtf8(text) | ScalarValue::Utf8View(text) => {
            text.as_deref()
        },
        _ => None,
    }
}

fn compact_dead_rows(columns: &mut [Vec<ScalarValue>], dead: &[bool]) {
    let live: Vec<usize> = dead
        .iter()
        .enumerate()
        .filter_map(|(index, flag)| (!flag).then_some(index))
        .collect();
    for column in columns {
        let mut taken = mem::take(column);
        let mut kept = Vec::with_capacity(live.len());
        for index in &live {
            kept.push(mem::replace(&mut taken[*index], ScalarValue::Null));
        }
        *column = kept;
    }
}

#[cfg(test)]
mod tests {
    use datafusion::arrow::{
        array::{Array, Int64Array},
        datatypes::DataType,
    };

    use super::*;

    #[test]
    fn record_batches_from_columns_splits_owned_chunks() {
        let schema = Arc::new(Schema::new(vec![Field::new("id", DataType::Int64, true)]));
        let columns = vec![vec![
            ScalarValue::Int64(Some(1)),
            ScalarValue::Int64(Some(2)),
            ScalarValue::Int64(Some(3)),
            ScalarValue::Int64(Some(4)),
            ScalarValue::Int64(Some(5)),
        ]];

        let batches = record_batches_from_columns(schema, columns, 2).expect("split columns");

        assert_eq!(batches.len(), 3);
        assert_eq!(batches[0].num_rows(), 2);
        assert_eq!(batches[2].num_rows(), 1);
        let values: Vec<i64> = batches
            .iter()
            .flat_map(|batch| {
                batch
                    .column(0)
                    .as_any()
                    .downcast_ref::<Int64Array>()
                    .expect("int64 column")
                    .values()
                    .iter()
                    .copied()
            })
            .collect();
        assert_eq!(values, vec![1, 2, 3, 4, 5]);
    }

    #[test]
    fn hot_predicate_drops_only_rows_it_can_prove_false() {
        use datafusion::logical_expr::col;

        let mut slots = HashMap::new();
        slots.insert("a".to_string(), 0);
        slots.insert("c".to_string(), 1);
        let mut fields = HashMap::new();
        fields.insert("a".to_string(), Field::new("a", DataType::Int32, true));
        fields.insert("c".to_string(), Field::new("c", DataType::Utf8, true));

        let range = col("a").gt_eq(lit_i64(10)).and(col("a").lt(lit_i64(20)));
        let predicate = compile_hot_predicate(&range, &slots, &fields).expect("range predicate");
        let row = |value: Option<i32>| {
            vec![
                ScalarValue::Int32(value),
                ScalarValue::Utf8(Some("fifty two".to_string())),
            ]
        };
        assert_eq!(predicate.evaluate(&row(Some(5))), PredicateTruth::False);
        assert_eq!(predicate.evaluate(&row(Some(15))), PredicateTruth::True);
        assert_eq!(predicate.evaluate(&row(Some(20))), PredicateTruth::False);
        assert_eq!(predicate.evaluate(&row(None)), PredicateTruth::Unknown);

        let like = col("c").like(lit_utf8("%fifty%"));
        let predicate = compile_hot_predicate(&like, &slots, &fields).expect("like predicate");
        assert_eq!(predicate.evaluate(&row(Some(1))), PredicateTruth::True);
        let other = vec![
            ScalarValue::Int32(Some(1)),
            ScalarValue::Utf8(Some("one".to_string())),
        ];
        assert_eq!(predicate.evaluate(&other), PredicateTruth::False);

        let partial = col("a").gt(lit_i64(0)).or(col("missing").eq(lit_i64(1)));
        assert!(compile_hot_predicate(&partial, &slots, &fields).is_none());
    }

    fn lit_i64(value: i64) -> Expr {
        Expr::Literal(ScalarValue::Int64(Some(value)), None)
    }

    fn lit_utf8(value: &str) -> Expr {
        Expr::Literal(ScalarValue::Utf8(Some(value.to_string())), None)
    }
}
