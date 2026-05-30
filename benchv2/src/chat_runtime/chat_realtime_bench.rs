use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use kalam_client::{
    AutoOffsetReset, ChangeEvent, KalamCellValue, KalamLinkClient, SubscriptionConfig,
    SubscriptionManager, TopicOp,
};
use serde_json::Value as JsonValue;
use sysinfo::{MemoryRefreshKind, Pid, ProcessRefreshKind, ProcessesToUpdate, RefreshKind, System};
use tokio::sync::{watch, Mutex as AsyncMutex, Semaphore};
use tokio::task::JoinHandle;
use tokio::task::JoinSet;
use tokio::time::{sleep, timeout, Instant};

use crate::benchmarks::Benchmark;
use crate::client::{KalamClient, SqlResponse};
use crate::config::Config;
use crate::metrics::BenchmarkDetail;

pub struct ChatRealtimeBench;

const DEFAULT_CHAT_MINUTES: u64 = 5;
const DEFAULT_CHAT_USERS: u32 = 1_000;
const DEFAULT_CHAT_REALTIME_CONVS: u32 = 100;
const DEFAULT_CHAT_MESSAGES_PER_MINUTE: u32 = 20;
const CHAT_USER_PASSWORD: &str = "BenchChatP@ss123";
const CHAT_MESSAGES_PER_CYCLE: u32 = 2;
const CHAT_TYPING_INTERVAL: Duration = Duration::from_secs(2);
const CHAT_TYPING_BURSTS: u64 = 3;
const CHAT_WORKER_START_STAGGER_MS: u64 = 5;
const CHAT_LOGIN_MAX_IN_FLIGHT: usize = 64;
const CHAT_LOGIN_RETRY_ATTEMPTS: u32 = 5;
const CHAT_LOGIN_RETRY_BASE_DELAY: Duration = Duration::from_millis(200);
const CHAT_SUBSCRIBE_RETRY_ATTEMPTS: u32 = 2;
const CHAT_SUBSCRIBE_RETRY_BASE_DELAY: Duration = Duration::from_millis(200);
const SQL_RETRY_ATTEMPTS: u32 = 8;
const CHAT_SQL_RETRY_BASE_DELAY: Duration = Duration::from_millis(200);
const CHAT_SQL_RETRY_MAX_DELAY: Duration = Duration::from_secs(5);
const SUBSCRIBE_TIMEOUT: Duration = Duration::from_secs(30);
const SUBSCRIPTION_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(10);
const CHAT_FORWARDER_POLL_TIMEOUT: Duration = Duration::from_secs(1);
const CHAT_MESSAGE_FORWARDER_MAX_IN_FLIGHT: usize = 16;
const CHAT_TYPING_FORWARDER_MAX_IN_FLIGHT: usize = 32;
const CHAT_DELIVERY_TIMEOUT_LOG_LIMIT: u64 = 5;
const CHAT_MIRROR_WAIT_MIN_TIMEOUT_SECS: u64 = 15;
const CHAT_MIRROR_WAIT_MAX_TIMEOUT_SECS: u64 = 120;
const CHAT_MEMORY_SAMPLE_INTERVAL: Duration = Duration::from_secs(1);
const CHAT_CONVERSATION_TOPIC_SUFFIX: &str = "chat_conversation_events";
const CHAT_MESSAGE_TOPIC_SUFFIX: &str = "chat_message_events";
const CHAT_TYPING_TOPIC_SUFFIX: &str = "chat_typing_events";

#[derive(Clone, Copy)]
struct ChatWorkloadSettings {
    minutes: u64,
    user_count: u32,
    realtime_conversations: u32,
    messages_per_minute: u32,
}

impl ChatWorkloadSettings {
    fn from_env() -> Result<Self, String> {
        let minutes = parse_u64_env("KALAMDB_BENCH_CHAT_MINUTES", DEFAULT_CHAT_MINUTES)?;
        let user_count = parse_u32_env("KALAMDB_BENCH_CHAT_USERS", DEFAULT_CHAT_USERS)?;
        let realtime_conversations =
            parse_u32_env("KALAMDB_BENCH_CHAT_REALTIME_CONVS", DEFAULT_CHAT_REALTIME_CONVS)?;
        let messages_per_minute = parse_u32_env(
            "KALAMDB_BENCH_CHAT_MESSAGES_PER_MINUTE",
            DEFAULT_CHAT_MESSAGES_PER_MINUTE,
        )?;

        if minutes == 0 {
            return Err("KALAMDB_BENCH_CHAT_MINUTES must be greater than zero".to_string());
        }
        if user_count < 2 {
            return Err("KALAMDB_BENCH_CHAT_USERS must be at least 2".to_string());
        }
        if realtime_conversations == 0 {
            return Err("KALAMDB_BENCH_CHAT_REALTIME_CONVS must be greater than zero".to_string());
        }

        Ok(Self {
            minutes,
            user_count,
            realtime_conversations,
            messages_per_minute,
        })
    }

    fn for_report() -> Self {
        Self::from_env().unwrap_or(Self {
            minutes: DEFAULT_CHAT_MINUTES,
            user_count: DEFAULT_CHAT_USERS,
            realtime_conversations: DEFAULT_CHAT_REALTIME_CONVS,
            messages_per_minute: DEFAULT_CHAT_MESSAGES_PER_MINUTE,
        })
    }

    fn duration(&self) -> Duration {
        Duration::from_secs(self.minutes.saturating_mul(60))
    }

    fn target_cycle_interval(&self) -> Option<Duration> {
        if self.messages_per_minute == 0 {
            return None;
        }

        Some(Duration::from_secs_f64(
            (60.0 * f64::from(CHAT_MESSAGES_PER_CYCLE)) / f64::from(self.messages_per_minute),
        ))
    }

    fn message_rate_label(&self) -> String {
        if self.messages_per_minute == 0 {
            "idle subscriptions only".to_string()
        } else {
            format!("{} messages/min per conversation", self.messages_per_minute)
        }
    }
}

#[derive(Default)]
struct ChatWorkloadStats {
    sessions_started: AtomicU64,
    sessions_completed: AtomicU64,
    delivery_wait_timeouts: AtomicU64,
    active_sessions: AtomicU64,
    peak_active_sessions: AtomicU64,
    logged_in_users: AtomicU64,
    selects: AtomicU64,
    inserts: AtomicU64,
    updates: AtomicU64,
    select_latency_us: AtomicU64,
    insert_latency_us: AtomicU64,
    update_latency_us: AtomicU64,
    select_max_us: AtomicU64,
    insert_max_us: AtomicU64,
    update_max_us: AtomicU64,
    messages_sent: AtomicU64,
    typing_events_sent: AtomicU64,
    conversations_forwarded: AtomicU64,
    messages_forwarded: AtomicU64,
    typing_events_forwarded: AtomicU64,
    conversation_topic_records: AtomicU64,
    message_topic_records: AtomicU64,
    typing_topic_records: AtomicU64,
    skipped_topic_records: AtomicU64,
    subscriptions_opened: AtomicU64,
    subscriptions_closed: AtomicU64,
    active_subscriptions: AtomicU64,
    peak_active_subscriptions: AtomicU64,
    subscription_open_latency_us: AtomicU64,
    subscription_open_max_us: AtomicU64,
    subscription_change_rows: AtomicU64,
    subscription_payload_bytes: AtomicU64,
    subscription_connect_timings: TimingSeries,
    create_conversation_timings: TimingSeries,
    insert_message_timings: TimingSeries,
    insert_typing_event_timings: TimingSeries,
}

impl Benchmark for ChatRealtimeBench {
    fn name(&self) -> &str {
        "chat_realtime"
    }

    fn category(&self) -> &str {
        "Load"
    }

    fn description(&self) -> &str {
        "Timed realtime chat workload with real auth users, USER tables, a Rust topic forwarder, and stream typing events"
    }

    fn report_description(&self, _config: &Config) -> String {
        let settings = ChatWorkloadSettings::for_report();
        format!(
            "Realtime chat scenario for {}m with {} regular users, {} active conversations, and {}",
            settings.minutes,
            settings.user_count,
            settings.realtime_conversations,
            settings.message_rate_label()
        )
    }

