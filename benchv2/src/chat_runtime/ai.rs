use std::{
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc,
    },
    time::Duration,
};

use kalam_client::{AutoOffsetReset, KalamLinkClient, TopicOp};
use serde_json::Value as JsonValue;
use tokio::{
    sync::watch,
    task::{JoinHandle, JoinSet},
    time::{sleep, timeout, Instant},
};

use crate::{
    chat_runtime::common::{
        ai_inbox_topic_name, chat_delivery_wait_timeout, chat_worker_start_delay,
        create_subscription, epoch_ms, execute_as_user_sql, historic_cutoff_ms, json_string_field,
        json_u64_field, maybe_log_delivery_timeout, maybe_mutate_message, pick_distinct_users,
        query_conversation_history, record_subscription_open, reopen_conversation, run_tracked_sql,
        sanitize_identifier_fragment, should_reconnect, should_run_history,
        spawn_subscription_drain, sql_literal, update_peak, wait_for_delivery, ChatSqlKind,
        ChatWorkloadSettings, ChatWorkloadStats, LiveSession, SessionActivityGuard,
        SessionDeliveryTracker, SimpleRng, SubscriptionKind, UserClientPool, AI_AGENT_SENDER,
        CHAT_FORWARDER_POLL_TIMEOUT, CHAT_RECONNECT_GAP, CHAT_TYPING_BURSTS, CHAT_TYPING_INTERVAL,
        SUBSCRIPTION_SHUTDOWN_TIMEOUT,
    },
    client::KalamClient,
};

pub struct AiInboxAgent {
    stop_tx: watch::Sender<bool>,
    handle:  JoinHandle<Result<(), String>>,
}

const AI_AGENT_MAX_IN_FLIGHT: usize = 64;

impl AiInboxAgent {
    pub fn start(
        admin_client: KalamClient,
        namespace: String,
        stats: Arc<ChatWorkloadStats>,
        global_stop: Arc<AtomicBool>,
        message_ids: Arc<AtomicU64>,
        typing_ids: Arc<AtomicU64>,
        iteration: u32,
    ) -> Self {
        let (stop_tx, stop_rx) = watch::channel(false);
        let handle = tokio::spawn(run_ai_inbox_agent(
            admin_client,
            namespace,
            stats,
            global_stop,
            message_ids,
            typing_ids,
            iteration,
            stop_rx,
        ));
        Self { stop_tx, handle }
    }

    pub async fn shutdown(self) -> Result<(), String> {
        let _ = self.stop_tx.send(true);
        let mut handle = self.handle;
        match timeout(SUBSCRIPTION_SHUTDOWN_TIMEOUT, &mut handle).await {
            Ok(Ok(Ok(()))) => Ok(()),
            Ok(Ok(Err(error))) => Err(error),
            Ok(Err(error)) => Err(format!("ai agent join error: {}", error)),
            Err(_) => {
                handle.abort();
                Err("timed out waiting for AI inbox agent shutdown".to_string())
            },
        }
    }
}

#[derive(Debug, Clone)]
struct AiInboxRecord {
    conversation_id: u64,
    sender_user:     String,
    role:            String,
    body:            String,
}

