use std::{future::Future, pin::Pin};

use crate::{benchmarks::Benchmark, client::KalamClient, config::Config};

// ---------------------------------------------------------------------------
// Concurrent INSERT: fires `concurrency` inserts in parallel per iteration
// ---------------------------------------------------------------------------
pub struct ConcurrentInsertBench;

impl Benchmark for ConcurrentInsertBench {
    fn name(&self) -> &str {
        "concurrent_insert"
    }
    fn category(&self) -> &str {
        "Concurrent"
    }
    fn description(&self) -> &str {
        "N concurrent INSERT operations in parallel (N = concurrency setting)"
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
                .sql(&format!("DROP TABLE IF EXISTS {}.conc_insert_bench", config.namespace))
                .await;
            client
                .sql_ok(&format!(
                    "CREATE TABLE {}.conc_insert_bench (id INT PRIMARY KEY, name TEXT, value \
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
            let mut handles = Vec::new();
            let conc = config.concurrency;
            for i in 0..conc {
                let c = client.clone();
                let ns = config.namespace.clone();
                let id = iteration * conc + i;
                handles.push(tokio::spawn(async move {
                    c.sql_ok(&format!(
                        "INSERT INTO {}.conc_insert_bench (id, name, value) VALUES ({}, \
                         'conc_{}', {:.2})",
                        ns, id, id, id as f64
                    ))
                    .await
                }));
            }
            for h in handles {
                h.await.map_err(|e| format!("Join error: {}", e))??;
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
                .sql(&format!("DROP TABLE IF EXISTS {}.conc_insert_bench", config.namespace))
                .await;
            Ok(())
        })
    }
}

// ---------------------------------------------------------------------------
// Concurrent SELECT: fires `concurrency` selects in parallel per iteration
// ---------------------------------------------------------------------------
pub struct ConcurrentSelectBench;

impl Benchmark for ConcurrentSelectBench {
    fn name(&self) -> &str {
        "concurrent_select"
    }
    fn category(&self) -> &str {
        "Concurrent"
    }
    fn description(&self) -> &str {
        "N concurrent SELECT operations in parallel (N = concurrency setting)"
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
                .sql(&format!("DROP TABLE IF EXISTS {}.conc_select_bench", config.namespace))
                .await;
            client
                .sql_ok(&format!(
                    "CREATE TABLE {}.conc_select_bench (id INT PRIMARY KEY, name TEXT, value \
                     DOUBLE)",
                    config.namespace
                ))
                .await?;

            // Seed rows
            for batch in 0..4 {
                let values: Vec<String> = (0..50)
                    .map(|i| {
                        let id = batch * 50 + i;
                        format!("({}, 'cs_user_{}', {:.2})", id, id, id as f64)
                    })
                    .collect();
                client
                    .sql_ok(&format!(
                        "INSERT INTO {}.conc_select_bench (id, name, value) VALUES {}",
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
            let mut handles = Vec::new();
            let conc = config.concurrency;
            for i in 0..conc {
                let c = client.clone();
                let ns = config.namespace.clone();
                handles.push(tokio::spawn(async move {
                    let id = i % 200;
                    c.sql_ok(&format!("SELECT * FROM {}.conc_select_bench WHERE id = {}", ns, id))
                        .await
                }));
            }
            for h in handles {
                h.await.map_err(|e| format!("Join error: {}", e))??;
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
                .sql(&format!("DROP TABLE IF EXISTS {}.conc_select_bench", config.namespace))
                .await;
            Ok(())
        })
    }
}
