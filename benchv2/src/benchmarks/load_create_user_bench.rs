use std::{
    future::Future,
    pin::Pin,
    sync::atomic::{AtomicU64, Ordering},
};

use crate::{benchmarks::Benchmark, client::KalamClient, config::Config};

// ---------------------------------------------------------------------------
// CREATE USER benchmark
// ---------------------------------------------------------------------------

pub struct CreateUserBench {
    pub counter: AtomicU64,
}

impl Benchmark for CreateUserBench {
    fn name(&self) -> &str {
        "create_user"
    }
    fn category(&self) -> &str {
        "Load"
    }
    fn description(&self) -> &str {
        "CREATE USER (auth subsystem stress test)"
    }

    fn setup<'a>(
        &'a self,
        _client: &'a KalamClient,
        _config: &'a Config,
    ) -> Pin<Box<dyn Future<Output = Result<(), String>> + Send + 'a>> {
        Box::pin(async move { Ok(()) })
    }

    fn run<'a>(
        &'a self,
        client: &'a KalamClient,
        _config: &'a Config,
        _iteration: u32,
    ) -> Pin<Box<dyn Future<Output = Result<(), String>> + Send + 'a>> {
        Box::pin(async move {
            let seq = self.counter.fetch_add(1, Ordering::Relaxed);
            let ts = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0);
            let username = format!("bcu_{}_{}", ts, seq);
            client
                .sql_ok(&format!(
                    "CREATE USER {} WITH PASSWORD 'BenchP@ss1234' ROLE 'user'",
                    username
                ))
                .await?;
            Ok(())
        })
    }

    fn teardown<'a>(
        &'a self,
        _client: &'a KalamClient,
        _config: &'a Config,
    ) -> Pin<Box<dyn Future<Output = Result<(), String>> + Send + 'a>> {
        let count = self.counter.load(Ordering::Relaxed);
        Box::pin(async move {
            // Best-effort cleanup; users with prefix bcu_ are bench-created
            // We can't easily enumerate them, but the namespace teardown will
            // drop the whole namespace anyway. If needed, a future version
            // can store created names in a Vec<Mutex<Vec<String>>>.
            let _ = count; // acknowledge
            Ok(())
        })
    }
}

// ---------------------------------------------------------------------------
// DROP USER benchmark
// ---------------------------------------------------------------------------

pub struct DropUserBench {
    pub counter:       AtomicU64,
    pub created_names: tokio::sync::Mutex<Vec<String>>,
}

impl Benchmark for DropUserBench {
    fn name(&self) -> &str {
        "drop_user"
    }
    fn category(&self) -> &str {
        "Load"
    }
    fn description(&self) -> &str {
        "DROP USER (auth subsystem teardown stress test)"
    }

    fn setup<'a>(
        &'a self,
        client: &'a KalamClient,
        config: &'a Config,
    ) -> Pin<Box<dyn Future<Output = Result<(), String>> + Send + 'a>> {
        Box::pin(async move {
            // Pre-create users for warmup IDs (10_000..10_000+warmup) and timed IDs (0..iterations)
            let mut names = self.created_names.lock().await;
            // Warmup range
            for i in 0..config.warmup {
                let id = 10_000 + i;
                let seq = self.counter.fetch_add(1, Ordering::Relaxed);
                let ts = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0);
                let username = format!("bdu_{}_{}", ts, seq);
                client
                    .sql_ok(&format!(
                        "CREATE USER {} WITH PASSWORD 'BenchP@ss1234' ROLE 'user'",
                        username
                    ))
                    .await
                    .map_err(|e| format!("DROP USER setup (warmup #{}) failed: {}", i, e))?;
                // Store at index matching iteration ID
                while names.len() <= id as usize {
                    names.push(String::new());
                }
                names[id as usize] = username;
            }
            // Timed range
            for i in 0..config.iterations {
                let seq = self.counter.fetch_add(1, Ordering::Relaxed);
                let ts = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0);
                let username = format!("bdu_{}_{}", ts, seq);
                client
                    .sql_ok(&format!(
                        "CREATE USER {} WITH PASSWORD 'BenchP@ss1234' ROLE 'user'",
                        username
                    ))
                    .await
                    .map_err(|e| format!("DROP USER setup (timed #{}) failed: {}", i, e))?;
                while names.len() <= i as usize {
                    names.push(String::new());
                }
                if i as usize == names.len() {
                    names.push(username);
                } else {
                    names[i as usize] = username;
                }
            }
            Ok(())
        })
    }

    fn run<'a>(
        &'a self,
        client: &'a KalamClient,
        _config: &'a Config,
        iteration: u32,
    ) -> Pin<Box<dyn Future<Output = Result<(), String>> + Send + 'a>> {
        Box::pin(async move {
            let names = self.created_names.lock().await;
            let idx = iteration as usize;
            if idx >= names.len() || names[idx].is_empty() {
                return Err(format!("No pre-created user for iteration {}", iteration));
            }
            let username = &names[idx];
            client.sql_ok(&format!("DROP USER IF EXISTS {}", username)).await?;
            Ok(())
        })
    }

    fn teardown<'a>(
        &'a self,
        client: &'a KalamClient,
        _config: &'a Config,
    ) -> Pin<Box<dyn Future<Output = Result<(), String>> + Send + 'a>> {
        Box::pin(async move {
            let names = self.created_names.lock().await;
            for name in names.iter() {
                let _ = client.sql(&format!("DROP USER IF EXISTS {}", name)).await;
            }
            Ok(())
        })
    }
}
