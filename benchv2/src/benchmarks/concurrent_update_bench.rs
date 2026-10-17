use std::{future::Future, pin::Pin};

use crate::{benchmarks::Benchmark, client::KalamClient, config::Config};

/// N concurrent UPDATE operations on the same table, testing write contention.
pub struct ConcurrentUpdateBench;

impl Benchmark for ConcurrentUpdateBench {
    fn name(&self) -> &str {
        "concurrent_update"
    }
    fn category(&self) -> &str {
        "Concurrent"
    }
    fn description(&self) -> &str {
        "N concurrent UPDATE operations on the same table (write contention test)"
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
                client.sql(&format!("DROP TABLE IF EXISTS {}.conc_upd", config.namespace)).await;
            client
                .sql_ok(&format!(
                    "CREATE TABLE {}.conc_upd (id INT PRIMARY KEY, counter INT, label TEXT)",
                    config.namespace
                ))
                .await?;

            // Seed rows — one per concurrent worker
            let mut values = Vec::new();
            for i in 0..config.concurrency.max(100) {
                values.push(format!("({}, 0, 'init')", i));
            }
            client
                .sql_ok(&format!(
                    "INSERT INTO {}.conc_upd (id, counter, label) VALUES {}",
                    config.namespace,
                    values.join(", ")
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
            let mut handles = Vec::new();
            for i in 0..config.concurrency {
                let c = client.clone();
                let ns = config.namespace.clone();
                let iter = iteration;
                handles.push(tokio::spawn(async move {
                    c.sql_ok(&format!(
                        "UPDATE {}.conc_upd SET counter = {}, label = 'iter_{}' WHERE id = {}",
                        ns,
                        iter * 100 + i,
                        iter,
                        i
                    ))
                    .await
                    .map_err(|e| format!("Concurrent update #{} failed: {}", i, e))
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
            let _ =
                client.sql(&format!("DROP TABLE IF EXISTS {}.conc_upd", config.namespace)).await;
            Ok(())
        })
    }
}
