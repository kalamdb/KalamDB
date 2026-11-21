//! SpaceTimeDB comparison driver.
//!
//! Best-practice client path: a pool of persistent WebSocket connections.
//! Inserts call the transactional reducer `insert_message`. Point reads call
//! the procedure `get_message`, which returns the primary-key row to the
//! caller. Reads are server round trips. They do not hit a local subscription
//! cache. The Rust SDK does not implement one-off SQL queries.
//!
//! Concurrency is 16 in-flight calls, the same limit as the HTTP drivers.
//! Each in-flight call holds one already-open socket from the pool.

mod module_bindings;

use std::env;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::Context;
use comparison_common::{
    message_data, print_latencies, LATENCY_INSERTS, LATENCY_READS, LIMIT, N,
};
use crossbeam_queue::SegQueue;
use module_bindings::{get_message, insert_message, DbConnection};
use tokio::sync::{oneshot, Semaphore};

const DATABASE: &str = "bench";

fn rows(default: i64, name: &str) -> i64 {
    env::var(name)
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(default)
}

fn inflight() -> usize {
    rows(LIMIT as i64, "COMPARISON_INFLIGHT") as usize
}

struct Pool {
    slots: Mutex<Vec<DbConnection>>,
}

impl Pool {
    fn checkout(&self) -> DbConnection {
        self.slots
            .lock()
            .expect("pool")
            .pop()
            .expect("connection pool underflow")
    }

    fn checkin(&self, conn: DbConnection) {
        self.slots.lock().expect("pool").push(conn);
    }
}

fn connect_one(base: &str) -> anyhow::Result<DbConnection> {
    let (tx, rx) = std::sync::mpsc::sync_channel(1);
    let err_tx = tx.clone();
    let conn = DbConnection::builder()
        .with_uri(base)
        .with_database_name(DATABASE)
        .on_connect(move |_conn, _identity, _token| {
            let _ = tx.send(Ok(()));
        })
        .on_connect_error(move |_ctx, err| {
            let _ = err_tx.send(Err(format!("{err:?}")));
        })
        .build()
        .context("websocket connect")?;
    conn.run_threaded();
    match rx.recv_timeout(Duration::from_secs(30))? {
        Ok(()) => Ok(conn),
        Err(err) => anyhow::bail!("websocket handshake failed: {err}"),
    }
}

fn connect_pool(base: &str) -> anyhow::Result<Arc<Pool>> {
    let width = inflight();
    let mut slots = Vec::with_capacity(width);
    for _ in 0..width {
        slots.push(connect_one(base)?);
    }
    Ok(Arc::new(Pool {
        slots: Mutex::new(slots),
    }))
}

async fn insert_one(pool: &Pool, id: i64) -> anyhow::Result<()> {
    let conn = pool.checkout();
    let (tx, rx) = oneshot::channel();
    conn.reducers
        .insert_message_then(
            id,
            "user1".to_string(),
            "room0".to_string(),
            message_data(id),
            move |_ctx, result| {
                let _ = tx.send(result);
            },
        )
        .context("send insert_message")?;
    let result = tokio::time::timeout(Duration::from_secs(60), rx)
        .await
        .context("insert timed out")?
        .context("insert callback dropped")?;
    pool.checkin(conn);
    match result {
        Ok(Ok(())) => Ok(()),
        Ok(Err(message)) => anyhow::bail!("reducer failed: {message}"),
        Err(err) => anyhow::bail!("reducer transport: {err:?}"),
    }
}

async fn read_one(pool: &Pool, id: i64) -> anyhow::Result<()> {
    let conn = pool.checkout();
    let (tx, rx) = oneshot::channel();
    conn.procedures.get_message_then(id, move |_ctx, result| {
        let _ = tx.send(result);
    });
    let result = tokio::time::timeout(Duration::from_secs(60), rx)
        .await
        .context("read timed out")?
        .context("read callback dropped")?;
    pool.checkin(conn);
    match result {
        Ok(Some(_)) => Ok(()),
        Ok(None) => anyhow::bail!("get_message returned no row for id {id}"),
        Err(err) => anyhow::bail!("procedure transport: {err:?}"),
    }
}

async fn insert_benchmark(pool: &Arc<Pool>) -> anyhow::Result<()> {
    let count = rows(N, "COMPARISON_INSERTS");
    let throttler = Arc::new(Semaphore::new(inflight()));
    let start = Instant::now();
    for id in 0..count {
        let pool = pool.clone();
        let permit = throttler.clone().acquire_owned().await?;
        tokio::spawn(async move {
            insert_one(&pool, id).await.expect("insert");
            drop(permit);
        });
    }
    let _ = throttler.acquire_many(inflight() as u32).await?;
    println!("Inserted {count} rows in {:?}", start.elapsed());
    Ok(())
}

async fn latency_benchmark(pool: &Arc<Pool>) -> anyhow::Result<()> {
    let latency_inserts = rows(LATENCY_INSERTS, "COMPARISON_LATENCY_INSERTS");
    let latency_reads = rows(LATENCY_READS, "COMPARISON_READS");
    let throttler = Arc::new(Semaphore::new(inflight()));
    let insert_latencies = Arc::new(SegQueue::<Duration>::new());
    let start = Instant::now();
    for id in 0..latency_inserts {
        let pool = pool.clone();
        let latencies = insert_latencies.clone();
        let permit = throttler.clone().acquire_owned().await?;
        let row_id = 1_000_000 + id;
        tokio::spawn(async move {
            let t0 = Instant::now();
            insert_one(&pool, row_id).await.expect("insert");
            latencies.push(t0.elapsed());
            drop(permit);
        });
    }
    let _ = throttler.acquire_many(inflight() as u32).await?;
    println!("Inserted {latency_inserts} rows in {:?}", start.elapsed());
    print_latencies(Arc::into_inner(insert_latencies).unwrap().into_iter().collect());

    let throttler = Arc::new(Semaphore::new(inflight()));
    let read_latencies = Arc::new(SegQueue::<Duration>::new());
    let start = Instant::now();
    for idx in 0..latency_reads {
        let pool = pool.clone();
        let latencies = read_latencies.clone();
        let permit = throttler.clone().acquire_owned().await?;
        let id = 1_000_000 + (idx % latency_inserts);
        tokio::spawn(async move {
            let t0 = Instant::now();
            read_one(&pool, id).await.expect("read");
            latencies.push(t0.elapsed());
            drop(permit);
        });
    }
    let _ = throttler.acquire_many(inflight() as u32).await?;
    println!("Read {latency_reads} rows in {:?}", start.elapsed());
    print_latencies(Arc::into_inner(read_latencies).unwrap().into_iter().collect());
    Ok(())
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let base = env::var("SPACETIMEDB_URL").unwrap_or_else(|_| "http://127.0.0.1:3000".to_string());
    let pool = connect_pool(&base)?;
    println!("logged in to {base}");
    println!(
        "mode=file-backed; {} persistent websockets; reducer insert_message; procedure get_message primary-key round trip",
        inflight()
    );
    println!("timed_read=server procedure result (not a client subscription cache)");
    insert_benchmark(&pool).await?;
    latency_benchmark(&pool).await?;
    Ok(())
}
