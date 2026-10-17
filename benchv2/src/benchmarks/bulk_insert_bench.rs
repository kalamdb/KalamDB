use std::{future::Future, pin::Pin};

use crate::{benchmarks::Benchmark, client::KalamClient, config::Config};

/// Benchmark: Bulk INSERT (50 rows per statement via multi-value INSERT).
pub struct BulkInsertBench;

impl Benchmark for BulkInsertBench {
    fn name(&self) -> &str {
        "bulk_insert"
    }
    fn category(&self) -> &str {
        "Insert"
    }
    fn description(&self) -> &str {
        "One INSERT statement with 50 rows (statement-scoped transaction)"
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
                .sql(&format!("DROP TABLE IF EXISTS {}.bulk_insert_bench", config.namespace))
                .await;
            client
                .sql_ok(&format!(
                    "CREATE TABLE {}.bulk_insert_bench (id INT PRIMARY KEY, name TEXT, value \
                     DOUBLE)",
                    config.namespace
                ))
                .await?;
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
            let base = iteration * 50;
            let values: Vec<String> = (0..50)
                .map(|i| {
                    let id = base + i;
                    format!("({}, 'bulk_user_{}', {:.2})", id, id, id as f64 * 0.7)
                })
                .collect();

            client
                .sql_ok(&format!(
                    "INSERT INTO {}.bulk_insert_bench (id, name, value) VALUES {}",
                    config.namespace,
                    values.join(", ")
                ))
                .await?;
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
                .sql(&format!("DROP TABLE IF EXISTS {}.bulk_insert_bench", config.namespace))
                .await;
            Ok(())
        })
    }
}
