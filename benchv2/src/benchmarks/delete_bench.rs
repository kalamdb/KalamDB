use std::{future::Future, pin::Pin};

use crate::{benchmarks::Benchmark, client::KalamClient, config::Config};

/// Benchmark: DELETE a single row.
pub struct SingleDeleteBench;

impl Benchmark for SingleDeleteBench {
    fn name(&self) -> &str {
        "single_delete"
    }
    fn category(&self) -> &str {
        "Delete"
    }
    fn description(&self) -> &str {
        "DELETE a single row by filter condition"
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
                .sql(&format!("DROP TABLE IF EXISTS {}.delete_bench", config.namespace))
                .await;
            client
                .sql_ok(&format!(
                    "CREATE TABLE {}.delete_bench (id INT PRIMARY KEY, name TEXT, value DOUBLE)",
                    config.namespace
                ))
                .await?;

            // Seed enough rows for all iterations + warmup
            // We insert 500 rows so we have room
            for batch in 0..10 {
                let values: Vec<String> = (0..50)
                    .map(|i| {
                        let id = batch * 50 + i;
                        format!("({}, 'del_user_{}', {:.2})", id, id, id as f64)
                    })
                    .collect();
                client
                    .sql_ok(&format!(
                        "INSERT INTO {}.delete_bench (id, name, value) VALUES {}",
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
            // Delete by id; each iteration deletes a different row
            client
                .sql_ok(&format!(
                    "DELETE FROM {}.delete_bench WHERE id = {}",
                    config.namespace, iteration
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
                .sql(&format!("DROP TABLE IF EXISTS {}.delete_bench", config.namespace))
                .await;
            Ok(())
        })
    }
}
