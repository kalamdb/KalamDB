use std::{future::Future, pin::Pin};

use crate::{benchmarks::Benchmark, client::KalamClient, config::Config};

/// Rapid-fire CRUD cycle: INSERT → UPDATE → SELECT → DELETE per iteration.
/// Tests the full DML lifecycle as a sequential pipeline.
pub struct SequentialCrudBench;

impl Benchmark for SequentialCrudBench {
    fn name(&self) -> &str {
        "sequential_crud"
    }
    fn category(&self) -> &str {
        "DML"
    }
    fn description(&self) -> &str {
        "INSERT → UPDATE → SELECT → DELETE full DML lifecycle per iteration"
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
                .sql(&format!("DROP TABLE IF EXISTS {}.crud_cycle", config.namespace))
                .await;
            client
                .sql_ok(&format!(
                    "CREATE TABLE {}.crud_cycle (id INT PRIMARY KEY, value TEXT, version INT)",
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
            let id = 800_000 + iteration;

            // 1. INSERT
            client
                .sql_ok(&format!(
                    "INSERT INTO {}.crud_cycle (id, value, version) VALUES ({}, 'created', 1)",
                    config.namespace, id
                ))
                .await
                .map_err(|e| format!("INSERT failed: {}", e))?;

            // 2. UPDATE
            client
                .sql_ok(&format!(
                    "UPDATE {}.crud_cycle SET value = 'updated', version = 2 WHERE id = {}",
                    config.namespace, id
                ))
                .await
                .map_err(|e| format!("UPDATE failed: {}", e))?;

            // 3. SELECT (verify update)
            let resp = client
                .sql_ok(&format!("SELECT * FROM {}.crud_cycle WHERE id = {}", config.namespace, id))
                .await
                .map_err(|e| format!("SELECT failed: {}", e))?;

            if let Some(result) = resp.results.first() {
                if let Some(rows) = &result.rows {
                    if rows.is_empty() {
                        return Err(format!("SELECT returned 0 rows for id={}", id));
                    }
                }
            }

            // 4. DELETE
            client
                .sql_ok(&format!("DELETE FROM {}.crud_cycle WHERE id = {}", config.namespace, id))
                .await
                .map_err(|e| format!("DELETE failed: {}", e))?;

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
                .sql(&format!("DROP TABLE IF EXISTS {}.crud_cycle", config.namespace))
                .await;
            Ok(())
        })
    }
}
