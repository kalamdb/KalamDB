use std::{future::Future, pin::Pin};

use crate::{benchmarks::Benchmark, client::KalamClient, config::Config};

/// Benchmark: Single row INSERT.
pub struct SingleInsertBench;

impl Benchmark for SingleInsertBench {
    fn name(&self) -> &str {
        "single_insert"
    }
    fn category(&self) -> &str {
        "Insert"
    }
    fn description(&self) -> &str {
        "INSERT a single row into a table"
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
                .sql(&format!("DROP TABLE IF EXISTS {}.insert_bench", config.namespace))
                .await;
            client
                .sql_ok(&format!(
                    "CREATE TABLE {}.insert_bench (id INT PRIMARY KEY, name TEXT, score DOUBLE, \
                     active BOOLEAN)",
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
            client
                .sql_ok(&format!(
                    "INSERT INTO {}.insert_bench (id, name, score, active) VALUES ({}, 'user_{}', \
                     {:.2}, {})",
                    config.namespace,
                    iteration,
                    iteration,
                    iteration as f64 * 1.5,
                    iteration % 2 == 0
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
                .sql(&format!("DROP TABLE IF EXISTS {}.insert_bench", config.namespace))
                .await;
            Ok(())
        })
    }
}
