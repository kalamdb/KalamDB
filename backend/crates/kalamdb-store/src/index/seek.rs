//! Prefix-index seek planning for DataFusion filters.
//!
//! Index values are still `scalar_value_to_bytes` (decimal strings for numbers)
//! wrapped in storekey tuples. Leading equalities are encoded as a prefix seek.
//! The next range is applied either as a UTF-8 byte bound or as a residual
//! comparison on decoded index-key columns — never as a numeric byte range,
//! because `"10"` sorts before `"2"`.

use std::collections::HashMap;

use datafusion::{
    logical_expr::{Expr, Operator},
    scalar::ScalarValue,
};
use kalamdb_commons::{
    conversions::scalar_value_to_bytes, models::UserId, storage_key::decode_key,
};

/// Comparison applied to one decoded index-key column before the row is fetched.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IndexCmpOp {
    Lt,
    LtEq,
    Gt,
    GtEq,
}

/// Residual predicate on a prefix-index column.
#[derive(Clone, Debug, PartialEq)]
pub struct IndexKeyPredicate {
    pub column_index: usize,
    pub op:           IndexCmpOp,
    pub bound:        ScalarValue,
}

/// Store-level scan recipe for one prefix index.
#[derive(Clone, Debug, PartialEq)]
pub struct IndexScanPlan {
    pub index_idx:     usize,
    pub prefix:        Vec<u8>,
    pub start_key:     Option<Vec<u8>>,
    pub exclusive_end: Option<Vec<u8>>,
    equality_columns:  u8,
    has_range:         bool,
    user_scoped:       bool,
    column_count:      u8,
    key_predicates:    Vec<IndexKeyPredicate>,
}

impl IndexScanPlan {
    /// Prefix-only plan used by indexes that only understand single equalities.
    pub fn equality_prefix(prefix: Vec<u8>) -> Self {
        Self {
            index_idx: 0,
            prefix,
            start_key: None,
            exclusive_end: None,
            equality_columns: 1,
            has_range: false,
            user_scoped: false,
            column_count: 1,
            key_predicates: Vec::new(),
        }
    }

    pub fn equality_columns(&self) -> u8 {
        self.equality_columns
    }

    pub fn has_range(&self) -> bool {
        self.has_range
    }

    pub fn covers_all_equality_columns(&self) -> bool {
        !self.has_range && self.equality_columns > 0 && self.equality_columns == self.column_count
    }

    pub fn key_predicates(&self) -> &[IndexKeyPredicate] {
        &self.key_predicates
    }

    /// Higher tuple wins. PK point lookups beat a single secondary equality.
    pub fn score(&self, is_pk: bool) -> (u8, u8, u8) {
        (
            self.equality_columns,
            u8::from(is_pk && self.covers_all_equality_columns()),
            u8::from(self.has_range),
        )
    }

    pub fn is_past_end(&self, index_key: &[u8]) -> bool {
        self.exclusive_end.as_deref().is_some_and(|end| index_key >= end)
    }

    /// Residual column checks. Unknown encodings fail open so rows are not dropped.
    pub fn matches_key_predicates(&self, index_key: &[u8]) -> bool {
        if self.key_predicates.is_empty() {
            return true;
        }
        let Some(columns) =
            decode_prefix_index_columns(index_key, self.user_scoped, self.column_count as usize)
        else {
            return true;
        };
        self.key_predicates.iter().all(|predicate| {
            columns
                .get(predicate.column_index)
                .map(|bytes| compare_index_bytes(bytes, predicate.op, &predicate.bound))
                .unwrap_or(true)
        })
    }
}

#[derive(Default)]
struct ColumnConstraints {
    equal: Option<ScalarValue>,
    lower: Option<(ScalarValue, bool)>,
    upper: Option<(ScalarValue, bool)>,
}

impl ColumnConstraints {
    fn bounds_are_utf8(&self) -> bool {
        self.lower.as_ref().map(|(v, _)| is_utf8(v)).unwrap_or(true)
            && self.upper.as_ref().map(|(v, _)| is_utf8(v)).unwrap_or(true)
    }
}

