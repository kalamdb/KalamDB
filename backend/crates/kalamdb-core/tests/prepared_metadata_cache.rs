use std::{
    hint::black_box,
    ptr,
    sync::Arc,
    time::{Duration, Instant},
};

use datafusion::{
    common::DFSchema,
    logical_expr::{EmptyRelation, LogicalPlan},
};
use kalamdb_commons::{models::UserId, Role, SchemaId, TableId};
use kalamdb_core::sql::{
    executor::PreparedExecutionStatement,
    plan_cache::{PlanAdmission, PlanCache, PlanCacheKey},
};
use kalamdb_sql::classifier::SqlStatement;
use moka::sync::Cache;
use sqlparser::{ast::Statement, dialect::GenericDialect, parser::Parser};

fn prepare(sql: &str) -> PreparedExecutionStatement {
    PreparedExecutionStatement::new(
        sql.to_owned(),
        Some(TableId::from_strings("bench", "message")),
        None,
        Some(SqlStatement::classify_and_parse(sql, &SchemaId::new("bench"), Role::Dba).unwrap()),
        false,
        Some(Parser::parse_sql(&GenericDialect, sql).unwrap().remove(0)),
    )
}

const INSERT: &str = "INSERT INTO bench.message (id, owner, room, data) VALUES ($1, $2, $3, $4)";

#[test]
fn cache_hits_share_dml_tree() {
    let cache = Cache::new(16);
    cache.insert(0, prepare(INSERT));
    let first = cache.get(&0).unwrap();
    let second = cache.get(&0).unwrap();
    let first_tree: &Statement = first.parsed_dml.as_ref().unwrap();
    let second_tree: &Statement = second.parsed_dml.as_ref().unwrap();
    assert!(ptr::eq(first_tree, second_tree), "cache hits must not deep-clone the DML tree");
    cache.invalidate_all();
    cache.run_pending_tasks();
    assert!(cache.get(&0).is_none());
    assert_eq!(first_tree.to_string(), second_tree.to_string());
}

#[test]
fn cache_hits_share_classification() {
    let cache = Cache::new(16);
    cache.insert(0, prepare(INSERT));
    let first = cache.get(&0).unwrap();
    let second = cache.get(&0).unwrap();
    let first_class: &SqlStatement = first.classified_statement.as_ref().unwrap();
    let second_class: &SqlStatement = second.classified_statement.as_ref().unwrap();
    assert!(
        ptr::eq(first_class, second_class),
        "cache hits must share parsed classification"
    );
}

#[test]
#[ignore = "manual cache microbenchmark; reports seconds, no timing assertion"]
fn benchmark_prepared_metadata_cache_hits() {
    let large = format!(
        "INSERT INTO bench.message (id, owner, room, data) VALUES {}",
        vec!["($1, $2, $3, $4)"; 128].join(", ")
    );
    for (label, sql) in [("single_row", INSERT), ("128_rows", large.as_str())] {
        let cache = Cache::new(16);
        cache.insert(0, prepare(sql));
        cache.run_pending_tasks();
        for _ in 0..1_000 {
            black_box(cache.get(&0).unwrap());
        }
        for sample in 1..=3 {
            let start = Instant::now();
            for _ in 0..20_000 {
                black_box(cache.get(&black_box(0)).unwrap());
            }
            eprintln!(
                "{label} sample={sample} hits=20000 seconds={:.6}",
                start.elapsed().as_secs_f64()
            );
        }
    }
}

fn empty_plan() -> LogicalPlan {
    LogicalPlan::EmptyRelation(EmptyRelation {
        produce_one_row: true,
        schema:          Arc::new(DFSchema::empty()),
    })
}

fn key(sql: &str) -> PlanCacheKey {
    PlanCacheKey::new(SchemaId::new("default"), Role::User, sql)
}

#[test]
fn parameter_oids_and_session_settings_do_not_share_a_plan() {
    let cache = PlanCache::new();
    let base = key("SELECT $1");
    assert_eq!(
        cache.insert(base.clone().parameter_oids(vec![23]), empty_plan()),
        PlanAdmission::Admitted
    );
    assert!(cache.get(&base.clone().parameter_oids(vec![25])).is_none());
    assert!(cache.get(&base.clone().parameter_oids(vec![23])).is_some());

    let configured = base
        .clone()
        .search_path("public")
        .session_settings(&[("TimeZone", "UTC"), ("DateStyle", "ISO, MDY")]);
    assert!(cache.get(&configured).is_none());
    cache.insert(configured.clone(), empty_plan());
    let reordered = base
        .search_path("public")
        .session_settings(&[("DateStyle", "ISO, MDY"), ("TimeZone", "UTC")]);
    assert!(cache.get(&reordered).is_some());
}

#[test]
fn shared_plans_ignore_principal_until_the_plan_captures_one() {
    let cache = PlanCache::new();
    let shared = key("SELECT shared");
    cache.insert(shared.clone(), empty_plan());
    assert!(cache.get(&shared).is_some());

    let alice = shared.clone().principal(UserId::new("alice"));
    let bob = shared.principal(UserId::new("bob"));
    assert!(cache.get(&alice).is_none());
    cache.insert(alice.clone(), empty_plan());
    assert!(cache.get(&bob).is_none());
    assert!(cache.get(&alice).is_some());
}

#[test]
fn rls_grant_and_dependency_changes_invalidate_prepared_plans() {
    let cache = PlanCache::new();
    let prepared = key("SELECT prepared");
    cache.insert(prepared.clone(), empty_plan());

    let permission = cache.bump_permission_epoch();
    assert!(cache.get(&prepared).is_none(), "grant or policy change drops the prepared plan");
    assert_eq!(cache.insert(prepared.clone(), empty_plan()), PlanAdmission::StaleEpoch);

    let current = prepared.clone().epochs(0, permission);
    assert_eq!(cache.insert(current.clone(), empty_plan()), PlanAdmission::Admitted);

    let dependency = cache.bump_dependency_epoch();
    assert!(cache.get(&current).is_none());
    let rebuilt = prepared.epochs(dependency, permission);
    assert_eq!(cache.insert(rebuilt.clone(), empty_plan()), PlanAdmission::Admitted);
    assert!(cache.get(&rebuilt).is_some());
}

#[test]
fn cache_churn_respects_entry_and_byte_caps_while_pins_remain_readable() {
    let cache = PlanCache::with_limits(1, 10_000, 400, Duration::from_secs(60));
    let small = key("SELECT 1");
    assert_eq!(cache.insert(small.clone(), empty_plan()), PlanAdmission::Admitted);
    let pinned = cache.get(&small).expect("admitted plan");

    assert_eq!(cache.insert(key(&"x".repeat(5_000)), empty_plan()), PlanAdmission::Rejected);
    assert_eq!(cache.insert(key("SELECT 2"), empty_plan()), PlanAdmission::Rejected);
    assert_eq!(cache.len(), 1);

    cache.clear();
    assert_eq!(cache.retained_bytes(), 0);
    assert!(cache.get(&small).is_none());
    assert!(matches!(pinned.as_ref(), LogicalPlan::EmptyRelation(_)));
}

#[test]
fn coalesced_lead_builds_once_outside_the_cache() {
    let cache = PlanCache::new();
    let prepared = key("SELECT lead");
    assert!(cache.try_lead(&prepared));
    assert!(!cache.try_lead(&prepared));
    assert_eq!(cache.finish_lead(prepared.clone(), empty_plan()), PlanAdmission::Admitted);
    assert!(cache.get(&prepared).is_some());
    assert!(!cache.try_lead(&prepared));
}
