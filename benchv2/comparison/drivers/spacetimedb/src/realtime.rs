use std::collections::HashSet;
use std::env;
use std::time::{Duration, Instant};

use anyhow::{bail, ensure, Context};
use comparison_common::message_data;
use serde_json::{json, Value};
use spacetimedb_sdk::{DbContext, Event, Table};
use tokio::sync::mpsc;
use tokio::task::JoinSet;
use tokio::time::{sleep, timeout, timeout_at, Instant as Deadline};

use super::module_bindings::{Message, MessageTableAccess, SubscriptionHandle};
use super::{connect_one, dispatch_insert, DbConnection};

const TIMEOUT: Duration = Duration::from_secs(15);
const DRAIN: Duration = Duration::from_millis(250);

enum Notice {
    History(usize, Vec<Message>, Instant),
    Insert(usize, Message, Instant),
    Failed(String),
}

fn setting(name: &str, default: usize, min: usize, max: usize) -> anyhow::Result<usize> {
    let value = match env::var(name) {
        Ok(raw) => {
            ensure!(raw.bytes().all(|byte| byte.is_ascii_digit()), "invalid {name}");
            raw.parse::<usize>().with_context(|| format!("invalid {name}"))?
        }
        Err(env::VarError::NotPresent) => default,
        Err(error) => return Err(error).with_context(|| format!("invalid {name}")),
    };
    ensure!((min..=max).contains(&value), "{name} must be in {min}..={max}");
    Ok(value)
}

fn verify_row(row: &Message) -> anyhow::Result<()> {
    ensure!(
        row.owner == "user1" && row.room == "room0" && row.data == message_data(row.id),
        "wrong subscription payload: {row:?}"
    );
    Ok(())
}

fn subscribe(
    base: &str,
    index: usize,
    notices: mpsc::UnboundedSender<Notice>,
) -> anyhow::Result<(DbConnection, SubscriptionHandle)> {
    let connection = connect_one(base)?;
    let inserts = notices.clone();
    connection.db.message().on_insert(move |ctx, row| {
        if !matches!(&ctx.event, Event::SubscribeApplied) {
            let received = Instant::now();
            let _ = inserts.send(Notice::Insert(index, row.clone(), received));
        }
    });
    let errors = notices.clone();
    let subscription = connection.subscription_builder()
        .on_applied(move |ctx| {
            let received = Instant::now();
            let rows = Table::iter(&ctx.db.message()).collect();
            let _ = notices.send(Notice::History(index, rows, received));
        })
        .on_error(move |_ctx, error| {
            let _ = errors.send(Notice::Failed(format!("subscriber {index}: {error:?}")));
        })
        .subscribe("SELECT * FROM message WHERE room = 'room0'");
    Ok((connection, subscription))
}

fn summary(samples: &mut [Duration]) -> Value {
    if samples.is_empty() {
        return Value::Null;
    }
    samples.sort_unstable();
    let percentile = |p: f64| samples[(samples.len() as f64 * p).floor() as usize].as_secs_f64() * 1000.0;
    json!({ "count": samples.len(), "p50_ms": percentile(0.50), "p95_ms": percentile(0.95) })
}

