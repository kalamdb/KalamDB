use std::{
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc,
    },
    time::Duration,
};

use kalam_client::KalamLinkClient;
use tokio::{
    sync::{watch, Barrier},
    time::{sleep, timeout, Instant},
};

use crate::{
    chat_runtime::common::{
        chat_delivery_wait_timeout, chat_worker_start_delay, create_subscription, epoch_ms,
        historic_cutoff_ms, maybe_log_delivery_timeout, maybe_mutate_message, pick_distinct_users,
        query_conversation_history, record_subscription_open, reopen_conversation, run_tracked_sql,
        should_reconnect, should_run_history, spawn_subscription_drain, sql_literal, update_peak,
        wait_for_incoming, ChatSqlKind, ChatWorkloadSettings, ChatWorkloadStats, LiveSession,
        SessionActivityGuard, SessionDeliveryTracker, SimpleRng, SubscriptionKind, UserClientPool,
        CHAT_RECONNECT_GAP,
    },
    client::KalamClient,
};

struct SharedMember {
    username:   String,
    client:     KalamClient,
    tracker:    Arc<SessionDeliveryTracker>,
    live:       Option<LiveSession>,
    generation: u64,
}

#[allow(clippy::too_many_arguments)]
pub async fn run_shared_worker(
    worker_id: u32,
    namespace: String,
    member_count: usize,
    run_deadline: Instant,
    stats: Arc<ChatWorkloadStats>,
    user_pool: Arc<UserClientPool>,
    users: Arc<Vec<String>>,
    conversation_ids: Arc<AtomicU64>,
    message_ids: Arc<AtomicU64>,
    settings: ChatWorkloadSettings,
    global_stop: Arc<AtomicBool>,
    setup_barrier: Option<Arc<Barrier>>,
) -> Result<(), String> {
    let mut rng = SimpleRng::seeded(0x5A11ED_u64 + u64::from(worker_id) * 3_254);
    let member_users = pick_distinct_users(&mut rng, &users, member_count);
    if member_users.len() < member_count {
        return Err(format!(
            "shared worker needs {} warmed users, got {}",
            member_count,
            member_users.len()
        ));
    }

    let conversation_id = conversation_ids.fetch_add(1, Ordering::Relaxed);
    let created_at_ms = epoch_ms();
    let _session_guard = SessionActivityGuard::new(stats.clone());
    let mut members = Vec::with_capacity(member_count);
    for username in member_users {
        let client = user_pool.client_for(&username).await?;
        let tracker = Arc::new(SessionDeliveryTracker::new(username.clone()));
        members.push(SharedMember {
            username,
            client,
            tracker,
            live: None,
            generation: 0,
        });
    }

    let creator_client = members[0].client.clone();
    let creator_username = members[0].username.clone();
    let create_conversation_started = Instant::now();
    run_tracked_sql(
        &creator_client,
        &format!(
            "INSERT INTO {}.conversations (id, title, created_by, created_at_ms) VALUES ({}, {}, \
             {}, {})",
            namespace,
            conversation_id,
            sql_literal(&format!("room_{}", conversation_id)),
            sql_literal(&creator_username),
            created_at_ms,
        ),
        ChatSqlKind::Insert,
        &stats,
    )
    .await?;
    stats.create_conversation_timings.record(create_conversation_started.elapsed());

    for member in &members {
        run_tracked_sql(
            &member.client,
            &format!(
                "INSERT INTO {}.conversation_members (id, user_id, conversation_id) VALUES ({}, \
                 {}, {})",
                namespace,
                sql_literal(&format!("{}:{}", member.username, conversation_id)),
                sql_literal(&member.username),
                conversation_id,
            ),
            ChatSqlKind::Insert,
            &stats,
        )
        .await?;
    }

    if let Some(setup_barrier) = setup_barrier {
        timeout(Duration::from_secs(30), setup_barrier.wait())
            .await
            .map_err(|_| "timed out waiting for shared room membership setup".to_string())?;
    }

    for member in &mut members {
        member.live = Some(
            start_shared_live_session(
                &member.client.link(),
                &namespace,
                conversation_id,
                &member.username,
                member.generation,
                false,
                &member.tracker,
                stats.clone(),
            )
            .await?,
        );
    }

    let target_cycle_interval = settings.target_cycle_interval(member_count as u32);
    let delivery_timeout =
        chat_delivery_wait_timeout(settings.realtime_conversations, settings.messages_per_minute);

    let worker_result = run_shared_cycles(
        worker_id,
        &namespace,
        conversation_id,
        created_at_ms,
        &mut members,
        &message_ids,
        target_cycle_interval,
        settings.mutation_every_messages,
        delivery_timeout,
        run_deadline,
        &stats,
        &global_stop,
    )
    .await;

    let mut shutdown_errors = Vec::new();
    for member in &mut members {
        if let Some(live) = member.live.take() {
            if let Err(error) = live.shutdown().await {
                shutdown_errors.push(error);
            }
        }
    }

    match (worker_result, shutdown_errors.is_empty()) {
        (Ok(()), true) => Ok(()),
        (Err(error), _) => Err(error),
        (Ok(()), false) => Err(shutdown_errors.remove(0)),
    }
}

