use std::{future::Future, pin::Pin};

use crate::{benchmarks::Benchmark, client::KalamClient, config::Config};

/// Measures write throughput when a topic is attached to the target table,
/// meaning every INSERT triggers the publish machinery.
pub struct ConcurrentPublisherBench;

impl Benchmark for ConcurrentPublisherBench {
    fn name(&self) -> &str {
        "concurrent_publishers"
    }
    fn category(&self) -> &str {
        "Load"
    }
    fn description(&self) -> &str {
        "N concurrent INSERTs into a topic-sourced table (measures publish overhead)"
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
                .sql(&format!("DROP TABLE IF EXISTS {}.pub_bench", config.namespace))
                .await;
            client
                .sql_ok(&format!(
                    "CREATE TABLE {}.pub_bench (id INT PRIMARY KEY, data TEXT, ts BIGINT)",
                    config.namespace
                ))
                .await?;

            // Create topic + wire it to the table (namespace-qualified)
            let topic_name = format!("{}.pub_topic", config.namespace);
            let _ = client.sql(&format!("DROP TOPIC IF EXISTS {}", topic_name)).await;
            let _ = client.sql(&format!("CREATE TOPIC {}", topic_name)).await;
            let _ = client
                .sql(&format!(
                    "ALTER TOPIC {} ADD SOURCE {}.pub_bench ON INSERT",
                    topic_name, config.namespace
                ))
                .await;
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
            let conc = config.concurrency;
            let mut handles = Vec::new();
            for i in 0..conc {
                let c = client.clone();
                let ns = config.namespace.clone();
                let id = iteration * conc * 2 + i;
                handles.push(tokio::spawn(async move {
                    c.sql_ok(&format!(
                        "INSERT INTO {}.pub_bench (id, data, ts) VALUES ({}, 'event_{}', {})",
                        ns, id, id, id
                    ))
                    .await
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
            let topic_name = format!("{}.pub_topic", config.namespace);
            let _ = client.sql(&format!("DROP TOPIC IF EXISTS {}", topic_name)).await;
            let _ = client
                .sql(&format!("DROP TABLE IF EXISTS {}.pub_bench", config.namespace))
                .await;
            Ok(())
        })
    }
}
