use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex,
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use kalam_client::{
    ChangeEvent, KalamCellValue, KalamLinkClient, SubscriptionConfig, SubscriptionManager,
};
use serde_json::Value as JsonValue;
use sysinfo::{MemoryRefreshKind, Pid, ProcessRefreshKind, ProcessesToUpdate, RefreshKind, System};
use tokio::{
    sync::{watch, Mutex as AsyncMutex, Semaphore},
    task::JoinHandle,
    time::{sleep, timeout, Instant},
};

use crate::client::{KalamClient, SqlResponse};

pub const DEFAULT_CHAT_MINUTES: u64 = 5;
pub const DEFAULT_CHAT_USERS: u32 = 1_000;
pub const DEFAULT_CHAT_REALTIME_CONVS: u32 = 100;
pub const DEFAULT_CHAT_MESSAGES_PER_MINUTE: u32 = 20;
pub const DEFAULT_CHAT_MUTATION_EVERY_MESSAGES: u64 = 10;
pub const CHAT_USER_PASSWORD: &str = "BenchChatP@ss123";
pub const CHAT_TYPING_INTERVAL: Duration = Duration::from_millis(400);
pub const CHAT_TYPING_BURSTS: u64 = 2;
pub const CHAT_WORKER_START_STAGGER_MS: u64 = 5;
pub const CHAT_LOGIN_MAX_IN_FLIGHT: usize = 64;
pub const CHAT_LOGIN_RETRY_ATTEMPTS: u32 = 5;
pub const CHAT_LOGIN_RETRY_BASE_DELAY: Duration = Duration::from_millis(200);
pub const CHAT_SUBSCRIBE_RETRY_ATTEMPTS: u32 = 8;
pub const CHAT_SUBSCRIBE_RETRY_BASE_DELAY: Duration = Duration::from_millis(200);
pub const SQL_RETRY_ATTEMPTS: u32 = 8;
pub const CHAT_SQL_RETRY_BASE_DELAY: Duration = Duration::from_millis(200);
pub const CHAT_SQL_RETRY_MAX_DELAY: Duration = Duration::from_secs(5);
pub const SUBSCRIBE_TIMEOUT: Duration = Duration::from_secs(30);
pub const SUBSCRIPTION_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(10);
pub const CHAT_FORWARDER_POLL_TIMEOUT: Duration = Duration::from_secs(1);
pub const CHAT_DELIVERY_TIMEOUT_LOG_LIMIT: u64 = 5;
pub const CHAT_MIRROR_WAIT_MIN_TIMEOUT_SECS: u64 = 30;
pub const CHAT_MIRROR_WAIT_MAX_TIMEOUT_SECS: u64 = 300;
pub const CHAT_MEMORY_SAMPLE_INTERVAL: Duration = Duration::from_secs(1);
pub const CHAT_STABILITY_SAMPLE_INTERVAL: Duration = Duration::from_secs(10);
pub const CHAT_STAGE_COUNT: usize = 6;
pub const CHAT_HISTORY_EVERY_CYCLES: u64 = 4;
pub const CHAT_RECONNECT_EVERY_CYCLES: u64 = 6;
pub const CHAT_RECONNECT_GAP: Duration = Duration::from_secs(2);
pub const CHAT_HISTORY_LIMIT: u32 = 50;
pub const AI_AGENT_SENDER: &str = "ai_agent";
pub const AI_INBOX_TOPIC_SUFFIX: &str = "chat_ai_inbox";

#[derive(Clone, Copy)]
pub struct ChatWorkloadSettings {
    pub minutes:                     u64,
    pub user_count:                  u32,
    pub realtime_conversations:      u32,
    pub messages_per_minute:         u32,
    pub mutation_every_messages:     u64,
    pub ai_conversations:            u32,
    pub shared_pair_conversations:   u32,
    pub shared_triple_conversations: u32,
}

impl ChatWorkloadSettings {
    pub fn from_env() -> Result<Self, String> {
        let minutes = parse_u64_env("KALAMDB_BENCH_CHAT_MINUTES", DEFAULT_CHAT_MINUTES)?;
        let user_count = parse_u32_env("KALAMDB_BENCH_CHAT_USERS", DEFAULT_CHAT_USERS)?;
        let realtime_conversations =
            parse_u32_env("KALAMDB_BENCH_CHAT_REALTIME_CONVS", DEFAULT_CHAT_REALTIME_CONVS)?;
        let messages_per_minute = parse_u32_env(
            "KALAMDB_BENCH_CHAT_MESSAGES_PER_MINUTE",
            DEFAULT_CHAT_MESSAGES_PER_MINUTE,
        )?;
        let mutation_every_messages = parse_u64_env(
            "KALAMDB_BENCH_CHAT_MUTATION_EVERY_MESSAGES",
            DEFAULT_CHAT_MUTATION_EVERY_MESSAGES,
        )?;
        let ai_override = parse_optional_u32_env("KALAMDB_BENCH_CHAT_AI_CONVS")?;
        let shared_override = parse_optional_u32_env("KALAMDB_BENCH_CHAT_SHARED_CONVS")?;
        let triple_override = parse_optional_u32_env("KALAMDB_BENCH_CHAT_TRIPLE_CONVS")?;

        if minutes == 0 {
            return Err("KALAMDB_BENCH_CHAT_MINUTES must be greater than zero".to_string());
        }
        if user_count < 2 {
            return Err("KALAMDB_BENCH_CHAT_USERS must be at least 2".to_string());
        }

        let (ai_conversations, shared_total, effective_total) = match (ai_override, shared_override)
        {
            (Some(ai), Some(shared)) => (ai, shared, ai.saturating_add(shared)),
            (Some(ai), None) => {
                if ai > realtime_conversations {
                    return Err("KALAMDB_BENCH_CHAT_AI_CONVS cannot exceed \
                                KALAMDB_BENCH_CHAT_REALTIME_CONVS"
                        .to_string());
                }
                (ai, realtime_conversations - ai, realtime_conversations)
            },
            (None, Some(shared)) => {
                if shared > realtime_conversations {
                    return Err("KALAMDB_BENCH_CHAT_SHARED_CONVS cannot exceed \
                                KALAMDB_BENCH_CHAT_REALTIME_CONVS"
                        .to_string());
                }
                (realtime_conversations - shared, shared, realtime_conversations)
            },
            (None, None) => {
                let ai = realtime_conversations / 2;
                (ai, realtime_conversations - ai, realtime_conversations)
            },
        };

        if effective_total == 0 {
            return Err("chat workload must run at least one conversation".to_string());
        }

        let shared_triple_conversations = match triple_override {
            Some(triple) if triple > shared_total => {
                return Err("KALAMDB_BENCH_CHAT_TRIPLE_CONVS cannot exceed shared conversation \
                            count"
                    .to_string());
            },
            Some(triple) => triple,
            None => shared_total / 5,
        };
        let shared_pair_conversations = shared_total - shared_triple_conversations;

        if shared_triple_conversations > 0 && user_count < 3 {
            return Err("KALAMDB_BENCH_CHAT_USERS must be at least 3 when triple conversations \
                        are enabled"
                .to_string());
        }

        Ok(Self {
            minutes,
            user_count,
            realtime_conversations: effective_total,
            messages_per_minute,
            mutation_every_messages,
            ai_conversations,
            shared_pair_conversations,
            shared_triple_conversations,
        })
    }

    pub fn for_report() -> Self {
        Self::from_env().unwrap_or(Self {
            minutes:                     DEFAULT_CHAT_MINUTES,
            user_count:                  DEFAULT_CHAT_USERS,
            realtime_conversations:      DEFAULT_CHAT_REALTIME_CONVS,
            messages_per_minute:         DEFAULT_CHAT_MESSAGES_PER_MINUTE,
            mutation_every_messages:     DEFAULT_CHAT_MUTATION_EVERY_MESSAGES,
            ai_conversations:            DEFAULT_CHAT_REALTIME_CONVS / 2,
            shared_pair_conversations:   DEFAULT_CHAT_REALTIME_CONVS
                - (DEFAULT_CHAT_REALTIME_CONVS / 2)
                - (DEFAULT_CHAT_REALTIME_CONVS / 2) / 5,
            shared_triple_conversations: (DEFAULT_CHAT_REALTIME_CONVS / 2) / 5,
        })
    }

    pub fn duration(&self) -> Duration {
        Duration::from_secs(self.minutes.saturating_mul(60))
    }

    pub fn target_cycle_interval(&self, messages_per_cycle: u32) -> Option<Duration> {
        if self.messages_per_minute == 0 || messages_per_cycle == 0 {
            return None;
        }

        Some(Duration::from_secs_f64(
            (60.0 * f64::from(messages_per_cycle)) / f64::from(self.messages_per_minute),
        ))
    }

    pub fn message_rate_label(&self) -> String {
        if self.messages_per_minute == 0 {
            "idle subscriptions only".to_string()
        } else {
            format!("{} messages/min per conversation", self.messages_per_minute)
        }
    }

    pub fn mix_label(&self) -> String {
        format!(
            "{} AI USER-table, {} shared pair, {} shared triple",
            self.ai_conversations, self.shared_pair_conversations, self.shared_triple_conversations
        )
    }

    pub fn mutation_label(&self) -> String {
        if self.mutation_every_messages == 0 {
            "message updates/deletes disabled".to_string()
        } else {
            format!("alternate update/delete every {} messages", self.mutation_every_messages)
        }
    }
}

#[derive(Default)]
pub struct ChatWorkloadStats {
    pub sessions_started: AtomicU64,
    pub sessions_completed: AtomicU64,
    pub delivery_wait_timeouts: AtomicU64,
    pub active_sessions: AtomicU64,
    pub peak_active_sessions: AtomicU64,
    pub logged_in_users: AtomicU64,
    pub selects: AtomicU64,
    pub inserts: AtomicU64,
    pub updates: AtomicU64,
    pub deletes: AtomicU64,
    pub select_latency_us: AtomicU64,
    pub insert_latency_us: AtomicU64,
    pub update_latency_us: AtomicU64,
    pub delete_latency_us: AtomicU64,
    pub select_max_us: AtomicU64,
    pub insert_max_us: AtomicU64,
    pub update_max_us: AtomicU64,
    pub delete_max_us: AtomicU64,
    pub messages_sent: AtomicU64,
    pub ai_user_messages_sent: AtomicU64,
    pub ai_replies_sent: AtomicU64,
    pub shared_messages_sent: AtomicU64,
    pub messages_updated: AtomicU64,
    pub messages_deleted: AtomicU64,
    pub typing_events_sent: AtomicU64,
    pub historic_selects: AtomicU64,
    pub conversation_reopens: AtomicU64,
    pub reconnects: AtomicU64,
    pub ai_topic_records: AtomicU64,
    pub skipped_topic_records: AtomicU64,
    pub subscriptions_opened: AtomicU64,
    pub subscriptions_closed: AtomicU64,
    pub active_subscriptions: AtomicU64,
    pub peak_active_subscriptions: AtomicU64,
    pub subscription_open_latency_us: AtomicU64,
    pub subscription_open_max_us: AtomicU64,
    pub subscription_change_rows: AtomicU64,
    pub subscription_payload_bytes: AtomicU64,
    pub subscription_connect_timings: TimingSeries,
    pub create_conversation_timings: TimingSeries,
    pub insert_message_timings: TimingSeries,
    pub update_message_timings: TimingSeries,
    pub delete_message_timings: TimingSeries,
    pub insert_typing_event_timings: TimingSeries,
    pub historic_select_timings: TimingSeries,
    pub reconnect_timings: TimingSeries,
}

pub struct UserClientPool {
    urls:          Arc<Vec<String>>,
    password:      Arc<String>,
    clients:       AsyncMutex<HashMap<String, KalamClient>>,
    login_permits: Arc<Semaphore>,
    stats:         Arc<ChatWorkloadStats>,
}

