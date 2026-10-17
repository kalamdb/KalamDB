use std::{future::Future, pin::Pin};

use crate::{benchmarks::Benchmark, client::KalamClient, config::Config};

/// Aggregation queries (GROUP BY, SUM, AVG, COUNT) on a 10K-row table.
/// Measures analytical query performance at moderate scale.
pub struct AggregateQueryBench;

const ROWS: u32 = 10_000;

impl Benchmark for AggregateQueryBench {
    fn name(&self) -> &str {
        "aggregate_query"
    }
    fn category(&self) -> &str {
        "Select"
    }
    fn description(&self) -> &str {
        "GROUP BY + SUM/AVG/COUNT on a 10K-row table (analytical query performance)"
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
                .sql(&format!("DROP TABLE IF EXISTS {}.agg_bench", config.namespace))
                .await;
            client
                .sql_ok(&format!(
                    "CREATE TABLE {}.agg_bench (id INT PRIMARY KEY, region TEXT, product TEXT, \
                     quantity INT, price INT)",
                    config.namespace
                ))
                .await?;

            // Seed 10K rows in batches of 50
            let regions = ["us-east", "us-west", "eu-west", "eu-east", "ap-south"];
            let products = ["widget", "gadget", "doohickey", "thingamajig", "whatsit"];

            for batch in 0..(ROWS / 50) {
                let mut values = Vec::new();
                for i in 0..50 {
                    let id = batch * 50 + i;
                    let region = regions[(id as usize) % regions.len()];
                    let product = products[(id as usize) % products.len()];
                    let qty = (id * 7) % 100 + 1;
                    let price = (id * 13) % 1000 + 10;
                    values
                        .push(format!("({}, '{}', '{}', {}, {})", id, region, product, qty, price));
                }
                client
                    .sql_ok(&format!(
                        "INSERT INTO {}.agg_bench (id, region, product, quantity, price) VALUES {}",
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
            match iteration % 4 {
                0 => {
                    // GROUP BY region with multiple aggregates
                    client
                        .sql_ok(&format!(
                            "SELECT region, COUNT(*) as cnt, SUM(quantity) as total_qty, \
                             AVG(price) as avg_price FROM {}.agg_bench GROUP BY region ORDER BY \
                             total_qty DESC",
                            config.namespace
                        ))
                        .await?;
                },
                1 => {
                    // GROUP BY product, region (multi-level grouping)
                    client
                        .sql_ok(&format!(
                            "SELECT product, region, SUM(quantity * price) as revenue FROM \
                             {}.agg_bench GROUP BY product, region ORDER BY revenue DESC LIMIT 10",
                            config.namespace
                        ))
                        .await?;
                },
                2 => {
                    // Filtered aggregation
                    client
                        .sql_ok(&format!(
                            "SELECT product, SUM(quantity) as total_qty, MIN(price) as min_price, \
                             MAX(price) as max_price FROM {}.agg_bench WHERE region = 'us-east' \
                             GROUP BY product",
                            config.namespace
                        ))
                        .await?;
                },
                _ => {
                    // COUNT with HAVING equivalent (filtered after group)
                    client
                        .sql_ok(&format!(
                            "SELECT region, COUNT(*) as cnt FROM {}.agg_bench GROUP BY region \
                             HAVING COUNT(*) > 1000 ORDER BY cnt DESC",
                            config.namespace
                        ))
                        .await?;
                },
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
                .sql(&format!("DROP TABLE IF EXISTS {}.agg_bench", config.namespace))
                .await;
            Ok(())
        })
    }
}