async fn run_ai_inbox_agent(
    admin_client: KalamClient,
    namespace: String,
    stats: Arc<ChatWorkloadStats>,
    global_stop: Arc<AtomicBool>,
    message_ids: Arc<AtomicU64>,
    typing_ids: Arc<AtomicU64>,
    iteration: u32,
    mut stop_rx: watch::Receiver<bool>,
) -> Result<(), String> {
    let topic = ai_inbox_topic_name(&namespace);
    let group_id =
        format!("chat_rt_ai_agent_{}_{}", sanitize_identifier_fragment(&namespace), iteration);
    let mut consumer = admin_client
        .link()
        .clone()
        .consumer()
        .topic(&topic)
        .group_id(&group_id)
        // Each benchmark iteration uses a fresh group. Starting at the beginning avoids losing
        // messages produced before this task has completed its first poll.
        .auto_offset_reset(AutoOffsetReset::Earliest)
        .max_poll_records(128)
        .poll_timeout(CHAT_FORWARDER_POLL_TIMEOUT)
        .build()
        .map_err(|error| format!("ai inbox agent build: {}", error))?;

    while !global_stop.load(Ordering::Relaxed) && !*stop_rx.borrow() {
        let records = tokio::select! {
            changed = stop_rx.changed() => {
                let _ = changed;
                break;
            }
            records = consumer.poll_with_timeout(CHAT_FORWARDER_POLL_TIMEOUT) => {
                records.map_err(|error| format!("ai inbox agent poll: {}", error))?
            }
        };

        if records.is_empty() {
            continue;
        }

        let mut pending_rows = Vec::with_capacity(records.len());
        for record in &records {
            stats.ai_topic_records.fetch_add(1, Ordering::Relaxed);

            if record.op != TopicOp::Insert {
                stats.skipped_topic_records.fetch_add(1, Ordering::Relaxed);
                continue;
            }

            let row = match parse_ai_inbox_record(&record.payload) {
                Ok(row) => row,
                Err(error) => {
                    let _ = consumer.close().await;
                    return Err(format!("ai inbox payload decode: {}", error));
                },
            };

            if row.role != "user" {
                stats.skipped_topic_records.fetch_add(1, Ordering::Relaxed);
                continue;
            }

            pending_rows.push(row);
        }

        if !process_ai_rows(
            pending_rows,
            &admin_client,
            &namespace,
            &stats,
            &message_ids,
            &typing_ids,
            &mut stop_rx,
        )
        .await?
        {
            break;
        }

        for record in &records {
            consumer.mark_processed(&record);
        }

        consumer
            .commit_sync()
            .await
            .map_err(|error| format!("ai inbox agent commit: {}", error))?;
    }

    consumer
        .close()
        .await
        .map_err(|error| format!("ai inbox agent close: {}", error))
}

async fn process_ai_rows(
    rows: Vec<AiInboxRecord>,
    admin_client: &KalamClient,
    namespace: &str,
    stats: &Arc<ChatWorkloadStats>,
    message_ids: &Arc<AtomicU64>,
    typing_ids: &Arc<AtomicU64>,
    stop_rx: &mut watch::Receiver<bool>,
) -> Result<bool, String> {
    let mut tasks = JoinSet::new();

    for row in rows {
        while tasks.len() >= AI_AGENT_MAX_IN_FLIGHT {
            if !join_ai_task_or_stop(&mut tasks, stop_rx).await? {
                return Ok(false);
            }
        }

        let admin_client = admin_client.clone();
        let namespace = namespace.to_string();
        let stats = stats.clone();
        let message_ids = message_ids.clone();
        let typing_ids = typing_ids.clone();
        tasks.spawn(async move {
            reply_as_agent(&admin_client, &namespace, &row, &stats, &message_ids, &typing_ids).await
        });
    }

    while !tasks.is_empty() {
        if !join_ai_task_or_stop(&mut tasks, stop_rx).await? {
            return Ok(false);
        }
    }
    Ok(true)
}

async fn join_ai_task_or_stop(
    tasks: &mut JoinSet<Result<(), String>>,
    stop_rx: &mut watch::Receiver<bool>,
) -> Result<bool, String> {
    tokio::select! {
        biased;
        changed = stop_rx.changed() => {
            let _ = changed;
            tasks.abort_all();
            while tasks.join_next().await.is_some() {}
            Ok(false)
        }
        result = tasks.join_next() => {
            match result {
                Some(Ok(Ok(()))) => Ok(true),
                Some(Ok(Err(error))) => {
                    tasks.abort_all();
                    while tasks.join_next().await.is_some() {}
                    Err(error)
                },
                Some(Err(error)) => {
                    tasks.abort_all();
                    while tasks.join_next().await.is_some() {}
                    Err(format!("ai reply task join error: {}", error))
                },
                None => Ok(true),
            }
        }
    }
}

fn parse_ai_inbox_record(payload_bytes: &[u8]) -> Result<AiInboxRecord, String> {
    let payload: JsonValue = serde_json::from_slice(payload_bytes)
        .map_err(|error| format!("invalid topic payload json: {}", error))?;
    let row = payload.get("row").unwrap_or(&payload);

    Ok(AiInboxRecord {
        conversation_id: json_u64_field(row, "conversation_id")?,
        sender_user:     json_string_field(row, "sender_user")?,
        role:            json_string_field(row, "role")?,
        body:            json_string_field(row, "body")?,
    })
}

