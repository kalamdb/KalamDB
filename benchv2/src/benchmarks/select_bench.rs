use std::{future::Future, pin::Pin};

use crate::{benchmarks::Benchmark, client::KalamClient, config::Config};

// ---------------------------------------------------------------------------
// Helper: seed data for SELECT benchmarks
// Each benchmark gets its own table to avoid PK conflicts
// ---------------------------------------------------------------------------
async fn seed_select_table(
    client: &KalamClient,
    config: &Config,
    table_suffix: &str,
) -> Result<(), String> {
    let table = format!("{}.select_{}", config.namespace, table_suffix);
    client
        .sql_ok(&format!("CREATE NAMESPACE IF NOT EXISTS {}", config.namespace))
        .await?;
    let _ = client.sql(&format!("DROP TABLE IF EXISTS {}", table)).await;
    client
        .sql_ok(&format!(
            "CREATE TABLE {} (id INT PRIMARY KEY, name TEXT, score DOUBLE, active BOOLEAN)",
            table
        ))
        .await?;

    // Seed 200 rows in batches of 50
    for batch in 0..4 {
        let values: Vec<String> = (0..50)
            .map(|i| {
                let id = batch * 50 + i;
                format!("({}, 'user_{}', {:.2}, {})", id, id, id as f64 * 1.1, id % 3 == 0)
            })
            .collect();
        client
            .sql_ok(&format!(
                "INSERT INTO {} (id, name, score, active) VALUES {}",
                table,
                values.join(", ")
            ))
            .await?;
    }
    Ok(())
}

async fn teardown_select_table(client: &KalamClient, config: &Config, table_suffix: &str) {
    let table = format!("{}.select_{}", config.namespace, table_suffix);
    let _ = client.sql(&format!("DROP TABLE IF EXISTS {}", table)).await;
}

// ---------------------------------------------------------------------------
// SELECT * (full scan)
// ---------------------------------------------------------------------------
pub struct SelectAllBench;

impl Benchmark for SelectAllBench {
    fn name(&self) -> &str {
        "select_all"
    }
    fn category(&self) -> &str {
        "Select"
    }
    fn description(&self) -> &str {
        "SELECT * from a 200-row table"
    }

    fn setup<'a>(
        &'a self,
        client: &'a KalamClient,
        config: &'a Config,
    ) -> Pin<Box<dyn Future<Output = Result<(), String>> + Send + 'a>> {
        Box::pin(async move { seed_select_table(client, config, "all").await })
    }

    fn run<'a>(
        &'a self,
        client: &'a KalamClient,
        config: &'a Config,
        _iteration: u32,
    ) -> Pin<Box<dyn Future<Output = Result<(), String>> + Send + 'a>> {
        Box::pin(async move {
            client.sql_ok(&format!("SELECT * FROM {}.select_all", config.namespace)).await?;
            Ok(())
        })
    }

    fn teardown<'a>(
        &'a self,
        client: &'a KalamClient,
        config: &'a Config,
    ) -> Pin<Box<dyn Future<Output = Result<(), String>> + Send + 'a>> {
        Box::pin(async move {
            teardown_select_table(client, config, "all").await;
            Ok(())
        })
    }
}

// ---------------------------------------------------------------------------
// SELECT with WHERE filter
// ---------------------------------------------------------------------------
pub struct SelectByFilterBench;

impl Benchmark for SelectByFilterBench {
    fn name(&self) -> &str {
        "select_by_filter"
    }
    fn category(&self) -> &str {
        "Select"
    }
    fn description(&self) -> &str {
        "SELECT with WHERE clause on a 200-row table"
    }

    fn setup<'a>(
        &'a self,
        client: &'a KalamClient,
        config: &'a Config,
    ) -> Pin<Box<dyn Future<Output = Result<(), String>> + Send + 'a>> {
        Box::pin(async move { seed_select_table(client, config, "filter").await })
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
                    "SELECT * FROM {}.select_filter WHERE id = {}",
                    config.namespace, id
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
            teardown_select_table(client, config, "filter").await;
            Ok(())
        })
    }
}

// ---------------------------------------------------------------------------
// SELECT COUNT(*)
// ---------------------------------------------------------------------------
pub struct SelectCountBench;

impl Benchmark for SelectCountBench {
    fn name(&self) -> &str {
        "select_count"
    }
    fn category(&self) -> &str {
        "Select"
    }
    fn description(&self) -> &str {
        "SELECT COUNT(*) on a 200-row table"
    }

    fn setup<'a>(
        &'a self,
        client: &'a KalamClient,
        config: &'a Config,
    ) -> Pin<Box<dyn Future<Output = Result<(), String>> + Send + 'a>> {
        Box::pin(async move { seed_select_table(client, config, "count").await })
    }

    fn run<'a>(
        &'a self,
        client: &'a KalamClient,
        config: &'a Config,
        _iteration: u32,
    ) -> Pin<Box<dyn Future<Output = Result<(), String>> + Send + 'a>> {
        Box::pin(async move {
            client
                .sql_ok(&format!("SELECT COUNT(*) FROM {}.select_count", config.namespace))
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
            teardown_select_table(client, config, "count").await;
            Ok(())
        })
    }
}

// ---------------------------------------------------------------------------
// SELECT with ORDER BY + LIMIT
// ---------------------------------------------------------------------------
pub struct SelectOrderByBench;

impl Benchmark for SelectOrderByBench {
    fn name(&self) -> &str {
        "select_order_by_limit"
    }
    fn category(&self) -> &str {
        "Select"
    }
    fn description(&self) -> &str {
        "SELECT with ORDER BY + LIMIT 10 on a 200-row table"
    }

    fn setup<'a>(
        &'a self,
        client: &'a KalamClient,
        config: &'a Config,
    ) -> Pin<Box<dyn Future<Output = Result<(), String>> + Send + 'a>> {
        Box::pin(async move { seed_select_table(client, config, "orderby").await })
    }

    fn run<'a>(
        &'a self,
        client: &'a KalamClient,
        config: &'a Config,
        _iteration: u32,
    ) -> Pin<Box<dyn Future<Output = Result<(), String>> + Send + 'a>> {
        Box::pin(async move {
            client
                .sql_ok(&format!(
                    "SELECT * FROM {}.select_orderby ORDER BY score DESC LIMIT 10",
                    config.namespace
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
            teardown_select_table(client, config, "orderby").await;
            Ok(())
        })
    }
}
