use std::{future::Future, pin::Pin};

use crate::{benchmarks::Benchmark, client::KalamClient, config::Config};

/// INSERT into a wide table with 20 columns.
/// Tests schema handling, serialization, and storage overhead with many columns.
pub struct WideColumnInsertBench;

impl Benchmark for WideColumnInsertBench {
    fn name(&self) -> &str {
        "wide_column_insert"
    }
    fn category(&self) -> &str {
        "Insert"
    }
    fn description(&self) -> &str {
        "INSERT into a 20-column table (wide schema overhead)"
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
                .sql(&format!("DROP TABLE IF EXISTS {}.wide_cols", config.namespace))
                .await;

            // Create a 20-column table
            let mut cols = vec!["id INT PRIMARY KEY".to_string()];
            for i in 1..20 {
                if i % 3 == 0 {
                    cols.push(format!("col_{} INT", i));
                } else {
                    cols.push(format!("col_{} TEXT", i));
                }
            }
            client
                .sql_ok(&format!(
                    "CREATE TABLE {}.wide_cols ({})",
                    config.namespace,
                    cols.join(", ")
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
            let base_id = iteration * 10;
            let mut values = Vec::new();

            for row in 0..10 {
                let id = base_id + row;
                let mut vals = vec![format!("{}", id)];
                for i in 1..20 {
                    if i % 3 == 0 {
                        vals.push(format!("{}", id * i));
                    } else {
                        vals.push(format!("'val_{}_{}'", id, i));
                    }
                }
                values.push(format!("({})", vals.join(", ")));
            }

            // Build column list
            let mut col_names = vec!["id".to_string()];
            for i in 1..20 {
                col_names.push(format!("col_{}", i));
            }

            client
                .sql_ok(&format!(
                    "INSERT INTO {}.wide_cols ({}) VALUES {}",
                    config.namespace,
                    col_names.join(", "),
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
                .sql(&format!("DROP TABLE IF EXISTS {}.wide_cols", config.namespace))
                .await;
            Ok(())
        })
    }
}