impl UserClientPool {
    pub fn new(urls: Vec<String>, password: &str, stats: Arc<ChatWorkloadStats>) -> Self {
        Self {
            urls: Arc::new(urls),
            password: Arc::new(password.to_string()),
            clients: AsyncMutex::new(HashMap::new()),
            login_permits: Arc::new(Semaphore::new(CHAT_LOGIN_MAX_IN_FLIGHT)),
            stats,
        }
    }

    pub async fn client_for(&self, username: &str) -> Result<KalamClient, String> {
        {
            let clients = self.clients.lock().await;
            if let Some(client) = clients.get(username) {
                return Ok(client.clone());
            }
        }

        let _login_permit = self
            .login_permits
            .clone()
            .acquire_owned()
            .await
            .map_err(|error| format!("failed to acquire login permit: {}", error))?;

        {
            let clients = self.clients.lock().await;
            if let Some(client) = clients.get(username) {
                return Ok(client.clone());
            }
        }

        let fresh_client =
            login_user_with_retry(self.urls.as_ref(), username, self.password.as_ref())
                .await
                .map_err(|error| format!("login failed for {}: {}", username, error))?;

        let mut clients = self.clients.lock().await;
        if let Some(client) = clients.get(username) {
            return Ok(client.clone());
        }

        clients.insert(username.to_string(), fresh_client.clone());
        self.stats.logged_in_users.fetch_add(1, Ordering::Relaxed);
        Ok(fresh_client)
    }
}

async fn login_user_with_retry(
    urls: &[String],
    username: &str,
    password: &str,
) -> Result<KalamClient, String> {
    let mut delay = CHAT_LOGIN_RETRY_BASE_DELAY;
    let mut attempts = 0_u32;

    loop {
        match KalamClient::login_steady_state(urls, username, password).await {
            Ok(client) => return Ok(client),
            Err(error)
                if attempts < CHAT_LOGIN_RETRY_ATTEMPTS && is_transient_chat_error(&error) =>
            {
                attempts += 1;
                sleep(delay).await;
                delay = (delay * 2).min(Duration::from_secs(5));
            },
            Err(error) => return Err(error),
        }
    }
}

pub struct SessionDeliveryTracker {
    pub self_user:         String,
    pub incoming_messages: AtomicU64,
    pub typing_events:     AtomicU64,
}

impl SessionDeliveryTracker {
    pub fn new(self_user: String) -> Self {
        Self {
            self_user,
            incoming_messages: AtomicU64::new(0),
            typing_events: AtomicU64::new(0),
        }
    }

    pub fn record_rows(&self, rows: &[HashMap<String, KalamCellValue>], kind: SubscriptionKind) {
        match kind {
            SubscriptionKind::Message => {
                for row in rows {
                    let sender = row.get("sender_user").and_then(KalamCellValue::as_text);
                    let is_self =
                        sender.map(|value| value == self.self_user.as_str()).unwrap_or(false);
                    if !is_self {
                        self.incoming_messages.fetch_add(1, Ordering::Relaxed);
                    }
                }
            },
            SubscriptionKind::Typing => {
                self.typing_events.fetch_add(rows.len() as u64, Ordering::Relaxed);
            },
        }
    }
}

pub async fn wait_for_delivery(
    tracker: &SessionDeliveryTracker,
    required_typing_events: u64,
    required_incoming_messages: u64,
    timeout_duration: Duration,
) -> Result<(), String> {
    let deadline = Instant::now() + timeout_duration;

    loop {
        if tracker.typing_events.load(Ordering::Relaxed) >= required_typing_events
            && tracker.incoming_messages.load(Ordering::Relaxed) >= required_incoming_messages
        {
            return Ok(());
        }

        if Instant::now() >= deadline {
            return Err(format!(
                "timed out waiting for live delivery (typing={}, incoming_messages={})",
                tracker.typing_events.load(Ordering::Relaxed),
                tracker.incoming_messages.load(Ordering::Relaxed),
            ));
        }

        sleep(Duration::from_millis(100)).await;
    }
}

pub async fn wait_for_incoming(
    tracker: &SessionDeliveryTracker,
    required_incoming_messages: u64,
    timeout_duration: Duration,
) -> Result<(), String> {
    wait_for_delivery(tracker, 0, required_incoming_messages, timeout_duration).await
}

pub fn build_chat_users(namespace: &str, user_count: u32) -> Vec<String> {
    let prefix = sanitize_identifier_fragment(namespace);
    let mut users = Vec::with_capacity(user_count as usize);
    for ordinal in 1..=user_count {
        users.push(format!("benchchat_{}_u_{:06}", prefix, ordinal));
    }
    users
}

pub fn target_active_chat_user_count(settings: &ChatWorkloadSettings) -> usize {
    let slots = settings.ai_conversations as usize
        + settings.shared_pair_conversations as usize * 2
        + settings.shared_triple_conversations as usize * 3;
    (settings.user_count as usize).min(slots.max(minimum_active_user_floor(settings)))
}

pub fn minimum_active_chat_user_count(settings: &ChatWorkloadSettings) -> usize {
    let target = target_active_chat_user_count(settings);
    (settings.user_count as usize).min((target / 2).max(minimum_active_user_floor(settings)))
}

fn minimum_active_user_floor(settings: &ChatWorkloadSettings) -> usize {
    if settings.shared_triple_conversations > 0 {
        3
    } else {
        2
    }
}

pub struct PrewarmedActiveUsers {
    pub usernames:       Vec<String>,
    pub failed_attempts: u64,
}

pub async fn prewarm_user_clients(
    user_pool: Arc<UserClientPool>,
    users: Arc<Vec<String>>,
    target_active_user_count: usize,
    minimum_active_user_count: usize,
) -> Result<PrewarmedActiveUsers, String> {
    let mut warmed_usernames = Vec::with_capacity(target_active_user_count);
    let mut failure_samples = Vec::new();
    let mut failed_attempts = 0_u64;
    let mut join_set = tokio::task::JoinSet::new();
    let mut user_candidates = users.iter().cloned();
    let mut in_flight = 0_usize;

    while in_flight < CHAT_LOGIN_MAX_IN_FLIGHT {
        let Some(username) = user_candidates.next() else {
            break;
        };
        let pool = user_pool.clone();
        join_set.spawn(async move { pool.client_for(&username).await.map(|_| username) });
        in_flight += 1;
    }

    while in_flight > 0 && warmed_usernames.len() < target_active_user_count {
        let Some(result) = join_set.join_next().await else {
            break;
        };
        in_flight = in_flight.saturating_sub(1);

        match result {
            Ok(Ok(username)) => warmed_usernames.push(username),
            Ok(Err(error)) => {
                failed_attempts += 1;
                if failure_samples.len() < 5 {
                    failure_samples.push(error);
                }
            },
            Err(error) => {
                failed_attempts += 1;
                if failure_samples.len() < 5 {
                    failure_samples.push(format!("active-user prewarm join error: {}", error));
                }
            },
        }

        while in_flight < CHAT_LOGIN_MAX_IN_FLIGHT
            && warmed_usernames.len() + in_flight < target_active_user_count
        {
            let Some(username) = user_candidates.next() else {
                break;
            };
            let pool = user_pool.clone();
            join_set.spawn(async move { pool.client_for(&username).await.map(|_| username) });
            in_flight += 1;
        }
    }

    join_set.abort_all();
    while join_set.join_next().await.is_some() {}

    if warmed_usernames.len() < minimum_active_user_count {
        let mut error = format!(
            "active-user prewarm only warmed {}/{} required users (target={}, \
             failed_login_attempts={})",
            warmed_usernames.len(),
            minimum_active_user_count,
            target_active_user_count,
            failed_attempts,
        );

        for sample in failure_samples.iter().take(5) {
            error.push_str("\n  - ");
            error.push_str(sample);
        }

        return Err(error);
    }

    Ok(PrewarmedActiveUsers {
        usernames: warmed_usernames,
        failed_attempts,
    })
}

pub fn sanitize_identifier_fragment(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for ch in value.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
        } else {
            out.push('_');
        }
    }
    if out.is_empty() {
        "bench".to_string()
    } else {
        out
    }
}

pub fn ai_inbox_topic_name(namespace: &str) -> String {
    format!("{}.{}", namespace, AI_INBOX_TOPIC_SUFFIX)
}

pub async fn wait_for_topic_ready(client: &KalamClient, topic_name: &str) -> Result<(), String> {
    let deadline = Instant::now() + Duration::from_secs(10);
    let sql = format!(
        "SELECT topic_id FROM system.topics WHERE topic_id = {} LIMIT 1",
        sql_literal(topic_name)
    );

    loop {
        let response = run_sql_with_retry(client, &sql).await?;
        if sql_response_has_rows(&response) {
            return Ok(());
        }

        if Instant::now() >= deadline {
            return Err(format!("timed out waiting for topic {} to become ready", topic_name));
        }

        sleep(Duration::from_millis(100)).await;
    }
}