    fn report_full_description(&self, _config: &Config) -> String {
        let settings = ChatWorkloadSettings::for_report();
        format!(
            "Creates {} regular KalamDB users, then runs {} concurrent chat-session workers for {} minute(s). Each worker logs in as real users, opens a USER-table conversation row for user A, keeps user A on live SQL subscriptions for message and typing delivery, and paces each conversation at {}. When message sending is enabled, a Rust topic consumer mirrors the user A conversation row into user B's USER table, forwards a user A message to user B, emits three typing events over six seconds from user B via a STREAM table, and forwards a user B reply back to user A.",
            settings.user_count,
            settings.realtime_conversations,
            settings.minutes,
            settings.message_rate_label(),
        )
    }

    fn report_details(&self, _config: &Config) -> Vec<BenchmarkDetail> {
        let settings = ChatWorkloadSettings::for_report();
        vec![
            BenchmarkDetail::new("Runtime", format!("{} minute(s)", settings.minutes)),
            BenchmarkDetail::new("Regular Users", settings.user_count.to_string()),
            BenchmarkDetail::new(
                "Active Conversations",
                settings.realtime_conversations.to_string(),
            ),
            BenchmarkDetail::new(
                "Conversation Message Rate",
                settings.message_rate_label(),
            ),
            BenchmarkDetail::new(
                "Responder Typing Burst",
                format!(
                    "{} events every {}s",
                    CHAT_TYPING_BURSTS,
                    CHAT_TYPING_INTERVAL.as_secs()
                ),
            ),
            BenchmarkDetail::new(
                "Tables",
                "USER conversations, USER messages, STREAM typing_events, TOPIC conversation/message forwarders",
            ),
        ]
    }

    fn single_run(&self) -> bool {
        true
    }

    fn setup<'a>(
        &'a self,
        client: &'a KalamClient,
        config: &'a Config,
    ) -> Pin<Box<dyn Future<Output = Result<(), String>> + Send + 'a>> {
        Box::pin(async move {
            let settings = ChatWorkloadSettings::from_env()?;
            let usernames = build_chat_users(&config.namespace, settings.user_count);

            client
                .sql_ok(&format!("CREATE NAMESPACE IF NOT EXISTS {}", config.namespace))
                .await?;

            let conversation_topic = conversation_topic_name(&config.namespace);
            let message_topic = message_topic_name(&config.namespace);

            let _ = client.sql(&format!("DROP TOPIC IF EXISTS {}", message_topic)).await;
            let _ = client.sql(&format!("DROP TOPIC IF EXISTS {}", conversation_topic)).await;
            let _ = client
                .sql(&format!("DROP TOPIC IF EXISTS {}", typing_topic_name(&config.namespace)))
                .await;
            let _ = client
                .sql(&format!("DROP STREAM TABLE IF EXISTS {}.typing_events", config.namespace))
                .await;
            let _ = client
                .sql(&format!("DROP USER TABLE IF EXISTS {}.messages", config.namespace))
                .await;
            let _ = client
                .sql(&format!("DROP USER TABLE IF EXISTS {}.conversations", config.namespace))
                .await;

            for username in &usernames {
                let _ = client.sql(&format!("DROP USER IF EXISTS {}", sql_literal(username))).await;
            }

            run_sql_with_retry(
                client,
                &format!(
                    "CREATE USER TABLE IF NOT EXISTS {}.conversations (id BIGINT PRIMARY KEY, peer_user TEXT NOT NULL, opened_by TEXT NOT NULL, direction TEXT NOT NULL, needs_forward BOOLEAN NOT NULL, state TEXT NOT NULL, created_at_ms BIGINT NOT NULL) WITH (FLUSH_POLICY = 'rows:10000')",
                    config.namespace
                ),
            )
            .await?;

            run_sql_with_retry(
                client,
                &format!(
                    "CREATE USER TABLE IF NOT EXISTS {}.messages (id BIGINT PRIMARY KEY, conversation_id BIGINT NOT NULL, sender_user TEXT NOT NULL, recipient_user TEXT NOT NULL, direction TEXT NOT NULL, needs_forward BOOLEAN NOT NULL, body TEXT NOT NULL, created_at_ms BIGINT NOT NULL) WITH (FLUSH_POLICY = 'rows:10000')",
                    config.namespace
                ),
            )
            .await?;

            run_sql_with_retry(
                client,
                &format!(
                    "CREATE STREAM TABLE IF NOT EXISTS {}.typing_events (id BIGINT PRIMARY KEY, conversation_id BIGINT NOT NULL, sender_user TEXT NOT NULL, recipient_user TEXT NOT NULL, phase TEXT NOT NULL, needs_forward BOOLEAN NOT NULL, created_at_ms BIGINT NOT NULL) WITH (TTL_SECONDS = 30)",
                    config.namespace
                ),
            )
            .await?;

            run_sql_with_retry(client, &format!("CREATE TOPIC {}", conversation_topic)).await?;
            run_sql_with_retry(
                client,
                &format!(
                    "ALTER TOPIC {} ADD SOURCE {}.conversations ON INSERT",
                    conversation_topic, config.namespace
                ),
            )
            .await?;

            run_sql_with_retry(client, &format!("CREATE TOPIC {}", message_topic)).await?;
            run_sql_with_retry(
                client,
                &format!(
                    "ALTER TOPIC {} ADD SOURCE {}.messages ON INSERT",
                    message_topic, config.namespace
                ),
            )
            .await?;

            let typing_topic = typing_topic_name(&config.namespace);
            run_sql_with_retry(client, &format!("CREATE TOPIC {}", typing_topic)).await?;
            run_sql_with_retry(
                client,
                &format!(
                    "ALTER TOPIC {} ADD SOURCE {}.typing_events ON INSERT",
                    typing_topic, config.namespace
                ),
            )
            .await?;

            wait_for_topic_ready(client, &conversation_topic).await?;
            wait_for_topic_ready(client, &message_topic).await?;
            wait_for_topic_ready(client, &typing_topic).await?;

            for username in &usernames {
                run_sql_with_retry(
                    client,
                    &format!(
                        "CREATE USER {} WITH PASSWORD {} ROLE user",
                        sql_literal(username),
                        sql_literal(CHAT_USER_PASSWORD),
                    ),
                )
                .await?;
            }

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
            let settings = ChatWorkloadSettings::from_env()?;
            let users = Arc::new(build_chat_users(&config.namespace, settings.user_count));
            let target_active_user_count =
                target_active_chat_user_count(users.len(), settings.realtime_conversations);
            let minimum_active_user_count =
                minimum_active_chat_user_count(users.len(), settings.realtime_conversations);
            let stats = Arc::new(ChatWorkloadStats::default());
            let user_pool = Arc::new(UserClientPool::new(
                config.urls.clone(),
                CHAT_USER_PASSWORD,
                stats.clone(),
            ));
            let global_stop = Arc::new(AtomicBool::new(false));
            let memory_probe = ChatManagedServerMemoryProbe::start();
            let forwarders = ChatTopicForwarder::start(
                client.clone(),
                config.namespace.clone(),
                stats.clone(),
                global_stop.clone(),
                iteration,
            );
            let conversation_ids =
                Arc::new(AtomicU64::new(40_000_000_000 + u64::from(iteration) * 1_000_000));
            let message_ids =
                Arc::new(AtomicU64::new(50_000_000_000 + u64::from(iteration) * 10_000_000));
            let typing_ids =
                Arc::new(AtomicU64::new(60_000_000_000 + u64::from(iteration) * 10_000_000));

            let prewarmed_active_users = prewarm_user_clients(
                user_pool.clone(),
                users.clone(),
                target_active_user_count,
                minimum_active_user_count,
            )
            .await?;
            let active_users = Arc::new(prewarmed_active_users.usernames);

            if prewarmed_active_users.failed_attempts > 0 {
                println!(
                    "  Active user prewarm: warmed={} target={} failed_login_attempts={}",
                    active_users.len(),
                    target_active_user_count,
                    prewarmed_active_users.failed_attempts,
                );
            }

            let scenario_started = Instant::now();
            let run_deadline = scenario_started + settings.duration();
            let target_cycle_interval = settings.target_cycle_interval();
            let delivery_timeout = chat_delivery_wait_timeout(settings.realtime_conversations);
            let mut handles = Vec::with_capacity(settings.realtime_conversations as usize);

            println!(
                "  Chat workload settings: duration={}m, regular_users={}, target_active_chat_users={}, active_conversations={}, message_rate={}, typing_burst={}x{}s",
                settings.minutes,
                settings.user_count,
                target_active_user_count,
                settings.realtime_conversations,
                settings.message_rate_label(),
                CHAT_TYPING_BURSTS,
                CHAT_TYPING_INTERVAL.as_secs(),
            );

            for worker_id in 0..settings.realtime_conversations {
                let namespace = config.namespace.clone();
                let worker_stats = stats.clone();
                let worker_pool = user_pool.clone();
                let worker_users = active_users.clone();
                let worker_stop = global_stop.clone();
                let worker_conversations = conversation_ids.clone();
                let worker_messages = message_ids.clone();
                let worker_typing = typing_ids.clone();
                let worker_start_delay = chat_worker_start_delay(worker_id);
                let worker_deadline = run_deadline + worker_start_delay;

                handles.push(tokio::spawn(async move {
                    if !worker_start_delay.is_zero() {
                        sleep(worker_start_delay).await;
                    }

                    run_chat_worker(
                        worker_id,
                        namespace,
                        worker_deadline,
                        worker_stats,
                        worker_pool,
                        worker_users,
                        worker_conversations,
                        worker_messages,
                        worker_typing,
                        target_cycle_interval,
                        delivery_timeout,
                        worker_stop.clone(),
                    )
                    .await
                }));
            }

            let mut errors = Vec::new();
            for handle in handles {
                match handle.await {
                    Ok(Ok(())) => {},
                    Ok(Err(error)) => errors.push(error),
                    Err(error) => errors.push(format!("worker join error: {}", error)),
                }
            }

            global_stop.store(true, Ordering::Relaxed);

            if let Err(error) = forwarders.shutdown().await {
                errors.push(error);
            }

            let memory_summary = memory_probe.finish().await;
            print_chat_summary(&stats, settings, scenario_started.elapsed(), &memory_summary);

            if errors.is_empty() {
                Ok(())
            } else {
                Err(format_chat_errors(&errors))
            }
        })
    }

    fn teardown<'a>(
        &'a self,
        client: &'a KalamClient,
        config: &'a Config,
    ) -> Pin<Box<dyn Future<Output = Result<(), String>> + Send + 'a>> {
        Box::pin(async move {
            let settings = ChatWorkloadSettings::from_env()?;
            let users = build_chat_users(&config.namespace, settings.user_count);

            let _ = client
                .sql(&format!("DROP TOPIC IF EXISTS {}", message_topic_name(&config.namespace)))
                .await;
            let _ = client
                .sql(&format!(
                    "DROP TOPIC IF EXISTS {}",
                    conversation_topic_name(&config.namespace)
                ))
                .await;
            let _ = client
                .sql(&format!("DROP TOPIC IF EXISTS {}", typing_topic_name(&config.namespace)))
                .await;
            let _ = client
                .sql(&format!("DROP STREAM TABLE IF EXISTS {}.typing_events", config.namespace))
                .await;
            let _ = client
                .sql(&format!("DROP USER TABLE IF EXISTS {}.messages", config.namespace))
                .await;
            let _ = client
                .sql(&format!("DROP USER TABLE IF EXISTS {}.conversations", config.namespace))
                .await;

            for username in users {
                let _ =
                    client.sql(&format!("DROP USER IF EXISTS {}", sql_literal(&username))).await;
            }

            Ok(())
        })
    }
}

