use std::{future::Future, pin::Pin};

use crate::{benchmarks::Benchmark, client::KalamClient, config::Config};

/// Benchmark: UPDATE a single row.
pub struct SingleUpdateBench;

impl Benchmark for SingleUpdateBench {
    fn name(&self) -> &str {
        "single_update"
    }
    fn category(&self) -> &str {
        "Update"
    }
    fn description(&self) -> &str {
        "UPDATE a single row by filter condition"
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
                .sql(&format!("DROP TABLE IF EXISTS {}.update_bench", config.namespace))
                .await;
            client
                .sql_ok(&format!(
                    "CREATE TABLE {}.update_bench (id INT PRIMARY KEY, name TEXT, score DOUBLE)",
                    config.namespace
                ))
                .await?;

            // Seed rows
            let values: Vec<String> = (0..200)
                .map(|i| format!("({}, 'user_{}', {:.2})", i, i, i as f64 * 1.0))
                .collect();
            // Insert in batches of 50
            for chunk in values.chunks(50) {
                client
                    .sql_ok(&format!(
                        "INSERT INTO {}.update_bench (id, name, score) VALUES {}",
                        config.namespace,
                        chunk.join(", ")
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
            let id = iteration % 200;
            client
                .sql_ok(&format!(
                    "UPDATE {}.update_bench SET score = {:.2} WHERE id = {}",
                    config.namespace,
                    iteration as f64 * 2.5,
                    id
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
                .sql(&format!("DROP TABLE IF EXISTS {}.update_bench", config.namespace))
                .await;
            Ok(())
        })
    }
}
