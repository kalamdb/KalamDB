use std::{future::Future, pin::Pin};

use crate::{benchmarks::Benchmark, client::KalamClient, config::Config};

/// Queries that return large result sets under concurrent load.
/// Tests serialization overhead, memory pressure, and network throughput.
pub struct WideFanoutQueryBench;

impl Benchmark for WideFanoutQueryBench {
    fn name(&self) -> &str {
        "wide_fanout_query"
    }
    fn category(&self) -> &str {
        "Load"
    }
    fn description(&self) -> &str {
        "N concurrent large-result-set SELECTs (serialization + memory pressure)"
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
                .sql(&format!("DROP TABLE IF EXISTS {}.fanout_bench", config.namespace))
                .await;
            client
                .sql_ok(&format!(
                    "CREATE TABLE {}.fanout_bench (id INT PRIMARY KEY, col_a TEXT, col_b TEXT, \
                     col_c DOUBLE, col_d BIGINT)",
                    config.namespace
                ))
                .await?;

            // Seed 1000 rows with wide data
            for chunk in 0..10 {
                let mut values = Vec::new();
                for i in 0..100 {
                    let id = chunk * 100 + i;
                    values.push(format!(
                        "({}, '{}', '{}', {:.4}, {})",
                        id,
                        format!("alpha_bravo_charlie_delta_echo_{}", id),
                        format!("foxtrot_golf_hotel_india_juliet_{}", id),
                        id as f64 * 3.14159,
                        id * 1_000_000,
                    ));
                }
                client
                    .sql_ok(&format!(
                        "INSERT INTO {}.fanout_bench (id, col_a, col_b, col_c, col_d) VALUES {}",
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
        _iteration: u32,
    ) -> Pin<Box<dyn Future<Output = Result<(), String>> + Send + 'a>> {
        Box::pin(async move {
            let conc = config.concurrency;
            let mut handles = Vec::new();

            for i in 0..conc {
                let c = client.clone();
                let ns = config.namespace.clone();
                // Each query returns a large result set (all 1000 rows or a big slice)
                let query = match i % 3 {
                    0 => format!("SELECT * FROM {}.fanout_bench", ns),
                    1 => format!(
                        "SELECT * FROM {}.fanout_bench WHERE col_c > 100.0 ORDER BY col_d DESC",
                        ns
                    ),
                    _ => format!(
                        "SELECT id, col_a, col_b, col_c, col_d FROM {}.fanout_bench WHERE id > 200",
                        ns
                    ),
                };
                handles.push(tokio::spawn(async move { c.sql_ok(&query).await }));
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
                .sql(&format!("DROP TABLE IF EXISTS {}.fanout_bench", config.namespace))
                .await;
            Ok(())
        })
    }
}