#[allow(clippy::too_many_arguments)]
async fn run_shared_cycles(
    worker_id: u32,
    namespace: &str,
    conversation_id: u64,
    created_at_ms: u64,
    members: &mut [SharedMember],
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
                &members[0].client,
                namespace,
                "messages",
                conversation_id,
                historic_cutoff_ms(created_at_ms),
                stats,
            )
            .await?;
        }

        let reconnect_cycle = should_reconnect(cycle_ordinal) && members.len() > 1;
        if reconnect_cycle {
            run_shared_reconnect_cycle(
                worker_id,
                namespace,
                conversation_id,
                members,
                message_ids,
                cycle_ordinal,
                mutation_every_messages,
                delivery_timeout,
                stats,
            )
            .await?;
        } else {
            run_shared_turn_cycle(
                worker_id,
                namespace,
                conversation_id,
                members,
                message_ids,
                cycle_ordinal,
                mutation_every_messages,
                delivery_timeout,
                stats,
            )
            .await?;
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

#[allow(clippy::too_many_arguments)]
async fn run_shared_turn_cycle(
    worker_id: u32,
    namespace: &str,
    conversation_id: u64,
    members: &[SharedMember],
    message_ids: &Arc<AtomicU64>,
    cycle_ordinal: u64,
    mutation_every_messages: u64,
    delivery_timeout: Duration,
    stats: &Arc<ChatWorkloadStats>,
) -> Result<(), String> {
    for sender_index in 0..members.len() {
        let incoming_before: Vec<u64> = members
            .iter()
            .enumerate()
            .map(|(index, member)| {
                if index == sender_index {
                    0
                } else {
                    member.tracker.incoming_messages.load(Ordering::Relaxed)
                }
            })
            .collect();

        let message_id = emit_shared_message(
            &members[sender_index].client,
            namespace,
            conversation_id,
            &members[sender_index].username,
            &format!(
                "shared_msg_{}_{}_{}_{}",
                conversation_id, worker_id, cycle_ordinal, sender_index
            ),
            stats,
            message_ids,
        )
        .await?;

        for (observer_index, member) in members.iter().enumerate() {
            if observer_index == sender_index {
                continue;
            }
            if let Err(error) = wait_for_incoming(
                &member.tracker,
                incoming_before[observer_index] + 1,
                delivery_timeout,
            )
            .await
            {
                maybe_log_delivery_timeout(
                    stats,
                    worker_id,
                    conversation_id,
                    cycle_ordinal,
                    &members.iter().map(|member| member.tracker.clone()).collect::<Vec<_>>(),
                    0,
                    incoming_before[observer_index] + 1,
                );
                let _ = error;
            }
        }

        maybe_mutate_message(
            &members[sender_index].client,
            namespace,
            "messages",
            message_id,
            mutation_every_messages,
            stats,
        )
        .await?;
    }

    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn run_shared_reconnect_cycle(
    worker_id: u32,
    namespace: &str,
    conversation_id: u64,
    members: &mut [SharedMember],
    message_ids: &Arc<AtomicU64>,
    cycle_ordinal: u64,
    mutation_every_messages: u64,
    delivery_timeout: Duration,
    stats: &Arc<ChatWorkloadStats>,
) -> Result<(), String> {
    let reconnect_started = Instant::now();
    if let Some(live) = members[0].live.take() {
        live.shutdown().await?;
    }

    let incoming_before = members[0].tracker.incoming_messages.load(Ordering::Relaxed);
    let gap_message_id = emit_shared_message(
        &members[1].client,
        namespace,
        conversation_id,
        &members[1].username,
        &format!("shared_gap_{}_{}_{}", conversation_id, worker_id, cycle_ordinal),
        stats,
        message_ids,
    )
    .await?;
    sleep(CHAT_RECONNECT_GAP).await;

    members[0].generation += 1;
    let reconnect_client = members[0].client.clone();
    let reconnect_user = members[0].username.clone();
    let reconnect_generation = members[0].generation;
    let reconnect_tracker = members[0].tracker.clone();
    members[0].live = Some(
        start_shared_live_session(
            &reconnect_client.link(),
            namespace,
            conversation_id,
            &reconnect_user,
            reconnect_generation,
            true,
            &reconnect_tracker,
            stats.clone(),
        )
        .await?,
    );
    stats.reconnect_timings.record(reconnect_started.elapsed());
    stats.reconnects.fetch_add(1, Ordering::Relaxed);

    reopen_conversation(
        &members[0].client,
        namespace,
        "conversations",
        "messages",
        conversation_id,
        stats,
    )
    .await?;

    if let Err(error) =
        wait_for_incoming(&members[0].tracker, incoming_before + 1, delivery_timeout).await
    {
        maybe_log_delivery_timeout(
            stats,
            worker_id,
            conversation_id,
            cycle_ordinal,
            &[members[0].tracker.clone()],
            0,
            incoming_before + 1,
        );
        let _ = error;
    }

    maybe_mutate_message(
        &members[1].client,
        namespace,
        "messages",
        gap_message_id,
        mutation_every_messages,
        stats,
    )
    .await?;

    let reply_message_id = emit_shared_message(
        &members[0].client,
        namespace,
        conversation_id,
        &members[0].username,
        &format!("shared_reply_{}_{}_{}", conversation_id, worker_id, cycle_ordinal),
        stats,
        message_ids,
    )
    .await?;
    maybe_mutate_message(
        &members[0].client,
        namespace,
        "messages",
        reply_message_id,
        mutation_every_messages,
        stats,
    )
    .await?;
    Ok(())
}

async fn emit_shared_message(
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
            "INSERT INTO {}.messages (id, conversation_id, sender_user, body, created_at_ms) \
             VALUES ({}, {}, {}, {}, {})",
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
    stats.shared_messages_sent.fetch_add(1, Ordering::Relaxed);
    Ok(message_id)
}

#[allow(clippy::too_many_arguments)]
async fn start_shared_live_session(
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
        format!("chat_shared_messages_{}_{}_{}", user, conversation_id, generation),
        format!(
            "SELECT id, conversation_id, sender_user, body, created_at_ms FROM {}.messages WHERE \
             conversation_id = {}",
            namespace, conversation_id
        ),
        with_initial_data,
    )
    .await?;
    record_subscription_open(&stats, message_open_started.elapsed());

    stats.subscriptions_opened.fetch_add(1, Ordering::Relaxed);
    let active_subscriptions = stats.active_subscriptions.fetch_add(1, Ordering::Relaxed) + 1;
    update_peak(&stats.peak_active_subscriptions, active_subscriptions);

    let handles = vec![spawn_subscription_drain(
        message_subscription,
        stop_tx.subscribe(),
        stats,
        tracker.clone(),
        SubscriptionKind::Message,
    )];
    Ok(LiveSession::new(stop_tx, handles))
}

pub fn shared_worker_start_delay(worker_id: u32) -> Duration {
    chat_worker_start_delay(worker_id)
}
