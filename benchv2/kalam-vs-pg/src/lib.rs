//! Shared KalamDB vs PostgreSQL comparison protocol.
//!
//! Timed INSERT/SELECT use the same SQL text, schema (`bench.message`), and
//! concurrency. KalamDB is measured twice — PostgreSQL wire and HTTP SQL —
//! next to native PostgreSQL wire.

use std::{
    future::Future,
    sync::Arc,
    time::{Duration, Instant},
};

use anyhow::Context;
use crossbeam_queue::SegQueue;
use tokio::{sync::Semaphore, task::JoinSet};
use tokio_postgres::{Client, Config, NoTls, Statement};

/// Total rows for the throughput insert phase. Matches `benchv2/comparison`.
pub const N: i64 = 100_000;

/// Concurrent in-flight requests / warm pool size. Matches `benchv2/comparison`.
pub const LIMIT: usize = 16;

/// Rows used when collecting insert latency percentiles.
pub const LATENCY_INSERTS: i64 = 10_000;

/// Point-read operations for latency percentiles.
pub const LATENCY_READS: i64 = 1_000_000;

/// Offset so latency-phase ids do not collide with the throughput phase.
pub const LATENCY_ID_OFFSET: i32 = 1_000_000;

pub const KALAMDB_USER: &str = "admin";
pub const KALAMDB_PASSWORD: &str = "kalamdb123";
pub const KALAMDB_NAMESPACE: &str = "bench";
pub const KALAMDB_LOGICAL_DATABASE: &str = "kalam";

pub const POSTGRES_USER: &str = "postgres";
pub const POSTGRES_PASSWORD: &str = "postgres";
pub const POSTGRES_DATABASE: &str = "bench";

/// Qualified table used on both systems (`namespace.table` / `schema.table`).
pub const TABLE: &str = "bench.message";

pub const INSERT_SQL: &str =
    "INSERT INTO bench.message (id, owner, room, data) VALUES ($1, $2, $3, $4)";
pub const SELECT_SQL: &str = "SELECT id, owner, room, data FROM bench.message WHERE id = $1";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Percentiles {
    pub p50: Duration,
    pub p75: Duration,
    pub p90: Duration,
    pub p95: Duration,
}

pub fn message_data(id: i32) -> String {
    format!("a message {id}")
}

pub fn percentiles(mut latencies: Vec<Duration>) -> Option<Percentiles> {
    if latencies.is_empty() {
        return None;
    }
    latencies.sort();
    let len = latencies.len();
    Some(Percentiles {
        p50: latencies[len / 2],
        p75: latencies[((len as f64) * 0.75).floor() as usize],
        p90: latencies[((len as f64) * 0.90).floor() as usize],
        p95: latencies[((len as f64) * 0.95).floor() as usize],
    })
}

pub fn print_latencies(latencies: Vec<Duration>) {
    match percentiles(latencies) {
        None => println!("Latencies: empty"),
        Some(p) => println!(
            "Latencies: \n\tp50={:?} \n\tp75={:?} \n\tp90={:?} \n\tp95={:?}",
            p.p50, p.p75, p.p90, p.p95
        ),
    }
}

pub fn kalamdb_setup_sql() -> Vec<String> {
    vec![
        format!("CREATE NAMESPACE IF NOT EXISTS {KALAMDB_NAMESPACE}"),
        format!("DROP TABLE IF EXISTS {TABLE}"),
        format!("CREATE TABLE {TABLE} (id INT PRIMARY KEY, owner TEXT, room TEXT, data TEXT)"),
    ]
}

pub fn postgres_setup_sql() -> Vec<String> {
    vec![
        format!("CREATE SCHEMA IF NOT EXISTS {KALAMDB_NAMESPACE}"),
        format!("DROP TABLE IF EXISTS {TABLE}"),
        format!("CREATE TABLE {TABLE} (id INT PRIMARY KEY, owner TEXT, room TEXT, data TEXT)"),
    ]
}

pub struct PooledConn {
    client: Client,
    insert: Statement,
    select: Statement,
}

pub struct WirePool {
    conns: SegQueue<PooledConn>,
}

impl WirePool {
    pub fn len(&self) -> usize {
        self.conns.len()
    }

    fn push(&self, conn: PooledConn) {
        self.conns.push(conn);
    }

    fn pop(&self) -> anyhow::Result<PooledConn> {
        self.conns.pop().ok_or_else(|| anyhow::anyhow!("wire pool exhausted"))
    }
}

pub async fn connect_client(config: &Config) -> anyhow::Result<Client> {
    let (client, connection) = config.connect(NoTls).await.context("pgwire connect")?;
    tokio::spawn(async move {
        if let Err(error) = connection.await {
            eprintln!("pgwire connection error: {error}");
        }
    });
    Ok(client)
}

