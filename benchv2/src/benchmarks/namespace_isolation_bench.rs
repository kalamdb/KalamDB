use std::{future::Future, pin::Pin};

use crate::{benchmarks::Benchmark, client::KalamClient, config::Config};

/// Concurrent queries across different namespaces.
/// Tests namespace isolation and whether cross-namespace traffic causes contention.
pub struct NamespaceIsolationBench;

impl Benchmark for NamespaceIsolationBench {
    fn name(&self) -> &str {
        "namespace_isolation"
    }
    fn category(&self) -> &str {
        "Concurrent"
    }
    fn description(&self) -> &str {
        "Concurrent queries across 5 different namespaces (isolation test)"
    }

    fn setup<'a>(
        &'a self,
        client: &'a KalamClient,
        config: &'a Config,
    ) -> Pin<Box<dyn Future<Output = Result<(), String>> + Send + 'a>> {
        Box::pin(async move {
            // Create 5 namespaces, each with the same table and data
            for i in 0..5 {
                let ns = format!("{}_iso_{}", config.namespace, i);
                client.sql_ok(&format!("CREATE NAMESPACE IF NOT EXISTS {}", ns)).await?;
                let _ = client.sql(&format!("DROP TABLE IF EXISTS {}.iso_data", ns)).await;
                client
                    .sql_ok(&format!(
                        "CREATE TABLE {}.iso_data (id INT PRIMARY KEY, ns_id INT, payload TEXT)",
                        ns
                    ))
                    .await?;

                // Seed 200 rows per namespace
                for batch in 0..4 {
                    let mut values = Vec::new();
                    for j in 0..50 {
                        let id = batch * 50 + j;
                        values.push(format!("({}, {}, 'ns_{}_row_{}')", id, i, i, id));
                    }
                    client
                        .sql_ok(&format!(
                            "INSERT INTO {}.iso_data (id, ns_id, payload) VALUES {}",
                            ns,
                            values.join(", ")
                        ))
                        .await?;
                }
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

            // Fire concurrent queries across all 5 namespaces
            for i in 0..5 {
                let c = client.clone();
                let ns = format!("{}_iso_{}", config.namespace, i);

                // Each namespace gets 2 concurrent queries
                let c2 = c.clone();
                let ns2 = ns.clone();

                handles.push(tokio::spawn(async move {
                    c.sql_ok(&format!("SELECT * FROM {}.iso_data WHERE ns_id = {}", ns, i))
                        .await
                        .map_err(|e| format!("NS {} query failed: {}", i, e))
                }));

                handles.push(tokio::spawn(async move {
                    c2.sql_ok(&format!("SELECT COUNT(*) FROM {}.iso_data", ns2))
                        .await
                        .map_err(|e| format!("NS {} count failed: {}", i, e))
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
            for i in 0..5 {
                let ns = format!("{}_iso_{}", config.namespace, i);
                let _ = client.sql(&format!("DROP TABLE IF EXISTS {}.iso_data", ns)).await;
                let _ = client.sql(&format!("DROP SCHEMA IF EXISTS {}", ns)).await;
            }
            Ok(())
        })
    }
}
