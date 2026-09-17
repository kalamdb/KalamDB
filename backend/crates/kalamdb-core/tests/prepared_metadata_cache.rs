use std::{hint::black_box, ptr, time::Instant};

use kalamdb_commons::{NamespaceId, Role, TableId};
use kalamdb_core::sql::executor::PreparedExecutionStatement;
use kalamdb_sql::classifier::SqlStatement;
use moka::sync::Cache;
use sqlparser::{ast::Statement, dialect::GenericDialect, parser::Parser};

fn prepare(sql: &str) -> PreparedExecutionStatement {
    PreparedExecutionStatement::new(
        sql.to_owned(),
        Some(TableId::from_strings("bench", "message")),
        None,
        Some(SqlStatement::classify_and_parse(sql, &NamespaceId::new("bench"), Role::Dba).unwrap()),
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
