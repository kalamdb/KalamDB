//! KalamDB issue #414: reference proof of concept, NOT production live-query code.
//!
//! DataFusion 55.2.0 builds the logical plan. This module:
//! - discovers all source tables, equi-JOIN keys, row-local filters and aggregates;
//! - chooses a client-JOIN *candidate* vs server SQL refresh;
//! - tracks change dependencies, old/new GROUP BY keys and coalesced refresh generations;
//! - models dynamic parent-key activation and revocation for selective child streams.
//!
//! SECURITY: This PoC never opens subscriptions or transmits rows. Production MUST
//! bind RLS to every source, bound dynamic key backfills and guard snapshot/replay.

use std::collections::{BTreeSet, HashMap, HashSet};

use datafusion::{
    common::{Column, JoinType, ScalarValue},
    logical_expr::{Expr, LogicalPlan, Operator},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecutionMode {
    SingleTableDelta,
    ClientJoinCandidate,
    ServerQueryRefresh,
}

#[derive(Debug, Clone)]
pub struct SourcePlan {
    pub table: String,
    pub alias: String,
    pub required_columns: BTreeSet<String>,
    pub local_filters: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JoinKeys {
    pub left: Column,
    pub right: Column,
}

#[derive(Debug, Clone)]
pub struct LivePlan {
    pub original_sql: String,
    pub mode: ExecutionMode,
    pub sources: Vec<SourcePlan>,
    pub join_keys: Vec<JoinKeys>,
    pub group_by: Vec<Expr>,
    pub aggregate_expressions: Vec<String>,
    pub result_columns: Vec<String>,
    pub reason: String,
}

#[derive(Default)]
struct Inspection {
    sources: Vec<SourcePlan>,
    filters: Vec<Expr>,
    projections: Option<Vec<Expr>>,
    join_keys: Vec<JoinKeys>,
    has_join: bool,
    complex_join: bool,
    aggregate: bool,
    group_by: Vec<Expr>,
    aggregate_expressions: Vec<String>,
    has_sort_or_limit: bool,
}

/// Inspects a DataFusion *unoptimized* plan. Call with:
/// context.sql(sql).await?.into_unoptimized_plan().
/// Unknown plan nodes fail closed: we MUST NOT silently miss dependencies.
pub fn prepare_live_plan(sql: &str, plan: &LogicalPlan) -> Result<LivePlan, String> {
    let mut inspection = Inspection::default();
    walk(plan, &mut inspection)?;
    if inspection.sources.is_empty() {
        return Err("live SELECT has no discoverable base tables".into());
    }

    let mut cross_table_filter = false;
    let all_expressions: Vec<Expr> = inspection
        .projections
        .iter()
        .flatten()
        .cloned()
        .chain(inspection.filters.iter().cloned())
        .chain(inspection.group_by.iter().cloned())
        .collect();

    for expr in &all_expressions {
        add_referenced_columns(expr, &mut inspection.sources);
    }
    for pair in &inspection.join_keys {
        add_referenced_columns(&Expr::Column(pair.left.clone()), &mut inspection.sources);
        add_referenced_columns(&Expr::Column(pair.right.clone()), &mut inspection.sources);
    }

    for predicate in &inspection.filters {
        let columns = predicate.column_refs();
        let source_ids: HashSet<usize> = columns
            .iter()
            .filter_map(|col| source_for_column(col, &inspection.sources))
            .collect();
        if columns.is_empty() || source_ids.len() != 1 || source_ids.len() != columns.len().min(1) {
            cross_table_filter = true;
        } else if let Some(idx) = source_ids.into_iter().next() {
            inspection.sources[idx].local_filters.push(predicate.to_string());
        }
    }

    let simple_projection = inspection
        .projections
        .as_ref()
        .is_some_and(|expressions| {
            expressions.iter().all(|expr| match expr {
                Expr::Column(_) => true,
                Expr::Alias(alias) => matches!(alias.expr.as_ref(), Expr::Column(_)),
                _ => false,
            })
        });

    let (mode, reason) = if inspection.aggregate {
        (
            ExecutionMode::ServerQueryRefresh,
            "GROUP BY / aggregate: rerun original SQL after coalesced table invalidations",
        )
    } else if inspection.has_join
        && inspection.sources.len() == 2
        && inspection.join_keys.len() >= 1
        && !inspection.complex_join
        && !cross_table_filter
        && !inspection.has_sort_or_limit
        && simple_projection
        && inspection.sources[0].table != inspection.sources[1].table
    {
        (
            ExecutionMode::ClientJoinCandidate,
            "INNER equi-JOIN: server must enforce source RLS, dynamic dependent keys and bounds",
        )
    } else if !inspection.has_join
        && inspection.sources.len() == 1
        && !inspection.has_sort_or_limit
        && !cross_table_filter
        && simple_projection
    {
        (ExecutionMode::SingleTableDelta, "existing single-table live path")
    } else {
        (
            ExecutionMode::ServerQueryRefresh,
            "complex SELECT: invalidate on every dependency table, rerun original SQL",
        )
    };

    Ok(LivePlan {
        original_sql: sql.to_owned(),
        mode,
        sources: inspection.sources,
        join_keys: inspection.join_keys,
        group_by: inspection.group_by,
        aggregate_expressions: inspection.aggregate_expressions,
        result_columns: plan
            .schema()
            .fields()
            .iter()
            .map(|field| field.name().to_string())
            .collect(),
        reason: reason.to_owned(),
    })
}

fn walk(plan: &LogicalPlan, out: &mut Inspection) -> Result<(), String> {
    match plan {
        LogicalPlan::Projection(projection) => {
            if out.projections.is_none() {
                out.projections = Some(projection.expr.clone());
            }
            walk(&projection.input, out)
        }
        LogicalPlan::Filter(filter) => {
            split_and(&filter.predicate, &mut out.filters);
            walk(&filter.input, out)
        }
        LogicalPlan::Join(join) => {
            out.has_join = true;
            if join.join_type != JoinType::Inner
                || join.filter.is_some()
                || join.on.is_empty()
            {
                out.complex_join = true;
            }
            for (left, right) in &join.on {
                match (left, right) {
                    (Expr::Column(l), Expr::Column(r)) => out.join_keys.push(JoinKeys {
                        left: l.clone(),
                        right: r.clone(),
                    }),
                    _ => out.complex_join = true,
                }
            }
            walk(&join.left, out)?;
            walk(&join.right, out)
        }
        LogicalPlan::Aggregate(aggregate) => {
            out.aggregate = true;
            out.group_by.extend(aggregate.group_expr.iter().cloned());
            out.aggregate_expressions
                .extend(aggregate.aggr_expr.iter().map(ToString::to_string));
            walk(&aggregate.input, out)
        }
        LogicalPlan::Sort(sort) => {
            out.has_sort_or_limit = true;
            walk(&sort.input, out)
        }
        LogicalPlan::Limit(limit) => {
            out.has_sort_or_limit = true;
            walk(&limit.input, out)
        }
        LogicalPlan::SubqueryAlias(alias) => {
            let start = out.sources.len();
            walk(&alias.input, out)?;
            if out.sources.len() != start + 1 {
                return Err("subquery alias has multiple sources: not yet inspected safely".into());
            }
            out.sources[start].alias = alias.alias.to_string();
            Ok(())
        }
        LogicalPlan::TableScan(scan) => {
            out.sources.push(SourcePlan {
                table: scan.table_name.to_string(),
                alias: scan.table_name.to_string(),
                required_columns: BTreeSet::new(),
                local_filters: Vec::new(),
            });
            for predicate in &scan.filters {
                split_and(predicate, &mut out.filters);
            }
            Ok(())
        }
        other => Err(format!(
            "unsupported logical plan node for dependency discovery: {}",
            other.display()
        )),
    }
}

fn split_and(expr: &Expr, into: &mut Vec<Expr>) {
    if let Expr::BinaryExpr(binary) = expr {
        if binary.op == Operator::And {
            split_and(&binary.left, into);
            split_and(&binary.right, into);
            return;
        }
    }
    into.push(expr.clone());
}

fn source_for_column(column: &Column, sources: &[SourcePlan]) -> Option<usize> {
    let relation = column.relation.as_ref()?.to_string();
    sources.iter().position(|source| {
        source.alias == relation || source.table == relation
    })
}

fn add_referenced_columns(expr: &Expr, sources: &mut [SourcePlan]) {
    for col in expr.column_refs() {
        if let Some(index) = source_for_column(col, sources) {
            sources[index].required_columns.insert(col.name.clone());
        }
    }
}

/// A change is a post-COMMIT source-table event. Old and new images are both
/// required for updates that cross GROUP BY keys or joined membership.
#[derive(Debug, Clone, Default)]
pub struct RowImage {
    pub values: HashMap<String, ScalarValue>,
}

impl RowImage {
    pub fn with_text(column: &str, value: &str) -> Self {
        let mut row = Self::default();
        row.values.insert(column.into(), ScalarValue::Utf8(Some(value.into())));
        row
    }
}

#[derive(Debug, Clone)]
pub struct TableChange {
    pub table: String,
    pub old: Option<RowImage>,
    pub new: Option<RowImage>,
}

/// Affected groups are a FUTURE optimization hint only.
/// V1 ALWAYS reexecutes the full original SQL on a scheduled refresh.
fn affected_group_keys(plan: &LivePlan, change: &TableChange) -> Option<HashSet<Vec<ScalarValue>>> {
    if plan.group_by.is_empty() {
        return None;
    }
    let mut keys = HashSet::new();
    for row in [&change.old, &change.new].into_iter().flatten() {
        let mut group_key = Vec::with_capacity(plan.group_by.len());
        for group_expr in &plan.group_by {
            let Expr::Column(column) = group_expr else {
                return None; // expressions like date_trunc need full refresh
            };
            let source = source_for_column(column, &plan.sources)?;
            if plan.sources[source].table != change.table {
                return None; // join: row came from the other source
            }
            group_key.push(row.values.get(&column.name)?.clone());
        }
        keys.insert(group_key);
    }
    Some(keys)
}

#[derive(Debug, Clone, Copy)]
pub struct RefreshPolicy {
    pub debounce_ms: u64,
    pub max_wait_ms: u64,
}

impl Default for RefreshPolicy {
    fn default() -> Self {
        Self { debounce_ms: 25, max_wait_ms: 200 }
    }
}

#[derive(Debug, Clone)]
pub struct RefreshJob {
    pub query_id: u64,
    pub generation: u64,
    pub original_sql: String,
    /// Informational ONLY; not used to restrict V1 SQL.
    pub affected_groups_hint: HashSet<Vec<ScalarValue>>,
    pub full_refresh_required: bool,
}

#[derive(Debug)]
struct RefreshState {
    plan: LivePlan,
    dirty: bool,
    first_dirty_ms: Option<u64>,
    latest_dirty_ms: Option<u64>,
    in_flight: Option<u64>,
    generation: u64,
    invalidation_count: u64,
    group_hints: HashSet<Vec<ScalarValue>>,
    force_full: bool,
}

/// Deterministic scheduling core, deliberately free of async tasks.
/// Production integration owns the Tokio timer, bounded executor, RLS and
/// snapshot/generation fences. One in-flight job per query, at most one
/// subsequent queued refresh when more commits arrive during execution.
#[derive(Default)]
pub struct RefreshCoordinator {
    next_id: u64,
    table_to_queries: HashMap<String, HashSet<u64>>,
    queries: HashMap<u64, RefreshState>,
}

impl RefreshCoordinator {
    pub fn register(&mut self, plan: LivePlan) -> Result<u64, String> {
        if plan.mode != ExecutionMode::ServerQueryRefresh {
            return Err("refresh coordinator only accepts SERVER_QUERY_REFRESH plans".into());
        }
        self.next_id += 1;
        let id = self.next_id;
        for source in &plan.sources {
            self.table_to_queries.entry(source.table.clone()).or_default().insert(id);
        }
        self.queries.insert(id, RefreshState {
            plan,
            dirty: false,
            first_dirty_ms: None,
            latest_dirty_ms: None,
            in_flight: None,
            generation: 0,
            invalidation_count: 0,
            group_hints: HashSet::new(),
            force_full: false,
        });
        Ok(id)
    }

    pub fn unregister(&mut self, query_id: u64) {
        if let Some(state) = self.queries.remove(&query_id) {
            for source in &state.plan.sources {
                if let Some(queries) = self.table_to_queries.get_mut(&source.table) {
                    queries.remove(&query_id);
                }
            }
        }
    }

    /// Called for EVERY committed change on a dependency table, even if a
    /// row-local WHERE would not match; V1 favors correctness over tight invalidation.
    pub fn notify_committed(&mut self, change: &TableChange, at_ms: u64) -> usize {
        let query_ids = self.table_to_queries.get(&change.table).cloned().unwrap_or_default();
        for id in &query_ids {
            let state = self.queries.get_mut(id).expect("registered query");
            if !state.dirty {
                state.first_dirty_ms = Some(at_ms);
            }
            state.dirty = true;
            state.latest_dirty_ms = Some(at_ms);
            state.invalidation_count += 1;
            if let Some(group_keys) = affected_group_keys(&state.plan, change) {
                state.group_hints.extend(group_keys);
            } else {
                state.force_full = true;
            }
        }
        query_ids.len()
    }

    pub fn take_due(
        &mut self,
        query_id: u64,
        now_ms: u64,
        policy: RefreshPolicy,
    ) -> Option<RefreshJob> {
        let state = self.queries.get_mut(&query_id)?;
        if !state.dirty || state.in_flight.is_some() {
            return None;
        }
        let quiet = now_ms.saturating_sub(state.latest_dirty_ms?) >= policy.debounce_ms;
        let overdue = now_ms.saturating_sub(state.first_dirty_ms?) >= policy.max_wait_ms;
        if !(quiet || overdue) {
            return None;
        }
        state.generation += 1;
        let generation = state.generation;
        state.in_flight = Some(generation);
        state.dirty = false;
        state.first_dirty_ms = None;
        state.latest_dirty_ms = None;
        Some(RefreshJob {
            query_id,
            generation,
            original_sql: state.plan.original_sql.clone(),
            affected_groups_hint: std::mem::take(&mut state.group_hints),
            full_refresh_required: std::mem::take(&mut state.force_full),
        })
    }

    /// Returns whether another refresh was invalidated while this one ran.
    /// Production MUST fence publication of result generations and gaps.
    pub fn finish(&mut self, query_id: u64, generation: u64, success: bool, at_ms: u64) -> Result<bool, String> {
        let state = self.queries.get_mut(&query_id).ok_or("unknown refresh query")?;
        if state.in_flight != Some(generation) {
            return Err("out-of-order/stale refresh completion".into());
        }
        state.in_flight = None;
        if !success {
            if !state.dirty {
                state.first_dirty_ms = Some(at_ms);
            }
            state.dirty = true;
            state.latest_dirty_ms = Some(at_ms);
            state.force_full = true;
        }
        Ok(state.dirty)
    }

    pub fn invalidation_count(&self, query_id: u64) -> Option<u64> {
        self.queries.get(&query_id).map(|state| state.invalidation_count)
    }
}

/// Parent-key membership proof: production routing must additionally check RLS,
/// source predicates, typed composite keys, snapshot ordering and resource budgets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeyAction {
    BackfillAndListen(ScalarValue),
    Revoke(ScalarValue),
}

#[derive(Default)]
pub struct DynamicJoinKeys {
    references: HashMap<ScalarValue, usize>,
}

impl DynamicJoinKeys {
    /// bool = whether this parent row is currently authorized AND matches its
    /// local WHERE. Parent inserts/deletes/owner changes and key moves use
    /// the same old/new transition.
    pub fn parent_change(
        &mut self,
        old: Option<(ScalarValue, bool)>,
        new: Option<(ScalarValue, bool)>,
    ) -> Vec<KeyAction> {
        if old == new {
            return Vec::new();
        }
        let mut actions = Vec::new();
        if let Some((key, true)) = old {
            if !key.is_null() {
                if let Some(count) = self.references.get_mut(&key) {
                    *count -= 1;
                    if *count == 0 {
                        self.references.remove(&key);
                        actions.push(KeyAction::Revoke(key));
                    }
                }
            }
        }
        if let Some((key, true)) = new {
            if !key.is_null() {
                let count = self.references.entry(key.clone()).or_default();
                if *count == 0 {
                    actions.push(KeyAction::BackfillAndListen(key));
                }
                *count += 1;
            }
        }
        actions
    }

    pub fn matches_child_key(&self, key: &ScalarValue) -> bool {
        !key.is_null() && self.references.contains_key(key)
    }

    /// (retract_old, deliver_new) for child FK changes.
    pub fn child_change(&self, old_fk: Option<&ScalarValue>, new_fk: Option<&ScalarValue>) -> (bool, bool) {
        (
            old_fk.is_some_and(|key| self.matches_child_key(key)),
            new_fk.is_some_and(|key| self.matches_child_key(key)),
        )
    }
}