/// Build a seek plan from flattened (or AND-tree) conjuncts.
pub fn plan_prefix_index_scan(
    columns: &[String],
    user_scoped: bool,
    user_id: Option<&UserId>,
    conjuncts: &[Expr],
    encode_prefix: impl Fn(Option<&UserId>, &[Vec<u8>]) -> Vec<u8>,
    encode_user_prefix: impl Fn(&UserId) -> Vec<u8>,
) -> Option<IndexScanPlan> {
    if columns.is_empty() {
        return None;
    }
    if user_scoped && user_id.is_none() {
        return None;
    }

    let mut flat = Vec::with_capacity(conjuncts.len());
    for conjunct in conjuncts {
        flatten_conjuncts(conjunct, &mut flat);
    }
    let constraints = collect_constraints(&flat);

    let mut column_bytes = Vec::new();
    let mut equality_columns = 0u8;
    let mut has_range = false;
    let mut key_predicates = Vec::new();
    let mut start_key = None;
    let mut exclusive_end = None;

    for (column_index, column) in columns.iter().enumerate() {
        let Some(constraint) = constraints.get(column) else {
            break;
        };
        if let Some(value) = &constraint.equal {
            column_bytes.push(scalar_value_to_bytes(value));
            equality_columns += 1;
            continue;
        }
        if constraint.lower.is_none() && constraint.upper.is_none() {
            break;
        }

        let order_preserving = constraint.bounds_are_utf8();
        if column_bytes.is_empty() && !order_preserving {
            return None;
        }

        has_range = true;
        if order_preserving {
            if let Some((lower, _)) = &constraint.lower {
                let mut start_cols = column_bytes.clone();
                start_cols.push(scalar_value_to_bytes(lower));
                start_key = Some(encode_prefix(user_id, &start_cols));
            }
            if let Some((upper, false)) = &constraint.upper {
                let mut end_cols = column_bytes.clone();
                end_cols.push(scalar_value_to_bytes(upper));
                exclusive_end = Some(encode_prefix(user_id, &end_cols));
            }
        }

        if let Some((lower, inclusive)) = &constraint.lower {
            key_predicates.push(IndexKeyPredicate {
                column_index,
                op: if *inclusive {
                    IndexCmpOp::GtEq
                } else {
                    IndexCmpOp::Gt
                },
                bound: lower.clone(),
            });
        }
        if let Some((upper, inclusive)) = &constraint.upper {
            key_predicates.push(IndexKeyPredicate {
                column_index,
                op: if *inclusive {
                    IndexCmpOp::LtEq
                } else {
                    IndexCmpOp::Lt
                },
                bound: upper.clone(),
            });
        }
        break;
    }

    if equality_columns == 0 && !has_range {
        return None;
    }

    let prefix = if column_bytes.is_empty() {
        if user_scoped {
            encode_user_prefix(user_id?)
        } else {
            Vec::new()
        }
    } else {
        encode_prefix(user_id, &column_bytes)
    };

    Some(IndexScanPlan {
        index_idx: 0,
        prefix,
        start_key,
        exclusive_end,
        equality_columns,
        has_range,
        user_scoped,
        column_count: u8::try_from(columns.len()).unwrap_or(u8::MAX),
        key_predicates,
    })
}

pub(crate) fn flatten_conjuncts(expr: &Expr, out: &mut Vec<Expr>) {
    match expr {
        Expr::BinaryExpr(binary) if binary.op == Operator::And => {
            flatten_conjuncts(binary.left.as_ref(), out);
            flatten_conjuncts(binary.right.as_ref(), out);
        },
        other => out.push(other.clone()),
    }
}