pub async fn connect_pool(config: &Config, size: usize) -> anyhow::Result<Arc<WirePool>> {
    anyhow::ensure!(size > 0, "pool size must be > 0");
    let pool = Arc::new(WirePool {
        conns: SegQueue::new(),
    });
    for index in 0..size {
        let client = connect_client(config)
            .await
            .with_context(|| format!("pool connection {index}"))?;
        let insert = client.prepare(INSERT_SQL).await?;
        let select = client.prepare(SELECT_SQL).await?;
        pool.push(PooledConn {
            client,
            insert,
            select,
        });
    }
    Ok(pool)
}

pub async fn run_setup(client: &Client, statements: &[String]) -> anyhow::Result<()> {
    for sql in statements {
        client.batch_execute(sql).await.with_context(|| format!("setup sql={sql}"))?;
    }
    Ok(())
}

pub async fn insert_row(pool: &WirePool, id: i32) -> anyhow::Result<()> {
    let conn = pool.pop()?;
    let data = message_data(id);
    let result = conn
        .client
        .execute(&conn.insert, &[&id, &"user1", &"room0", &data])
        .await
        .with_context(|| format!("insert id={id}"));
    pool.push(conn);
    result?;
    Ok(())
}

pub async fn read_row(pool: &WirePool, id: i32) -> anyhow::Result<()> {
    let conn = pool.pop()?;
    let result = async {
        let row = conn
            .client
            .query_one(&conn.select, &[&id])
            .await
            .with_context(|| format!("select id={id}"))?;
        // Consume the row the way a client would: read every projected column.
        let _: i32 =
            row.try_get(0).or_else(|_| row.try_get::<_, i64>(0).map(|value| value as i32))?;
        let _: String = row.get(1);
        let _: String = row.get(2);
        let _: String = row.get(3);
        Ok::<(), anyhow::Error>(())
    }
    .await;
    pool.push(conn);
    result
}

pub async fn run_insert_phase<F, Fut>(n: i64, insert: F) -> anyhow::Result<Duration>
where
    F: Fn(i32) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = anyhow::Result<()>> + Send + 'static,
{
    let insert = Arc::new(insert);
    let semaphore = Arc::new(Semaphore::new(LIMIT));
    let mut joins = JoinSet::new();
    let start = Instant::now();
    for id in 0..n {
        let insert = insert.clone();
        let semaphore = semaphore.clone();
        joins.spawn(async move {
            let _permit = semaphore.acquire().await.expect("semaphore closed");
            insert(id as i32).await
        });
    }
    join_all(joins).await?;
    Ok(start.elapsed())
}

pub async fn run_latency_insert_phase<F, Fut>(
    n: i64,
    insert: F,
) -> anyhow::Result<(Duration, Vec<Duration>)>
where
    F: Fn(i32) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = anyhow::Result<()>> + Send + 'static,
{
    let insert = Arc::new(insert);
    let semaphore = Arc::new(Semaphore::new(LIMIT));
    let latencies = Arc::new(SegQueue::<Duration>::new());
    let mut joins = JoinSet::new();
    let start = Instant::now();
    for id in 0..n {
        let insert = insert.clone();
        let semaphore = semaphore.clone();
        let latencies = latencies.clone();
        joins.spawn(async move {
            let _permit = semaphore.acquire().await.expect("semaphore closed");
            let t0 = Instant::now();
            insert(LATENCY_ID_OFFSET + id as i32).await?;
            latencies.push(t0.elapsed());
            Ok::<(), anyhow::Error>(())
        });
    }
    join_all(joins).await?;
    let samples = Arc::into_inner(latencies)
        .expect("latency queue still shared")
        .into_iter()
        .collect();
    Ok((start.elapsed(), samples))
}

pub async fn run_latency_read_phase<F, Fut>(
    n: i64,
    reads: i64,
    read: F,
) -> anyhow::Result<(Duration, Vec<Duration>)>
where
    F: Fn(i32) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = anyhow::Result<()>> + Send + 'static,
{
    let read = Arc::new(read);
    let semaphore = Arc::new(Semaphore::new(LIMIT));
    let latencies = Arc::new(SegQueue::<Duration>::new());
    let mut joins = JoinSet::new();
    let start = Instant::now();
    for idx in 0..reads {
        let read = read.clone();
        let semaphore = semaphore.clone();
        let latencies = latencies.clone();
        joins.spawn(async move {
            let _permit = semaphore.acquire().await.expect("semaphore closed");
            let id = LATENCY_ID_OFFSET + (idx % n) as i32;
            let t0 = Instant::now();
            read(id).await?;
            latencies.push(t0.elapsed());
            Ok::<(), anyhow::Error>(())
        });
    }
    join_all(joins).await?;
    let samples = Arc::into_inner(latencies)
        .expect("latency queue still shared")
        .into_iter()
        .collect();
    Ok((start.elapsed(), samples))
}