async fn reply_as_agent(
    admin_client: &KalamClient,
    namespace: &str,
    row: &AiInboxRecord,
    stats: &Arc<ChatWorkloadStats>,
    message_ids: &Arc<AtomicU64>,
    typing_ids: &Arc<AtomicU64>,
) -> Result<(), String> {
    for _ in 0..CHAT_TYPING_BURSTS {
        let event_id = typing_ids.fetch_add(1, Ordering::Relaxed);
        let insert_typing_started = Instant::now();
        let statement = format!(
            "INSERT INTO {}.typing_events (id, conversation_id, sender_user, recipient_user, \
             phase, created_at_ms) VALUES ({}, {}, {}, {}, 'typing', {})",
            namespace,
            event_id,
            row.conversation_id,
            sql_literal(AI_AGENT_SENDER),
            sql_literal(&row.sender_user),
            epoch_ms(),
        );
        run_tracked_sql(
            admin_client,
            &execute_as_user_sql(&row.sender_user, &statement),
            ChatSqlKind::Insert,
            stats,
        )
        .await?;
        stats.insert_typing_event_timings.record(insert_typing_started.elapsed());
        stats.typing_events_sent.fetch_add(1, Ordering::Relaxed);
        sleep(CHAT_TYPING_INTERVAL).await;
    }

    let message_id = message_ids.fetch_add(1, Ordering::Relaxed);
    let insert_message_started = Instant::now();
    let reply_body = format!("AI reply to {}", row.body);
    let statement = format!(
        "INSERT INTO {}.messages_ai (id, conversation_id, role, sender_user, body, created_at_ms) \
         VALUES ({}, {}, 'assistant', {}, {}, {})",
        namespace,
        message_id,
        row.conversation_id,
        sql_literal(AI_AGENT_SENDER),
        sql_literal(&reply_body),
        epoch_ms(),
    );
    run_tracked_sql(
        admin_client,
        &execute_as_user_sql(&row.sender_user, &statement),
        ChatSqlKind::Insert,
        stats,
    )
    .await?;
    stats.insert_message_timings.record(insert_message_started.elapsed());
    stats.messages_sent.fetch_add(1, Ordering::Relaxed);
    stats.ai_replies_sent.fetch_add(1, Ordering::Relaxed);
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub async fn run_ai_worker(
    worker_id: u32,
    namespace: String,
    run_deadline: Instant,
    stats: Arc<ChatWorkloadStats>,
    user_pool: Arc<UserClientPool>,
    users: Arc<Vec<String>>,
    conversation_ids: Arc<AtomicU64>,
    message_ids: Arc<AtomicU64>,
    settings: ChatWorkloadSettings,
    global_stop: Arc<AtomicBool>,
) -> Result<(), String> {
    let mut rng = SimpleRng::seeded(0xA11A1_u64 + u64::from(worker_id) * 4_861);
    let Some(user) = pick_distinct_users(&mut rng, &users, 1).into_iter().next() else {
        return Err("ai worker needs at least one warmed user".to_string());
    };

    let conversation_id = conversation_ids.fetch_add(1, Ordering::Relaxed);
    let created_at_ms = epoch_ms();
    let _session_guard = SessionActivityGuard::new(stats.clone());
    let client = user_pool.client_for(&user).await?;
    let delivery_tracker = Arc::new(SessionDeliveryTracker::new(user.clone()));
    let mut subscription_generation = 0_u64;
    let mut live = Some(
        start_ai_live_session(
            &client.link(),
            &namespace,
            conversation_id,
            &user,
            subscription_generation,
            false,
            &delivery_tracker,
            stats.clone(),
        )
        .await?,
    );

    let target_cycle_interval = settings.target_cycle_interval(2);
    let delivery_timeout =
        chat_delivery_wait_timeout(settings.realtime_conversations, settings.messages_per_minute);

    let create_conversation_started = Instant::now();
    let create_result = run_tracked_sql(
        &client,
        &format!(
            "INSERT INTO {}.conversations_ai (id, title, state, created_at_ms) VALUES ({}, {}, \
             'open', {})",
            namespace,
            conversation_id,
            sql_literal(&format!("ai_{}", conversation_id)),
            created_at_ms,
        ),
        ChatSqlKind::Insert,
        &stats,
    )
    .await;
    if let Err(error) = create_result {
        if let Some(live) = live.take() {
            let _ = live.shutdown().await;
        }
        return Err(error);
    }
    stats.create_conversation_timings.record(create_conversation_started.elapsed());

    let worker_result = run_ai_cycles(
        worker_id,
        &namespace,
        &user,
        conversation_id,
        created_at_ms,
        &client,
        &mut live,
        &mut subscription_generation,
        &delivery_tracker,
        &message_ids,
        target_cycle_interval,
        settings.mutation_every_messages,
        delivery_timeout,
        run_deadline,
        &stats,
        &global_stop,
    )
    .await;

    let shutdown_result = match live.take() {
        Some(live) => live.shutdown().await,
        None => Ok(()),
    };
    match (worker_result, shutdown_result) {
        (Ok(()), Ok(())) => Ok(()),
        (Err(error), _) => Err(error),
        (Ok(()), Err(error)) => Err(error),
    }
}

#[allow(clippy::too_many_arguments)]
async fn run_ai_cycles(
    worker_id: u32,
    namespace: &str,
    user: &str,
    conversation_id: u64,
    created_at_ms: u64,
    client: &KalamClient,
    live: &mut Option<LiveSession>,
    subscription_generation: &mut u64,
    delivery_tracker: &Arc<SessionDeliveryTracker>,
    message_ids: &Arc<AtomicU64>,
    target_cycle_interval: Option<Duration>,
    mutation_every_messages: u64,
    delivery_timeout: Duration,
    run_deadline: Instant,
    stats: &Arc<ChatWorkloadStats>,
    global_stop: &Arc<AtomicBool>,
) -> Result<(), String> {
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

        if should_run_history(cycle_ordinal) {
            query_conversation_history(
                client,
                namespace,
                "messages_ai",
                conversation_id,
                historic_cutoff_ms(created_at_ms),
                stats,
            )
            .await?;
        }

        let typing_before = delivery_tracker.typing_events.load(Ordering::Relaxed);
        let incoming_before = delivery_tracker.incoming_messages.load(Ordering::Relaxed);
        let reconnecting = should_reconnect(cycle_ordinal);

        let emitted_message_id = if reconnecting {
            let reconnect_started = Instant::now();
            if let Some(old_live) = live.take() {
                old_live.shutdown().await?;
            }
            let message_id = emit_ai_user_message(
                client,
                namespace,
                conversation_id,
                user,
                &format!("ai_msg_{}_{}_{}", conversation_id, worker_id, cycle_ordinal),
                stats,
                message_ids,
            )
            .await?;
            sleep(CHAT_RECONNECT_GAP).await;
            *subscription_generation += 1;
            *live = Some(
                start_ai_live_session(
                    &client.link(),
                    namespace,
                    conversation_id,
                    user,
                    *subscription_generation,
                    true,
                    delivery_tracker,
                    stats.clone(),
                )
                .await?,
            );
            stats.reconnect_timings.record(reconnect_started.elapsed());
            stats.reconnects.fetch_add(1, Ordering::Relaxed);
            reopen_conversation(
                client,
                namespace,
                "conversations_ai",
                "messages_ai",
                conversation_id,
                stats,
            )
            .await?;
            message_id
        } else {
            emit_ai_user_message(
                client,
                namespace,
                conversation_id,
                user,
                &format!("ai_msg_{}_{}_{}", conversation_id, worker_id, cycle_ordinal),
                stats,
                message_ids,
            )
            .await?
        };

        let required_typing = if reconnecting {
            typing_before
        } else {
            typing_before + CHAT_TYPING_BURSTS
        };
        if let Err(error) = wait_for_delivery(
            delivery_tracker,
            required_typing,
            incoming_before + 1,
            delivery_timeout,
        )
        .await
        {
            maybe_log_delivery_timeout(
                stats,
                worker_id,
                conversation_id,
                cycle_ordinal,
                &[delivery_tracker.clone()],
                required_typing,
                incoming_before + 1,
            );
            let _ = error;
            continue;
        }

        maybe_mutate_message(
            client,
            namespace,
            "messages_ai",
            emitted_message_id,
            mutation_every_messages,
            stats,
        )
        .await?;

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

async fn emit_ai_user_message(
    client: &KalamClient,
    namespace: &str,
    conversation_id: u64,
    sender_user: &str,
    body: &str,
    stats: &Arc<ChatWorkloadStats>,
    message_ids: &Arc<AtomicU64>,
) -> Result<u64, String> {
    let message_id = message_ids.fetch_add(1, Ordering::Relaxed);
    let insert_message_started = Instant::now();
    run_tracked_sql(
        client,
        &format!(
            "INSERT INTO {}.messages_ai (id, conversation_id, role, sender_user, body, \
             created_at_ms) VALUES ({}, {}, 'user', {}, {}, {})",
            namespace,
            message_id,
            conversation_id,
            sql_literal(sender_user),
            sql_literal(body),
            epoch_ms(),
        ),
        ChatSqlKind::Insert,
        stats,
    )
    .await?;
    stats.insert_message_timings.record(insert_message_started.elapsed());
    stats.messages_sent.fetch_add(1, Ordering::Relaxed);
    stats.ai_user_messages_sent.fetch_add(1, Ordering::Relaxed);
    Ok(message_id)
}

#[allow(clippy::too_many_arguments)]
async fn start_ai_live_session(
    link: &KalamLinkClient,
    namespace: &str,
    conversation_id: u64,
    user: &str,
    generation: u64,
    with_initial_data: bool,
    tracker: &Arc<SessionDeliveryTracker>,
    stats: Arc<ChatWorkloadStats>,
) -> Result<LiveSession, String> {
    let (stop_tx, _) = watch::channel(false);
    let message_open_started = Instant::now();
    let message_subscription = create_subscription(
        link,
        format!("chat_ai_messages_{}_{}_{}", user, conversation_id, generation),
        format!(
            "SELECT id, conversation_id, role, sender_user, body, created_at_ms FROM \
             {}.messages_ai WHERE conversation_id = {}",
            namespace, conversation_id
        ),
        with_initial_data,
    )
    .await?;
    record_subscription_open(&stats, message_open_started.elapsed());

    let typing_open_started = Instant::now();
    let typing_subscription = match create_subscription(
        link,
        format!("chat_ai_typing_{}_{}_{}", user, conversation_id, generation),
        format!(
            "SELECT id, conversation_id, sender_user, recipient_user, phase, created_at_ms FROM \
             {}.typing_events WHERE conversation_id = {} AND recipient_user = {}",
            namespace,
            conversation_id,
            sql_literal(user),
        ),
        with_initial_data,
    )
    .await
    {
        Ok(subscription) => subscription,
        Err(error) => {
            let mut message_subscription = message_subscription;
            let _ = message_subscription.close().await;
            return Err(error);
        },
    };
    record_subscription_open(&stats, typing_open_started.elapsed());

    stats.subscriptions_opened.fetch_add(2, Ordering::Relaxed);
    let active_subscriptions = stats.active_subscriptions.fetch_add(2, Ordering::Relaxed) + 2;
    update_peak(&stats.peak_active_subscriptions, active_subscriptions);

    let handles = vec![
        spawn_subscription_drain(
            message_subscription,
            stop_tx.subscribe(),
            stats.clone(),
            tracker.clone(),
            SubscriptionKind::Message,
        ),
        spawn_subscription_drain(
            typing_subscription,
            stop_tx.subscribe(),
            stats,
            tracker.clone(),
            SubscriptionKind::Typing,
        ),
    ];
    Ok(LiveSession::new(stop_tx, handles))
}

pub fn ai_worker_start_delay(worker_id: u32) -> Duration {
    chat_worker_start_delay(worker_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn ai_task_wait_is_interrupted_by_shutdown() {
        let (stop_tx, mut stop_rx) = watch::channel(false);
        let mut tasks = tokio::task::JoinSet::new();
        tasks.spawn(async {
            sleep(Duration::from_secs(30)).await;
            Ok(())
        });

        stop_tx.send(true).expect("shutdown receiver is alive");
        let completed =
            timeout(Duration::from_millis(100), join_ai_task_or_stop(&mut tasks, &mut stop_rx))
                .await
                .expect("shutdown must interrupt an in-flight AI reply")
                .expect("shutdown is not an AI task failure");

        assert!(!completed);
        assert!(tasks.is_empty());
    }
}