fn collect_constraints(conjuncts: &[Expr]) -> HashMap<String, ColumnConstraints> {
    let mut constraints: HashMap<String, ColumnConstraints> = HashMap::new();
    for expr in conjuncts {
        if let Expr::Between(between) = expr {
            if between.negated {
                continue;
            }
            let Some(column) = unwrap_column(between.expr.as_ref()) else {
                continue;
            };
            let Some(low) = unwrap_literal(between.low.as_ref()) else {
                continue;
            };
            let Some(high) = unwrap_literal(between.high.as_ref()) else {
                continue;
            };
            let entry = constraints.entry(column.to_string()).or_default();
            if entry.lower.is_none() {
                entry.lower = Some((low.clone(), true));
            }
            if entry.upper.is_none() {
                entry.upper = Some((high.clone(), true));
            }
            continue;
        }

        let Some((column, op, value)) = comparison_parts(expr) else {
            continue;
        };
        let entry = constraints.entry(column.to_string()).or_default();
        match op {
            Operator::Eq if entry.equal.is_none() => entry.equal = Some(value.clone()),
            Operator::Gt if entry.lower.is_none() => {
                entry.lower = Some((value.clone(), false));
            },
            Operator::GtEq if entry.lower.is_none() => {
                entry.lower = Some((value.clone(), true));
            },
            Operator::Lt if entry.upper.is_none() => {
                entry.upper = Some((value.clone(), false));
            },
            Operator::LtEq if entry.upper.is_none() => {
                entry.upper = Some((value.clone(), true));
            },
            _ => {},
        }
    }
    constraints
}

fn comparison_parts(expr: &Expr) -> Option<(&str, Operator, &ScalarValue)> {
    let Expr::BinaryExpr(binary) = expr else {
        return None;
    };
    if !matches!(
        binary.op,
        Operator::Eq | Operator::Lt | Operator::LtEq | Operator::Gt | Operator::GtEq
    ) {
        return None;
    }
    if let (Some(column), Some(value)) =
        (unwrap_column(binary.left.as_ref()), unwrap_literal(binary.right.as_ref()))
    {
        return Some((column, binary.op, value));
    }
    if let (Some(value), Some(column)) =
        (unwrap_literal(binary.left.as_ref()), unwrap_column(binary.right.as_ref()))
    {
        return Some((column, flip_comparison(binary.op), value));
    }
    None
}

fn flip_comparison(op: Operator) -> Operator {
    match op {
        Operator::Lt => Operator::Gt,
        Operator::LtEq => Operator::GtEq,
        Operator::Gt => Operator::Lt,
        Operator::GtEq => Operator::LtEq,
        other => other,
    }
}

fn unwrap_column(expr: &Expr) -> Option<&str> {
    match expr {
        Expr::Column(column) => Some(column.name.as_str()),
        Expr::Alias(alias) => unwrap_column(alias.expr.as_ref()),
        Expr::Cast(cast) => unwrap_column(cast.expr.as_ref()),
        Expr::TryCast(cast) => unwrap_column(cast.expr.as_ref()),
        _ => None,
    }
}

fn unwrap_literal(expr: &Expr) -> Option<&ScalarValue> {
    match expr {
        Expr::Literal(value, _) if !value.is_null() => Some(value),
        Expr::Alias(alias) => unwrap_literal(alias.expr.as_ref()),
        Expr::Cast(cast) => unwrap_literal(cast.expr.as_ref()),
        Expr::TryCast(cast) => unwrap_literal(cast.expr.as_ref()),
        _ => None,
    }
}

fn is_utf8(value: &ScalarValue) -> bool {
    matches!(value, ScalarValue::Utf8(Some(_)) | ScalarValue::LargeUtf8(Some(_)))
}