#[allow(clippy::too_many_arguments)]
async fn run_chat_worker(
    worker_id: u32,
    namespace: String,
    run_deadline: Instant,
    stats: Arc<ChatWorkloadStats>,
    user_pool: Arc<UserClientPool>,
    users: Arc<Vec<String>>,
    conversation_ids: Arc<AtomicU64>,
    message_ids: Arc<AtomicU64>,
    typing_ids: Arc<AtomicU64>,
    target_cycle_interval: Option<Duration>,
    delivery_timeout: Duration,
    global_stop: Arc<AtomicBool>,
) -> Result<(), String> {
    let mut rng = SimpleRng::seeded(0xC0FFEE_u64 + u64::from(worker_id) * 7_919);
    let initiator_index = random_index(&mut rng, users.len());
    let mut responder_index = random_index(&mut rng, users.len());
    while responder_index == initiator_index {
        responder_index = random_index(&mut rng, users.len());
    }

    let initiator_user = users[initiator_index].clone();
    let responder_user = users[responder_index].clone();
    let conversation_id = conversation_ids.fetch_add(1, Ordering::Relaxed);
    let created_at_ms = epoch_ms();
    let _session_guard = SessionActivityGuard::new(stats.clone());

    let initiator_client = user_pool.client_for(&initiator_user).await?;
    let responder_client = user_pool.client_for(&responder_user).await?;
    let delivery_tracker = Arc::new(SessionDeliveryTracker::new(responder_user.clone()));
    let (session_stop_tx, _) = watch::channel(false);
    let subscription_handles = start_initiator_subscriptions(
        &initiator_client.link(),
        &namespace,
        conversation_id,
        &initiator_user,
        &delivery_tracker,
        &session_stop_tx,
        stats.clone(),
    )
    .await?;

    let worker_result = async {
        let create_conversation_started = Instant::now();
        run_tracked_sql(
            &initiator_client,
            &format!(
                "INSERT INTO {}.conversations (id, peer_user, opened_by, direction, needs_forward, state, created_at_ms) VALUES ({}, {}, {}, 'outbound', TRUE, 'open', {})",
                namespace,
                conversation_id,
                sql_literal(&responder_user),
                sql_literal(&initiator_user),
                created_at_ms,
            ),
            ChatSqlKind::Insert,
            &stats,
        )
        .await?;
        stats
            .create_conversation_timings
            .record(create_conversation_started.elapsed());

        if target_cycle_interval.is_none() {
            while Instant::now() < run_deadline && !global_stop.load(Ordering::Relaxed) {
                sleep(Duration::from_secs(1)).await;
            }

            return Ok(());
        }

        let mut cycle_ordinal = 0_u64;
        while Instant::now() < run_deadline && !global_stop.load(Ordering::Relaxed) {
            let cycle_started = Instant::now();
            stats.sessions_started.fetch_add(1, Ordering::Relaxed);
            cycle_ordinal += 1;

            let typing_before = delivery_tracker.typing_events.load(Ordering::Relaxed);
            let peer_messages_before = delivery_tracker.peer_messages.load(Ordering::Relaxed);

            let _first_message_id = emit_message(
                &initiator_client,
                &namespace,
                conversation_id,
                &initiator_user,
                &responder_user,
                &format!("msg_{}_{}_a_{}", conversation_id, worker_id, cycle_ordinal),
                &stats,
                &message_ids,
            )
            .await?;

            let mut emitted_typing_events = 0_u64;
            for _ in 0..CHAT_TYPING_BURSTS {
                emit_typing_event(
                    &responder_client,
                    &namespace,
                    conversation_id,
                    &responder_user,
                    &initiator_user,
                    &typing_ids,
                    &stats,
                )
                .await?;
                emitted_typing_events += 1;

                if global_stop.load(Ordering::Relaxed) {
                    break;
                }

                sleep(CHAT_TYPING_INTERVAL).await;
            }

            let _reply_message_id = emit_message(
                &responder_client,
                &namespace,
                conversation_id,
                &responder_user,
                &initiator_user,
                &format!("msg_{}_{}_b_{}", conversation_id, worker_id, cycle_ordinal),
                &stats,
                &message_ids,
            )
            .await?;

            if let Err(error) = wait_for_delivery(
                &delivery_tracker,
                typing_before + emitted_typing_events,
                peer_messages_before + 1,
                delivery_timeout,
            )
            .await
            {
                let timeout_count =
                    stats.delivery_wait_timeouts.fetch_add(1, Ordering::Relaxed) + 1;
                if timeout_count <= CHAT_DELIVERY_TIMEOUT_LOG_LIMIT {
                    let delivered_typing = delivery_tracker.typing_events.load(Ordering::Relaxed);
                    let delivered_peer_messages =
                        delivery_tracker.peer_messages.load(Ordering::Relaxed);
                    let messages_sent = stats.messages_sent.load(Ordering::Relaxed);
                    let messages_forwarded = stats.messages_forwarded.load(Ordering::Relaxed);
                    let typing_sent = stats.typing_events_sent.load(Ordering::Relaxed);
                    let typing_forwarded = stats.typing_events_forwarded.load(Ordering::Relaxed);
                    println!(
                        "  Live delivery timeout sample #{}: worker={} conversation_id={} cycle={} | typing={}/{} | peer_messages={}/{} | message_forward_gap={} | typing_forward_gap={} | active_subscriptions={}",
                        timeout_count,
                        worker_id,
                        conversation_id,
                        cycle_ordinal,
                        delivered_typing,
                        typing_before + emitted_typing_events,
                        delivered_peer_messages,
                        peer_messages_before + 1,
                        messages_sent.saturating_sub(messages_forwarded),
                        typing_sent.saturating_sub(typing_forwarded),
                        stats.active_subscriptions.load(Ordering::Relaxed),
                    );
                }
                return Err(error);
            }

            stats.sessions_completed.fetch_add(1, Ordering::Relaxed);

            if let Some(target_cycle_interval) = target_cycle_interval {
                let cycle_elapsed = cycle_started.elapsed();
                if cycle_elapsed < target_cycle_interval {
                    let remaining_run = run_deadline.saturating_duration_since(Instant::now());
                    let sleep_for = (target_cycle_interval - cycle_elapsed).min(remaining_run);
                    if !sleep_for.is_zero() {
                        sleep(sleep_for).await;
                    }
                }
            }
        }

        Ok(())
    }
    .await;

    let _ = session_stop_tx.send(true);
    let shutdown_result = shutdown_subscription_tasks(subscription_handles).await;

    match (worker_result, shutdown_result) {
        (Ok(()), Ok(())) => Ok(()),
        (Err(error), _) => Err(error),
        (Ok(()), Err(error)) => Err(error),
    }
}

