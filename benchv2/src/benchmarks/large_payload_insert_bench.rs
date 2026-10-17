use std::{future::Future, pin::Pin};

use crate::{benchmarks::Benchmark, client::KalamClient, config::Config};

/// INSERT rows with large TEXT payloads (~4KB each).
/// Tests serialization overhead and storage throughput with realistic payload sizes.
pub struct LargePayloadInsertBench;

impl Benchmark for LargePayloadInsertBench {
    fn name(&self) -> &str {
        "large_payload_insert"
    }
    fn category(&self) -> &str {
        "Insert"
    }
    fn description(&self) -> &str {
        "INSERT rows with ~4KB TEXT payloads (serialization + storage throughput)"
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
                .sql(&format!("DROP TABLE IF EXISTS {}.large_payload", config.namespace))
                .await;
            client
                .sql_ok(&format!(
                    "CREATE TABLE {}.large_payload (id INT PRIMARY KEY, body TEXT, metadata TEXT)",
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
            // ~4KB payload per row, insert 10 rows per iteration
            let payload = "X".repeat(4000);
            let metadata = "M".repeat(512);

            let mut values = Vec::new();
            for i in 0..10 {
                let id = iteration * 100 + i;
                values.push(format!("({}, '{}', '{}')", id, payload, metadata));
            }

            client
                .sql_ok(&format!(
                    "INSERT INTO {}.large_payload (id, body, metadata) VALUES {}",
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
                .sql(&format!("DROP TABLE IF EXISTS {}.large_payload", config.namespace))
                .await;
            Ok(())
        })
    }
}