fn decode_prefix_index_columns(
    key: &[u8],
    user_scoped: bool,
    column_count: usize,
) -> Option<Vec<Vec<u8>>> {
    match (user_scoped, column_count) {
        (false, 1) => {
            let (a, _seq): (Vec<u8>, i64) = decode_key(key).ok()?;
            Some(vec![a])
        },
        (false, 2) => {
            let (a, b, _seq): (Vec<u8>, Vec<u8>, i64) = decode_key(key).ok()?;
            Some(vec![a, b])
        },
        (false, 3) => {
            let (a, b, c, _seq): (Vec<u8>, Vec<u8>, Vec<u8>, i64) = decode_key(key).ok()?;
            Some(vec![a, b, c])
        },
        (false, 4) => {
            let (a, b, c, d, _seq): (Vec<u8>, Vec<u8>, Vec<u8>, Vec<u8>, i64) =
                decode_key(key).ok()?;
            Some(vec![a, b, c, d])
        },
        (true, 1) => {
            let (_user, a, _seq): (String, Vec<u8>, i64) = decode_key(key).ok()?;
            Some(vec![a])
        },
        (true, 2) => {
            let (_user, a, b, _seq): (String, Vec<u8>, Vec<u8>, i64) = decode_key(key).ok()?;
            Some(vec![a, b])
        },
        (true, 3) => {
            let (_user, a, b, c, _seq): (String, Vec<u8>, Vec<u8>, Vec<u8>, i64) =
                decode_key(key).ok()?;
            Some(vec![a, b, c])
        },
        (true, 4) => {
            let (_user, a, b, c, d, _seq): (String, Vec<u8>, Vec<u8>, Vec<u8>, Vec<u8>, i64) =
                decode_key(key).ok()?;
            Some(vec![a, b, c, d])
        },
        _ => None,
    }
}

fn compare_index_bytes(stored: &[u8], op: IndexCmpOp, bound: &ScalarValue) -> bool {
    if let Some(bound_int) = bound_i128(bound) {
        let Ok(text) = std::str::from_utf8(stored) else {
            return true;
        };
        let Ok(stored_int) = text.parse::<i128>() else {
            return true;
        };
        return cmp_ord(stored_int, bound_int, op);
    }
    if let Some(bound_text) = bound_utf8(bound) {
        return cmp_ord(stored, bound_text.as_bytes(), op);
    }
    true
}

fn bound_i128(value: &ScalarValue) -> Option<i128> {
    match value {
        ScalarValue::Int8(Some(n)) => Some(i128::from(*n)),
        ScalarValue::Int16(Some(n)) => Some(i128::from(*n)),
        ScalarValue::Int32(Some(n)) => Some(i128::from(*n)),
        ScalarValue::Int64(Some(n)) => Some(i128::from(*n)),
        ScalarValue::UInt8(Some(n)) => Some(i128::from(*n)),
        ScalarValue::UInt16(Some(n)) => Some(i128::from(*n)),
        ScalarValue::UInt32(Some(n)) => Some(i128::from(*n)),
        ScalarValue::UInt64(Some(n)) => Some(i128::from(*n)),
        ScalarValue::Utf8(Some(text))
        | ScalarValue::LargeUtf8(Some(text))
        | ScalarValue::Utf8View(Some(text)) => text.parse().ok(),
        _ => None,
    }
}

fn bound_utf8(value: &ScalarValue) -> Option<&str> {
    match value {
        ScalarValue::Utf8(Some(text))
        | ScalarValue::LargeUtf8(Some(text))
        | ScalarValue::Utf8View(Some(text)) => Some(text.as_str()),
        _ => None,
    }
}

fn cmp_ord<T: Ord>(left: T, right: T, op: IndexCmpOp) -> bool {
    match op {
        IndexCmpOp::Lt => left < right,
        IndexCmpOp::LtEq => left <= right,
        IndexCmpOp::Gt => left > right,
        IndexCmpOp::GtEq => left >= right,
    }
}

#[cfg(test)]
mod tests {
    use datafusion::logical_expr::{col, lit};
    use kalamdb_commons::{ids::SeqId, KSerializable};

    use super::*;
    use crate::{
        index::prefix::{PrefixIndex, PrefixIndexedValue},
        IndexDefinition,
    };

    #[derive(Clone, serde::Serialize, serde::Deserialize)]
    struct TestRow {
        fields: std::collections::BTreeMap<String, Vec<u8>>,
    }

    impl KSerializable for TestRow {}

    impl PrefixIndexedValue for TestRow {
        fn prefix_index_field_bytes(&self, column: &str) -> Option<Vec<u8>> {
            self.fields.get(column).cloned()
        }
    }

