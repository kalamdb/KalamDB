use std::{
    future::Future,
    pin::Pin,
    sync::atomic::{AtomicU32, Ordering},
};

use crate::{benchmarks::Benchmark, client::KalamClient, config::Config};

// ---------------------------------------------------------------------------
// CREATE TABLE benchmark
// ---------------------------------------------------------------------------

pub struct CreateTableBench {
    pub counter: AtomicU32,
}

impl Benchmark for CreateTableBench {
    fn name(&self) -> &str {
        "create_table"
    }
    fn category(&self) -> &str {
        "DDL"
    }
    fn description(&self) -> &str {
        "CREATE TABLE with 3 columns"
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
            let seq = self.counter.fetch_add(1, Ordering::Relaxed);
            let table = format!("{}.ct_bench_{}", config.namespace, seq);
            client
                .sql_ok(&format!(
                    "CREATE TABLE {} (id INT PRIMARY KEY, name TEXT, value DOUBLE)",
                    table
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
        let count = self.counter.load(Ordering::Relaxed);
        Box::pin(async move {
            for i in 0..count {
                let _ = client
                    .sql(&format!("DROP TABLE IF EXISTS {}.ct_bench_{}", config.namespace, i))
                    .await;
            }
            Ok(())
        })
    }
}

// ---------------------------------------------------------------------------
// DROP TABLE benchmark
// ---------------------------------------------------------------------------

pub struct DropTableBench;

impl Benchmark for DropTableBench {
    fn name(&self) -> &str {
        "drop_table"
    }
    fn category(&self) -> &str {
        "DDL"
    }
    fn description(&self) -> &str {
        "DROP TABLE on a previously created table"
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
            // Pre-create tables for warmup IDs (10_000..10_000+warmup) and timed IDs
            // (0..iterations)
            for i in 0..config.warmup {
                let id = 10_000 + i;
                client
                    .sql_ok(&format!(
                        "CREATE TABLE {}.dt_bench_{} (id INT PRIMARY KEY, name TEXT)",
                        config.namespace, id
                    ))
                    .await?;
            }
            for i in 0..config.iterations {
                client
                    .sql_ok(&format!(
                        "CREATE TABLE {}.dt_bench_{} (id INT PRIMARY KEY, name TEXT)",
                        config.namespace, i
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
            let table = format!("{}.dt_bench_{}", config.namespace, iteration);
            client.sql_ok(&format!("DROP TABLE {}", table)).await?;
            Ok(())
        })
    }

    fn teardown<'a>(
        &'a self,
        client: &'a KalamClient,
        config: &'a Config,
    ) -> Pin<Box<dyn Future<Output = Result<(), String>> + Send + 'a>> {
        Box::pin(async move {
            // Clean up any leftovers from both ranges
            for i in 0..config.warmup {
                let id = 10_000 + i;
                let _ = client
                    .sql(&format!("DROP TABLE IF EXISTS {}.dt_bench_{}", config.namespace, id))
                    .await;
            }
            for i in 0..config.iterations {
                let _ = client
                    .sql(&format!("DROP TABLE IF EXISTS {}.dt_bench_{}", config.namespace, i))
                    .await;
            }
            Ok(())
        })
    }
}