pub async fn run_comparison_ops<FI, FR, InsertFut, ReadFut>(
    insert: FI,
    read: FR,
) -> anyhow::Result<()>
where
    FI: Fn(i32) -> InsertFut + Send + Sync + 'static,
    InsertFut: Future<Output = anyhow::Result<()>> + Send + 'static,
    FR: Fn(i32) -> ReadFut + Send + Sync + 'static,
    ReadFut: Future<Output = anyhow::Result<()>> + Send + 'static,
{
    let insert = Arc::new(insert);
    let read = Arc::new(read);

    let insert_fn = insert.clone();
    let insert_wall = run_insert_phase(N, move |id| insert_fn(id)).await?;
    println!("Inserted {N} rows in {insert_wall:?}");

    let insert_fn = insert;
    let (insert_latency_wall, insert_latencies) =
        run_latency_insert_phase(LATENCY_INSERTS, move |id| insert_fn(id)).await?;
    println!("Inserted {LATENCY_INSERTS} rows in {insert_latency_wall:?}");
    print_latencies(insert_latencies);

    let (read_wall, read_latencies) =
        run_latency_read_phase(LATENCY_INSERTS, LATENCY_READS, move |id| read(id)).await?;
    println!("Read {LATENCY_READS} rows in {read_wall:?}");
    print_latencies(read_latencies);
    Ok(())
}

pub async fn run_comparison(pool: Arc<WirePool>) -> anyhow::Result<()> {
    anyhow::ensure!(
        pool.len() == LIMIT,
        "pool must hold LIMIT={LIMIT} warm connections, got {}",
        pool.len()
    );

    let insert_pool = pool.clone();
    let read_pool = pool;
    run_comparison_ops(
        move |id| {
            let insert_pool = insert_pool.clone();
            async move { insert_row(&insert_pool, id).await }
        },
        move |id| {
            let read_pool = read_pool.clone();
            async move { read_row(&read_pool, id).await }
        },
    )
    .await
}

async fn join_all(mut joins: JoinSet<anyhow::Result<()>>) -> anyhow::Result<()> {
    while let Some(joined) = joins.join_next().await {
        joined.context("worker task panicked")??;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percentiles_match_comparison_indexes() {
        let samples: Vec<_> = (0..100).map(|i| Duration::from_micros(i)).collect();
        let p = percentiles(samples).expect("percentiles");
        assert_eq!(p.p50, Duration::from_micros(50));
        assert_eq!(p.p75, Duration::from_micros(75));
        assert_eq!(p.p90, Duration::from_micros(90));
        assert_eq!(p.p95, Duration::from_micros(95));
    }

    #[test]
    fn timed_sql_is_parameterized_and_shared() {
        assert!(INSERT_SQL.contains("$1"));
        assert!(INSERT_SQL.contains("$4"));
        assert!(SELECT_SQL.contains("$1"));
        assert!(!INSERT_SQL.contains("user1"));
        assert_eq!(kalamdb_setup_sql()[2], postgres_setup_sql()[2]);
    }

    #[tokio::test]
    async fn insert_phase_issues_n_calls_under_limit() {
        let count = Arc::new(std::sync::atomic::AtomicU64::new(0));
        let in_flight = Arc::new(std::sync::atomic::AtomicU64::new(0));
        let max_in_flight = Arc::new(std::sync::atomic::AtomicU64::new(0));
        let n = 64_i64;
        let elapsed = run_insert_phase(n, {
            let count = count.clone();
            let in_flight = in_flight.clone();
            let max_in_flight = max_in_flight.clone();
            move |_id| {
                let count = count.clone();
                let in_flight = in_flight.clone();
                let max_in_flight = max_in_flight.clone();
                async move {
                    let current = in_flight.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
                    max_in_flight.fetch_max(current, std::sync::atomic::Ordering::SeqCst);
                    count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    in_flight.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
                    Ok(())
                }
            }
        })
        .await
        .expect("phase");
        assert_eq!(count.load(std::sync::atomic::Ordering::SeqCst), n as u64);
        assert!(max_in_flight.load(std::sync::atomic::Ordering::SeqCst) as usize <= LIMIT);
        assert!(elapsed > Duration::ZERO);
    }
}