    fn conversation_index() -> PrefixIndex<SeqId, TestRow> {
        PrefixIndex::new(
            "shared_chat:messages_idx_conversation",
            vec!["conversation_id".to_string(), "created_at_ms".to_string()],
            false,
        )
    }

    fn plan_for(index: &PrefixIndex<SeqId, TestRow>, expr: &Expr) -> IndexScanPlan {
        index.plan_scan(None, std::slice::from_ref(expr)).expect("plan")
    }

    #[test]
    fn composite_equality_encodes_both_columns() {
        let index = conversation_index();
        let expr = col("conversation_id").eq(lit("42")).and(col("created_at_ms").eq(lit("1000")));
        let plan = plan_for(&index, &expr);
        let expected = index.encode_column_prefix(None, &[b"42".to_vec(), b"1000".to_vec()]);
        assert_eq!(plan.prefix, expected);
        assert_eq!(plan.equality_columns(), 2);
        assert!(!plan.has_range());
        assert!(plan.key_predicates().is_empty());
    }

    #[test]
    fn numeric_range_keeps_equality_prefix_and_residual_predicate() {
        let index = conversation_index();
        let expr = col("conversation_id").eq(lit(1_i64)).and(col("created_at_ms").lt(lit(15_i64)));
        let plan = plan_for(&index, &expr);
        assert_eq!(plan.prefix, index.encode_column_prefix(None, &[b"1".to_vec()]));
        assert!(plan.start_key.is_none());
        assert!(plan.exclusive_end.is_none());
        assert_eq!(plan.key_predicates().len(), 1);
        assert_eq!(plan.key_predicates()[0].column_index, 1);
        assert_eq!(plan.key_predicates()[0].op, IndexCmpOp::Lt);
    }

    #[test]
    fn numeric_residual_includes_digit_boundary_values() {
        let index = conversation_index();
        let expr = col("conversation_id").eq(lit(1_i64)).and(col("created_at_ms").lt(lit(15_i64)));
        let plan = plan_for(&index, &expr);

        let key_for = |created_at: &str| {
            let mut fields = std::collections::BTreeMap::new();
            fields.insert("conversation_id".to_string(), b"1".to_vec());
            fields.insert("created_at_ms".to_string(), created_at.as_bytes().to_vec());
            index.extract_key(&SeqId::new(1), &TestRow { fields }).unwrap()
        };

        assert!(plan.matches_key_predicates(&key_for("2")));
        assert!(plan.matches_key_predicates(&key_for("10")));
        assert!(!plan.matches_key_predicates(&key_for("20")));
    }

    #[test]
    fn utf8_range_sets_start_and_exclusive_end() {
        let index = PrefixIndex::<SeqId, TestRow>::new(
            "shared_chat:messages_idx_name",
            vec!["conversation_id".to_string(), "name".to_string()],
            false,
        );
        let expr = col("conversation_id")
            .eq(lit("room-a"))
            .and(col("name").gt_eq(lit("alice")))
            .and(col("name").lt(lit("carol")));
        let plan = index.plan_scan(None, std::slice::from_ref(&expr)).unwrap();
        assert_eq!(
            plan.start_key.as_deref(),
            Some(
                index
                    .encode_column_prefix(None, &[b"room-a".to_vec(), b"alice".to_vec()])
                    .as_slice()
            )
        );
        assert_eq!(
            plan.exclusive_end.as_deref(),
            Some(
                index
                    .encode_column_prefix(None, &[b"room-a".to_vec(), b"carol".to_vec()])
                    .as_slice()
            )
        );
    }

    fn name_key(
        index: &PrefixIndex<SeqId, TestRow>,
        conversation_id: &str,
        name: &str,
        seq: i64,
    ) -> Vec<u8> {
        let mut fields = std::collections::BTreeMap::new();
        fields.insert("conversation_id".to_string(), conversation_id.as_bytes().to_vec());
        fields.insert("name".to_string(), name.as_bytes().to_vec());
        index.extract_key(&SeqId::new(seq), &TestRow { fields }).unwrap()
    }

