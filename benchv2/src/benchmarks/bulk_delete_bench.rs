use std::{future::Future, pin::Pin};

use crate::{benchmarks::Benchmark, client::KalamClient, config::Config};

/// DELETE many rows at once with a range filter.
/// Tests bulk deletion performance (vs the existing single-row delete benchmark).
pub struct BulkDeleteBench;

const SEED_ROWS: u32 = 500;

impl Benchmark for BulkDeleteBench {
    fn name(&self) -> &str {
        "bulk_delete"
    }
    fn category(&self) -> &str {
        "Delete"
    }
    fn description(&self) -> &str {
        "DELETE 100 rows at once with a range filter (bulk deletion)"
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
            let _ =
                client.sql(&format!("DROP TABLE IF EXISTS {}.bulk_del", config.namespace)).await;
            client
                .sql_ok(&format!(
                    "CREATE TABLE {}.bulk_del (id INT PRIMARY KEY, bucket INT, payload TEXT)",
                    config.namespace
                ))
                .await?;

            // Seed enough rows for all iterations (warmup + timed)
            // Each iteration deletes 100 rows from a unique bucket, so we need
            // at least (warmup + iterations) * 100 rows
            let total_needed = SEED_ROWS * 50; // generous
            for batch in 0..(total_needed / 50) {
                let mut values = Vec::new();
                for i in 0..50 {
                    let id = batch * 50 + i;
                    let bucket = id / 100; // 100 rows per bucket
                    values.push(format!("({}, {}, 'data_{}')", id, bucket, id));
                }
                client
                    .sql_ok(&format!(
                        "INSERT INTO {}.bulk_del (id, bucket, payload) VALUES {}",
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
            // Delete all rows in a specific bucket (100 rows)
            client
                .sql_ok(&format!(
                    "DELETE FROM {}.bulk_del WHERE bucket = {}",
                    config.namespace, iteration
                ))
                .await
                .map_err(|e| format!("Bulk delete bucket {} failed: {}", iteration, e))?;
            Ok(())
        })
    }

    fn teardown<'a>(
        &'a self,
        client: &'a KalamClient,
        config: &'a Config,
    ) -> Pin<Box<dyn Future<Output = Result<(), String>> + Send + 'a>> {
        Box::pin(async move {
            let _ =
                client.sql(&format!("DROP TABLE IF EXISTS {}.bulk_del", config.namespace)).await;
            Ok(())
        })
    }
}
