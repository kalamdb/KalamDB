use std::{
    future::Future,
    pin::Pin,
    sync::atomic::{AtomicU32, Ordering},
};

use crate::{benchmarks::Benchmark, client::KalamClient, config::Config};

/// ALTER TABLE ADD COLUMN / DROP COLUMN DDL latency.
/// Tests schema evolution performance.
pub struct AlterTableBench {
    pub counter: AtomicU32,
}

impl Benchmark for AlterTableBench {
    fn name(&self) -> &str {
        "alter_table"
    }
    fn category(&self) -> &str {
        "DDL"
    }
    fn description(&self) -> &str {
        "ALTER TABLE ADD COLUMN + DROP COLUMN (schema evolution latency)"
    }

    fn setup<'a>(
        &'a self,
        client: &'a KalamClient,
        config: &'a Config,
    ) -> Pin<Box<dyn Future<Output = Result<(), String>> + Send + 'a>> {
        Box::pin(async move {
            self.counter.store(0, Ordering::SeqCst);
            client
                .sql_ok(&format!("CREATE NAMESPACE IF NOT EXISTS {}", config.namespace))
                .await?;
            let _ = client
                .sql(&format!("DROP TABLE IF EXISTS {}.alter_bench", config.namespace))
                .await;
            client
                .sql_ok(&format!(
                    "CREATE TABLE {}.alter_bench (id INT PRIMARY KEY, name TEXT)",
                    config.namespace
                ))
                .await?;

            // Seed some data so ALTER operates on a non-empty table
            let mut values = Vec::new();
            for i in 0..100 {
                values.push(format!("({}, 'row_{}')", i, i));
            }
            client
                .sql_ok(&format!(
                    "INSERT INTO {}.alter_bench (id, name) VALUES {}",
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
        _iteration: u32,
    ) -> Pin<Box<dyn Future<Output = Result<(), String>> + Send + 'a>> {
        let col_num = self.counter.fetch_add(1, Ordering::SeqCst);
        Box::pin(async move {
            let col_name = format!("extra_col_{}", col_num);

            // ADD COLUMN
            client
                .sql_ok(&format!(
                    "ALTER TABLE {}.alter_bench ADD COLUMN {} TEXT",
                    config.namespace, col_name
                ))
                .await
                .map_err(|e| format!("ADD COLUMN failed: {}", e))?;

            // Verify column exists via a query
            client
                .sql_ok(&format!(
                    "SELECT id, {} FROM {}.alter_bench LIMIT 1",
                    col_name, config.namespace
                ))
                .await
                .map_err(|e| format!("SELECT new column failed: {}", e))?;

            // DROP COLUMN
            client
                .sql_ok(&format!(
                    "ALTER TABLE {}.alter_bench DROP COLUMN {}",
                    config.namespace, col_name
                ))
                .await
                .map_err(|e| format!("DROP COLUMN failed: {}", e))?;

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
                .sql(&format!("DROP TABLE IF EXISTS {}.alter_bench", config.namespace))
                .await;
            Ok(())
        })
    }
}