pub(super) async fn run(base: &str) -> anyhow::Result<()> {
    let history = setting("REALTIME_HISTORY", 20, 1, 499)?;
    let messages = setting("REALTIME_MESSAGES", 100, 1, 500 - history)?;
    let subscribers = setting("REALTIME_SUBSCRIBERS", 16, 1, 128)?;
    let interval = Duration::from_millis(setting("REALTIME_INTERVAL_MS", 50, 1, 60_000)? as u64);
    let expected = messages * subscribers;
    let mut history_latencies = Vec::with_capacity(subscribers);
    let mut write_latencies = Vec::with_capacity(messages);
    let mut deliveries = Vec::with_capacity(expected);
    let mut duplicates = 0;

    println!("{}", json!({
        "workload": "realtime-v1", "system": "spacetimedb", "history": history,
        "messages": messages, "subscribers": subscribers, "interval_ms": interval.as_millis() as u64,
        "delivery_timeout_ms": TIMEOUT.as_millis() as u64, "drain_ms": DRAIN.as_millis() as u64,
    }));

    let result = async {
        let writer_base = base.to_owned();
        let opening = tokio::task::spawn_blocking(move || connect_one(&writer_base));
        let writer = timeout(TIMEOUT, opening)
            .await
            .context("writer setup timed out")?
            .context("writer setup task failed")??;
        timeout(TIMEOUT, async {
            for sequence in 0..history {
                dispatch_insert(&writer, sequence as i64)?.await?;
            }
            Ok::<_, anyhow::Error>(())
        }).await.context("history seed timed out")??;

        let (notices, mut receiver) = mpsc::unbounded_channel();
        let mut connecting = JoinSet::new();
        let mut opened = Vec::with_capacity(subscribers);
        for index in 0..subscribers {
            let subscriber_base = base.to_owned();
            let notices = notices.clone();
            opened.push(Instant::now());
            connecting.spawn_blocking(move || subscribe(&subscriber_base, index, notices));
        }
        drop(notices);

        let history_deadline = Deadline::now() + TIMEOUT;
        let mut connections = Vec::with_capacity(subscribers);
        while let Some(connection) = timeout_at(history_deadline, connecting.join_next()).await
            .context("subscriber connection timed out")?
        {
            connections.push(connection??);
        }
        let mut initialized = vec![false; subscribers];
        for _ in 0..subscribers {
            let notice = timeout_at(history_deadline, receiver.recv()).await
                .context("initial history timed out")?.context("subscriber channel closed")?;
            match notice {
                Notice::History(index, rows, received) => {
                    ensure!(!initialized[index] && rows.len() == history, "wrong initial history for subscriber {index}");
                    let mut keys = HashSet::new();
                    for row in &rows {
                        verify_row(row)?;
                        ensure!((0..history as i64).contains(&row.id) && keys.insert(row.id), "wrong history row {row:?}");
                    }
                    initialized[index] = true;
                    history_latencies.push(received.duration_since(opened[index]));
                }
                Notice::Failed(error) => bail!("{error}"),
                Notice::Insert(..) => bail!("received a live insert before the write phase"),
            }
        }

        let mut started = Vec::with_capacity(messages);
        let mut writes = JoinSet::new();
        for offset in 0..messages {
            if offset > 0 {
                sleep(interval).await;
            }
            let sent = Instant::now();
            started.push(sent);
            let ack = dispatch_insert(&writer, (history + offset) as i64)?;
            writes.spawn(async move {
                ack.await?;
                Ok::<_, anyhow::Error>(sent.elapsed())
            });
        }
        let delivery_deadline = Deadline::now() + TIMEOUT;
        let mut delivered = vec![vec![false; messages]; subscribers];
        let mut drain_deadline = None;
        let read_deliveries = async {
            loop {
                if deliveries.len() == expected && drain_deadline.is_none() {
                    drain_deadline = Some(Deadline::now() + DRAIN);
                }
                match timeout_at(drain_deadline.unwrap_or(delivery_deadline), receiver.recv()).await {
                    Err(_) => break,
                    Ok(Some(Notice::Insert(index, row, received))) => {
                        verify_row(&row)?;
                        ensure!((history as i64..(history + messages) as i64).contains(&row.id), "unexpected live message {row:?}");
                        let offset = row.id as usize - history;
                        if delivered[index][offset] {
                            duplicates += 1;
                        } else {
                            delivered[index][offset] = true;
                            deliveries.push(received.checked_duration_since(started[offset]).context("received a message before it was sent")?);
                        }
                    }
                    Ok(Some(Notice::Failed(error))) => bail!("{error}"),
                    Ok(Some(Notice::History(..))) => bail!("unexpected second snapshot"),
                    Ok(None) => bail!("subscriber channel closed"),
                }
            }
            Ok::<_, anyhow::Error>(())
        };
        let read_acknowledgments = async {
            while let Some(write) = timeout_at(delivery_deadline, writes.join_next()).await
                .context("write acknowledgments timed out")?
            {
                write_latencies.push(write??);
            }
            Ok::<_, anyhow::Error>(())
        };
        tokio::try_join!(read_deliveries, read_acknowledgments)?;
        ensure!(connections.iter().all(|(connection, _)| connection.is_active()), "subscriber disconnected");
        for (connection, _subscription) in connections {
            let _ = connection.disconnect();
        }
        let _ = writer.disconnect();
        ensure!(deliveries.len() == expected && duplicates == 0, "realtime delivery counts failed verification");
        Ok::<_, anyhow::Error>(())
    }.await;

    println!("{}", json!({
        "history_latency": summary(&mut history_latencies),
        "write_ack_latency": summary(&mut write_latencies),
        "delivery_latency": summary(&mut deliveries),
        "expected": expected, "received": deliveries.len(), "missed": expected - deliveries.len(),
        "duplicates": duplicates,
    }));
    result?;
    println!("verified=true");
    Ok(())
}
