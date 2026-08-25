use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

use kalam_client::{ChangeEvent, SubscriptionConfig};

use crate::benchmarks::Benchmark;
use crate::client::KalamClient;
use crate::config::Config;

/// Measures a full graceful WebSocket disconnect, reconnect, authentication,
/// subscription registration, and initial result delivery cycle.
pub struct ReconnectSubscribeBench;

impl Benchmark for ReconnectSubscribeBench {
    fn name(&self) -> &str {
        "reconnect_subscribe"
    }
    fn category(&self) -> &str {
        "Subscribe"
    }
    fn description(&self) -> &str {
        "WebSocket disconnect + reconnect + authenticate + initial subscription results"
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
                .sql(&format!("DROP USER TABLE IF EXISTS {}.reconnect_sub", config.namespace))
                .await;
            client
                .sql_ok(&format!(
                    "CREATE USER TABLE {}.reconnect_sub (id INT PRIMARY KEY, data TEXT)",
                    config.namespace
                ))
                .await?;

            // Seed initial data
            let mut values = Vec::new();
            for i in 0..100 {
                values.push(format!("({}, 'initial_{}')", i, i));
            }
            client
                .sql_ok(&format!(
                    "INSERT INTO {}.reconnect_sub (id, data) VALUES {}",
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
            let ns = config.namespace.clone();
            let sql = format!("SELECT * FROM {}.reconnect_sub", ns);

            // Keep the endpoint fixed for this cycle. Disconnecting the shared client
            // tears down the actual WebSocket; the next subscription reconnects and
            // authenticates it before sending Subscribe.
            let link = client.link();
            link.disconnect().await;

            {
                let subscription_id = format!("reconnect_{}", iteration);
                let sub_config = SubscriptionConfig::new(subscription_id, sql);
                let mut sub = link
                    .live_events_with_config(sub_config)
                    .await
                    .map_err(|error| format!("Reconnect subscribe error: {error}"))?;

                let mut got_ack = false;
                let mut result_rows = 0usize;
                loop {
                    match tokio::time::timeout(Duration::from_secs(10), sub.next()).await {
                        Ok(Some(Ok(event))) => match &event {
                            ChangeEvent::Ack { .. } => {
                                got_ack = true;
                            },
                            ChangeEvent::InitialDataBatch {
                                rows,
                                batch_control,
                                ..
                            } => {
                                result_rows += rows.len();
                                if batch_control.status == kalam_client::models::BatchStatus::Ready
                                    || !batch_control.has_more
                                {
                                    break;
                                }
                            },
                            ChangeEvent::Error { message, .. } => {
                                return Err(format!("Server error on reconnect: {message}"));
                            },
                            _ => break,
                        },
                        _ => break,
                    }
                }

                let _ = sub.close().await;

                if !got_ack || result_rows == 0 {
                    return Err(format!(
                        "Reconnect did not return the expected first result set (ack={got_ack}, rows={result_rows})"
                    ));
                }
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
                .sql(&format!("DROP USER TABLE IF EXISTS {}.reconnect_sub", config.namespace))
                .await;
            Ok(())
        })
    }
}