async fn emit_typing_event(
    client: &KalamClient,
    namespace: &str,
    conversation_id: u64,
    sender_user: &str,
    recipient_user: &str,
    typing_ids: &Arc<AtomicU64>,
    stats: &Arc<ChatWorkloadStats>,
) -> Result<(), String> {
    let event_id = typing_ids.fetch_add(1, Ordering::Relaxed);
    let created_at_ms = epoch_ms();
    let insert_typing_started = Instant::now();

    run_tracked_sql(
        client,
        &format!(
            "INSERT INTO {}.typing_events (id, conversation_id, sender_user, recipient_user, phase, needs_forward, created_at_ms) VALUES ({}, {}, {}, {}, 'typing', TRUE, {})",
            namespace,
            event_id,
            conversation_id,
            sql_literal(sender_user),
            sql_literal(recipient_user),
            created_at_ms,
        ),
        ChatSqlKind::Insert,
        stats,
    )
    .await?;

    stats.insert_typing_event_timings.record(insert_typing_started.elapsed());
    stats.typing_events_sent.fetch_add(1, Ordering::Relaxed);
    Ok(())
}

async fn emit_message(
    client: &KalamClient,
    namespace: &str,
    conversation_id: u64,
    sender_user: &str,
    recipient_user: &str,
    body: &str,
    stats: &Arc<ChatWorkloadStats>,
    message_ids: &Arc<AtomicU64>,
) -> Result<u64, String> {
    let message_id = message_ids.fetch_add(1, Ordering::Relaxed);
    let created_at_ms = epoch_ms();
    let insert_message_started = Instant::now();

    run_tracked_sql(
        client,
        &format!(
            "INSERT INTO {}.messages (id, conversation_id, sender_user, recipient_user, direction, needs_forward, body, created_at_ms) VALUES ({}, {}, {}, {}, 'outbound', TRUE, {}, {})",
            namespace,
            message_id,
            conversation_id,
            sql_literal(sender_user),
            sql_literal(recipient_user),
            sql_literal(body),
            created_at_ms,
        ),
        ChatSqlKind::Insert,
        stats,
    )
    .await?;

    stats.insert_message_timings.record(insert_message_started.elapsed());
    stats.messages_sent.fetch_add(1, Ordering::Relaxed);
    Ok(message_id)
}

async fn start_initiator_subscriptions(
    link: &KalamLinkClient,
    namespace: &str,
    conversation_id: u64,
    initiator_user: &str,
    tracker: &Arc<SessionDeliveryTracker>,
    session_stop_tx: &watch::Sender<bool>,
    stats: Arc<ChatWorkloadStats>,
) -> Result<Vec<JoinHandle<Result<(), String>>>, String> {
    let message_open_started = Instant::now();
    let message_subscription = create_subscription(
        link,
        format!("chat_messages_{}_{}", initiator_user, conversation_id),
        format!(
            "SELECT id, conversation_id, sender_user, recipient_user, direction, body, created_at_ms FROM {}.messages WHERE conversation_id = {}",
            namespace, conversation_id
        ),
    )
    .await?;
    record_subscription_open(&stats, message_open_started.elapsed());

    let typing_open_started = Instant::now();
    let typing_subscription = match create_subscription(
        link,
        format!("chat_typing_{}_{}", initiator_user, conversation_id),
        format!(
            "SELECT id, conversation_id, sender_user, recipient_user, phase, created_at_ms FROM {}.typing_events WHERE conversation_id = {} AND recipient_user = {}",
            namespace,
            conversation_id,
            sql_literal(initiator_user),
        ),
    )
    .await
    {
        Ok(subscription) => subscription,
        Err(error) => {
            let mut message_subscription = message_subscription;
            let _ = message_subscription.close().await;
            return Err(error);
        }
    };
    record_subscription_open(&stats, typing_open_started.elapsed());

    stats.subscriptions_opened.fetch_add(2, Ordering::Relaxed);
    let active_subscriptions = stats.active_subscriptions.fetch_add(2, Ordering::Relaxed) + 2;
    update_peak(&stats.peak_active_subscriptions, active_subscriptions);

    Ok(vec![
        spawn_subscription_drain(
            message_subscription,
            session_stop_tx.subscribe(),
            stats.clone(),
            tracker.clone(),
            SubscriptionKind::Message,
        ),
        spawn_subscription_drain(
            typing_subscription,
            session_stop_tx.subscribe(),
            stats,
            tracker.clone(),
            SubscriptionKind::Typing,
        ),
    ])
}

