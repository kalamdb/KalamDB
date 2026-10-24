use std::{future::Future, pin::Pin};

use crate::{benchmarks::Benchmark, client::KalamClient, config::Config};

/// Measures query performance when joining/subquerying across two tables.
/// Uses a correlated subquery pattern: SELECT from orders WHERE customer_id IN (SELECT ...).
pub struct MultiTableJoinBench;

impl Benchmark for MultiTableJoinBench {
    fn name(&self) -> &str {
        "multi_table_join"
    }
    fn category(&self) -> &str {
        "Select"
    }
    fn description(&self) -> &str {
        "SELECT with subquery across two tables (200 customers, 1000 orders)"
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

            // Customers table
            let _ = client
                .sql(&format!("DROP TABLE IF EXISTS {}.join_customers", config.namespace))
                .await;
            client
                .sql_ok(&format!(
                    "CREATE TABLE {}.join_customers (id INT PRIMARY KEY, name TEXT, tier TEXT)",
                    config.namespace
                ))
                .await?;

            // Orders table
            let _ = client
                .sql(&format!("DROP TABLE IF EXISTS {}.join_orders", config.namespace))
                .await;
            client
                .sql_ok(&format!(
                    "CREATE TABLE {}.join_orders (id INT PRIMARY KEY, customer_id INT, amount \
                     INT, status TEXT)",
                    config.namespace
                ))
                .await?;

            // Seed 200 customers
            for batch in 0..4 {
                let mut values = Vec::new();
                for i in 0..50 {
                    let id = batch * 50 + i;
                    let tier = match id % 3 {
                        0 => "gold",
                        1 => "silver",
                        _ => "bronze",
                    };
                    values.push(format!("({}, 'customer_{}', '{}')", id, id, tier));
                }
                client
                    .sql_ok(&format!(
                        "INSERT INTO {}.join_customers (id, name, tier) VALUES {}",
                        config.namespace,
                        values.join(", ")
                    ))
                    .await?;
            }

            // Seed 1000 orders
            for batch in 0..20 {
                let mut values = Vec::new();
                for i in 0..50 {
                    let id = batch * 50 + i;
                    let cust_id = id % 200;
                    let status = match id % 4 {
                        0 => "pending",
                        1 => "shipped",
                        2 => "delivered",
                        _ => "cancelled",
                    };
                    values.push(format!(
                        "({}, {}, {}, '{}')",
                        id,
                        cust_id,
                        (id * 17) % 10000,
                        status
                    ));
                }
                client
                    .sql_ok(&format!(
                        "INSERT INTO {}.join_orders (id, customer_id, amount, status) VALUES {}",
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
            match iteration % 3 {
                0 => {
                    // Subquery: orders from gold-tier customers
                    client
                        .sql_ok(&format!(
                            "SELECT o.id, o.amount, o.status FROM {ns}.join_orders o WHERE \
                             o.customer_id IN (SELECT c.id FROM {ns}.join_customers c WHERE \
                             c.tier = 'gold')",
                            ns = config.namespace
                        ))
                        .await?;
                },
                1 => {
                    // Aggregation across join: total spent per tier
                    client
                        .sql_ok(&format!(
                            "SELECT c.tier, SUM(o.amount) as total_spent FROM {ns}.join_orders o \
                             JOIN {ns}.join_customers c ON o.customer_id = c.id GROUP BY c.tier \
                             ORDER BY total_spent DESC",
                            ns = config.namespace
                        ))
                        .await?;
                },
                _ => {
                    // Filtered join with LIMIT
                    client
                        .sql_ok(&format!(
                            "SELECT c.name, o.amount, o.status FROM {ns}.join_orders o JOIN \
                             {ns}.join_customers c ON o.customer_id = c.id WHERE o.status = \
                             'shipped' ORDER BY o.amount DESC LIMIT 20",
                            ns = config.namespace
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
                .sql(&format!("DROP TABLE IF EXISTS {}.join_orders", config.namespace))
                .await;
            let _ = client
                .sql(&format!("DROP TABLE IF EXISTS {}.join_customers", config.namespace))
                .await;
            Ok(())
        })
    }
}
