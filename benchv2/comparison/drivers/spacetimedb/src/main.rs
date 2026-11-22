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
mod realtime;

use std::env;
use std::future::Future;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::Context;
use comparison_common::{message_data, run, LIMIT};
use module_bindings::{get_message, insert_message, DbConnection};
use tokio::sync::oneshot;

const DATABASE: &str = "bench";

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
        .with_confirmed_reads(true)
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
    let width = LIMIT;
    let mut slots = Vec::with_capacity(width);
    for _ in 0..width {
        slots.push(connect_one(base)?);
    }
    Ok(Arc::new(Pool {
        slots: Mutex::new(slots),
    }))
}

async fn insert_one(pool: &Pool, id: i64) -> anyhow::Result<i64> {
    let conn = pool.checkout();
    let result = dispatch_insert(&conn, id)?.await;
    pool.checkin(conn);
    result?;
    Ok(id)
}

fn dispatch_insert(
    conn: &DbConnection,
    id: i64,
) -> anyhow::Result<impl Future<Output = anyhow::Result<()>>> {
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
    Ok(async move {
        let result = tokio::time::timeout(Duration::from_secs(60), rx)
            .await
            .context("insert timed out")?
            .context("insert callback dropped")?;
        match result {
            Ok(Ok(())) => Ok(()),
            Ok(Err(message)) => anyhow::bail!("reducer failed: {message}"),
            Err(err) => anyhow::bail!("reducer transport: {err:?}"),
        }
    })
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
        Ok(Some(row)) => {
            anyhow::ensure!(
                row.id == id && row.owner == "user1" && row.room == "room0" && row.data == message_data(id),
                "wrong SpacetimeDB row for {id}: {row:?}"
            );
            Ok(())
        }
        Ok(None) => anyhow::bail!("get_message returned no row for id {id}"),
        Err(err) => anyhow::bail!("procedure transport: {err:?}"),
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let base = env::var("SPACETIMEDB_URL").unwrap_or_else(|_| "http://127.0.0.1:3000".to_string());
    if env::var("COMPARISON_WORKLOAD").as_deref() == Ok("realtime") {
        return realtime::run(&base).await;
    }
    let pool = connect_pool(&base)?;
    println!("logged in to {base}");
    println!(
        "mode=file-backed; {} persistent websockets; reducer insert_message; procedure get_message primary-key round trip",
        LIMIT
    );
    println!("timed_read=server procedure result (not a client subscription cache)");
    run(|id| insert_one(&pool, id), |_, id| read_one(&pool, id)).await
}
