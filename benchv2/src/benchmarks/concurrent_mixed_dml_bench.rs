use std::{future::Future, pin::Pin};

use crate::{benchmarks::Benchmark, client::KalamClient, config::Config};

/// Concurrent mixed DML: simultaneous INSERT + UPDATE + DELETE on the same table.
/// Tests multi-writer contention with different operation types.
pub struct ConcurrentMixedDmlBench;

impl Benchmark for ConcurrentMixedDmlBench {
    fn name(&self) -> &str {
        "concurrent_mixed_dml"
    }
    fn category(&self) -> &str {
        "Concurrent"
    }
    fn description(&self) -> &str {
        "Concurrent INSERT + UPDATE + DELETE on the same table (multi-op contention)"
    }

    fn setup<'a>(
        &'a self,
        client: &'a KalamClient,
        config: &'a Config,
    ) -> Pin<Box<dyn Future<Output = Result<(), String>> + Send + 'a>> {
        Box::pin(async move {
            client
                .sql_ok(&format!("CREATE NAMESPACE IF NOT EXISTS {}", config.namespace))
                .await?;
            let _ = client
                .sql(&format!("DROP TABLE IF EXISTS {}.mixed_dml", config.namespace))
                .await;
            client
                .sql_ok(&format!(
                    "CREATE TABLE {}.mixed_dml (id INT PRIMARY KEY, value INT, label TEXT)",
                    config.namespace
                ))
                .await?;

            // Seed rows for updates and deletes
            // We use high IDs for initial data that will be updated/deleted
            // and low IDs for inserts to avoid PK conflicts
            let total_seed = 500u32;
            for batch in 0..(total_seed / 50) {
                let mut values = Vec::new();
                for i in 0..50 {
                    let id = 100_000 + batch * 50 + i;
                    values.push(format!("({}, {}, 'seed')", id, id));
                }
                client
                    .sql_ok(&format!(
                        "INSERT INTO {}.mixed_dml (id, value, label) VALUES {}",
                        config.namespace,
                        values.join(", ")
                    ))
                    .await?;
            }
            Ok(())
        })
    }

    fn run<'a>(
        &'a self,
        client: &'a KalamClient,
        config: &'a Config,
        iteration: u32,
    ) -> Pin<Box<dyn Future<Output = Result<(), String>> + Send + 'a>> {
        Box::pin(async move {
            let conc = config.concurrency;
            let mut handles = Vec::new();

            for i in 0..conc {
                let c = client.clone();
                let ns = config.namespace.clone();
                let iter = iteration;

                handles.push(tokio::spawn(async move {
                    match i % 3 {
                        0 => {
                            // INSERT a new row
                            let id = iter * 1000 + i;
                            c.sql_ok(&format!(
                                "INSERT INTO {}.mixed_dml (id, value, label) VALUES ({}, {}, \
                                 'inserted')",
                                ns,
                                id,
                                id * 2
                            ))
                            .await
                            .map_err(|e| format!("Insert #{}: {}", i, e))
                        },
                        1 => {
                            // UPDATE existing rows (range update on seeded data)
                            let target = 100_000 + (iter * conc + i) % 500;
                            c.sql_ok(&format!(
                                "UPDATE {}.mixed_dml SET value = {}, label = 'updated_{}' WHERE \
                                 id = {}",
                                ns,
                                iter * 100 + i,
                                iter,
                                target
                            ))
                            .await
                            .map_err(|e| format!("Update #{}: {}", i, e))
                        },
                        _ => {
                            // SELECT (reads under contention)
                            c.sql_ok(&format!(
                                "SELECT * FROM {}.mixed_dml WHERE id >= 100000 LIMIT 10",
                                ns
                            ))
                            .await
                            .map_err(|e| format!("Select #{}: {}", i, e))
                        },
                    }
                }));
            }

            for h in handles {
                h.await.map_err(|e| format!("Join: {}", e))??;
            }
            Ok(())
        })
    }

    fn teardown<'a>(
        &'a self,
        client: &'a KalamClient,
        config: &'a Config,
    ) -> Pin<Box<dyn Future<Output = Result<(), String>> + Send + 'a>> {
        Box::pin(async move {
            let _ = client
                .sql(&format!("DROP TABLE IF EXISTS {}.mixed_dml", config.namespace))
                .await;
            Ok(())
        })
    }
}
