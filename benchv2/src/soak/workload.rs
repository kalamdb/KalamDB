//! Buyer, dispatcher, subscriber, and sweeper tasks.

use std::{
    collections::VecDeque,
    fs::OpenOptions,
    io::Write,
    path::PathBuf,
    sync::{
        atomic::{AtomicI64, AtomicU64, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use kalam_client::AutoOffsetReset;
use rand::RngExt;
use serde_json::Value;
use tokio::{
    sync::watch,
    task::JoinSet,
    time::{sleep, timeout},
};

use super::{memory::MemorySampler, SoakOptions};
use crate::client::{KalamClient, SqlResponse};

const SKUS: u64 = 32;
const LATENCY_CAP: usize = 8192;
const SWEEP_BATCH: i64 = 64;
const SWEEP_CUTOFF_MS: i64 = 30_000;
const SWEEP_SLOW_BATCH: Duration = Duration::from_millis(80);
const SWEEP_SAMPLE_KEEP_MS: i64 = 180_000;

#[derive(Default)]
struct Counters {
    quotes_ok:       AtomicU64,
    orders_ok:       AtomicU64,
    payments_ok:     AtomicU64,
    shipments_ok:    AtomicU64,
    sweeps_ok:       AtomicU64,
    reads_ok:        AtomicU64,
    ok:              AtomicU64,
    rejected:        AtomicU64,
    errors:          AtomicU64,
    topic_records:   AtomicU64,
    live_events:     AtomicU64,
    health_failures: AtomicU64,
    order_ids:       AtomicU64,
    sweep_next_id:   AtomicI64,
    sweep_samples:   Mutex<VecDeque<(i64, i64)>>,
}

pub struct DriveOutcome {
    pub quotes_ok:       u64,
    pub orders_ok:       u64,
    pub payments_ok:     u64,
    pub shipments_ok:    u64,
    pub sweeps_ok:       u64,
    pub reads_ok:        u64,
    pub ok:              u64,
    pub rejected:        u64,
    pub errors:          u64,
    pub topic_records:   u64,
    pub live_events:     u64,
    pub health_failures: u64,
    pub last_error:      String,
    latencies_ms:        Vec<u32>,
}

impl DriveOutcome {
    pub fn attempts(&self) -> u64 {
        self.ok + self.rejected + self.errors
    }

    pub fn latency_ms(&self) -> (u64, u64) {
        percentile(&self.latencies_ms, 50, 95)
    }
}

struct Latency {
    samples: Mutex<Vec<u32>>,
}

impl Latency {
    fn new() -> Self {
        Self {
            samples: Mutex::new(Vec::with_capacity(LATENCY_CAP)),
        }
    }

    fn record(&self, elapsed: Duration) {
        let millis = elapsed.as_millis().min(u128::from(u32::MAX)) as u32;
        let Ok(mut samples) = self.samples.lock() else {
            return;
        };
        if samples.len() < LATENCY_CAP {
            samples.push(millis);
            return;
        }
        let index = rand::rng().random_range(0..LATENCY_CAP);
        samples[index] = millis;
    }

    fn snapshot(&self) -> Vec<u32> {
        self.samples.lock().map(|samples| samples.clone()).unwrap_or_default()
    }
}

pub async fn drive(
    client: &KalamClient,
    namespace: &str,
    options: &SoakOptions,
    duration: Duration,
    memory: &MemorySampler,
) -> DriveOutcome {
    let counters = Arc::new(Counters::default());
    let latency = Arc::new(Latency::new());
    let last_error = Arc::new(Mutex::new(String::new()));
    let (stop_tx, stop_rx) = watch::channel(false);
    let deadline = Instant::now() + duration;
    let mut tasks = JoinSet::new();

    spawn_subscribers(
        &mut tasks,
        client,
        namespace,
        options.subscribers,
        Arc::clone(&counters),
        stop_rx.clone(),
    );
    spawn_consumer(
        &mut tasks,
        client,
        namespace,
        "order_placed",
        2,
        false,
        Arc::clone(&counters),
        Arc::clone(&last_error),
        stop_rx.clone(),
    );
    spawn_consumer(
        &mut tasks,
        client,
        namespace,
        "order_paid",
        options.dispatchers,
        true,
        Arc::clone(&counters),
        Arc::clone(&last_error),
        stop_rx.clone(),
    );
    spawn_consumer(
        &mut tasks,
        client,
        namespace,
        "order_shipped",
        2,
        false,
        Arc::clone(&counters),
        Arc::clone(&last_error),
        stop_rx.clone(),
    );

    for _ in 0..1 {
        let client = client.clone();
        let namespace = namespace.to_string();
        let counters = Arc::clone(&counters);
        let last_error = Arc::clone(&last_error);
        let mut stop_rx = stop_rx.clone();
        tasks.spawn(async move {
            sweeper(client, namespace, counters, last_error, &mut stop_rx).await;
        });
    }
    for _ in 0..8 {
        let client = client.clone();
        let namespace = namespace.to_string();
        let counters = Arc::clone(&counters);
        let mut stop_rx = stop_rx.clone();
        tasks.spawn(async move {
            reader(client, namespace, counters, &mut stop_rx).await;
        });
    }
    for _ in 0..options.buyers {
        let client = client.clone();
        let namespace = namespace.to_string();
        let counters = Arc::clone(&counters);
        let latency = Arc::clone(&latency);
        let last_error = Arc::clone(&last_error);
        let customers = options.customers.max(1);
        let mut stop_rx = stop_rx.clone();
        tasks.spawn(async move {
            buyer(
                client,
                namespace,
                customers,
                counters,
                latency,
                last_error,
                deadline,
                &mut stop_rx,
            )
            .await;
        });
    }

    {
        let client = client.clone();
        let counters = Arc::clone(&counters);
        let mut stop_rx = stop_rx.clone();
        tasks.spawn(async move {
            health_loop(client, counters, &mut stop_rx).await;
        });
    }
    {
        let counters = Arc::clone(&counters);
        let mut stop_rx = stop_rx.clone();
        tasks.spawn(async move {
            frontier_loop(counters, &mut stop_rx).await;
        });
    }
    {
        let counters = Arc::clone(&counters);
        let mut stop_rx = stop_rx.clone();
        let started = Instant::now();
        let client = client.clone();
        let readings = Arc::clone(&memory.latest_handle());
        let series_path =
            PathBuf::from(&options.output_dir).join(format!("{namespace}.series.jsonl"));
        tasks.spawn(async move {
            progress(client, counters, readings, started, series_path, &mut stop_rx).await;
        });
    }

    sleep(duration).await;
    let _ = stop_tx.send(true);
    let join_deadline = Duration::from_secs(30);
    let _ = timeout(join_deadline, async { while tasks.join_next().await.is_some() {} }).await;
    tasks.abort_all();

    DriveOutcome {
        quotes_ok:       counters.quotes_ok.load(Ordering::Relaxed),
        orders_ok:       counters.orders_ok.load(Ordering::Relaxed),
        payments_ok:     counters.payments_ok.load(Ordering::Relaxed),
        shipments_ok:    counters.shipments_ok.load(Ordering::Relaxed),
        sweeps_ok:       counters.sweeps_ok.load(Ordering::Relaxed),
        reads_ok:        counters.reads_ok.load(Ordering::Relaxed),
        ok:              counters.ok.load(Ordering::Relaxed),
        rejected:        counters.rejected.load(Ordering::Relaxed),
        errors:          counters.errors.load(Ordering::Relaxed),
        topic_records:   counters.topic_records.load(Ordering::Relaxed),
        live_events:     counters.live_events.load(Ordering::Relaxed),
        health_failures: counters.health_failures.load(Ordering::Relaxed),
        last_error:      last_error.lock().map(|error| error.clone()).unwrap_or_default(),
        latencies_ms:    latency.snapshot(),
    }
}

fn spawn_subscribers(
    tasks: &mut JoinSet<()>,
    client: &KalamClient,
    namespace: &str,
    subscribers: u32,
    counters: Arc<Counters>,
    stop_rx: watch::Receiver<bool>,
) {
    let sql = format!("SELECT id, order_id, kind, body, created_ms FROM {namespace}.inbox");
    for index in 0..subscribers {
        let link = match client.new_isolated_link_for_ws_url(None) {
            Ok(link) => link,
            Err(error) => {
                eprintln!("subscriber {index} link: {error}");
                continue;
            },
        };
        let sql = sql.clone();
        let counters = Arc::clone(&counters);
        let mut stop_rx = stop_rx.clone();
        tasks.spawn(async move {
            let mut subscription = match link.live_events(&sql).await {
                Ok(subscription) => subscription,
                Err(error) => {
                    eprintln!("subscribe: {error}");
                    counters.errors.fetch_add(1, Ordering::Relaxed);
                    return;
                },
            };
            loop {
                tokio::select! {
                    changed = stop_rx.changed() => {
                        let _ = changed;
                        break;
                    }
                    event = subscription.next() => {
                        match event {
                            Some(Ok(_)) => {
                                counters.live_events.fetch_add(1, Ordering::Relaxed);
                            }
                            Some(Err(_)) => {
                                counters.errors.fetch_add(1, Ordering::Relaxed);
                            }
                            None => break,
                        }
                    }
                }
            }
            let _ = subscription.close().await;
        });
    }
}

fn spawn_consumer(
    tasks: &mut JoinSet<()>,
    client: &KalamClient,
    namespace: &str,
    topic: &str,
    workers: u32,
    dispatch: bool,
    counters: Arc<Counters>,
    last_error: Arc<Mutex<String>>,
    stop_rx: watch::Receiver<bool>,
) {
    for _worker in 0..workers.max(1) {
        let client = client.clone();
        let namespace = namespace.to_string();
        let topic_name = format!("{namespace}.{topic}");
        let group = format!("soak_{topic}");
        let counters = Arc::clone(&counters);
        let last_error = Arc::clone(&last_error);
        let mut stop_rx = stop_rx.clone();
        tasks.spawn(async move {
            consume_loop(
                client,
                namespace,
                topic_name,
                group,
                dispatch,
                counters,
                last_error,
                &mut stop_rx,
            )
            .await;
        });
    }
}

async fn consume_loop(
    client: KalamClient,
    namespace: String,
    topic_name: String,
    group: String,
    dispatch: bool,
    counters: Arc<Counters>,
    last_error: Arc<Mutex<String>>,
    stop_rx: &mut watch::Receiver<bool>,
) {
    let mut consumer = match client
        .link()
        .consumer()
        .topic(&topic_name)
        .group_id(&group)
        .auto_offset_reset(AutoOffsetReset::Earliest)
        .max_poll_records(64)
        .poll_timeout(Duration::from_secs(1))
        .build()
    {
        Ok(consumer) => consumer,
        Err(error) => {
            note_error(&last_error, &counters, format!("consumer {topic_name}: {error}"));
            return;
        },
    };

    loop {
        if *stop_rx.borrow() {
            break;
        }
        let records = tokio::select! {
            changed = stop_rx.changed() => {
                let _ = changed;
                break;
            }
            records = consumer.poll_with_timeout(Duration::from_secs(1)) => {
                match records {
                    Ok(records) => records,
                    Err(error) => {
                        note_error(&last_error, &counters, format!("poll {topic_name}: {error}"));
                        sleep(Duration::from_millis(200)).await;
                        continue;
                    }
                }
            }
        };

        for record in &records {
            counters.topic_records.fetch_add(1, Ordering::Relaxed);
            if dispatch {
                if let Some(order_id) = parse_order_id(&record.payload) {
                    let created_ms = now_ms();
                    let sql = format!(
                        "SELECT {namespace}.dispatch_shipment({order_id}, {created_ms}) AS result"
                    );
                    match classify(&client, &sql).await {
                        CallClass::Ok => {
                            counters.shipments_ok.fetch_add(1, Ordering::Relaxed);
                            counters.ok.fetch_add(1, Ordering::Relaxed);
                        },
                        CallClass::Reject => {
                            sleep(Duration::from_millis(30)).await;
                            if matches!(classify(&client, &sql).await, CallClass::Ok) {
                                counters.shipments_ok.fetch_add(1, Ordering::Relaxed);
                                counters.ok.fetch_add(1, Ordering::Relaxed);
                            } else {
                                counters.rejected.fetch_add(1, Ordering::Relaxed);
                            }
                        },
                        CallClass::Error(error) => note_error(&last_error, &counters, error),
                    }
                }
            }
            consumer.mark_processed(record);
        }
        if !records.is_empty() {
            if let Err(error) = consumer.commit_sync().await {
                note_error(&last_error, &counters, format!("commit {topic_name}: {error}"));
            }
        }
    }
    let _ = consumer.close().await;
}

async fn buyer(
    client: KalamClient,
    namespace: String,
    customers: u64,
    counters: Arc<Counters>,
    latency: Arc<Latency>,
    last_error: Arc<Mutex<String>>,
    deadline: Instant,
    stop_rx: &mut watch::Receiver<bool>,
) {
    while Instant::now() < deadline && !*stop_rx.borrow() {
        let id = counters.order_ids.fetch_add(1, Ordering::Relaxed) + 1;
        let sku = format!("sku_{:02}", id % SKUS);
        let qty = 1 + (id % 4);
        let promo = match id % 5 {
            0 => "SAVE10",
            1 => "BULK",
            2 => "FREESHIP",
            _ => "NONE",
        };
        let customer = format!("c{}", id % customers);
        let method = match id % 3 {
            0 => "card",
            1 => "wallet",
            _ => "cod",
        };
        let started = Instant::now();
        let created_ms = now_ms();

        let quote = format!(
            "SELECT {namespace}.quote_cart('{customer}', '{sku}', {qty}, '{promo}') AS result"
        );
        match classify(&client, &quote).await {
            CallClass::Ok => {
                counters.quotes_ok.fetch_add(1, Ordering::Relaxed);
            },
            CallClass::Reject => {
                counters.rejected.fetch_add(1, Ordering::Relaxed);
                continue;
            },
            CallClass::Error(error) => {
                note_error(&last_error, &counters, error);
                sleep(Duration::from_millis(50)).await;
                continue;
            },
        }

        let place = format!(
            "SELECT {namespace}.place_order({id}, '{customer}', '{sku}', {qty}, '{promo}', \
             {created_ms}) AS result"
        );
        match classify(&client, &place).await {
            CallClass::Ok => {
                counters.orders_ok.fetch_add(1, Ordering::Relaxed);
            },
            CallClass::Reject => {
                counters.rejected.fetch_add(1, Ordering::Relaxed);
                latency.record(started.elapsed());
                continue;
            },
            CallClass::Error(error) => {
                note_error(&last_error, &counters, error);
                latency.record(started.elapsed());
                sleep(Duration::from_millis(50)).await;
                continue;
            },
        }

        let pay =
            format!("SELECT {namespace}.capture_payment({id}, '{method}', {created_ms}) AS result");
        match classify(&client, &pay).await {
            CallClass::Ok => {
                counters.payments_ok.fetch_add(1, Ordering::Relaxed);
                counters.ok.fetch_add(1, Ordering::Relaxed);
            },
            CallClass::Reject => {
                counters.rejected.fetch_add(1, Ordering::Relaxed);
            },
            CallClass::Error(error) => {
                note_error(&last_error, &counters, error);
                sleep(Duration::from_millis(50)).await;
            },
        }
        latency.record(started.elapsed());
    }
}

async fn sweeper(
    client: KalamClient,
    namespace: String,
    counters: Arc<Counters>,
    last_error: Arc<Mutex<String>>,
    stop_rx: &mut watch::Receiver<bool>,
) {
    loop {
        if *stop_rx.borrow() {
            break;
        }
        let limit = sweep_limit(&counters, now_ms());
        let Some((start, end)) = reserve_sweep_range(&counters.sweep_next_id, limit) else {
            tokio::select! {
                _ = sleep(Duration::from_millis(50)) => {}
                changed = stop_rx.changed() => {
                    let _ = changed;
                    break;
                }
            }
            continue;
        };
        let started = Instant::now();
        let failed = match delete_sweep_range(&client, &namespace, start, end).await {
            Ok(()) => {
                counters.sweeps_ok.fetch_add((end - start) as u64, Ordering::Relaxed);
                false
            },
            Err(error) => {
                rewind_sweep_cursor(&counters.sweep_next_id, start);
                note_error(&last_error, &counters, error);
                true
            },
        };
        // One batch at a time. If a batch is slow, wait so buyers are not queued
        // behind cleanup. A fast batch keeps the retained window bounded.
        let pause = if failed {
            Duration::from_millis(50)
        } else {
            let elapsed = started.elapsed();
            if elapsed > SWEEP_SLOW_BATCH {
                elapsed
            } else {
                Duration::ZERO
            }
        };
        if pause.is_zero() {
            continue;
        }
        tokio::select! {
            _ = sleep(pause.min(Duration::from_millis(500))) => {}
            changed = stop_rx.changed() => {
                let _ = changed;
                break;
            }
        }
    }
}

async fn delete_sweep_range(
    client: &KalamClient,
    namespace: &str,
    start: i64,
    end: i64,
) -> Result<(), String> {
    let mut ids = String::new();
    let mut inbox = String::new();
    for id in start..end {
        if !ids.is_empty() {
            ids.push(',');
        }
        ids.push_str(&id.to_string());
        for offset in 1..=3 {
            if !inbox.is_empty() {
                inbox.push(',');
            }
            inbox.push_str(&(id * 10 + offset).to_string());
        }
    }
    for sql in [
        format!("DELETE FROM {namespace}.orders WHERE id IN ({ids})"),
        format!("DELETE FROM {namespace}.payments WHERE order_id IN ({ids})"),
        format!("DELETE FROM {namespace}.shipments WHERE order_id IN ({ids})"),
        format!("DELETE FROM {namespace}.inbox WHERE id IN ({inbox})"),
    ] {
        let response = client.sql(&sql).await?;
        if let CallClass::Error(error) = classify_response(&response) {
            return Err(error);
        }
    }
    Ok(())
}

fn reserve_sweep_range(cursor: &AtomicI64, limit: i64) -> Option<(i64, i64)> {
    loop {
        let current = cursor.load(Ordering::Relaxed);
        let start = current.max(1);
        if start >= limit {
            return None;
        }
        let end = start.saturating_add(SWEEP_BATCH).min(limit);
        if end <= start {
            return None;
        }
        let expected = if current < 1 { 0 } else { start };
        if cursor
            .compare_exchange_weak(expected, end, Ordering::Relaxed, Ordering::Relaxed)
            .is_ok()
        {
            return Some((start, end));
        }
    }
}

fn note_order_frontier(counters: &Counters) {
    let id = i64::try_from(counters.order_ids.load(Ordering::Relaxed)).unwrap_or(i64::MAX);
    let now = now_ms();
    let Ok(mut samples) = counters.sweep_samples.lock() else {
        return;
    };
    samples.push_back((id, now));
    let keep_after = now.saturating_sub(SWEEP_SAMPLE_KEEP_MS);
    while samples.front().copied().is_some_and(|(_, ts)| ts < keep_after) {
        samples.pop_front();
    }
}

fn sweep_limit(counters: &Counters, now: i64) -> i64 {
    let cutoff = now.saturating_sub(SWEEP_CUTOFF_MS);
    let Ok(samples) = counters.sweep_samples.lock() else {
        return 1;
    };
    let mut limit = 1i64;
    for &(id, ts) in samples.iter() {
        if ts <= cutoff {
            limit = id.saturating_add(1);
        } else {
            break;
        }
    }
    limit
}

async fn frontier_loop(counters: Arc<Counters>, stop_rx: &mut watch::Receiver<bool>) {
    note_order_frontier(&counters);
    loop {
        tokio::select! {
            _ = sleep(Duration::from_millis(200)) => {}
            changed = stop_rx.changed() => {
                let _ = changed;
                break;
            }
        }
        if *stop_rx.borrow() {
            break;
        }
        note_order_frontier(&counters);
    }
}

fn rewind_sweep_cursor(cursor: &AtomicI64, to: i64) {
    let to = to.max(1);
    let mut current = cursor.load(Ordering::Relaxed);
    while current > to {
        match cursor.compare_exchange_weak(current, to, Ordering::Relaxed, Ordering::Relaxed) {
            Ok(_) => break,
            Err(observed) => current = observed,
        }
    }
}

fn sweep_next_id(response: &SqlResponse) -> Option<i64> {
    response.results.iter().find_map(|result| {
        result
            .rows
            .as_ref()?
            .iter()
            .find_map(|row| row.iter().find_map(sweep_next_id_value))
    })
}

fn sweep_next_id_value(value: &Value) -> Option<i64> {
    match value {
        Value::Object(map) => map
            .get("next_id")
            .and_then(json_i64)
            .or_else(|| map.values().find_map(sweep_next_id_value)),
        Value::Array(items) => items.iter().find_map(sweep_next_id_value),
        Value::String(text) if text.starts_with('{') => serde_json::from_str::<Value>(text)
            .ok()
            .and_then(|parsed| sweep_next_id_value(&parsed)),
        _ => None,
    }
}

fn json_i64(value: &Value) -> Option<i64> {
    match value {
        Value::Number(number) => {
            number.as_i64().or_else(|| number.as_u64().and_then(|n| i64::try_from(n).ok()))
        },
        Value::String(text) => text.parse().ok(),
        _ => None,
    }
}

async fn reader(
    client: KalamClient,
    namespace: String,
    counters: Arc<Counters>,
    stop_rx: &mut watch::Receiver<bool>,
) {
    loop {
        if *stop_rx.borrow() {
            break;
        }
        let id = counters.order_ids.load(Ordering::Relaxed).max(1);
        let sql = format!("SELECT id, status, total_cents FROM {namespace}.orders WHERE id = {id}");
        match client.sql(&sql).await {
            Ok(response) if response.status == "success" => {
                counters.reads_ok.fetch_add(1, Ordering::Relaxed);
            },
            Ok(response) => {
                let message = response
                    .error
                    .map(|error| error.message)
                    .unwrap_or_else(|| "read failed".to_string());
                if !message.contains("does not exist") {
                    counters.errors.fetch_add(1, Ordering::Relaxed);
                }
            },
            Err(_) => {
                counters.errors.fetch_add(1, Ordering::Relaxed);
            },
        }
        tokio::select! {
            _ = sleep(Duration::from_millis(20)) => {}
            changed = stop_rx.changed() => {
                let _ = changed;
                break;
            }
        }
    }
}

async fn health_loop(
    client: KalamClient,
    counters: Arc<Counters>,
    stop_rx: &mut watch::Receiver<bool>,
) {
    let mut consecutive = 0u32;
    loop {
        if *stop_rx.borrow() {
            break;
        }
        if client.health_check().await {
            consecutive = 0;
        } else {
            consecutive += 1;
            counters.health_failures.fetch_add(1, Ordering::Relaxed);
            if consecutive >= 3 {
                eprintln!("server health failed 3 times in a row");
            }
        }
        tokio::select! {
            _ = sleep(Duration::from_secs(2)) => {}
            changed = stop_rx.changed() => {
                let _ = changed;
                break;
            }
        }
    }
}

async fn progress(
    client: KalamClient,
    counters: Arc<Counters>,
    readings: Arc<Mutex<super::memory::MemoryReading>>,
    started: Instant,
    series_path: PathBuf,
    stop_rx: &mut watch::Receiver<bool>,
) {
    let mut previous = ProgressSample::default();
    if let Some(parent) = series_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::remove_file(&series_path);
    loop {
        tokio::select! {
            _ = sleep(Duration::from_secs(10)) => {}
            changed = stop_rx.changed() => {
                let _ = changed;
                break;
            }
        }
        if *stop_rx.borrow() {
            break;
        }
        let reading = readings.lock().map(|reading| reading.clone()).unwrap_or(
            super::memory::MemoryReading {
                server_rss:  0,
                server_pids: 0,
            },
        );
        let sample = ProgressSample {
            elapsed:   0,
            orders:    counters.orders_ok.load(Ordering::Relaxed),
            paid:      counters.payments_ok.load(Ordering::Relaxed),
            shipped:   counters.shipments_ok.load(Ordering::Relaxed),
            swept:     counters.sweeps_ok.load(Ordering::Relaxed),
            reads:     counters.reads_ok.load(Ordering::Relaxed),
            quotes:    counters.quotes_ok.load(Ordering::Relaxed),
            rejected:  counters.rejected.load(Ordering::Relaxed),
            errors:    counters.errors.load(Ordering::Relaxed),
            live:      counters.live_events.load(Ordering::Relaxed),
            topics:    counters.topic_records.load(Ordering::Relaxed),
            rss_bytes: reading.server_rss,
        };
        let (rss_mb, footprint_mb) = match timeout(
            Duration::from_secs(2),
            client.sql("SELECT metric_name, metric_value FROM system.stats WHERE metric_name IN ('memory_rss_mb', 'memory_usage_mb')"),
        ).await {
            Ok(Ok(response)) => (memory_metric_mb(&response, "memory_rss_mb"), memory_metric_mb(&response, "memory_usage_mb")),
            _ => (None, None),
        };
        let elapsed = started.elapsed().as_secs();
        let dt = elapsed.saturating_sub(previous.elapsed).max(1);
        let orders_per_sec = rate(sample.orders, previous.orders, dt);
        let paid_per_sec = rate(sample.paid, previous.paid, dt);
        let shipped_per_sec = rate(sample.shipped, previous.shipped, dt);
        let swept_per_sec = rate(sample.swept, previous.swept, dt);
        let reads_per_sec = rate(sample.reads, previous.reads, dt);
        let quotes_per_sec = rate(sample.quotes, previous.quotes, dt);
        println!(
            "t={}s orders/s={:.0} paid/s={:.0} shipped/s={:.0} swept/s={:.0} quotes/s={:.0} \
             reads/s={:.0} rss={} footprint_mb={} rejected={} errors={} totals={}/{}/{}",
            elapsed,
            orders_per_sec,
            paid_per_sec,
            shipped_per_sec,
            swept_per_sec,
            quotes_per_sec,
            reads_per_sec,
            super::format_bytes(reading.server_rss),
            footprint_mb.map(|mb| format!("{mb:.1}")).unwrap_or_else(|| "unavailable".into()),
            sample.rejected,
            sample.errors,
            sample.orders,
            sample.paid,
            sample.shipped
        );
        if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(&series_path) {
            let line = serde_json::json!({
                "t": elapsed,
                "orders_per_sec": orders_per_sec,
                "paid_per_sec": paid_per_sec,
                "shipped_per_sec": shipped_per_sec,
                "swept_per_sec": swept_per_sec,
                "quotes_per_sec": quotes_per_sec,
                "reads_per_sec": reads_per_sec,
                "rss_bytes": sample.rss_bytes,
                "memory_usage_mb": footprint_mb,
                "memory_rss_mb": rss_mb,
                "orders": sample.orders,
                "paid": sample.paid,
                "shipped": sample.shipped,
                "swept": sample.swept,
                "rejected": sample.rejected,
                "errors": sample.errors,
                "live": sample.live,
                "topics": sample.topics,
            });
            let _ = writeln!(file, "{line}");
        }
        previous = sample;
        previous.elapsed = elapsed;
    }
}

#[derive(Clone, Copy, Default)]
struct ProgressSample {
    elapsed:   u64,
    orders:    u64,
    paid:      u64,
    shipped:   u64,
    swept:     u64,
    reads:     u64,
    quotes:    u64,
    rejected:  u64,
    errors:    u64,
    live:      u64,
    topics:    u64,
    rss_bytes: u64,
}

fn memory_metric_mb(response: &SqlResponse, name: &str) -> Option<f64> {
    let rows = response.results.first()?.rows.as_ref()?;
    let row = rows.iter().find(|row| row.first().and_then(Value::as_str) == Some(name))?;
    let value = row.get(1)?;
    value.as_f64().or_else(|| value.as_str()?.parse().ok())
}

fn rate(current: u64, previous: u64, dt: u64) -> f64 {
    current.saturating_sub(previous) as f64 / dt as f64
}

enum CallClass {
    Ok,
    Reject,
    Error(String),
}

async fn classify(client: &KalamClient, sql: &str) -> CallClass {
    match client.sql(sql).await {
        Ok(response) => classify_response(&response),
        Err(error) => CallClass::Error(trim_error(error)),
    }
}

fn classify_response(response: &SqlResponse) -> CallClass {
    if response.status != "success" {
        let message = response
            .error
            .as_ref()
            .map(|error| error.message.clone())
            .unwrap_or_else(|| "sql failed".to_string());
        return CallClass::Error(trim_error(message));
    }
    if response_rejects(response) {
        CallClass::Reject
    } else {
        CallClass::Ok
    }
}

fn response_rejects(response: &SqlResponse) -> bool {
    for result in &response.results {
        if result.message.as_deref().is_some_and(text_rejects) {
            return true;
        }
        if let Some(rows) = &result.rows {
            for row in rows {
                for cell in row {
                    if value_rejects(cell) {
                        return true;
                    }
                }
            }
        }
    }
    false
}

fn value_rejects(value: &Value) -> bool {
    match value {
        Value::Object(map) => {
            map.get("ok").is_some_and(|ok| ok == &Value::Bool(false))
                || map.values().any(value_rejects)
        },
        Value::Array(items) => items.iter().any(value_rejects),
        Value::String(text) => text_rejects(text),
        _ => false,
    }
}

fn text_rejects(text: &str) -> bool {
    text.contains("\"ok\":false") || text.contains("\"ok\": false")
}

fn note_error(last_error: &Mutex<String>, counters: &Counters, error: String) {
    counters.errors.fetch_add(1, Ordering::Relaxed);
    if let Ok(mut slot) = last_error.lock() {
        *slot = trim_error(error);
    }
}

fn trim_error(error: String) -> String {
    let mut trimmed = error.replace('\n', " ");
    if trimmed.len() > 240 {
        trimmed.truncate(240);
    }
    trimmed
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as i64)
        .unwrap_or(0)
}

pub fn parse_order_id(payload: &[u8]) -> Option<i64> {
    if let Some(id) = parse_order_id_json(payload) {
        return Some(id);
    }
    // Typed publishes are stored as a binary envelope. The functions embed an
    // ASCII `ord:<id>` token so a consumer can recover the id without that codec.
    let text = String::from_utf8_lossy(payload);
    let start = text.find("ord:")?;
    let digits: String = text[start + 4..].chars().take_while(|ch| ch.is_ascii_digit()).collect();
    if digits.is_empty() {
        None
    } else {
        digits.parse().ok()
    }
}

fn parse_order_id_json(payload: &[u8]) -> Option<i64> {
    let value: Value = serde_json::from_slice(payload).ok()?;
    let object = match &value {
        Value::String(text) => serde_json::from_str::<Value>(text).unwrap_or(Value::Null),
        other => other.clone(),
    };
    let row = object.get("row").cloned().unwrap_or(object);
    let id = row
        .get("orderId")
        .or_else(|| row.get("order_id"))
        .or_else(|| row.get("token"))?;
    match id {
        Value::Number(number) => {
            number.as_i64().or_else(|| number.as_u64().map(|value| value as i64))
        },
        Value::String(text) => text.strip_prefix("ord:").unwrap_or(text).parse().ok(),
        _ => None,
    }
}

fn percentile(samples: &[u32], p50: u8, p95: u8) -> (u64, u64) {
    if samples.is_empty() {
        return (0, 0);
    }
    let mut sorted = samples.to_vec();
    sorted.sort_unstable();
    (pick(&sorted, p50), pick(&sorted, p95))
}

fn pick(sorted: &[u32], percentile: u8) -> u64 {
    let index = (sorted.len() - 1) * usize::from(percentile) / 100;
    u64::from(sorted[index])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_published_order_id() {
        let payload = br#"{"orderId":42,"method":"card"}"#;
        assert_eq!(parse_order_id(payload), Some(42));
    }

    #[test]
    fn reads_row_wrapped_order_id() {
        let payload = br#"{"row":{"order_id":"99"}}"#;
        assert_eq!(parse_order_id(payload), Some(99));
    }

    #[test]
    fn reads_order_token_inside_binary_payload() {
        let payload = b"\x00\x01ord:17074\xff";
        assert_eq!(parse_order_id(payload), Some(17074));
    }

    #[test]
    fn reads_sweep_cursor_from_function_json() {
        let value = serde_json::json!({"ok": true, "next_id": 48, "caught_up": false});
        assert_eq!(sweep_next_id_value(&value), Some(48));
        let wrapped = Value::String(r#"{"next_id":"48","caught_up":true}"#.to_string());
        assert_eq!(sweep_next_id_value(&wrapped), Some(48));
    }
}