pub fn sql_literal(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

pub fn execute_as_user_sql(user: &str, inner_sql: &str) -> String {
    format!("EXECUTE AS USER {} ({})", sql_literal(user), inner_sql)
}

pub fn json_string_field(row: &JsonValue, key: &str) -> Result<String, String> {
    match row.get(key) {
        Some(JsonValue::String(value)) => Ok(value.clone()),
        Some(other) => Err(format!("expected string field {} but found {}", key, other)),
        None => Err(format!("missing field {}", key)),
    }
}

pub fn json_u64_field(row: &JsonValue, key: &str) -> Result<u64, String> {
    match row.get(key) {
        Some(JsonValue::Number(value)) => {
            value.as_u64().ok_or_else(|| format!("field {} is not a u64", key))
        },
        Some(JsonValue::String(value)) => value
            .parse::<u64>()
            .map_err(|error| format!("field {} is not a valid u64: {}", key, error)),
        Some(other) => Err(format!("expected numeric field {} but found {}", key, other)),
        None => Err(format!("missing field {}", key)),
    }
}

fn sql_response_has_rows(response: &SqlResponse) -> bool {
    response.results.iter().any(|result| {
        result.rows.as_ref().map(|rows| !rows.is_empty()).unwrap_or(false)
            || result.row_count.unwrap_or(0) > 0
    })
}

pub fn format_chat_errors(errors: &[String]) -> String {
    let mut summary = format!("{} error(s)", errors.len());
    for error in errors.iter().take(5) {
        summary.push_str("\n  - ");
        summary.push_str(error);
    }
    summary
}

fn parse_u64_env(name: &str, default: u64) -> Result<u64, String> {
    match std::env::var(name) {
        Ok(value) => value
            .parse::<u64>()
            .map_err(|error| format!("{} must be an integer: {}", name, error)),
        Err(_) => Ok(default),
    }
}

fn parse_u32_env(name: &str, default: u32) -> Result<u32, String> {
    match std::env::var(name) {
        Ok(value) => value
            .parse::<u32>()
            .map_err(|error| format!("{} must be an integer: {}", name, error)),
        Err(_) => Ok(default),
    }
}

fn parse_optional_u32_env(name: &str) -> Result<Option<u32>, String> {
    match std::env::var(name) {
        Ok(value) => value
            .parse::<u32>()
            .map(Some)
            .map_err(|error| format!("{} must be an integer: {}", name, error)),
        Err(_) => Ok(None),
    }
}

#[derive(Clone, Copy)]
pub enum ChatSqlKind {
    Insert,
    Select,
    Update,
    Delete,
}

pub async fn run_tracked_sql(
    client: &KalamClient,
    sql: &str,
    kind: ChatSqlKind,
    stats: &Arc<ChatWorkloadStats>,
) -> Result<SqlResponse, String> {
    let started = Instant::now();
    let result = run_sql_with_retry(client, sql).await;

    if result.is_ok() {
        record_sql_metric(stats, kind, started.elapsed());
    }

    result
}

fn record_sql_metric(stats: &ChatWorkloadStats, kind: ChatSqlKind, elapsed: Duration) {
    let elapsed_us = duration_to_us(elapsed);
    match kind {
        ChatSqlKind::Insert => {
            stats.inserts.fetch_add(1, Ordering::Relaxed);
            stats.insert_latency_us.fetch_add(elapsed_us, Ordering::Relaxed);
            update_peak(&stats.insert_max_us, elapsed_us);
        },
        ChatSqlKind::Select => {
            stats.selects.fetch_add(1, Ordering::Relaxed);
            stats.select_latency_us.fetch_add(elapsed_us, Ordering::Relaxed);
            update_peak(&stats.select_max_us, elapsed_us);
        },
        ChatSqlKind::Update => {
            stats.updates.fetch_add(1, Ordering::Relaxed);
            stats.update_latency_us.fetch_add(elapsed_us, Ordering::Relaxed);
            update_peak(&stats.update_max_us, elapsed_us);
        },
        ChatSqlKind::Delete => {
            stats.deletes.fetch_add(1, Ordering::Relaxed);
            stats.delete_latency_us.fetch_add(elapsed_us, Ordering::Relaxed);
            update_peak(&stats.delete_max_us, elapsed_us);
        },
    }
}

pub fn record_subscription_open(stats: &ChatWorkloadStats, elapsed: Duration) {
    let elapsed_us = duration_to_us(elapsed);
    stats.subscription_open_latency_us.fetch_add(elapsed_us, Ordering::Relaxed);
    update_peak(&stats.subscription_open_max_us, elapsed_us);
    stats.subscription_connect_timings.record(elapsed);
}

#[derive(Default)]
pub struct TimingSeries {
    samples_us: Mutex<Vec<u64>>,
}

impl TimingSeries {
    pub fn record(&self, elapsed: Duration) {
        let elapsed_us = duration_to_us(elapsed);
        let mut samples = lock_unpoisoned(&self.samples_us);
        samples.push(elapsed_us);
    }

    pub fn snapshot(&self) -> TimingStatsSnapshot {
        let mut samples = lock_unpoisoned(&self.samples_us).clone();
        if samples.is_empty() {
            return TimingStatsSnapshot::default();
        }

        samples.sort_unstable();
        let count = samples.len() as u64;
        let total_us = samples.iter().copied().sum::<u64>();

        TimingStatsSnapshot {
            count,
            mean_us: total_us as f64 / count as f64,
            p50_us: percentile_us(&samples, 50.0),
            p90_us: percentile_us(&samples, 90.0),
            p95_us: percentile_us(&samples, 95.0),
            p99_us: percentile_us(&samples, 99.0),
        }
    }
}

#[derive(Default)]
pub struct TimingStatsSnapshot {
    count:   u64,
    mean_us: f64,
    p50_us:  f64,
    p90_us:  f64,
    p95_us:  f64,
    p99_us:  f64,
}

impl TimingStatsSnapshot {
    pub fn display_ms(&self, label: &str) -> String {
        if self.count == 0 {
            return format!("  {}: n/a", label);
        }

        format!(
            "  {}: n={} mean={:.2}ms p50={:.2}ms p90={:.2}ms p95={:.2}ms p99={:.2}ms",
            label,
            self.count,
            self.mean_us / 1000.0,
            self.p50_us / 1000.0,
            self.p90_us / 1000.0,
            self.p95_us / 1000.0,
            self.p99_us / 1000.0,
        )
    }
}

pub struct SessionActivityGuard {
    stats: Arc<ChatWorkloadStats>,
}

impl SessionActivityGuard {
    pub fn new(stats: Arc<ChatWorkloadStats>) -> Self {
        let active_sessions = stats.active_sessions.fetch_add(1, Ordering::Relaxed) + 1;
        update_peak(&stats.peak_active_sessions, active_sessions);
        Self { stats }
    }
}

impl Drop for SessionActivityGuard {
    fn drop(&mut self) {
        self.stats.active_sessions.fetch_sub(1, Ordering::Relaxed);
    }
}

#[derive(Default)]
pub struct ChatManagedServerMemorySummary {
    start_rss: Option<u64>,
    peak_rss:  Option<u64>,
    end_rss:   Option<u64>,
    windows:   Vec<ChatStabilityWindow>,
}

#[derive(Debug, Clone)]
struct ChatStabilityWindow {
    elapsed_secs:          f64,
    duration_secs:         f64,
    sql_ops:               u64,
    sql_latency_us:        u64,
    selects:               u64,
    inserts:               u64,
    updates:               u64,
    deletes:               u64,
    live_changes:          u64,
    historic_selects:      u64,
    reconnects:            u64,
    sql_ops_per_sec:       f64,
    avg_sql_latency_ms:    f64,
    live_changes_per_sec:  f64,
    select_per_sec:        f64,
    insert_per_sec:        f64,
    update_per_sec:        f64,
    delete_per_sec:        f64,
    historic_per_sec:      f64,
    timeout_delta:         u64,
    active_sessions:       u64,
    active_subscriptions:  u64,
    ai_reply_gap:          u64,
    rss_bytes:             Option<u64>,
}

#[derive(Debug, Clone)]
struct ChatRunStage {
    index:      usize,
    start_secs: f64,
    end_secs:   f64,
    window:     ChatStabilityWindow,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MetricTrend {
    Up,
    Flat,
    Down,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StageShape {
    Warmup,
    Rising,
    Stable,
    Improving,
    WindingDown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StabilityVerdict {
    Flat,
    Rising,
    InsufficientData,
}

struct ChatStabilityAssessment {
    verdict:             StabilityVerdict,
    throughput_ratio:    Option<f64>,
    latency_ratio:       Option<f64>,
    latency_end_ratio:   Option<f64>,
    steady_rss_growth:   Option<f64>,
    ai_reply_gap_growth: i64,
}

#[derive(Clone, Copy)]
struct ChatMetricSnapshot {
    selects:              u64,
    inserts:              u64,
    updates:              u64,
    deletes:              u64,
    sql_latency_us:       u64,
    live_changes:         u64,
    delivery_timeouts:    u64,
    historic_selects:     u64,
    reconnects:           u64,
    active_sessions:      u64,
    active_subscriptions: u64,
    ai_user_messages:     u64,
    ai_replies:           u64,
}

impl ChatMetricSnapshot {
    fn capture(stats: &ChatWorkloadStats) -> Self {
        Self {
            selects:              stats.selects.load(Ordering::Relaxed),
            inserts:              stats.inserts.load(Ordering::Relaxed),
            updates:              stats.updates.load(Ordering::Relaxed),
            deletes:              stats.deletes.load(Ordering::Relaxed),
            sql_latency_us:       stats
                .select_latency_us
                .load(Ordering::Relaxed)
                .saturating_add(stats.insert_latency_us.load(Ordering::Relaxed))
                .saturating_add(stats.update_latency_us.load(Ordering::Relaxed))
                .saturating_add(stats.delete_latency_us.load(Ordering::Relaxed)),
            live_changes:         stats.subscription_change_rows.load(Ordering::Relaxed),
            delivery_timeouts:    stats.delivery_wait_timeouts.load(Ordering::Relaxed),
            historic_selects:     stats.historic_selects.load(Ordering::Relaxed),
            reconnects:           stats.reconnects.load(Ordering::Relaxed),
            active_sessions:      stats.active_sessions.load(Ordering::Relaxed),
            active_subscriptions: stats.active_subscriptions.load(Ordering::Relaxed),
            ai_user_messages:     stats.ai_user_messages_sent.load(Ordering::Relaxed),
            ai_replies:           stats.ai_replies_sent.load(Ordering::Relaxed),
        }
    }

    fn sql_ops(self) -> u64 {
        self.selects
            .saturating_add(self.inserts)
            .saturating_add(self.updates)
            .saturating_add(self.deletes)
    }
}

pub struct ChatManagedServerMemoryProbe {
    stop_tx:  Option<watch::Sender<bool>>,
    handle:   Option<JoinHandle<ChatManagedServerMemorySummary>>,
    fallback: ChatManagedServerMemorySummary,
}

impl ChatManagedServerMemoryProbe {
    pub fn start(stats: Arc<ChatWorkloadStats>) -> Self {
        let mut tracker = ManagedServerMemoryTracker::from_env();
        let start_rss = tracker.as_mut().and_then(ManagedServerMemoryTracker::sample_rss_bytes);
        let fallback = ChatManagedServerMemorySummary {
            start_rss,
            peak_rss: start_rss,
            end_rss: start_rss,
            windows: Vec::new(),
        };
        let (stop_tx, mut stop_rx) = watch::channel(false);

        let handle = tokio::spawn(async move {
            let mut peak_rss = start_rss;
            let mut end_rss = start_rss;
            let scenario_started = Instant::now();
            let mut window_started = scenario_started;
            let mut previous_metrics = ChatMetricSnapshot::capture(&stats);
            let mut windows = Vec::new();

            loop {
                tokio::select! {
                    changed = stop_rx.changed() => {
                        if changed.is_ok() || changed.is_err() {
                            if let Some(current_rss) = tracker
                                .as_mut()
                                .and_then(ManagedServerMemoryTracker::sample_rss_bytes)
                            {
                                end_rss = Some(current_rss);
                                peak_rss = max_option_u64(peak_rss, Some(current_rss));
                            }
                            if window_started.elapsed() >= Duration::from_secs(1) {
                                let current_metrics = ChatMetricSnapshot::capture(&stats);
                                windows.push(build_stability_window(
                                    previous_metrics,
                                    current_metrics,
                                    window_started.elapsed(),
                                    scenario_started.elapsed(),
                                    end_rss,
                                ));
                            }
                            return ChatManagedServerMemorySummary {
                                start_rss,
                                peak_rss,
                                end_rss,
                                windows,
                            };
                        }
                    }
                    _ = sleep(CHAT_MEMORY_SAMPLE_INTERVAL) => {
                        if let Some(current_rss) = tracker
                            .as_mut()
                            .and_then(ManagedServerMemoryTracker::sample_rss_bytes)
                        {
                            end_rss = Some(current_rss);
                            peak_rss = max_option_u64(peak_rss, Some(current_rss));
                        }
                        if window_started.elapsed() >= CHAT_STABILITY_SAMPLE_INTERVAL {
                            let current_metrics = ChatMetricSnapshot::capture(&stats);
                            windows.push(build_stability_window(
                                previous_metrics,
                                current_metrics,
                                window_started.elapsed(),
                                scenario_started.elapsed(),
                                end_rss,
                            ));
                            previous_metrics = current_metrics;
                            window_started = Instant::now();
                        }
                    }
                }
            }
        });

        Self {
            stop_tx: Some(stop_tx),
            handle: Some(handle),
            fallback,
        }
    }

    pub async fn finish(mut self) -> ChatManagedServerMemorySummary {
        if let Some(stop_tx) = self.stop_tx.take() {
            let _ = stop_tx.send(true);
        }

        match self.handle.take() {
            Some(handle) => match handle.await {
                Ok(summary) => summary,
                Err(_) => self.fallback,
            },
            None => self.fallback,
        }
    }
}

fn build_stability_window(
    previous: ChatMetricSnapshot,
    current: ChatMetricSnapshot,
    sample_duration: Duration,
    scenario_elapsed: Duration,
    rss_bytes: Option<u64>,
) -> ChatStabilityWindow {
    let duration_secs = sample_duration.as_secs_f64().max(f64::EPSILON);
    let selects = current.selects.saturating_sub(previous.selects);
    let inserts = current.inserts.saturating_sub(previous.inserts);
    let updates = current.updates.saturating_sub(previous.updates);
    let deletes = current.deletes.saturating_sub(previous.deletes);
    let sql_ops = current.sql_ops().saturating_sub(previous.sql_ops());
    let sql_latency_us = current.sql_latency_us.saturating_sub(previous.sql_latency_us);
    let live_changes = current.live_changes.saturating_sub(previous.live_changes);
    let historic_selects = current.historic_selects.saturating_sub(previous.historic_selects);
    let reconnects = current.reconnects.saturating_sub(previous.reconnects);
    ChatStabilityWindow {
        elapsed_secs: scenario_elapsed.as_secs_f64(),
        duration_secs,
        sql_ops,
        sql_latency_us,
        selects,
        inserts,
        updates,
        deletes,
        live_changes,
        historic_selects,
        reconnects,
        sql_ops_per_sec: sql_ops as f64 / duration_secs,
        avg_sql_latency_ms: avg_latency_ms(sql_latency_us, sql_ops),
        live_changes_per_sec: live_changes as f64 / duration_secs,
        select_per_sec: selects as f64 / duration_secs,
        insert_per_sec: inserts as f64 / duration_secs,
        update_per_sec: updates as f64 / duration_secs,
        delete_per_sec: deletes as f64 / duration_secs,
        historic_per_sec: historic_selects as f64 / duration_secs,
        timeout_delta: current.delivery_timeouts.saturating_sub(previous.delivery_timeouts),
        active_sessions: current.active_sessions,
        active_subscriptions: current.active_subscriptions,
        ai_reply_gap: current.ai_user_messages.saturating_sub(current.ai_replies),
        rss_bytes,
    }
}

fn fold_windows_into_stages(
    windows: &[ChatStabilityWindow],
    stage_count: usize,
) -> Vec<ChatRunStage> {
    if windows.is_empty() || stage_count == 0 {
        return Vec::new();
    }

    let mut buckets: Vec<Vec<&ChatStabilityWindow>> = vec![Vec::new(); stage_count];
    let total_elapsed = windows.last().map(|window| window.elapsed_secs).unwrap_or(0.0);
    let use_time = total_elapsed > f64::EPSILON;

    for (index, window) in windows.iter().enumerate() {
        let stage_idx = if use_time {
            let stage_len = (total_elapsed / stage_count as f64).max(f64::EPSILON);
            let sample_mid = if window.duration_secs > f64::EPSILON {
                (window.elapsed_secs - window.duration_secs / 2.0).max(0.0)
            } else {
                window.elapsed_secs
            };
            ((sample_mid / stage_len).floor() as usize).min(stage_count - 1)
        } else {
            (index * stage_count / windows.len()).min(stage_count - 1)
        };
        buckets[stage_idx].push(window);
    }

    buckets
        .into_iter()
        .enumerate()
        .filter(|(_, bucket)| !bucket.is_empty())
        .map(|(idx, bucket)| {
            let (start_secs, end_secs) = if use_time {
                (
                    total_elapsed * idx as f64 / stage_count as f64,
                    total_elapsed * (idx + 1) as f64 / stage_count as f64,
                )
            } else {
                let end = bucket.last().map(|window| window.elapsed_secs).unwrap_or(0.0);
                (0.0, end)
            };
            ChatRunStage {
                index: idx + 1,
                start_secs,
                end_secs,
                window: aggregate_windows(&bucket),
            }
        })
        .collect()
}

fn aggregate_windows(windows: &[&ChatStabilityWindow]) -> ChatStabilityWindow {
    let last = windows.last().copied().expect("stage bucket is not empty");
    let duration_secs: f64 = windows.iter().map(|window| window.duration_secs).sum();
    let sql_ops: u64 = windows.iter().map(|window| window.sql_ops).sum();
    let sql_latency_us: u64 = windows.iter().map(|window| window.sql_latency_us).sum();
    let selects: u64 = windows.iter().map(|window| window.selects).sum();
    let inserts: u64 = windows.iter().map(|window| window.inserts).sum();
    let updates: u64 = windows.iter().map(|window| window.updates).sum();
    let deletes: u64 = windows.iter().map(|window| window.deletes).sum();
    let live_changes: u64 = windows.iter().map(|window| window.live_changes).sum();
    let historic_selects: u64 = windows.iter().map(|window| window.historic_selects).sum();
    let reconnects: u64 = windows.iter().map(|window| window.reconnects).sum();
    let timeout_delta: u64 = windows.iter().map(|window| window.timeout_delta).sum();
    let count = windows.len() as f64;

    if sql_ops > 0 || duration_secs > f64::EPSILON {
        let duration = duration_secs.max(f64::EPSILON);
        ChatStabilityWindow {
            elapsed_secs: last.elapsed_secs,
            duration_secs: duration,
            sql_ops,
            sql_latency_us,
            selects,
            inserts,
            updates,
            deletes,
            live_changes,
            historic_selects,
            reconnects,
            sql_ops_per_sec: sql_ops as f64 / duration,
            avg_sql_latency_ms: avg_latency_ms(sql_latency_us, sql_ops),
            live_changes_per_sec: live_changes as f64 / duration,
            select_per_sec: selects as f64 / duration,
            insert_per_sec: inserts as f64 / duration,
            update_per_sec: updates as f64 / duration,
            delete_per_sec: deletes as f64 / duration,
            historic_per_sec: historic_selects as f64 / duration,
            timeout_delta,
            active_sessions: last.active_sessions,
            active_subscriptions: last.active_subscriptions,
            ai_reply_gap: last.ai_reply_gap,
            rss_bytes: last.rss_bytes,
        }
    } else {
        ChatStabilityWindow {
            elapsed_secs: last.elapsed_secs,
            duration_secs: 0.0,
            sql_ops: 0,
            sql_latency_us: 0,
            selects: 0,
            inserts: 0,
            updates: 0,
            deletes: 0,
            live_changes: 0,
            historic_selects: 0,
            reconnects: 0,
            sql_ops_per_sec: windows.iter().map(|window| window.sql_ops_per_sec).sum::<f64>()
                / count,
            avg_sql_latency_ms: windows
                .iter()
                .map(|window| window.avg_sql_latency_ms)
                .sum::<f64>()
                / count,
            live_changes_per_sec: windows
                .iter()
                .map(|window| window.live_changes_per_sec)
                .sum::<f64>()
                / count,
            select_per_sec: windows.iter().map(|window| window.select_per_sec).sum::<f64>() / count,
            insert_per_sec: windows.iter().map(|window| window.insert_per_sec).sum::<f64>() / count,
            update_per_sec: windows.iter().map(|window| window.update_per_sec).sum::<f64>() / count,
            delete_per_sec: windows.iter().map(|window| window.delete_per_sec).sum::<f64>() / count,
            historic_per_sec: windows.iter().map(|window| window.historic_per_sec).sum::<f64>()
                / count,
            timeout_delta,
            active_sessions: last.active_sessions,
            active_subscriptions: last.active_subscriptions,
            ai_reply_gap: last.ai_reply_gap,
            rss_bytes: last.rss_bytes,
        }
    }
}

fn assess_stability(windows: &[ChatStabilityWindow]) -> ChatStabilityAssessment {
    let stages = fold_windows_into_stages(windows, CHAT_STAGE_COUNT);
    let folded = stages.iter().map(|stage| &stage.window).collect::<Vec<_>>();
    assess_folded_windows(&folded)
}

fn assess_folded_windows(windows: &[&ChatStabilityWindow]) -> ChatStabilityAssessment {
    let peak_active_sessions =
        windows.iter().map(|window| window.active_sessions).max().unwrap_or(0);
    let full_load_floor = peak_active_sessions.saturating_mul(9) / 10;
    let full_load_windows = windows
        .iter()
        .copied()
        .filter(|window| {
            peak_active_sessions == 0 || window.active_sessions >= full_load_floor.max(1)
        })
        .collect::<Vec<_>>();

    if full_load_windows.len() < 4 {
        return ChatStabilityAssessment {
            verdict:             StabilityVerdict::InsufficientData,
            throughput_ratio:    None,
            latency_ratio:       None,
            latency_end_ratio:   None,
            steady_rss_growth:   None,
            ai_reply_gap_growth: 0,
        };
    }

    // Ignore stage 1 / the first folded window: it includes subscription and cache warm-up.
    let steady = &full_load_windows[1..];
    let split = (steady.len() / 2).max(1);
    let (early, late) = steady.split_at(split);
    let throughput_ratio = ratio_of_window_averages(early, late, |window| window.sql_ops_per_sec);
    let latency_ratio = ratio_of_window_averages(early, late, |window| window.avg_sql_latency_ms);
    let latency_end_ratio = steady.first().zip(steady.last()).and_then(|(first, last)| {
        (first.avg_sql_latency_ms > f64::EPSILON)
            .then_some(last.avg_sql_latency_ms / first.avg_sql_latency_ms)
    });
    let steady_rss_growth = late
        .iter()
        .find_map(|window| window.rss_bytes)
        .zip(late.iter().rev().find_map(|window| window.rss_bytes))
        .and_then(|(first, last)| {
            (first > 0).then_some((last as f64 - first as f64) / first as f64)
        });
    let first_gap = steady.first().map(|window| window.ai_reply_gap).unwrap_or(0);
    let last_gap = steady.last().map(|window| window.ai_reply_gap).unwrap_or(first_gap);
    let ai_reply_gap_growth = last_gap as i64 - first_gap as i64;

    let rising = throughput_ratio.is_some_and(|ratio| !(0.65..=1.50).contains(&ratio))
        || (latency_ratio.is_some_and(|ratio| ratio > 1.75)
            && latency_end_ratio.is_some_and(|ratio| ratio > 1.75))
        || steady_rss_growth.is_some_and(|growth| growth > 0.50)
        || ai_reply_gap_growth > 10;

    ChatStabilityAssessment {
        verdict: if rising {
            StabilityVerdict::Rising
        } else {
            StabilityVerdict::Flat
        },
        throughput_ratio,
        latency_ratio,
        latency_end_ratio,
        steady_rss_growth,
        ai_reply_gap_growth,
    }
}

fn ratio_of_window_averages(
    early: &[&ChatStabilityWindow],
    late: &[&ChatStabilityWindow],
    value: impl Fn(&ChatStabilityWindow) -> f64,
) -> Option<f64> {
    if early.is_empty() || late.is_empty() {
        return None;
    }
    let early_average = early.iter().map(|window| value(window)).sum::<f64>() / early.len() as f64;
    let late_average = late.iter().map(|window| value(window)).sum::<f64>() / late.len() as f64;
    (early_average > f64::EPSILON).then_some(late_average / early_average)
}

struct ManagedServerMemoryTracker {
    pid:    Pid,
    system: System,
}

impl ManagedServerMemoryTracker {
    fn from_env() -> Option<Self> {
        if std::env::var("KALAMDB_BENCH_MANAGED_SERVER").ok().as_deref() != Some("1") {
            return None;
        }

        let pid = std::env::var("KALAMDB_BENCH_SERVER_PID")
            .ok()
            .and_then(|value| value.parse::<u32>().ok())?;

        Some(Self {
            pid:    Pid::from_u32(pid),
            system: System::new_with_specifics(
                RefreshKind::nothing().with_memory(MemoryRefreshKind::everything()),
            ),
        })
    }

    fn sample_rss_bytes(&mut self) -> Option<u64> {
        let process_refresh = ProcessRefreshKind::nothing().with_memory();
        self.system.refresh_processes_specifics(
            ProcessesToUpdate::Some(&[self.pid]),
            false,
            process_refresh,
        );
        self.system.process(self.pid).map(|process| process.memory())
    }
}

pub async fn create_subscription(
    link: &KalamLinkClient,
    subscription_id: String,
    sql: String,
    with_initial_data: bool,
) -> Result<SubscriptionManager, String> {
    let mut delay = CHAT_SUBSCRIBE_RETRY_BASE_DELAY;
    let mut attempts = 0_u32;

    loop {
        let config = if with_initial_data {
            SubscriptionConfig::new(subscription_id.clone(), sql.clone())
        } else {
            SubscriptionConfig::without_initial_data(subscription_id.clone(), sql.clone())
        };

        match timeout(SUBSCRIBE_TIMEOUT, link.live_events_with_config(config)).await {
            Ok(Ok(subscription)) => return Ok(subscription),
            Ok(Err(error))
                if attempts < CHAT_SUBSCRIBE_RETRY_ATTEMPTS
                    && is_transient_chat_error(&error.to_string()) =>
            {
                attempts += 1;
                sleep(delay).await;
                delay = (delay * 2).min(Duration::from_secs(5));
            },
            Ok(Err(error)) => return Err(format!("subscribe error: {}", error)),
            Err(_) if attempts < CHAT_SUBSCRIBE_RETRY_ATTEMPTS => {
                attempts += 1;
                sleep(delay).await;
                delay = (delay * 2).min(Duration::from_secs(5));
            },
            Err(_) => return Err("subscription timed out before becoming ready".to_string()),
        }
    }
}

#[derive(Clone, Copy)]
pub enum SubscriptionKind {
    Message,
    Typing,
}

pub fn spawn_subscription_drain(
    mut subscription: SubscriptionManager,
    mut stop_rx: watch::Receiver<bool>,
    stats: Arc<ChatWorkloadStats>,
    tracker: Arc<SessionDeliveryTracker>,
    kind: SubscriptionKind,
) -> JoinHandle<Result<(), String>> {
    tokio::spawn(async move {
        loop {
            tokio::select! {
                changed = stop_rx.changed() => {
                    if changed.is_ok() || changed.is_err() {
                        let close_result = subscription.close().await;
                        stats.subscriptions_closed.fetch_add(1, Ordering::Relaxed);
                        stats.active_subscriptions.fetch_sub(1, Ordering::Relaxed);
                        return close_result.map_err(|error| format!("failed to close subscription: {}", error));
                    }
                }
                event = subscription.next() => {
                    match event {
                        Some(Ok(ChangeEvent::Ack { .. })) => {}
                        Some(Ok(ChangeEvent::InitialDataBatch { rows, .. })) => {
                            stats.subscription_change_rows.fetch_add(rows.len() as u64, Ordering::Relaxed);
                            stats.subscription_payload_bytes.fetch_add(
                                estimate_rows_payload_bytes(&rows),
                                Ordering::Relaxed,
                            );
                            tracker.record_rows(&rows, kind);
                        }
                        Some(Ok(ChangeEvent::Insert { rows, .. })) => {
                            stats.subscription_change_rows.fetch_add(rows.len() as u64, Ordering::Relaxed);
                            stats.subscription_payload_bytes.fetch_add(
                                estimate_rows_payload_bytes(&rows),
                                Ordering::Relaxed,
                            );
                            tracker.record_rows(&rows, kind);
                        }
                        Some(Ok(ChangeEvent::Update { rows, .. })) => {
                            stats.subscription_change_rows.fetch_add(rows.len() as u64, Ordering::Relaxed);
                            stats.subscription_payload_bytes.fetch_add(
                                estimate_rows_payload_bytes(&rows),
                                Ordering::Relaxed,
                            );
                            tracker.record_rows(&rows, kind);
                        }
                        Some(Ok(ChangeEvent::Delete { old_rows, .. })) => {
                            stats
                                .subscription_change_rows
                                .fetch_add(old_rows.len() as u64, Ordering::Relaxed);
                            stats.subscription_payload_bytes.fetch_add(
                                estimate_rows_payload_bytes(&old_rows),
                                Ordering::Relaxed,
                            );
                        }
                        Some(Ok(ChangeEvent::Error { message, .. })) => {
                            let _ = subscription.close().await;
                            stats.subscriptions_closed.fetch_add(1, Ordering::Relaxed);
                            stats.active_subscriptions.fetch_sub(1, Ordering::Relaxed);
                            return Err(format!("subscription server error: {}", message));
                        }
                        Some(Ok(ChangeEvent::Unknown { .. })) => {}
                        Some(Err(error)) => {
                            let _ = subscription.close().await;
                            stats.subscriptions_closed.fetch_add(1, Ordering::Relaxed);
                            stats.active_subscriptions.fetch_sub(1, Ordering::Relaxed);
                            return Err(format!("subscription stream error: {}", error));
                        }
                        None => {
                            stats.subscriptions_closed.fetch_add(1, Ordering::Relaxed);
                            stats.active_subscriptions.fetch_sub(1, Ordering::Relaxed);
                            return Ok(());
                        }
                    }
                }
            }
        }
    })
}

pub struct LiveSession {
    pub stop_tx: watch::Sender<bool>,
    pub handles: Vec<JoinHandle<Result<(), String>>>,
}

impl LiveSession {
    pub fn new(stop_tx: watch::Sender<bool>, handles: Vec<JoinHandle<Result<(), String>>>) -> Self {
        Self { stop_tx, handles }
    }

    pub async fn shutdown(self) -> Result<(), String> {
        let _ = self.stop_tx.send(true);
        shutdown_subscription_tasks(self.handles).await
    }
}

pub async fn shutdown_subscription_tasks(
    handles: Vec<JoinHandle<Result<(), String>>>,
) -> Result<(), String> {
    let mut errors = Vec::new();

    for mut handle in handles {
        match timeout(SUBSCRIPTION_SHUTDOWN_TIMEOUT, &mut handle).await {
            Ok(Ok(Ok(()))) => {},
            Ok(Ok(Err(error))) => errors.push(error),
            Ok(Err(error)) => errors.push(format!("subscription join error: {}", error)),
            Err(_) => {
                handle.abort();
                errors.push("timed out waiting for subscription shutdown".to_string());
            },
        }
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(format_chat_errors(&errors))
    }
}

pub async fn query_conversation_history(
    client: &KalamClient,
    namespace: &str,
    table: &str,
    conversation_id: u64,
    cutoff_ms: u64,
    stats: &Arc<ChatWorkloadStats>,
) -> Result<(), String> {
    let started = Instant::now();
    run_tracked_sql(
        client,
        &format!(
            "SELECT id, sender_user, body, created_at_ms FROM {}.{} WHERE conversation_id = {} \
             AND created_at_ms < {} ORDER BY created_at_ms DESC LIMIT {}",
            namespace, table, conversation_id, cutoff_ms, CHAT_HISTORY_LIMIT
        ),
        ChatSqlKind::Select,
        stats,
    )
    .await?;
    stats.historic_select_timings.record(started.elapsed());
    stats.historic_selects.fetch_add(1, Ordering::Relaxed);
    Ok(())
}

pub async fn reopen_conversation(
    client: &KalamClient,
    namespace: &str,
    conversations_table: &str,
    messages_table: &str,
    conversation_id: u64,
    stats: &Arc<ChatWorkloadStats>,
) -> Result<(), String> {
    run_tracked_sql(
        client,
        &format!(
            "SELECT id, created_at_ms FROM {}.{} WHERE id = {}",
            namespace, conversations_table, conversation_id
        ),
        ChatSqlKind::Select,
        stats,
    )
    .await?;
    run_tracked_sql(
        client,
        &format!(
            "SELECT id, sender_user, body, created_at_ms FROM {}.{} WHERE conversation_id = {} \
             ORDER BY created_at_ms DESC LIMIT {}",
            namespace, messages_table, conversation_id, CHAT_HISTORY_LIMIT
        ),
        ChatSqlKind::Select,
        stats,
    )
    .await?;
    stats.conversation_reopens.fetch_add(1, Ordering::Relaxed);
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageMutation {
    Update,
    Delete,
}

pub fn message_mutation(message_id: u64, every_messages: u64) -> Option<MessageMutation> {
    if every_messages == 0 || !message_id.is_multiple_of(every_messages) {
        return None;
    }

    let mutation_ordinal = message_id / every_messages;
    if mutation_ordinal.is_multiple_of(2) {
        Some(MessageMutation::Delete)
    } else {
        Some(MessageMutation::Update)
    }
}

pub async fn maybe_mutate_message(
    client: &KalamClient,
    namespace: &str,
    table: &str,
    message_id: u64,
    every_messages: u64,
    stats: &Arc<ChatWorkloadStats>,
) -> Result<Option<MessageMutation>, String> {
    let Some(mutation) = message_mutation(message_id, every_messages) else {
        return Ok(None);
    };

    let started = Instant::now();
    match mutation {
        MessageMutation::Update => {
            run_tracked_sql(
                client,
                &format!(
                    "UPDATE {}.{} SET body = {} WHERE id = {}",
                    namespace,
                    table,
                    sql_literal(&format!("edited_message_{}", message_id)),
                    message_id,
                ),
                ChatSqlKind::Update,
                stats,
            )
            .await?;
            stats.update_message_timings.record(started.elapsed());
            stats.messages_updated.fetch_add(1, Ordering::Relaxed);
        },
        MessageMutation::Delete => {
            run_tracked_sql(
                client,
                &format!("DELETE FROM {}.{} WHERE id = {}", namespace, table, message_id),
                ChatSqlKind::Delete,
                stats,
            )
            .await?;
            stats.delete_message_timings.record(started.elapsed());
            stats.messages_deleted.fetch_add(1, Ordering::Relaxed);
        },
    }

    Ok(Some(mutation))
}

pub fn print_chat_summary(
    stats: &Arc<ChatWorkloadStats>,
    settings: ChatWorkloadSettings,
    scenario_elapsed: Duration,
    memory: &ChatManagedServerMemorySummary,
) {
    let subscription_connect_stats = stats.subscription_connect_timings.snapshot();
    let create_conversation_stats = stats.create_conversation_timings.snapshot();
    let insert_message_stats = stats.insert_message_timings.snapshot();
    let update_message_stats = stats.update_message_timings.snapshot();
    let delete_message_stats = stats.delete_message_timings.snapshot();
    let insert_typing_stats = stats.insert_typing_event_timings.snapshot();
    let historic_select_stats = stats.historic_select_timings.snapshot();
    let reconnect_stats = stats.reconnect_timings.snapshot();
    let sessions_started = stats.sessions_started.load(Ordering::Relaxed);
    let sessions_completed = stats.sessions_completed.load(Ordering::Relaxed);
    let delivery_wait_timeouts = stats.delivery_wait_timeouts.load(Ordering::Relaxed);
    let peak_active_sessions = stats.peak_active_sessions.load(Ordering::Relaxed);
    let logged_in_users = stats.logged_in_users.load(Ordering::Relaxed);
    let selects = stats.selects.load(Ordering::Relaxed);
    let inserts = stats.inserts.load(Ordering::Relaxed);
    let updates = stats.updates.load(Ordering::Relaxed);
    let deletes = stats.deletes.load(Ordering::Relaxed);
    let select_latency_us = stats.select_latency_us.load(Ordering::Relaxed);
    let insert_latency_us = stats.insert_latency_us.load(Ordering::Relaxed);
    let update_latency_us = stats.update_latency_us.load(Ordering::Relaxed);
    let delete_latency_us = stats.delete_latency_us.load(Ordering::Relaxed);
    let select_max_us = stats.select_max_us.load(Ordering::Relaxed);
    let insert_max_us = stats.insert_max_us.load(Ordering::Relaxed);
    let update_max_us = stats.update_max_us.load(Ordering::Relaxed);
    let delete_max_us = stats.delete_max_us.load(Ordering::Relaxed);
    let messages_sent = stats.messages_sent.load(Ordering::Relaxed);
    let ai_user_messages_sent = stats.ai_user_messages_sent.load(Ordering::Relaxed);
    let ai_replies_sent = stats.ai_replies_sent.load(Ordering::Relaxed);
    let shared_messages_sent = stats.shared_messages_sent.load(Ordering::Relaxed);
    let messages_updated = stats.messages_updated.load(Ordering::Relaxed);
    let messages_deleted = stats.messages_deleted.load(Ordering::Relaxed);
    let typing_events_sent = stats.typing_events_sent.load(Ordering::Relaxed);
    let historic_selects = stats.historic_selects.load(Ordering::Relaxed);
    let conversation_reopens = stats.conversation_reopens.load(Ordering::Relaxed);
    let reconnects = stats.reconnects.load(Ordering::Relaxed);
    let ai_topic_records = stats.ai_topic_records.load(Ordering::Relaxed);
    let skipped_topic_records = stats.skipped_topic_records.load(Ordering::Relaxed);
    let subscriptions_opened = stats.subscriptions_opened.load(Ordering::Relaxed);
    let subscriptions_closed = stats.subscriptions_closed.load(Ordering::Relaxed);
    let peak_active_subscriptions = stats.peak_active_subscriptions.load(Ordering::Relaxed);
    let subscription_open_latency_us = stats.subscription_open_latency_us.load(Ordering::Relaxed);
    let subscription_open_max_us = stats.subscription_open_max_us.load(Ordering::Relaxed);
    let subscription_change_rows = stats.subscription_change_rows.load(Ordering::Relaxed);
    let subscription_payload_bytes = stats.subscription_payload_bytes.load(Ordering::Relaxed);
    let total_ops = selects + inserts + updates + deletes;
    let duration_secs = scenario_elapsed.as_secs_f64();

    println!(
        "  Chat pressure: elapsed={:.1}s | peak_active_sessions={} | logged_in_users={} | \
         peak_live_subscriptions={}",
        duration_secs, peak_active_sessions, logged_in_users, peak_active_subscriptions,
    );
    println!("  Chat mix: {}", settings.mix_label(),);
    println!("  Message mutations: {}", settings.mutation_label());
    println!(
        "  Chat rates: sessions={:.1}/s | user_messages={:.1}/s | typing={:.1}/s | \
         total_sql={:.1}/s | delivered_live_changes={:.1}/s",
        per_sec(sessions_completed, duration_secs),
        per_sec(messages_sent, duration_secs),
        per_sec(typing_events_sent, duration_secs),
        per_sec(total_ops, duration_secs),
        per_sec(subscription_change_rows, duration_secs),
    );
    println!(
        "  Path split: ai_user_messages={:.1}/s ({}) | ai_replies={:.1}/s ({}) | \
         shared_messages={:.1}/s ({})",
        per_sec(ai_user_messages_sent, duration_secs),
        ai_user_messages_sent,
        per_sec(ai_replies_sent, duration_secs),
        ai_replies_sent,
        per_sec(shared_messages_sent, duration_secs),
        shared_messages_sent,
    );
    println!(
        "  Live payload: total={} | throughput={}",
        format_memory(Some(subscription_payload_bytes)),
        format_bytes_per_sec(per_sec(subscription_payload_bytes, duration_secs)),
    );
    println!(
        "  SQL breakdown: select={:.1}/s avg={:.2}ms max={:.2}ms | insert={:.1}/s avg={:.2}ms \
         max={:.2}ms | update={:.1}/s avg={:.2}ms max={:.2}ms | delete={:.1}/s avg={:.2}ms \
         max={:.2}ms",
        per_sec(selects, duration_secs),
        avg_latency_ms(select_latency_us, selects),
        us_to_ms(select_max_us),
        per_sec(inserts, duration_secs),
        avg_latency_ms(insert_latency_us, inserts),
        us_to_ms(insert_max_us),
        per_sec(updates, duration_secs),
        avg_latency_ms(update_latency_us, updates),
        us_to_ms(update_max_us),
        per_sec(deletes, duration_secs),
        avg_latency_ms(delete_latency_us, deletes),
        us_to_ms(delete_max_us),
    );
    print_stability_summary(memory);
    println!(
        "  App behaviors: historic_reads={:.1}/s ({}) | conversation_reopens={} | \
         reconnect_replays={} | ai_inbox_records={} | skipped_ai_records={}",
        per_sec(historic_selects, duration_secs),
        historic_selects,
        conversation_reopens,
        reconnects,
        ai_topic_records,
        skipped_topic_records,
    );
    println!(
        "  Delivery diagnostics: timed_out_waits={} | ai_reply_gap={} | message_updates={} | \
         message_deletes={}",
        delivery_wait_timeouts,
        ai_user_messages_sent.saturating_sub(ai_replies_sent),
        messages_updated,
        messages_deleted,
    );
    println!(
        "  Session shape: avg_messages/session={:.2} | avg_typing/session={:.2} | \
         subscriptions_opened={} | subscriptions_closed={} | avg_subscribe_open={:.2}ms \
         max={:.2}ms",
        ratio(messages_sent, sessions_completed),
        ratio(typing_events_sent, sessions_completed),
        subscriptions_opened,
        subscriptions_closed,
        avg_latency_ms(subscription_open_latency_us, subscriptions_opened),
        us_to_ms(subscription_open_max_us),
    );
    println!("  Timing percentiles:");
    println!("{}", subscription_connect_stats.display_ms("subscription_connect"));
    println!("{}", create_conversation_stats.display_ms("create_conversation"));
    println!("{}", insert_message_stats.display_ms("insert_message"));
    println!("{}", update_message_stats.display_ms("update_message"));
    println!("{}", delete_message_stats.display_ms("delete_message"));
    println!("{}", insert_typing_stats.display_ms("insert_typing_event"));
    println!("{}", historic_select_stats.display_ms("historic_select"));
    println!("{}", reconnect_stats.display_ms("reconnect_replay"));
    println!(
        "  Managed server RSS: {}",
        format_memory_summary(
            memory,
            logged_in_users.max(u64::from(settings.realtime_conversations)),
            peak_active_subscriptions
        ),
    );
    println!(
        "  Backend note: conversations_ai/messages_ai are USER tables with a topic AI agent; \
         conversations/conversation_members/messages are SHARED tables with CREATE POLICY \
         membership RLS; typing_events is a STREAM table used by the AI agent.",
    );
    println!(
        "  Scenario note: AI sessions send one user message, stream typing, then wait for an \
         assistant reply. Shared sessions rotate 2- or 3-user sends on the same RLS-protected \
         rows, alternately update/delete one in every configured number of messages, occasionally \
         load history before a cutoff, disconnect/reconnect with snapshot replay, and reopen a \
         conversation by SELECT.",
    );
    println!(
        "  Coverage: configured_users={} | sessions_started={} | sessions_completed={}",
        settings.user_count, sessions_started, sessions_completed,
    );
}

fn print_stability_summary(memory: &ChatManagedServerMemorySummary) {
    if memory.windows.is_empty() {
        println!("  Run stages: n/a");
        return;
    }

    let stages = fold_windows_into_stages(&memory.windows, CHAT_STAGE_COUNT);
    let peak_sessions = stages.iter().map(|stage| stage.window.active_sessions).max().unwrap_or(0);
    println!(
        "  Run stages ({} equal slices; stage 1 is warm-up):",
        CHAT_STAGE_COUNT
    );
    for (index, stage) in stages.iter().enumerate() {
        let previous = index.checked_sub(1).and_then(|prev| stages.get(prev));
        print_run_stage(stage, previous.map(|stage| &stage.window), peak_sessions);
    }

    let assessment = assess_stability(&memory.windows);
    let verdict = match assessment.verdict {
        StabilityVerdict::Flat => "flat",
        StabilityVerdict::Rising => "rising/unstable",
        StabilityVerdict::InsufficientData => "insufficient data",
    };
    println!(
        "  Stability assessment: {} | late/early sql={} | late/early avg_sql={} | \
         end/start avg_sql={} | steady_rss_change={} | ai_gap_change={:+}",
        verdict,
        format_optional_ratio(assessment.throughput_ratio),
        format_optional_ratio(assessment.latency_ratio),
        format_optional_ratio(assessment.latency_end_ratio),
        format_optional_percent(assessment.steady_rss_growth),
        assessment.ai_reply_gap_growth,
    );
}

fn print_run_stage(
    stage: &ChatRunStage,
    previous: Option<&ChatStabilityWindow>,
    peak_sessions: u64,
) {
    let window = &stage.window;
    let queries = if window.sql_ops > 0 {
        window.sql_ops.to_string()
    } else {
        "n/a".to_string()
    };
    println!(
        "    {}  {:<9}  queries={:<6} sql={:>5.1}/s  sel={:.1} ins={:.1} upd={:.1} del={:.1}  \
         avg={:.2}ms",
        stage.index,
        format!("{:.0}–{:.0}s", stage.start_secs, stage.end_secs),
        queries,
        window.sql_ops_per_sec,
        window.select_per_sec,
        window.insert_per_sec,
        window.update_per_sec,
        window.delete_per_sec,
        window.avg_sql_latency_ms,
    );
    println!(
        "                 live={:.1}/s  hist={:.1}/s  recon={}  timeouts=+{}  sessions={} \
         subs={}  gap={}  rss={}  | {}",
        window.live_changes_per_sec,
        window.historic_per_sec,
        window.reconnects,
        window.timeout_delta,
        window.active_sessions,
        window.active_subscriptions,
        window.ai_reply_gap,
        format_memory(window.rss_bytes),
        format_stage_vs_previous(stage.index, previous, window, peak_sessions),
    );
}

fn format_stage_vs_previous(
    index: usize,
    previous: Option<&ChatStabilityWindow>,
    current: &ChatStabilityWindow,
    peak_sessions: u64,
) -> String {
    let shape = classify_stage(index, previous, current, peak_sessions);
    let shape_label = match shape {
        StageShape::Warmup => return "warm-up".to_string(),
        StageShape::WindingDown => "winding down",
        StageShape::Rising => "rising",
        StageShape::Stable => "stable",
        StageShape::Improving => "improving",
    };
    let Some(previous) = previous else {
        return shape_label.to_string();
    };
    format!(
        "{}  {}  {}  {}  | {}",
        format_trend_part("sql", previous.sql_ops_per_sec, current.sql_ops_per_sec, 0.15),
        format_trend_part(
            "lat",
            previous.avg_sql_latency_ms,
            current.avg_sql_latency_ms,
            0.15
        ),
        format_trend_part(
            "live",
            previous.live_changes_per_sec,
            current.live_changes_per_sec,
            0.20
        ),
        format_rss_trend(previous.rss_bytes, current.rss_bytes),
        shape_label,
    )
}

fn classify_stage(
    index: usize,
    previous: Option<&ChatStabilityWindow>,
    current: &ChatStabilityWindow,
    peak_sessions: u64,
) -> StageShape {
    if index == 1 {
        return StageShape::Warmup;
    }
    let full_load_floor = peak_sessions.saturating_mul(9) / 10;
    if peak_sessions > 0 && current.active_sessions < full_load_floor.max(1) {
        return StageShape::WindingDown;
    }
    let Some(previous) = previous else {
        return StageShape::Stable;
    };
    let latency = metric_trend(previous.avg_sql_latency_ms, current.avg_sql_latency_ms, 0.15);
    let sql = metric_trend(previous.sql_ops_per_sec, current.sql_ops_per_sec, 0.25);
    let rss = match (previous.rss_bytes, current.rss_bytes) {
        (Some(prev), Some(curr)) => metric_trend(prev as f64, curr as f64, 0.10),
        _ => MetricTrend::Flat,
    };
    if latency == MetricTrend::Up || rss == MetricTrend::Up || sql == MetricTrend::Down {
        return StageShape::Rising;
    }
    if latency == MetricTrend::Down && rss != MetricTrend::Up {
        return StageShape::Improving;
    }
    StageShape::Stable
}

fn metric_trend(previous: f64, current: f64, threshold: f64) -> MetricTrend {
    if previous <= f64::EPSILON {
        return if current > f64::EPSILON {
            MetricTrend::Up
        } else {
            MetricTrend::Flat
        };
    }
    let ratio = current / previous;
    if ratio >= 1.0 + threshold {
        MetricTrend::Up
    } else if ratio <= 1.0 - threshold {
        MetricTrend::Down
    } else {
        MetricTrend::Flat
    }
}

fn format_trend_part(label: &str, previous: f64, current: f64, threshold: f64) -> String {
    match metric_trend(previous, current, threshold) {
        MetricTrend::Flat => format!("{}→", label),
        MetricTrend::Up if previous > f64::EPSILON => {
            format!("{}↑{:.0}%", label, (current / previous - 1.0) * 100.0)
        },
        MetricTrend::Up => format!("{}↑", label),
        MetricTrend::Down if previous > f64::EPSILON => {
            format!("{}↓{:.0}%", label, (1.0 - current / previous) * 100.0)
        },
        MetricTrend::Down => format!("{}↓", label),
    }
}

fn format_rss_trend(previous: Option<u64>, current: Option<u64>) -> String {
    match (previous, current) {
        (Some(prev), Some(curr)) => format_trend_part("rss", prev as f64, curr as f64, 0.10),
        _ => "rss n/a".to_string(),
    }
}

pub fn chat_stability_error(memory: &ChatManagedServerMemorySummary) -> Option<String> {
    let assessment = assess_stability(&memory.windows);
    match assessment.verdict {
        StabilityVerdict::Rising => Some(format!(
            "run-stage metrics are rising, not flat (late/early sql={}, late/early avg_sql={}, \
             end/start avg_sql={}, steady_rss_change={}, ai_gap_change={:+})",
            format_optional_ratio(assessment.throughput_ratio),
            format_optional_ratio(assessment.latency_ratio),
            format_optional_ratio(assessment.latency_end_ratio),
            format_optional_percent(assessment.steady_rss_growth),
            assessment.ai_reply_gap_growth,
        )),
        StabilityVerdict::Flat | StabilityVerdict::InsufficientData => None,
    }
}

pub async fn run_sql_with_retry(client: &KalamClient, sql: &str) -> Result<SqlResponse, String> {
    let mut delay = CHAT_SQL_RETRY_BASE_DELAY;

    for attempt in 0..SQL_RETRY_ATTEMPTS {
        match client.sql_ok(sql).await {
            Ok(response) => return Ok(response),
            Err(error) if attempt + 1 < SQL_RETRY_ATTEMPTS && is_transient_chat_error(&error) => {
                sleep(delay).await;
                delay = (delay * 2).min(CHAT_SQL_RETRY_MAX_DELAY);
            },
            Err(error) => return Err(error),
        }
    }

    Err("sql retry loop exhausted unexpectedly".to_string())
}

pub fn is_transient_chat_error(error: &str) -> bool {
    let lower = error.to_ascii_lowercase();
    lower.contains("timeout")
        || lower.contains("temporarily unavailable")
        || lower.contains("connection failed")
        || lower.contains("connection reset")
        || lower.contains("connection refused")
        || lower.contains("broken pipe")
        || lower.contains("error sending request")
        || lower.contains("transport")
        || lower.contains("too many open files")
        || (lower.contains("rls relation")
            && (lower.contains(" is changing")
                || lower.contains(" changed while authorization was built")))
        || lower.contains("503")
}

fn per_sec(count: u64, duration_secs: f64) -> f64 {
    if duration_secs > 0.0 {
        count as f64 / duration_secs
    } else {
        0.0
    }
}

fn estimate_rows_payload_bytes(rows: &[HashMap<String, KalamCellValue>]) -> u64 {
    rows.iter().map(estimate_row_payload_bytes).sum::<usize>() as u64
}

fn estimate_row_payload_bytes(row: &HashMap<String, KalamCellValue>) -> usize {
    let mut total = 2usize;
    let mut first = true;

    for (key, value) in row {
        if !first {
            total = total.saturating_add(1);
        }
        first = false;

        total = total
            .saturating_add(key.len().saturating_add(2))
            .saturating_add(1)
            .saturating_add(estimate_json_value_bytes(value.inner()));
    }

    total
}

fn estimate_json_value_bytes(value: &JsonValue) -> usize {
    match value {
        JsonValue::Null => 4,
        JsonValue::Bool(true) => 4,
        JsonValue::Bool(false) => 5,
        JsonValue::Number(number) => number.to_string().len(),
        JsonValue::String(text) => text.len().saturating_add(2),
        JsonValue::Array(values) => {
            let mut total = 2usize;
            for (index, entry) in values.iter().enumerate() {
                if index > 0 {
                    total = total.saturating_add(1);
                }
                total = total.saturating_add(estimate_json_value_bytes(entry));
            }
            total
        },
        JsonValue::Object(map) => {
            let mut total = 2usize;
            for (index, (key, entry)) in map.iter().enumerate() {
                if index > 0 {
                    total = total.saturating_add(1);
                }
                total = total
                    .saturating_add(key.len().saturating_add(2))
                    .saturating_add(1)
                    .saturating_add(estimate_json_value_bytes(entry));
            }
            total
        },
    }
}

fn ratio(numerator: u64, denominator: u64) -> f64 {
    if denominator == 0 {
        0.0
    } else {
        numerator as f64 / denominator as f64
    }
}

fn avg_latency_ms(total_us: u64, count: u64) -> f64 {
    if count == 0 {
        0.0
    } else {
        us_to_ms(total_us) / count as f64
    }
}

fn us_to_ms(value_us: u64) -> f64 {
    value_us as f64 / 1000.0
}

fn duration_to_us(elapsed: Duration) -> u64 {
    elapsed.as_micros().min(u64::MAX as u128) as u64
}

fn percentile_us(sorted_samples: &[u64], percentile: f64) -> f64 {
    if sorted_samples.is_empty() {
        return 0.0;
    }

    if sorted_samples.len() == 1 {
        return sorted_samples[0] as f64;
    }

    let idx = (percentile / 100.0) * (sorted_samples.len() - 1) as f64;
    let lower = idx.floor() as usize;
    let upper = idx.ceil() as usize;
    let fraction = idx - lower as f64;

    sorted_samples[lower] as f64 * (1.0 - fraction) + sorted_samples[upper] as f64 * fraction
}

fn lock_unpoisoned<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    match mutex.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

pub fn update_peak(peak: &AtomicU64, candidate: u64) {
    let mut current = peak.load(Ordering::Relaxed);
    while candidate > current {
        match peak.compare_exchange_weak(current, candidate, Ordering::Relaxed, Ordering::Relaxed) {
            Ok(_) => break,
            Err(observed) => current = observed,
        }
    }
}

fn max_option_u64(lhs: Option<u64>, rhs: Option<u64>) -> Option<u64> {
    match (lhs, rhs) {
        (Some(left), Some(right)) => Some(left.max(right)),
        (Some(left), None) => Some(left),
        (None, Some(right)) => Some(right),
        (None, None) => None,
    }
}

fn format_memory_summary(
    memory: &ChatManagedServerMemorySummary,
    active_users: u64,
    peak_active_subscriptions: u64,
) -> String {
    if memory.start_rss.is_none() && memory.peak_rss.is_none() && memory.end_rss.is_none() {
        return "n/a (external server or RSS tracking unavailable)".to_string();
    }

    let peak_delta = memory
        .peak_rss
        .zip(memory.start_rss)
        .map(|(peak, start)| peak.saturating_sub(start));
    let per_user = peak_delta.and_then(|delta| {
        if active_users > 0 {
            Some(delta / active_users)
        } else {
            None
        }
    });
    let per_subscription = peak_delta.and_then(|delta| {
        if peak_active_subscriptions > 0 {
            Some(delta / peak_active_subscriptions)
        } else {
            None
        }
    });

    format!(
        "start={} | peak={} | end={} | peak_delta={} | approx/user={} | approx/peak_sub={}",
        format_memory(memory.start_rss),
        format_memory(memory.peak_rss),
        format_memory(memory.end_rss),
        format_memory(peak_delta),
        format_memory(per_user),
        format_memory(per_subscription),
    )
}

fn format_memory(memory_bytes: Option<u64>) -> String {
    match memory_bytes {
        Some(bytes) if bytes >= 1024 * 1024 * 1024 => {
            format!("{:.2} GiB", bytes as f64 / (1024.0 * 1024.0 * 1024.0))
        },
        Some(bytes) if bytes >= 1024 * 1024 => {
            format!("{:.1} MiB", bytes as f64 / (1024.0 * 1024.0))
        },
        Some(bytes) if bytes >= 1024 => format!("{:.1} KiB", bytes as f64 / 1024.0),
        Some(bytes) => format!("{} B", bytes),
        None => "n/a".to_string(),
    }
}

fn format_optional_ratio(value: Option<f64>) -> String {
    value.map(|ratio| format!("{:.2}x", ratio)).unwrap_or_else(|| "n/a".to_string())
}

fn format_optional_percent(value: Option<f64>) -> String {
    value
        .map(|ratio| format!("{:+.1}%", ratio * 100.0))
        .unwrap_or_else(|| "n/a".to_string())
}

fn format_bytes_per_sec(bytes_per_sec: f64) -> String {
    if bytes_per_sec >= 1024.0 * 1024.0 * 1024.0 {
        format!("{:.2} GiB/s", bytes_per_sec / (1024.0 * 1024.0 * 1024.0))
    } else if bytes_per_sec >= 1024.0 * 1024.0 {
        format!("{:.1} MiB/s", bytes_per_sec / (1024.0 * 1024.0))
    } else if bytes_per_sec >= 1024.0 {
        format!("{:.1} KiB/s", bytes_per_sec / 1024.0)
    } else {
        format!("{:.1} B/s", bytes_per_sec)
    }
}

pub fn epoch_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(u64::MAX as u128) as u64
}

pub struct SimpleRng {
    state: u64,
}

impl SimpleRng {
    pub fn seeded(seed: u64) -> Self {
        Self { state: seed.max(1) }
    }

    pub fn next_u64(&mut self) -> u64 {
        let mut x = self.state;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.state = x;
        x.wrapping_mul(0x2545F4914F6CDD1D)
    }
}

pub fn random_index(rng: &mut SimpleRng, len: usize) -> usize {
    if len <= 1 {
        0
    } else {
        (rng.next_u64() % len as u64) as usize
    }
}

pub fn pick_distinct_users(rng: &mut SimpleRng, users: &[String], count: usize) -> Vec<String> {
    let mut picked = Vec::with_capacity(count);
    if users.is_empty() || count == 0 {
        return picked;
    }

    let mut attempts = 0_usize;
    while picked.len() < count && attempts < count.saturating_mul(8).max(8) {
        attempts += 1;
        let candidate = users[random_index(rng, users.len())].clone();
        if !picked.iter().any(|existing| existing == &candidate) {
            picked.push(candidate);
        }
        if picked.len() == users.len() {
            break;
        }
    }
    picked
}

pub fn chat_worker_start_delay(worker_id: u32) -> Duration {
    Duration::from_millis(u64::from(worker_id) * CHAT_WORKER_START_STAGGER_MS)
}

pub fn chat_delivery_wait_timeout(
    realtime_conversations: u32,
    messages_per_minute: u32,
) -> Duration {
    let conversation_component = (u64::from(realtime_conversations) / 50).max(1);
    let rate_component = (u64::from(messages_per_minute) / 10).max(1);
    let timeout_secs = (conversation_component + rate_component)
        .clamp(CHAT_MIRROR_WAIT_MIN_TIMEOUT_SECS, CHAT_MIRROR_WAIT_MAX_TIMEOUT_SECS);
    Duration::from_secs(timeout_secs)
}

pub fn historic_cutoff_ms(created_at_ms: u64) -> u64 {
    let now = epoch_ms();
    if now <= created_at_ms {
        now
    } else {
        created_at_ms + (now - created_at_ms) / 2
    }
}

pub fn maybe_log_delivery_timeout(
    stats: &ChatWorkloadStats,
    worker_id: u32,
    conversation_id: u64,
    cycle_ordinal: u64,
    trackers: &[Arc<SessionDeliveryTracker>],
    required_typing: u64,
    required_incoming: u64,
) {
    let timeout_count = stats.delivery_wait_timeouts.fetch_add(1, Ordering::Relaxed) + 1;
    if timeout_count > CHAT_DELIVERY_TIMEOUT_LOG_LIMIT {
        return;
    }

    let incoming: Vec<String> = trackers
        .iter()
        .map(|tracker| {
            format!(
                "{}={}/{}",
                tracker.self_user,
                tracker.incoming_messages.load(Ordering::Relaxed),
                required_incoming
            )
        })
        .collect();
    let typing: Vec<String> = trackers
        .iter()
        .map(|tracker| {
            format!(
                "{}={}/{}",
                tracker.self_user,
                tracker.typing_events.load(Ordering::Relaxed),
                required_typing
            )
        })
        .collect();

    println!(
        "  Live delivery timeout sample #{}: worker={} conversation_id={} cycle={} | incoming={} \
         | typing={} | ai_reply_gap={} | active_subscriptions={}",
        timeout_count,
        worker_id,
        conversation_id,
        cycle_ordinal,
        incoming.join(","),
        typing.join(","),
        stats
            .ai_user_messages_sent
            .load(Ordering::Relaxed)
            .saturating_sub(stats.ai_replies_sent.load(Ordering::Relaxed)),
        stats.active_subscriptions.load(Ordering::Relaxed),
    );
}

pub fn should_run_history(cycle_ordinal: u64) -> bool {
    cycle_ordinal > 0 && cycle_ordinal.is_multiple_of(CHAT_HISTORY_EVERY_CYCLES)
}

pub fn should_reconnect(cycle_ordinal: u64) -> bool {
    cycle_ordinal > 0 && cycle_ordinal.is_multiple_of(CHAT_RECONNECT_EVERY_CYCLES)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn treats_membership_binding_races_as_transient() {
        assert!(is_transient_chat_error(
            "Subscription failed (UNSUPPORTED): RLS relation app:conversation_members is \
             changing; authorization fails closed"
        ));
    }

    #[test]
    fn allows_membership_authorization_to_stabilize_during_startup() {
        assert_eq!(CHAT_SUBSCRIBE_RETRY_ATTEMPTS, 8);
    }

    #[test]
    fn mutates_exactly_every_tenth_message_and_alternates_operation() {
        assert_eq!(message_mutation(9, 10), None);
        assert_eq!(message_mutation(10, 10), Some(MessageMutation::Update));
        assert_eq!(message_mutation(19, 10), None);
        assert_eq!(message_mutation(20, 10), Some(MessageMutation::Delete));
        assert_eq!(message_mutation(30, 10), Some(MessageMutation::Update));
    }

    #[test]
    fn zero_mutation_interval_disables_message_mutation() {
        assert_eq!(message_mutation(10, 0), None);
    }

    #[test]
    fn stability_assessment_accepts_flat_steady_state_windows() {
        let windows = vec![
            stability_window(100.0, 5.0, 200 * 1024 * 1024),
            stability_window(101.0, 5.1, 201 * 1024 * 1024),
            stability_window(99.0, 4.9, 202 * 1024 * 1024),
            stability_window(100.0, 5.0, 202 * 1024 * 1024),
            stability_window(102.0, 5.2, 203 * 1024 * 1024),
            stability_window(100.0, 5.0, 203 * 1024 * 1024),
        ];

        assert_eq!(assess_stability(&windows).verdict, StabilityVerdict::Flat);
    }

    #[test]
    fn stability_assessment_flags_exponential_latency_growth() {
        let windows = vec![
            stability_window(100.0, 2.0, 200 * 1024 * 1024),
            stability_window(100.0, 2.0, 201 * 1024 * 1024),
            stability_window(100.0, 4.0, 202 * 1024 * 1024),
            stability_window(100.0, 8.0, 203 * 1024 * 1024),
            stability_window(100.0, 16.0, 204 * 1024 * 1024),
            stability_window(100.0, 32.0, 205 * 1024 * 1024),
        ];

        assert_eq!(assess_stability(&windows).verdict, StabilityVerdict::Rising);
    }

    #[test]
    fn stability_assessment_excludes_partial_shutdown_window() {
        let mut windows = vec![
            stability_window(100.0, 5.0, 200 * 1024 * 1024),
            stability_window(100.0, 5.0, 220 * 1024 * 1024),
            stability_window(100.0, 5.0, 300 * 1024 * 1024),
            stability_window(100.0, 5.0, 303 * 1024 * 1024),
        ];
        for window in &mut windows {
            window.active_sessions = 100;
        }
        let mut shutdown = stability_window(0.0, 0.0, 303 * 1024 * 1024);
        shutdown.active_sessions = 0;
        windows.push(shutdown);

        assert_eq!(assess_stability(&windows).verdict, StabilityVerdict::Flat);
    }

    #[test]
    fn stability_assessment_does_not_call_a_transient_latency_burst_exponential() {
        let mut windows = vec![
            stability_window(300.0, 3.2, 700 * 1024 * 1024),
            stability_window(300.0, 2.3, 710 * 1024 * 1024),
            stability_window(300.0, 6.2, 720 * 1024 * 1024),
            stability_window(300.0, 9.4, 730 * 1024 * 1024),
            stability_window(300.0, 17.2, 740 * 1024 * 1024),
            stability_window(300.0, 3.8, 742 * 1024 * 1024),
        ];
        for window in &mut windows {
            window.active_sessions = 500;
        }

        assert_eq!(assess_stability(&windows).verdict, StabilityVerdict::Flat);
    }

    #[test]
    fn rising_stability_fails_the_chat_run() {
        let mut memory = ChatManagedServerMemorySummary::default();
        memory.windows = vec![
            stability_window(100.0, 2.0, 200 * 1024 * 1024),
            stability_window(100.0, 2.0, 201 * 1024 * 1024),
            stability_window(100.0, 4.0, 202 * 1024 * 1024),
            stability_window(100.0, 8.0, 203 * 1024 * 1024),
            stability_window(100.0, 16.0, 204 * 1024 * 1024),
            stability_window(100.0, 32.0, 205 * 1024 * 1024),
        ];
        for window in &mut memory.windows {
            window.active_sessions = 100;
        }

        let error = chat_stability_error(&memory).expect("rising windows must fail the run");
        assert!(error.contains("rising, not flat"));
        assert!(error.contains("late/early sql="));
    }

    #[test]
    fn flat_stability_does_not_fail_the_chat_run() {
        let mut memory = ChatManagedServerMemorySummary::default();
        memory.windows = vec![
            stability_window(100.0, 5.0, 200 * 1024 * 1024),
            stability_window(101.0, 5.1, 201 * 1024 * 1024),
            stability_window(99.0, 4.9, 202 * 1024 * 1024),
            stability_window(100.0, 5.0, 202 * 1024 * 1024),
            stability_window(102.0, 5.2, 203 * 1024 * 1024),
            stability_window(100.0, 5.0, 203 * 1024 * 1024),
        ];
        for window in &mut memory.windows {
            window.active_sessions = 100;
        }

        assert_eq!(chat_stability_error(&memory), None);
    }

    #[test]
    fn folds_thirty_ten_second_windows_into_six_stages() {
        let mut windows = Vec::new();
        for step in 1_u64..=30 {
            let elapsed = step as f64 * 10.0;
            let inserts = 500 + step * 2;
            let selects = 80;
            let updates = 12;
            let deletes = 12;
            let sql_ops = selects + inserts + updates + deletes;
            let live = if step % 3 == 0 { 4000 } else { 900 };
            windows.push(counted_window(
                elapsed,
                10.0,
                sql_ops,
                selects,
                inserts,
                updates,
                deletes,
                live,
                40,
                25,
                100 + step * 8 * 1024 * 1024,
            ));
        }

        let stages = fold_windows_into_stages(&windows, CHAT_STAGE_COUNT);
        assert_eq!(stages.len(), 6);
        assert_eq!(stages[0].index, 1);
        assert!((stages[0].end_secs - 50.0).abs() < 0.01);
        assert!((stages[5].start_secs - 250.0).abs() < 0.01);
        assert_eq!(stages[0].window.sql_ops, windows[..5].iter().map(|w| w.sql_ops).sum::<u64>());
        assert!(stages[5].window.insert_per_sec > stages[0].window.insert_per_sec);
        assert_eq!(
            classify_stage(2, Some(&stages[0].window), &stages[1].window, 100),
            StageShape::Rising
        );
    }

    #[test]
    fn stage_shape_marks_stable_sql_and_latency_as_stable() {
        let mut previous = stability_window(100.0, 5.0, 200 * 1024 * 1024);
        let mut current = stability_window(102.0, 5.1, 205 * 1024 * 1024);
        previous.active_sessions = 100;
        current.active_sessions = 100;
        assert_eq!(
            classify_stage(2, Some(&previous), &current, 100),
            StageShape::Stable
        );
    }

    fn stability_window(
        sql_ops_per_sec: f64,
        avg_sql_latency_ms: f64,
        rss_bytes: u64,
    ) -> ChatStabilityWindow {
        counted_rates(0.0, sql_ops_per_sec, avg_sql_latency_ms, rss_bytes)
    }

    fn counted_rates(
        elapsed_secs: f64,
        sql_ops_per_sec: f64,
        avg_sql_latency_ms: f64,
        rss_bytes: u64,
    ) -> ChatStabilityWindow {
        ChatStabilityWindow {
            elapsed_secs,
            duration_secs: 0.0,
            sql_ops: 0,
            sql_latency_us: 0,
            selects: 0,
            inserts: 0,
            updates: 0,
            deletes: 0,
            live_changes: 0,
            historic_selects: 0,
            reconnects: 0,
            sql_ops_per_sec,
            avg_sql_latency_ms,
            live_changes_per_sec: 0.0,
            select_per_sec: 0.0,
            insert_per_sec: 0.0,
            update_per_sec: 0.0,
            delete_per_sec: 0.0,
            historic_per_sec: 0.0,
            timeout_delta: 0,
            active_sessions: 0,
            active_subscriptions: 0,
            ai_reply_gap: 0,
            rss_bytes: Some(rss_bytes),
        }
    }

    fn counted_window(
        elapsed_secs: f64,
        duration_secs: f64,
        sql_ops: u64,
        selects: u64,
        inserts: u64,
        updates: u64,
        deletes: u64,
        live_changes: u64,
        historic_selects: u64,
        reconnects: u64,
        rss_bytes: u64,
    ) -> ChatStabilityWindow {
        let duration = duration_secs.max(f64::EPSILON);
        ChatStabilityWindow {
            elapsed_secs,
            duration_secs,
            sql_ops,
            sql_latency_us: sql_ops * 4_000,
            selects,
            inserts,
            updates,
            deletes,
            live_changes,
            historic_selects,
            reconnects,
            sql_ops_per_sec: sql_ops as f64 / duration,
            avg_sql_latency_ms: 4.0,
            live_changes_per_sec: live_changes as f64 / duration,
            select_per_sec: selects as f64 / duration,
            insert_per_sec: inserts as f64 / duration,
            update_per_sec: updates as f64 / duration,
            delete_per_sec: deletes as f64 / duration,
            historic_per_sec: historic_selects as f64 / duration,
            timeout_delta: 0,
            active_sessions: 100,
            active_subscriptions: 210,
            ai_reply_gap: 0,
            rss_bytes: Some(rss_bytes),
        }
    }
}