async fn create_subscription(
    link: &KalamLinkClient,
    subscription_id: String,
    sql: String,
) -> Result<SubscriptionManager, String> {
    let mut delay = CHAT_SUBSCRIBE_RETRY_BASE_DELAY;
    let mut attempts = 0_u32;

    loop {
        let config = SubscriptionConfig::without_initial_data(subscription_id.clone(), sql.clone());

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
enum SubscriptionKind {
    Message,
    Typing,
}

fn spawn_subscription_drain(
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

async fn shutdown_subscription_tasks(
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

struct ChatTopicForwarder {
    handles: Vec<JoinHandle<Result<(), String>>>,
}

impl ChatTopicForwarder {
    fn start(
        admin_client: KalamClient,
        namespace: String,
        stats: Arc<ChatWorkloadStats>,
        global_stop: Arc<AtomicBool>,
        iteration: u32,
    ) -> Self {
        let conversation_handle = tokio::spawn(run_conversation_forwarder(
            admin_client.clone(),
            namespace.clone(),
            stats.clone(),
            global_stop.clone(),
            iteration,
        ));
        let message_handle = tokio::spawn(run_message_forwarder(
            admin_client.clone(),
            namespace.clone(),
            stats.clone(),
            global_stop.clone(),
            iteration,
        ));
        let typing_handle = tokio::spawn(run_typing_forwarder(
            admin_client,
            namespace,
            stats,
            global_stop,
            iteration,
        ));

        Self {
            handles: vec![conversation_handle, message_handle, typing_handle],
        }
    }

    async fn shutdown(self) -> Result<(), String> {
        let mut errors = Vec::new();

        for mut handle in self.handles {
            match timeout(SUBSCRIPTION_SHUTDOWN_TIMEOUT, &mut handle).await {
                Ok(Ok(Ok(()))) => {},
                Ok(Ok(Err(error))) => errors.push(error),
                Ok(Err(error)) => errors.push(format!("forwarder join error: {}", error)),
                Err(_) => {
                    handle.abort();
                    errors.push("timed out waiting for topic forwarder shutdown".to_string());
                },
            }
        }

        if errors.is_empty() {
            Ok(())
        } else {
            Err(format_chat_errors(&errors))
        }
    }
}

async fn run_conversation_forwarder(
    admin_client: KalamClient,
    namespace: String,
    stats: Arc<ChatWorkloadStats>,
    global_stop: Arc<AtomicBool>,
    iteration: u32,
) -> Result<(), String> {
    let topic = conversation_topic_name(&namespace);
    let forwarder_group = format!(
        "chat_rt_conv_forward_{}_{}",
        sanitize_identifier_fragment(&namespace),
        iteration
    );
    let mut consumer = admin_client
        .link()
        .clone()
        .consumer()
        .topic(&topic)
        .group_id(&forwarder_group)
        .auto_offset_reset(AutoOffsetReset::Earliest)
        .max_poll_records(128)
        .poll_timeout(CHAT_FORWARDER_POLL_TIMEOUT)
        .build()
        .map_err(|error| format!("conversation forwarder build: {}", error))?;

    while !global_stop.load(Ordering::Relaxed) {
        let records = consumer
            .poll_with_timeout(CHAT_FORWARDER_POLL_TIMEOUT)
            .await
            .map_err(|error| format!("conversation forwarder poll: {}", error))?;

        if records.is_empty() {
            continue;
        }

        for record in records {
            stats.conversation_topic_records.fetch_add(1, Ordering::Relaxed);

            if record.op != TopicOp::Insert {
                stats.skipped_topic_records.fetch_add(1, Ordering::Relaxed);
                consumer.mark_processed(&record);
                continue;
            }

            let forwarded = match parse_conversation_forward_record(&record.payload) {
                Ok(row) if row.needs_forward => {
                    forward_conversation(&admin_client, &namespace, &row, &stats).await?
                },
                Ok(_) => {
                    stats.skipped_topic_records.fetch_add(1, Ordering::Relaxed);
                    false
                },
                Err(error) => {
                    let _ = consumer.close().await;
                    return Err(format!("conversation topic payload decode: {}", error));
                },
            };

            if forwarded {
                stats.conversations_forwarded.fetch_add(1, Ordering::Relaxed);
            }

            consumer.mark_processed(&record);
        }

        consumer
            .commit_sync()
            .await
            .map_err(|error| format!("conversation forwarder commit: {}", error))?;
    }

    consumer
        .close()
        .await
        .map_err(|error| format!("conversation forwarder close: {}", error))
}

async fn run_message_forwarder(
    admin_client: KalamClient,
    namespace: String,
    stats: Arc<ChatWorkloadStats>,
    global_stop: Arc<AtomicBool>,
    iteration: u32,
) -> Result<(), String> {
    let topic = message_topic_name(&namespace);
    let forwarder_group = format!(
        "chat_rt_msg_forward_{}_{}",
        sanitize_identifier_fragment(&namespace),
        iteration
    );
    let mut consumer = admin_client
        .link()
        .clone()
        .consumer()
        .topic(&topic)
        .group_id(&forwarder_group)
        .auto_offset_reset(AutoOffsetReset::Earliest)
        .max_poll_records(128)
        .poll_timeout(CHAT_FORWARDER_POLL_TIMEOUT)
        .build()
        .map_err(|error| format!("message forwarder build: {}", error))?;

    while !global_stop.load(Ordering::Relaxed) {
        let records = consumer
            .poll_with_timeout(CHAT_FORWARDER_POLL_TIMEOUT)
            .await
            .map_err(|error| format!("message forwarder poll: {}", error))?;

        if records.is_empty() {
            continue;
        }

        let mut records_to_commit = Vec::new();
        let mut rows_to_forward = Vec::new();

        for record in records {
            stats.message_topic_records.fetch_add(1, Ordering::Relaxed);

            if record.op != TopicOp::Insert {
                stats.skipped_topic_records.fetch_add(1, Ordering::Relaxed);
                consumer.mark_processed(&record);
                continue;
            }

            match parse_message_forward_record(&record.payload) {
                Ok(row) if row.needs_forward => {
                    rows_to_forward.push(row);
                    records_to_commit.push(record);
                },
                Ok(_) => {
                    stats.skipped_topic_records.fetch_add(1, Ordering::Relaxed);
                    consumer.mark_processed(&record);
                },
                Err(error) => {
                    let _ = consumer.close().await;
                    return Err(format!("message topic payload decode: {}", error));
                },
            }
        }

        if !rows_to_forward.is_empty() {
            let forwarded_count =
                forward_message_rows(&admin_client, &namespace, rows_to_forward, &stats).await?;
            stats.messages_forwarded.fetch_add(forwarded_count, Ordering::Relaxed);

            for record in &records_to_commit {
                consumer.mark_processed(record);
            }
        }

        consumer
            .commit_sync()
            .await
            .map_err(|error| format!("message forwarder commit: {}", error))?;
    }

    consumer
        .close()
        .await
        .map_err(|error| format!("message forwarder close: {}", error))
}

async fn run_typing_forwarder(
    admin_client: KalamClient,
    namespace: String,
    stats: Arc<ChatWorkloadStats>,
    global_stop: Arc<AtomicBool>,
    iteration: u32,
) -> Result<(), String> {
    let topic = typing_topic_name(&namespace);
    let forwarder_group = format!(
        "chat_rt_typing_forward_{}_{}",
        sanitize_identifier_fragment(&namespace),
        iteration
    );
    let mut consumer = admin_client
        .link()
        .clone()
        .consumer()
        .topic(&topic)
        .group_id(&forwarder_group)
        .auto_offset_reset(AutoOffsetReset::Earliest)
        .max_poll_records(128)
        .poll_timeout(CHAT_FORWARDER_POLL_TIMEOUT)
        .build()
        .map_err(|error| format!("typing forwarder build: {}", error))?;

    while !global_stop.load(Ordering::Relaxed) {
        let records = consumer
            .poll_with_timeout(CHAT_FORWARDER_POLL_TIMEOUT)
            .await
            .map_err(|error| format!("typing forwarder poll: {}", error))?;

        if records.is_empty() {
            continue;
        }

        let mut records_to_commit = Vec::new();
        let mut rows_to_forward = Vec::new();

        for record in records {
            stats.typing_topic_records.fetch_add(1, Ordering::Relaxed);

            if record.op != TopicOp::Insert {
                stats.skipped_topic_records.fetch_add(1, Ordering::Relaxed);
                consumer.mark_processed(&record);
                continue;
            }

            match parse_typing_forward_record(&record.payload) {
                Ok(row) if row.needs_forward => {
                    rows_to_forward.push(row);
                    records_to_commit.push(record);
                },
                Ok(_) => {
                    stats.skipped_topic_records.fetch_add(1, Ordering::Relaxed);
                    consumer.mark_processed(&record);
                },
                Err(error) => {
                    let _ = consumer.close().await;
                    return Err(format!("typing topic payload decode: {}", error));
                },
            }
        }

        if !rows_to_forward.is_empty() {
            let forwarded_count =
                forward_typing_rows(&admin_client, &namespace, rows_to_forward, &stats).await?;
            stats.typing_events_forwarded.fetch_add(forwarded_count, Ordering::Relaxed);

            for record in &records_to_commit {
                consumer.mark_processed(record);
            }
        }

        consumer
            .commit_sync()
            .await
            .map_err(|error| format!("typing forwarder commit: {}", error))?;
    }

    consumer
        .close()
        .await
        .map_err(|error| format!("typing forwarder close: {}", error))
}

#[derive(Debug, Clone)]
struct ConversationForwardRecord {
    id: u64,
    peer_user: String,
    opened_by: String,
    needs_forward: bool,
    state: String,
    created_at_ms: u64,
}

#[derive(Debug, Clone)]
struct MessageForwardRecord {
    id: u64,
    conversation_id: u64,
    sender_user: String,
    recipient_user: String,
    needs_forward: bool,
    body: String,
    created_at_ms: u64,
}

#[derive(Debug, Clone)]
struct TypingForwardRecord {
    id: u64,
    conversation_id: u64,
    sender_user: String,
    recipient_user: String,
    phase: String,
    needs_forward: bool,
    created_at_ms: u64,
}

async fn forward_conversation(
    admin_client: &KalamClient,
    namespace: &str,
    row: &ConversationForwardRecord,
    stats: &Arc<ChatWorkloadStats>,
) -> Result<bool, String> {
    let statement = format!(
        "INSERT INTO {}.conversations (id, peer_user, opened_by, direction, needs_forward, state, created_at_ms) VALUES ({}, {}, {}, 'inbound', FALSE, {}, {})",
        namespace,
        row.id,
        sql_literal(&row.opened_by),
        sql_literal(&row.opened_by),
        sql_literal(&row.state),
        row.created_at_ms,
    );

    match run_tracked_sql(
        admin_client,
        &execute_as_user_sql(&row.peer_user, &statement),
        ChatSqlKind::Insert,
        stats,
    )
    .await
    {
        Ok(_) => Ok(true),
        Err(error) if is_duplicate_chat_error(&error) => Ok(false),
        Err(error) => Err(error),
    }
}

async fn forward_message(
    admin_client: &KalamClient,
    namespace: &str,
    row: &MessageForwardRecord,
    stats: &Arc<ChatWorkloadStats>,
) -> Result<bool, String> {
    let statement = format!(
        "INSERT INTO {}.messages (id, conversation_id, sender_user, recipient_user, direction, needs_forward, body, created_at_ms) VALUES ({}, {}, {}, {}, 'inbound', FALSE, {}, {})",
        namespace,
        row.id,
        row.conversation_id,
        sql_literal(&row.sender_user),
        sql_literal(&row.recipient_user),
        sql_literal(&row.body),
        row.created_at_ms,
    );

    match run_tracked_sql(
        admin_client,
        &execute_as_user_sql(&row.recipient_user, &statement),
        ChatSqlKind::Insert,
        stats,
    )
    .await
    {
        Ok(_) => Ok(true),
        Err(error) if is_duplicate_chat_error(&error) => Ok(false),
        Err(error) => Err(error),
    }
}

async fn forward_message_rows(
    admin_client: &KalamClient,
    namespace: &str,
    rows: Vec<MessageForwardRecord>,
    stats: &Arc<ChatWorkloadStats>,
) -> Result<u64, String> {
    let mut join_set = JoinSet::new();
    let mut rows = rows.into_iter();
    let mut in_flight = 0_usize;
    let mut forwarded_count = 0_u64;

    loop {
        while in_flight < CHAT_MESSAGE_FORWARDER_MAX_IN_FLIGHT {
            let Some(row) = rows.next() else {
                break;
            };

            let forward_client = admin_client.clone();
            let forward_namespace = namespace.to_string();
            let forward_stats = stats.clone();
            join_set.spawn(async move {
                forward_message(&forward_client, &forward_namespace, &row, &forward_stats).await
            });
            in_flight += 1;
        }

        if in_flight == 0 {
            break;
        }

        match join_set.join_next().await {
            Some(Ok(Ok(forwarded))) => {
                if forwarded {
                    forwarded_count += 1;
                }
            },
            Some(Ok(Err(error))) => {
                join_set.abort_all();
                while join_set.join_next().await.is_some() {}
                return Err(error);
            },
            Some(Err(error)) => {
                join_set.abort_all();
                while join_set.join_next().await.is_some() {}
                return Err(format!("message forward task join error: {}", error));
            },
            None => break,
        }

        in_flight = in_flight.saturating_sub(1);
    }

    Ok(forwarded_count)
}

async fn forward_typing_event(
    admin_client: &KalamClient,
    namespace: &str,
    row: &TypingForwardRecord,
    stats: &Arc<ChatWorkloadStats>,
) -> Result<bool, String> {
    let statement = format!(
        "INSERT INTO {}.typing_events (id, conversation_id, sender_user, recipient_user, phase, needs_forward, created_at_ms) VALUES ({}, {}, {}, {}, {}, FALSE, {})",
        namespace,
        row.id,
        row.conversation_id,
        sql_literal(&row.sender_user),
        sql_literal(&row.recipient_user),
        sql_literal(&row.phase),
        row.created_at_ms,
    );

    run_tracked_sql(
        admin_client,
        &execute_as_user_sql(&row.recipient_user, &statement),
        ChatSqlKind::Insert,
        stats,
    )
    .await
    .map(|_| true)
}

async fn forward_typing_rows(
    admin_client: &KalamClient,
    namespace: &str,
    rows: Vec<TypingForwardRecord>,
    stats: &Arc<ChatWorkloadStats>,
) -> Result<u64, String> {
    let mut join_set = JoinSet::new();
    let mut rows = rows.into_iter();
    let mut in_flight = 0_usize;
    let mut forwarded_count = 0_u64;

    loop {
        while in_flight < CHAT_TYPING_FORWARDER_MAX_IN_FLIGHT {
            let Some(row) = rows.next() else {
                break;
            };

            let forward_client = admin_client.clone();
            let forward_namespace = namespace.to_string();
            let forward_stats = stats.clone();
            join_set.spawn(async move {
                forward_typing_event(&forward_client, &forward_namespace, &row, &forward_stats)
                    .await
            });
            in_flight += 1;
        }

        if in_flight == 0 {
            break;
        }

        match join_set.join_next().await {
            Some(Ok(Ok(forwarded))) => {
                if forwarded {
                    forwarded_count += 1;
                }
            },
            Some(Ok(Err(error))) => {
                join_set.abort_all();
                while join_set.join_next().await.is_some() {}
                return Err(error);
            },
            Some(Err(error)) => {
                join_set.abort_all();
                while join_set.join_next().await.is_some() {}
                return Err(format!("typing forward task join error: {}", error));
            },
            None => break,
        }

        in_flight = in_flight.saturating_sub(1);
    }

    Ok(forwarded_count)
}

fn parse_conversation_forward_record(
    payload_bytes: &[u8],
) -> Result<ConversationForwardRecord, String> {
    let payload: JsonValue = serde_json::from_slice(payload_bytes)
        .map_err(|error| format!("invalid topic payload json: {}", error))?;
    let row = payload.get("row").unwrap_or(&payload);

    Ok(ConversationForwardRecord {
        id: json_u64_field(row, "id")?,
        peer_user: json_string_field(row, "peer_user")?,
        opened_by: json_string_field(row, "opened_by")?,
        needs_forward: json_bool_field(row, "needs_forward")?,
        state: json_string_field(row, "state")?,
        created_at_ms: json_u64_field(row, "created_at_ms")?,
    })
}

fn parse_message_forward_record(payload_bytes: &[u8]) -> Result<MessageForwardRecord, String> {
    let payload: JsonValue = serde_json::from_slice(payload_bytes)
        .map_err(|error| format!("invalid topic payload json: {}", error))?;
    let row = payload.get("row").unwrap_or(&payload);

    Ok(MessageForwardRecord {
        id: json_u64_field(row, "id")?,
        conversation_id: json_u64_field(row, "conversation_id")?,
        sender_user: json_string_field(row, "sender_user")?,
        recipient_user: json_string_field(row, "recipient_user")?,
        needs_forward: json_bool_field(row, "needs_forward")?,
        body: json_string_field(row, "body")?,
        created_at_ms: json_u64_field(row, "created_at_ms")?,
    })
}

fn parse_typing_forward_record(payload_bytes: &[u8]) -> Result<TypingForwardRecord, String> {
    let payload: JsonValue = serde_json::from_slice(payload_bytes)
        .map_err(|error| format!("invalid topic payload json: {}", error))?;
    let row = payload.get("row").unwrap_or(&payload);

    Ok(TypingForwardRecord {
        id: json_u64_field(row, "id")?,
        conversation_id: json_u64_field(row, "conversation_id")?,
        sender_user: json_string_field(row, "sender_user")?,
        recipient_user: json_string_field(row, "recipient_user")?,
        phase: json_string_field(row, "phase")?,
        needs_forward: json_bool_field(row, "needs_forward")?,
        created_at_ms: json_u64_field(row, "created_at_ms")?,
    })
}

struct UserClientPool {
    urls: Arc<Vec<String>>,
    password: Arc<String>,
    clients: AsyncMutex<HashMap<String, KalamClient>>,
    login_permits: Arc<Semaphore>,
    stats: Arc<ChatWorkloadStats>,
}

impl UserClientPool {
    fn new(urls: Vec<String>, password: &str, stats: Arc<ChatWorkloadStats>) -> Self {
        Self {
            urls: Arc::new(urls),
            password: Arc::new(password.to_string()),
            clients: AsyncMutex::new(HashMap::new()),
            login_permits: Arc::new(Semaphore::new(CHAT_LOGIN_MAX_IN_FLIGHT)),
            stats,
        }
    }

    async fn client_for(&self, username: &str) -> Result<KalamClient, String> {
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

struct SessionDeliveryTracker {
    peer_sender: String,
    typing_events: AtomicU64,
    peer_messages: AtomicU64,
}

impl SessionDeliveryTracker {
    fn new(peer_sender: String) -> Self {
        Self {
            peer_sender,
            typing_events: AtomicU64::new(0),
            peer_messages: AtomicU64::new(0),
        }
    }

    fn record_rows(&self, rows: &[HashMap<String, KalamCellValue>], kind: SubscriptionKind) {
        match kind {
            SubscriptionKind::Message => {
                for row in rows {
                    if row
                        .get("sender_user")
                        .and_then(KalamCellValue::as_text)
                        .map(|value| value == self.peer_sender.as_str())
                        .unwrap_or(false)
                    {
                        self.peer_messages.fetch_add(1, Ordering::Relaxed);
                    }
                }
            },
            SubscriptionKind::Typing => {
                self.typing_events.fetch_add(rows.len() as u64, Ordering::Relaxed);
            },
        }
    }
}

async fn wait_for_delivery(
    tracker: &SessionDeliveryTracker,
    required_typing_events: u64,
    required_peer_messages: u64,
    timeout_duration: Duration,
) -> Result<(), String> {
    let deadline = Instant::now() + timeout_duration;

    loop {
        if tracker.typing_events.load(Ordering::Relaxed) >= required_typing_events
            && tracker.peer_messages.load(Ordering::Relaxed) >= required_peer_messages
        {
            return Ok(());
        }

        if Instant::now() >= deadline {
            return Err(format!(
                "timed out waiting for live delivery (typing={}, peer_messages={})",
                tracker.typing_events.load(Ordering::Relaxed),
                tracker.peer_messages.load(Ordering::Relaxed),
            ));
        }

        sleep(Duration::from_millis(100)).await;
    }
}

fn build_chat_users(namespace: &str, user_count: u32) -> Vec<String> {
    let prefix = sanitize_identifier_fragment(namespace);
    let mut users = Vec::with_capacity(user_count as usize);
    for ordinal in 1..=user_count {
        users.push(format!("benchchat_{}_u_{:06}", prefix, ordinal));
    }
    users
}

fn target_active_chat_user_count(total_user_count: usize, realtime_conversations: u32) -> usize {
    total_user_count.min((realtime_conversations as usize).max(2))
}

fn minimum_active_chat_user_count(total_user_count: usize, realtime_conversations: u32) -> usize {
    total_user_count.min(((realtime_conversations as usize) / 2).max(2))
}

struct PrewarmedActiveUsers {
    usernames: Vec<String>,
    failed_attempts: u64,
}

async fn prewarm_user_clients(
    user_pool: Arc<UserClientPool>,
    users: Arc<Vec<String>>,
    target_active_user_count: usize,
    minimum_active_user_count: usize,
) -> Result<PrewarmedActiveUsers, String> {
    let mut warmed_usernames = Vec::with_capacity(target_active_user_count);
    let mut failure_samples = Vec::new();
    let mut failed_attempts = 0_u64;
    let mut join_set = JoinSet::new();
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
            "active-user prewarm only warmed {}/{} required users (target={}, failed_login_attempts={})",
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

fn sanitize_identifier_fragment(value: &str) -> String {
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

fn conversation_topic_name(namespace: &str) -> String {
    format!("{}.{}", namespace, CHAT_CONVERSATION_TOPIC_SUFFIX)
}

fn message_topic_name(namespace: &str) -> String {
    format!("{}.{}", namespace, CHAT_MESSAGE_TOPIC_SUFFIX)
}

fn typing_topic_name(namespace: &str) -> String {
    format!("{}.{}", namespace, CHAT_TYPING_TOPIC_SUFFIX)
}

async fn wait_for_topic_ready(client: &KalamClient, topic_name: &str) -> Result<(), String> {
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

fn sql_literal(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

fn execute_as_user_sql(user: &str, inner_sql: &str) -> String {
    format!("EXECUTE AS USER {} ({})", sql_literal(user), inner_sql)
}

fn json_string_field(row: &JsonValue, key: &str) -> Result<String, String> {
    match row.get(key) {
        Some(JsonValue::String(value)) => Ok(value.clone()),
        Some(other) => Err(format!("expected string field {} but found {}", key, other)),
        None => Err(format!("missing field {}", key)),
    }
}

fn json_u64_field(row: &JsonValue, key: &str) -> Result<u64, String> {
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

fn json_bool_field(row: &JsonValue, key: &str) -> Result<bool, String> {
    match row.get(key) {
        Some(JsonValue::Bool(value)) => Ok(*value),
        Some(JsonValue::String(value)) => value
            .parse::<bool>()
            .map_err(|error| format!("field {} is not a valid bool: {}", key, error)),
        Some(other) => Err(format!("expected bool field {} but found {}", key, other)),
        None => Err(format!("missing field {}", key)),
    }
}

fn sql_response_has_rows(response: &SqlResponse) -> bool {
    response.results.iter().any(|result| {
        result.rows.as_ref().map(|rows| !rows.is_empty()).unwrap_or(false)
            || result.row_count.unwrap_or(0) > 0
    })
}

fn format_chat_errors(errors: &[String]) -> String {
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

#[derive(Clone, Copy)]
enum ChatSqlKind {
    Insert,
}

async fn run_tracked_sql(
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
    }
}

fn record_subscription_open(stats: &ChatWorkloadStats, elapsed: Duration) {
    let elapsed_us = duration_to_us(elapsed);
    stats.subscription_open_latency_us.fetch_add(elapsed_us, Ordering::Relaxed);
    update_peak(&stats.subscription_open_max_us, elapsed_us);
    stats.subscription_connect_timings.record(elapsed);
}

#[derive(Default)]
struct TimingSeries {
    samples_us: Mutex<Vec<u64>>,
}

impl TimingSeries {
    fn record(&self, elapsed: Duration) {
        let elapsed_us = duration_to_us(elapsed);
        let mut samples = lock_unpoisoned(&self.samples_us);
        samples.push(elapsed_us);
    }

    fn snapshot(&self) -> TimingStatsSnapshot {
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
struct TimingStatsSnapshot {
    count: u64,
    mean_us: f64,
    p50_us: f64,
    p90_us: f64,
    p95_us: f64,
    p99_us: f64,
}

impl TimingStatsSnapshot {
    fn display_ms(&self, label: &str) -> String {
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

struct SessionActivityGuard {
    stats: Arc<ChatWorkloadStats>,
}

impl SessionActivityGuard {
    fn new(stats: Arc<ChatWorkloadStats>) -> Self {
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
struct ChatManagedServerMemorySummary {
    start_rss: Option<u64>,
    peak_rss: Option<u64>,
    end_rss: Option<u64>,
}

struct ChatManagedServerMemoryProbe {
    stop_tx: Option<watch::Sender<bool>>,
    handle: Option<JoinHandle<ChatManagedServerMemorySummary>>,
    fallback: ChatManagedServerMemorySummary,
}

impl ChatManagedServerMemoryProbe {
    fn start() -> Self {
        let Some(mut tracker) = ManagedServerMemoryTracker::from_env() else {
            return Self {
                stop_tx: None,
                handle: None,
                fallback: ChatManagedServerMemorySummary::default(),
            };
        };

        let start_rss = tracker.sample_rss_bytes();
        let fallback = ChatManagedServerMemorySummary {
            start_rss,
            peak_rss: start_rss,
            end_rss: start_rss,
        };
        let (stop_tx, mut stop_rx) = watch::channel(false);

        let handle = tokio::spawn(async move {
            let mut peak_rss = start_rss;
            let mut end_rss = start_rss;

            loop {
                tokio::select! {
                    changed = stop_rx.changed() => {
                        if changed.is_ok() || changed.is_err() {
                            if let Some(current_rss) = tracker.sample_rss_bytes() {
                                end_rss = Some(current_rss);
                                peak_rss = max_option_u64(peak_rss, Some(current_rss));
                            }
                            return ChatManagedServerMemorySummary {
                                start_rss,
                                peak_rss,
                                end_rss,
                            };
                        }
                    }
                    _ = sleep(CHAT_MEMORY_SAMPLE_INTERVAL) => {
                        if let Some(current_rss) = tracker.sample_rss_bytes() {
                            end_rss = Some(current_rss);
                            peak_rss = max_option_u64(peak_rss, Some(current_rss));
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

    async fn finish(mut self) -> ChatManagedServerMemorySummary {
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

struct ManagedServerMemoryTracker {
    pid: Pid,
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
            pid: Pid::from_u32(pid),
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

fn print_chat_summary(
    stats: &Arc<ChatWorkloadStats>,
    settings: ChatWorkloadSettings,
    scenario_elapsed: Duration,
    memory: &ChatManagedServerMemorySummary,
) {
    let subscription_connect_stats = stats.subscription_connect_timings.snapshot();
    let create_conversation_stats = stats.create_conversation_timings.snapshot();
    let insert_message_stats = stats.insert_message_timings.snapshot();
    let insert_typing_stats = stats.insert_typing_event_timings.snapshot();
    let sessions_started = stats.sessions_started.load(Ordering::Relaxed);
    let sessions_completed = stats.sessions_completed.load(Ordering::Relaxed);
    let delivery_wait_timeouts = stats.delivery_wait_timeouts.load(Ordering::Relaxed);
    let peak_active_sessions = stats.peak_active_sessions.load(Ordering::Relaxed);
    let logged_in_users = stats.logged_in_users.load(Ordering::Relaxed);
    let selects = stats.selects.load(Ordering::Relaxed);
    let inserts = stats.inserts.load(Ordering::Relaxed);
    let updates = stats.updates.load(Ordering::Relaxed);
    let select_latency_us = stats.select_latency_us.load(Ordering::Relaxed);
    let insert_latency_us = stats.insert_latency_us.load(Ordering::Relaxed);
    let update_latency_us = stats.update_latency_us.load(Ordering::Relaxed);
    let select_max_us = stats.select_max_us.load(Ordering::Relaxed);
    let insert_max_us = stats.insert_max_us.load(Ordering::Relaxed);
    let update_max_us = stats.update_max_us.load(Ordering::Relaxed);
    let messages_sent = stats.messages_sent.load(Ordering::Relaxed);
    let typing_events_sent = stats.typing_events_sent.load(Ordering::Relaxed);
    let conversations_forwarded = stats.conversations_forwarded.load(Ordering::Relaxed);
    let messages_forwarded = stats.messages_forwarded.load(Ordering::Relaxed);
    let typing_events_forwarded = stats.typing_events_forwarded.load(Ordering::Relaxed);
    let conversation_topic_records = stats.conversation_topic_records.load(Ordering::Relaxed);
    let message_topic_records = stats.message_topic_records.load(Ordering::Relaxed);
    let typing_topic_records = stats.typing_topic_records.load(Ordering::Relaxed);
    let skipped_topic_records = stats.skipped_topic_records.load(Ordering::Relaxed);
    let subscriptions_opened = stats.subscriptions_opened.load(Ordering::Relaxed);
    let subscriptions_closed = stats.subscriptions_closed.load(Ordering::Relaxed);
    let peak_active_subscriptions = stats.peak_active_subscriptions.load(Ordering::Relaxed);
    let subscription_open_latency_us = stats.subscription_open_latency_us.load(Ordering::Relaxed);
    let subscription_open_max_us = stats.subscription_open_max_us.load(Ordering::Relaxed);
    let subscription_change_rows = stats.subscription_change_rows.load(Ordering::Relaxed);
    let subscription_payload_bytes = stats.subscription_payload_bytes.load(Ordering::Relaxed);
    let total_ops = selects + inserts + updates;
    let duration_secs = scenario_elapsed.as_secs_f64();

    println!(
        "  Chat pressure: elapsed={:.1}s | peak_active_sessions={} | logged_in_users={} | peak_live_subscriptions={}",
        duration_secs,
        peak_active_sessions,
        logged_in_users,
        peak_active_subscriptions,
    );
    println!(
        "  Chat rates: sessions={:.1}/s | user_messages={:.1}/s | typing={:.1}/s | total_sql={:.1}/s | delivered_live_changes={:.1}/s",
        per_sec(sessions_completed, duration_secs),
        per_sec(messages_sent, duration_secs),
        per_sec(typing_events_sent, duration_secs),
        per_sec(total_ops, duration_secs),
        per_sec(subscription_change_rows, duration_secs),
    );
    println!(
        "  Live payload: total={} | throughput={}",
        format_memory(Some(subscription_payload_bytes)),
        format_bytes_per_sec(per_sec(subscription_payload_bytes, duration_secs)),
    );
    println!(
        "  SQL breakdown: select={:.1}/s avg={:.2}ms max={:.2}ms | insert={:.1}/s avg={:.2}ms max={:.2}ms | update={:.1}/s avg={:.2}ms max={:.2}ms",
        per_sec(selects, duration_secs),
        avg_latency_ms(select_latency_us, selects),
        us_to_ms(select_max_us),
        per_sec(inserts, duration_secs),
        avg_latency_ms(insert_latency_us, inserts),
        us_to_ms(insert_max_us),
        per_sec(updates, duration_secs),
        avg_latency_ms(update_latency_us, updates),
        us_to_ms(update_max_us),
    );
    println!(
        "  Forwarder: mirrored_conversations={:.1}/s ({}) | mirrored_messages={:.1}/s ({}) | mirrored_typing={:.1}/s ({}) | conversation_topic_records={} | message_topic_records={} | typing_topic_records={} | skipped_topic_records={}",
        per_sec(conversations_forwarded, duration_secs),
        conversations_forwarded,
        per_sec(messages_forwarded, duration_secs),
        messages_forwarded,
        per_sec(typing_events_forwarded, duration_secs),
        typing_events_forwarded,
        conversation_topic_records,
        message_topic_records,
        typing_topic_records,
        skipped_topic_records,
    );
    println!(
        "  Delivery diagnostics: timed_out_waits={} | message_forward_gap={} | typing_forward_gap={}",
        delivery_wait_timeouts,
        messages_sent.saturating_sub(messages_forwarded),
        typing_events_sent.saturating_sub(typing_events_forwarded),
    );
    println!(
        "  Session shape: avg_messages/session={:.2} | avg_typing/session={:.2} | subscriptions_opened={} | subscriptions_closed={} | avg_subscribe_open={:.2}ms max={:.2}ms",
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
    println!("{}", insert_typing_stats.display_ms("insert_typing_event"));
    println!(
        "  Managed server RSS: {}",
        format_memory_summary(
            memory,
            logged_in_users.max(u64::from(settings.realtime_conversations)),
            peak_active_subscriptions
        ),
    );
    println!(
        "  Backend note: conversations and messages are USER tables, typing_events is a STREAM table, and Rust topic consumers mirror conversation, message, and typing INSERTs back into recipient user scopes with EXECUTE AS USER.",
    );
    println!(
        "  Scenario note: each session creates one conversation, sends one initiator message, emits {} typing events from the responder, and forwards one responder reply back to the subscribed initiator.",
        CHAT_TYPING_BURSTS,
    );
    println!(
        "  Coverage: configured_users={} | sessions_started={} | sessions_completed={}",
        settings.user_count, sessions_started, sessions_completed,
    );
}

async fn run_sql_with_retry(client: &KalamClient, sql: &str) -> Result<SqlResponse, String> {
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

fn is_transient_chat_error(error: &str) -> bool {
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
        || lower.contains("503")
}

fn is_duplicate_chat_error(error: &str) -> bool {
    let lower = error.to_ascii_lowercase();
    lower.contains("duplicate")
        || lower.contains("already exists")
        || lower.contains("primary key")
        || lower.contains("unique")
}

fn per_sec(count: u64, duration_secs: f64) -> f64 {
    if duration_secs > 0.0 {
        count as f64 / duration_secs
    } else {
        0.0
    }
}

fn estimate_rows_payload_bytes(rows: &[HashMap<String, KalamCellValue>]) -> u64 {
    rows.iter()
        .map(estimate_row_payload_bytes)
        .sum::<usize>() as u64
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

fn update_peak(peak: &AtomicU64, candidate: u64) {
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

fn epoch_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(u64::MAX as u128) as u64
}

struct SimpleRng {
    state: u64,
}

impl SimpleRng {
    fn seeded(seed: u64) -> Self {
        Self { state: seed.max(1) }
    }

    fn next_u64(&mut self) -> u64 {
        let mut x = self.state;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.state = x;
        x.wrapping_mul(0x2545F4914F6CDD1D)
    }
}

fn random_index(rng: &mut SimpleRng, len: usize) -> usize {
    if len <= 1 {
        0
    } else {
        (rng.next_u64() % len as u64) as usize
    }
}

fn chat_worker_start_delay(worker_id: u32) -> Duration {
    Duration::from_millis(u64::from(worker_id) * CHAT_WORKER_START_STAGGER_MS)
}

fn chat_delivery_wait_timeout(realtime_conversations: u32) -> Duration {
    let timeout_secs = (u64::from(realtime_conversations) / 100)
        .clamp(CHAT_MIRROR_WAIT_MIN_TIMEOUT_SECS, CHAT_MIRROR_WAIT_MAX_TIMEOUT_SECS);
    Duration::from_secs(timeout_secs)
}