    #[test]
    fn two_column_equality_prefix_is_prefix_of_full_keys() {
        let index = conversation_index();
        let mut fields = std::collections::BTreeMap::new();
        fields.insert("conversation_id".to_string(), b"42".to_vec());
        fields.insert("created_at_ms".to_string(), b"1000".to_vec());
        let key = index.extract_key(&SeqId::new(7), &TestRow { fields }).unwrap();
        let prefix = index.encode_column_prefix(None, &[b"42".to_vec(), b"1000".to_vec()]);
        assert!(key.starts_with(&prefix), "composite equality prefix must seek full index keys");
    }

    #[test]
    fn utf8_range_bounds_partition_actual_index_keys() {
        let index = PrefixIndex::<SeqId, TestRow>::new(
            "shared_chat:messages_idx_name",
            vec!["conversation_id".to_string(), "name".to_string()],
            false,
        );
        let expr = col("conversation_id")
            .eq(lit("room-a"))
            .and(col("name").gt_eq(lit("alice")))
            .and(col("name").lt(lit("carol")));
        let plan = index.plan_scan(None, std::slice::from_ref(&expr)).unwrap();
        let start = plan.start_key.as_deref().expect("utf8 start");
        let alice = name_key(&index, "room-a", "alice", 1);
        let bob = name_key(&index, "room-a", "bob", 2);
        let carol = name_key(&index, "room-a", "carol", 3);
        let dave = name_key(&index, "room-a", "dave", 4);

        assert!(alice.as_slice() >= start);
        assert!(bob.as_slice() >= start);
        assert!(carol.as_slice() >= start);
        assert!(!plan.is_past_end(&alice), "inclusive lower bound must keep alice");
        assert!(!plan.is_past_end(&bob), "bob is before exclusive carol");
        assert!(plan.is_past_end(&carol), "exclusive upper bound must stop at carol");
        assert!(plan.is_past_end(&dave));
        assert!(plan.matches_key_predicates(&alice));
        assert!(plan.matches_key_predicates(&bob));
        assert!(!plan.matches_key_predicates(&carol));
    }

    #[test]
    fn first_column_numeric_range_is_not_planned() {
        let index = conversation_index();
        let expr = col("created_at_ms").lt(lit(15_i64));
        assert!(index.plan_scan(None, std::slice::from_ref(&expr)).is_none());
    }

    #[test]
    fn pk_point_lookup_outscores_single_secondary_equality() {
        let pk = IndexScanPlan::equality_prefix(b"id".to_vec());
        let secondary = IndexScanPlan::equality_prefix(b"conversation".to_vec());
        assert!(pk.score(true) > secondary.score(false));
    }

    #[test]
    fn two_equalities_outscore_pk_point_lookup() {
        let mut secondary = IndexScanPlan::equality_prefix(b"conv".to_vec());
        secondary.equality_columns = 2;
        secondary.column_count = 2;
        let pk = IndexScanPlan::equality_prefix(b"id".to_vec());
        assert!(secondary.score(false) > pk.score(true));
    }

    #[test]
    fn user_scoped_range_plan_stays_inside_user_prefix() {
        use kalamdb_commons::ids::UserTableRowId;

        let index = PrefixIndex::<UserTableRowId, TestRow>::new(
            "user_chat:messages_idx_conversation",
            vec!["conversation_id".to_string(), "created_at_ms".to_string()],
            true,
        );
        let user = UserId::new("alice");
        let expr = col("conversation_id").eq(lit(1_i64)).and(col("created_at_ms").lt(lit(15_i64)));
        let plan = index.plan_scan(Some(&user), std::slice::from_ref(&expr)).unwrap();
        assert!(plan.prefix.starts_with(&index.encode_user_prefix(&user)));
        assert_eq!(plan.equality_columns(), 1);
        assert!(plan.has_range());
        assert!(index.plan_scan(None, std::slice::from_ref(&expr)).is_none());
    }
}
