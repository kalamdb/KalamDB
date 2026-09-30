//! Connection-related models for WebSocket and consumer connections
//!
//! These models support both live query WebSocket subscriptions and topic consumer connections.

use std::{
    collections::{hash_map::Entry, HashMap, VecDeque},
    sync::{
        atomic::{AtomicBool, AtomicI64, AtomicU64, AtomicU8, Ordering},
        Arc, OnceLock,
    },
    time::{Instant, SystemTime, UNIX_EPOCH},
};

use kalamdb_commons::{
    ids::VersionId,
    models::{ConnectionId, ConnectionInfo, LiveQueryId, TableId, UserId},
    websocket::{CompressionType, ProtocolOptions, SerializationType, WireNotification},
    Role,
};
use kalamdb_row_filter::RowFilter;
use parking_lot::{Mutex, RwLock};
use tokio::sync::mpsc;

/// Get current epoch time in milliseconds (for lock-free heartbeat and metadata tracking)
#[inline]
pub(crate) fn epoch_millis() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis() as u64
}

/// Maximum live-query subscriptions allowed on a single WebSocket connection.
pub const MAX_SUBSCRIPTIONS_PER_CONNECTION: usize = 100;

/// Maximum pending notifications per connection before dropping new ones.
/// This must cover one full table fanout for a saturated shared WebSocket.
/// The benchmark and backend contract allow 100 subscriptions per connection;
/// 128 keeps a small cushion while staying tight for idle-connection memory.
pub const NOTIFICATION_CHANNEL_CAPACITY: usize = 128;

/// Maximum pending control events per connection.
///
/// Auth timeout, heartbeat timeout, shutdown, and a subscription lapse each
/// need a slot. A full data channel must still be able to report that the
/// client has to resubscribe.
pub const EVENT_CHANNEL_CAPACITY: usize = 8;

/// Maximum buffered notifications per subscription while initial snapshot loading is in progress.
///
/// Without this bound, a slow initial snapshot can accumulate unbounded change events and
/// exhaust memory under high write volume or many concurrent subscribers.
pub const MAX_BUFFERED_NOTIFICATIONS_PER_SUBSCRIPTION: usize = 1_024;

/// Type alias for sending notifications to WebSocket or consumer clients
pub type NotificationSender = mpsc::Sender<Arc<WireNotification>>;

/// Type alias for receiving notifications
pub type NotificationReceiver = mpsc::Receiver<Arc<WireNotification>>;

/// Type alias for sending control events to connections
pub type EventSender = mpsc::Sender<ConnectionEvent>;

/// Type alias for receiving control events
pub type EventReceiver = mpsc::Receiver<ConnectionEvent>;

/// Shared connection state — lock-free, held by handlers for direct access.
///
/// All fields are either immutable after construction, set-once (OnceLock/AtomicBool),
/// or interior-mutable (DashMap/AtomicU64), so no outer RwLock is needed.
pub type SharedConnectionState = Arc<ConnectionState>;

/// Events sent to connection tasks from the heartbeat checker
#[derive(Debug, Clone)]
pub enum ConnectionEvent {
    /// Authentication timeout - close connection
    AuthTimeout,
    /// Heartbeat timeout - close connection
    HeartbeatTimeout,
    /// Server is shutting down - close connection gracefully
    Shutdown,
    /// This subscription missed a change. The client must resubscribe
    /// without a resume cursor and replace its local snapshot.
    SubscriptionLapsed { subscription_id: Arc<str> },
}

/// Routing handle stored in subscription indices.
///
/// Filter, authorization, flow control, and the notification channel are Arc-shared
/// with `SubscriptionState`. Cloning a handle does not clone the evaluator or buffer.
#[derive(Debug, Clone)]
pub struct SubscriptionHandle {
    /// Stable subscription identifier shared across all notifications for this subscriber.
    pub subscription_id:  Arc<str>,
    /// Shared compiled filter expression (Arc for zero-copy across indices)
    pub filter_expr:      Option<Arc<RowFilter>>,
    /// Shared-table RLS evaluator bound to this subscription's principal.
    pub authorization:    Option<Arc<dyn crate::traits::LiveAuthorization>>,
    /// Column projections for filtering notification payload (None = all columns)
    pub projections:      Option<Arc<Vec<String>>>,
    /// Shared notification channel
    pub notification_tx:  NotificationSender,
    /// Control channel used to report a delivery gap for this subscription.
    pub event_tx:         EventSender,
    /// Flow control for initial load buffering and snapshot gating.
    /// None means the subscription was created without initial data.
    pub flow_control:     Option<Arc<SubscriptionFlowControl>>,
    /// Runtime metadata shared with the full subscription state.
    pub runtime_metadata: Arc<SubscriptionRuntimeMetadata>,
}

/// In-memory metadata tracked for active subscriptions only.
#[derive(Debug)]
pub struct SubscriptionRuntimeMetadata {
    query:           Arc<str>,
    options_json:    Option<Arc<str>>,
    created_at_ms:   i64,
    last_update_ms:  AtomicI64,
    changes:         AtomicI64,
    delivery_gapped: AtomicBool,
}

impl SubscriptionRuntimeMetadata {
    pub fn new(query: &str, options_json: Option<&str>, created_at_ms: i64) -> Self {
        Self {
            query: Arc::from(query),
            options_json: options_json.map(Arc::from),
            created_at_ms,
            last_update_ms: AtomicI64::new(created_at_ms),
            changes: AtomicI64::new(0),
            delivery_gapped: AtomicBool::new(false),
        }
    }

    /// Returns true the first time delivery for this subscription becomes incomplete.
    #[inline]
    pub fn claim_gap(&self) -> bool {
        self.delivery_gapped
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
    }

    #[inline]
    pub fn is_gapped(&self) -> bool {
        self.delivery_gapped.load(Ordering::Acquire)
    }

    #[inline]
    pub fn query(&self) -> &str {
        &self.query
    }

    #[inline]
    pub fn options_json(&self) -> Option<&str> {
        self.options_json.as_deref()
    }

    #[inline]
    pub fn created_at_ms(&self) -> i64 {
        self.created_at_ms
    }

    #[inline]
    pub fn last_update_ms(&self) -> i64 {
        self.last_update_ms.load(Ordering::Acquire)
    }

    #[inline]
    pub fn changes(&self) -> i64 {
        self.changes.load(Ordering::Acquire)
    }

    #[inline]
    pub fn record_delivery(&self) {
        self.record_delivery_at(epoch_millis());
    }

    #[inline]
    pub fn record_delivery_at(&self, epoch_millis: u64) {
        self.last_update_ms.store(epoch_millis as i64, Ordering::Release);
        self.changes.fetch_add(1, Ordering::AcqRel);
    }
}

/// One change held until the snapshot has been handed to the client.
#[derive(Debug, Clone)]
pub struct BufferedNotification {
    pub seq:          Option<VersionId>,
    pub notification: Arc<WireNotification>,
}

/// Result of offering one change to a subscription.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Accept {
    /// Kept until the snapshot handoff.
    Stored,
    /// Already inside the snapshot.
    Covered,
    /// Written to the live channel.
    Sent,
    /// The subscriber is gone.
    Closed,
    /// A change cannot be delivered. The client must take a fresh snapshot.
    Failed,
}

const PHASE_CATCHUP: u8 = 0;
const PHASE_LIVE: u8 = 1;
const PHASE_FAILED: u8 = 2;

#[derive(Debug)]
struct Gate {
    snapshot_end: Option<i64>,
    pending:      VecDeque<BufferedNotification>,
}

/// Catch-up buffer for one subscription.
///
/// After the snapshot prefix is queued, `phase` is live and later sends only
/// touch the channel. The mutex stays on the catch-up path so that prefix
/// cannot be overtaken.
#[derive(Debug)]
pub struct SubscriptionFlowControl {
    phase: AtomicU8,
    gate:  Mutex<Gate>,
}

impl SubscriptionFlowControl {
    pub fn new() -> Self {
        Self {
            phase: AtomicU8::new(PHASE_CATCHUP),
            gate:  Mutex::new(Gate {
                snapshot_end: None,
                pending:      VecDeque::new(),
            }),
        }
    }

    pub fn set_snapshot_end_seq(&self, snapshot_end_seq: Option<VersionId>) {
        let mut gate = self.gate.lock();
        gate.snapshot_end = snapshot_end_seq.map(|version| version.as_i64());
        if let Some(end) = gate.snapshot_end {
            gate.pending
                .retain(|item| item.seq.is_none_or(|version| version.as_i64() > end));
        }
    }

    /// Offer a change. Once live, this does not take the catch-up mutex.
    pub fn accept(
        &self,
        notification: Arc<WireNotification>,
        seq: Option<VersionId>,
        tx: &NotificationSender,
    ) -> Accept {
        match self.phase.load(Ordering::Acquire) {
            PHASE_FAILED => return Accept::Failed,
            PHASE_LIVE => return send_live(&self.phase, notification, tx),
            _ => {},
        }

        let mut gate = self.gate.lock();
        match self.phase.load(Ordering::Acquire) {
            PHASE_FAILED => Accept::Failed,
            PHASE_LIVE => send_live(&self.phase, notification, tx),
            _ => {
                let result = queue_change(&mut gate, notification, seq);
                if result == Accept::Failed {
                    self.phase.store(PHASE_FAILED, Ordering::Release);
                }
                result
            },
        }
    }

    /// Move the catch-up prefix onto the live channel, then accept live sends.
    pub fn finish(&self, tx: &NotificationSender) -> Result<usize, ()> {
        let mut gate = self.gate.lock();
        if self.phase.load(Ordering::Acquire) == PHASE_FAILED {
            return Err(());
        }
        let pending = take_pending(&mut gate.pending);
        let mut sent = 0usize;
        for item in pending {
            match send_live(&self.phase, item.notification, tx) {
                Accept::Sent => sent += 1,
                Accept::Failed => return Err(()),
                Accept::Closed => {
                    self.phase.store(PHASE_FAILED, Ordering::Release);
                    return Ok(sent);
                },
                Accept::Stored | Accept::Covered => {},
            }
        }
        self.phase.store(PHASE_LIVE, Ordering::Release);
        Ok(sent)
    }

    /// Test and pre-completed subscriptions skip catch-up.
    pub fn mark_initial_complete(&self) {
        let mut gate = self.gate.lock();
        gate.pending.clear();
        gate.pending.shrink_to_fit();
        self.phase.store(PHASE_LIVE, Ordering::Release);
    }

    pub fn is_gapped(&self) -> bool {
        self.phase.load(Ordering::Acquire) == PHASE_FAILED
    }

    pub fn buffer_notification(&self, notification: Arc<WireNotification>, seq: Option<VersionId>) {
        let mut gate = self.gate.lock();
        if queue_change(&mut gate, notification, seq) == Accept::Failed {
            self.phase.store(PHASE_FAILED, Ordering::Release);
        }
    }

    pub fn drain_buffered_notifications(&self) -> Vec<BufferedNotification> {
        let mut gate = self.gate.lock();
        take_pending(&mut gate.pending)
    }

    pub fn release_buffer(&self) {
        let mut gate = self.gate.lock();
        gate.pending.clear();
        gate.pending.shrink_to_fit();
    }
}

fn queue_change(
    gate: &mut Gate,
    notification: Arc<WireNotification>,
    seq: Option<VersionId>,
) -> Accept {
    if let (Some(end), Some(version)) = (gate.snapshot_end, seq) {
        if version.as_i64() <= end {
            return Accept::Covered;
        }
    }
    if seq.is_some_and(|version| gate.pending.iter().any(|item| item.seq == Some(version))) {
        return Accept::Stored;
    }
    if gate.pending.len() >= MAX_BUFFERED_NOTIFICATIONS_PER_SUBSCRIPTION {
        gate.pending.clear();
        return Accept::Failed;
    }
    gate.pending.push_back(BufferedNotification { seq, notification });
    Accept::Stored
}

fn send_live(
    phase: &AtomicU8,
    notification: Arc<WireNotification>,
    tx: &NotificationSender,
) -> Accept {
    match tx.try_send(notification) {
        Ok(()) => Accept::Sent,
        Err(mpsc::error::TrySendError::Full(_)) => {
            phase.store(PHASE_FAILED, Ordering::Release);
            Accept::Failed
        },
        Err(mpsc::error::TrySendError::Closed(_)) => Accept::Closed,
    }
}

fn take_pending(pending: &mut VecDeque<BufferedNotification>) -> Vec<BufferedNotification> {
    let slice = pending.make_contiguous();
    slice.sort_by(|left, right| match (left.seq, right.seq) {
        (Some(left_seq), Some(right_seq)) => left_seq.cmp(&right_seq),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => std::cmp::Ordering::Equal,
    });
    let drained: Vec<BufferedNotification> = pending.drain(..).collect();
    pending.shrink_to_fit();
    drained
}

/// Optional initial-load state for subscriptions that fetch a snapshot.
#[derive(Debug, Clone)]
pub struct InitialLoadState {
    /// Batch size for initial data loading
    pub batch_size:              usize,
    /// Snapshot boundary VersionId for consistent batch loading
    pub snapshot_end_seq:        Option<VersionId>,
    /// Deterministic snapshot boundary for reconnects across followers
    pub snapshot_end_commit_seq: Option<u64>,
    /// Current batch number for pagination tracking (0-indexed)
    /// Incremented after each batch is sent
    pub current_batch_num:       u32,
    /// Flow control for initial load buffering and snapshot gating
    pub flow_control:            Arc<SubscriptionFlowControl>,
}

/// Subscription state - stored only in ConnectionState.subscriptions
///
/// Contains all metadata needed for:
/// - Notification filtering (filter_expr)
/// - Column projections (projections)
/// - Optional initial data batch fetching and pagination
///
/// Memory optimization: Uses Arc<RowFilter> for filter sharing and allocates
/// snapshot/batch state only when the subscription actually requests it.
#[derive(Debug, Clone)]
pub struct SubscriptionState {
    pub live_id:          LiveQueryId,
    pub table_id:         TableId,
    /// Compiled filter expression from WHERE clause (parsed once at subscription time)
    /// None means no filter (SELECT * without WHERE)
    /// Arc-wrapped for sharing with SubscriptionHandle
    pub filter_expr:      Option<Arc<RowFilter>>,
    /// Column projections from SELECT clause (None = SELECT *, i.e., all columns)
    /// Arc-wrapped for sharing with SubscriptionHandle
    pub projections:      Option<Arc<Vec<String>>>,
    /// Optional initial-load state. None means the subscription streams live changes only.
    pub initial_load:     Option<InitialLoadState>,
    /// Whether this subscription is for a shared table (affects index cleanup)
    pub is_shared:        bool,
    /// Runtime metadata exposed by the in-memory live views.
    pub runtime_metadata: Arc<SubscriptionRuntimeMetadata>,
}

/// Connection state — lock-free interior mutability for 100k+ concurrent connections.
///
/// Identity and channel fields are immutable after construction.
/// Auth fields use set-once primitives (OnceLock + AtomicBool).
/// Subscriptions use a compact per-connection map.
/// Notification fanout reads from manager-level indices, so connection-local
/// subscription access is low-contention and does not need a sharded map.
/// Heartbeat uses AtomicU64 for zero-contention updates.
///
/// Used for both WebSocket live query connections and topic consumer connections.
pub struct ConnectionState {
    // === Identity (immutable) ===
    connection_id: ConnectionId,
    client_ip:     ConnectionInfo,
    connected_at:  Instant,

    // === Authentication (set-once) ===
    is_authenticated: AtomicBool,
    auth_started:     AtomicBool,
    user_id:          OnceLock<UserId>,
    user_role:        OnceLock<Role>,

    // === Protocol negotiation (set-once at auth time) ===
    protocol: OnceLock<ProtocolOptions>,

    // === Heartbeat (atomic) ===
    last_heartbeat_ms: AtomicU64,

    // === Subscriptions (concurrent interior-mutable) ===
    subscriptions: RwLock<HashMap<Arc<str>, SubscriptionState>>,

    // === Channels (immutable after construction) ===
    pub notification_tx: NotificationSender,
    pub event_tx:        EventSender,
}

impl ConnectionState {
    /// Create a new connection state with the given identity and channels.
    pub fn new(
        connection_id: ConnectionId,
        client_ip: ConnectionInfo,
        notification_tx: NotificationSender,
        event_tx: EventSender,
    ) -> Self {
        Self {
            connection_id,
            client_ip,
            connected_at: Instant::now(),
            is_authenticated: AtomicBool::new(false),
            auth_started: AtomicBool::new(false),
            user_id: OnceLock::new(),
            user_role: OnceLock::new(),
            protocol: OnceLock::new(),
            last_heartbeat_ms: AtomicU64::new(epoch_millis()),
            subscriptions: RwLock::new(HashMap::new()),
            notification_tx,
            event_tx,
        }
    }

    // === Identity accessors ===

    #[inline]
    pub fn connection_id(&self) -> &ConnectionId {
        &self.connection_id
    }

    #[inline]
    pub fn client_ip(&self) -> Option<&ConnectionInfo> {
        Some(&self.client_ip)
    }

    #[inline]
    pub fn connected_at(&self) -> Instant {
        self.connected_at
    }

    // === Authentication ===

    /// Mark that authentication has started (for timeout logic). Lock-free.
    #[inline]
    pub fn mark_auth_started(&self) {
        self.auth_started.store(true, Ordering::Release);
    }

    /// Whether auth attempt has started.
    #[inline]
    pub fn auth_started(&self) -> bool {
        self.auth_started.load(Ordering::Acquire)
    }

    /// Mark connection as authenticated with user identity. Lock-free (set-once).
    #[inline]
    pub fn mark_authenticated(&self, user_id: UserId, user_role: Role) {
        let _ = self.user_id.set(user_id);
        let _ = self.user_role.set(user_role);
        self.is_authenticated.store(true, Ordering::Release);
    }

    #[inline]
    pub fn is_authenticated(&self) -> bool {
        self.is_authenticated.load(Ordering::Acquire)
    }

    #[inline]
    pub fn user_id(&self) -> Option<&UserId> {
        self.user_id.get()
    }

    #[inline]
    pub fn user_role(&self) -> Option<Role> {
        self.user_role.get().copied()
    }

    // === Protocol ===

    /// Store negotiated protocol options (set-once at auth time).
    #[inline]
    pub fn set_protocol(&self, opts: ProtocolOptions) {
        let _ = self.protocol.set(opts);
    }

    /// Negotiated serialization type (defaults to Json if not yet set).
    #[inline]
    pub fn serialization_type(&self) -> SerializationType {
        self.protocol.get().map_or(SerializationType::Json, |p| p.serialization)
    }

    /// Negotiated compression type (defaults to Gzip if not yet set).
    #[inline]
    pub fn compression_type(&self) -> CompressionType {
        self.protocol.get().map_or(CompressionType::Gzip, |p| p.compression)
    }

    /// Negotiated protocol options (defaults if not yet set).
    #[inline]
    pub fn protocol(&self) -> ProtocolOptions {
        self.protocol.get().copied().unwrap_or_default()
    }

    // === Heartbeat ===

    /// Update heartbeat timestamp — lock-free atomic store.
    #[inline]
    pub fn update_heartbeat(&self) {
        self.last_heartbeat_ms.store(epoch_millis(), Ordering::Release);
    }

    /// Milliseconds elapsed since last heartbeat activity.
    #[inline]
    pub fn millis_since_heartbeat(&self) -> u64 {
        epoch_millis().saturating_sub(self.last_heartbeat_ms.load(Ordering::Acquire))
    }

    /// Last observed heartbeat epoch timestamp in milliseconds.
    #[inline]
    pub fn last_heartbeat_ms(&self) -> u64 {
        self.last_heartbeat_ms.load(Ordering::Acquire)
    }

    // === Subscription management ===

    /// Number of active subscriptions.
    pub fn subscription_count(&self) -> usize {
        self.subscriptions.read().len()
    }

    /// Insert a subscription into the connection's map.
    ///
    /// Returns `false` when the ID is already active so malformed or racing clients
    /// cannot replace state while incrementing registry and rate-limit counters again.
    pub fn insert_subscription(&self, key: Arc<str>, state: SubscriptionState) -> bool {
        let mut subscriptions = self.subscriptions.write();
        match subscriptions.entry(key) {
            Entry::Vacant(entry) => {
                entry.insert(state);
                true
            },
            Entry::Occupied(_) => false,
        }
    }

    /// Remove a subscription by key, returning the removed value.
    pub fn remove_subscription(&self, key: &str) -> Option<(Arc<str>, SubscriptionState)> {
        let mut subscriptions = self.subscriptions.write();
        let removed = subscriptions.remove_entry(key);
        if let Some((_, state)) = &removed {
            release_subscription_buffer(state);
        }
        shrink_subscription_map(&mut subscriptions);
        removed
    }

    /// Remove a subscription by primary key, falling back to a secondary key
    /// produced by `fallback_fn` if the primary is not found.
    pub fn remove_subscription_with_fallback<F>(
        &self,
        primary_key: &str,
        fallback_fn: F,
    ) -> Option<(Arc<str>, SubscriptionState)>
    where
        F: FnOnce() -> Option<String>,
    {
        let mut subscriptions = self.subscriptions.write();
        let removed = subscriptions
            .remove_entry(primary_key)
            .or_else(|| fallback_fn().and_then(|key| subscriptions.remove_entry(key.as_str())));
        if let Some((_, state)) = &removed {
            release_subscription_buffer(state);
        }
        shrink_subscription_map(&mut subscriptions);
        removed
    }

    /// Get a subscription by ID (cloned out of the connection map).
    pub fn get_subscription(&self, subscription_id: &str) -> Option<SubscriptionState> {
        self.subscriptions.read().get(subscription_id).cloned()
    }

    /// Iterate all subscriptions, calling `f` for each.
    pub fn for_each_subscription<F>(&self, mut f: F)
    where
        F: FnMut(&str, &SubscriptionState),
    {
        let subscriptions = self.subscriptions.read();
        for (key, value) in subscriptions.iter() {
            f(key.as_ref(), value);
        }
    }

    /// Collect info from all subscriptions, removing them in the process.
    /// Returns the collected values.
    pub fn collect_subscription_info<F, T>(&self, mut f: F) -> Vec<T>
    where
        F: FnMut(&SubscriptionState) -> T,
    {
        let mut subscriptions = self.subscriptions.write();
        let mut result = Vec::with_capacity(subscriptions.len());
        for (_, value) in subscriptions.drain() {
            release_subscription_buffer(&value);
            result.push(f(&value));
        }
        subscriptions.shrink_to_fit();
        result
    }

    /// Update snapshot_end_seq for a subscription.
    pub fn update_snapshot_end_seq(
        &self,
        subscription_id: &str,
        snapshot_end_seq: Option<VersionId>,
    ) {
        self.update_snapshot_boundaries(subscription_id, snapshot_end_seq, None);
    }

    /// Update snapshot boundaries for a subscription.
    pub fn update_snapshot_boundaries(
        &self,
        subscription_id: &str,
        snapshot_end_seq: Option<VersionId>,
        snapshot_end_commit_seq: Option<u64>,
    ) {
        if let Some(sub) = self.subscriptions.write().get_mut(subscription_id) {
            if let Some(initial_load) = sub.initial_load.as_mut() {
                initial_load.snapshot_end_seq = snapshot_end_seq;
                initial_load.snapshot_end_commit_seq = snapshot_end_commit_seq;
                initial_load.flow_control.set_snapshot_end_seq(snapshot_end_seq);
            }
        }
    }

    /// Mark initial load complete and flush buffered notifications.
    ///
    /// Clones the flow_control Arc and drops the map read lock before flushing
    /// to avoid holding the lock during channel sends.
    pub fn complete_initial_load(&self, subscription_id: &str) -> usize {
        let (flow_control, runtime_metadata) = {
            let subscriptions = self.subscriptions.read();
            match subscriptions.get(subscription_id) {
                Some(sub) => (
                    sub.initial_load
                        .as_ref()
                        .map(|initial_load| Arc::clone(&initial_load.flow_control)),
                    Arc::clone(&sub.runtime_metadata),
                ),
                None => return 0,
            }
        };
        let Some(flow_control) = flow_control else {
            return 0;
        };

        let delivery_timestamp_ms = epoch_millis();
        match flow_control.finish(&self.notification_tx) {
            Ok(sent) => {
                for _ in 0..sent {
                    runtime_metadata.record_delivery_at(delivery_timestamp_ms);
                }
                sent
            },
            Err(()) => {
                self.signal_subscription_lapse(subscription_id, &runtime_metadata);
                0
            },
        }
    }

    fn signal_subscription_lapse(
        &self,
        subscription_id: &str,
        runtime_metadata: &SubscriptionRuntimeMetadata,
    ) {
        if !runtime_metadata.claim_gap() {
            return;
        }
        let _ = self.event_tx.try_send(ConnectionEvent::SubscriptionLapsed {
            subscription_id: Arc::from(subscription_id),
        });
    }

    /// Increment current batch number for a subscription and return the new value.
    pub fn increment_batch_num(&self, subscription_id: &str) -> Option<u32> {
        if let Some(sub) = self.subscriptions.write().get_mut(subscription_id) {
            if let Some(initial_load) = sub.initial_load.as_mut() {
                initial_load.current_batch_num += 1;
                Some(initial_load.current_batch_num)
            } else {
                None
            }
        } else {
            None
        }
    }
}

fn release_subscription_buffer(state: &SubscriptionState) {
    if let Some(initial_load) = state.initial_load.as_ref() {
        initial_load.flow_control.release_buffer();
    }
}

fn shrink_subscription_map(subscriptions: &mut HashMap<Arc<str>, SubscriptionState>) {
    if subscriptions.capacity() > subscriptions.len().saturating_mul(4).max(8) {
        subscriptions.shrink_to_fit();
    }
}

/// Registration info returned when a connection is registered
pub struct ConnectionRegistration {
    pub connection_id:   ConnectionId,
    pub state:           SharedConnectionState,
    pub notification_rx: NotificationReceiver,
    pub event_rx:        EventReceiver,
}

#[cfg(test)]
mod tests {
    use kalamdb_commons::websocket::{ChangeType, SharedChangePayload};

    use super::*;

    fn make_notification(subscription_id: &str) -> Arc<WireNotification> {
        Arc::new(WireNotification {
            subscription_id: Arc::from(subscription_id),
            payload:         Arc::new(SharedChangePayload::new(
                ChangeType::Insert,
                Some(Vec::new()),
                None,
            )),
        })
    }

    #[test]
    fn test_subscription_flow_control_limits_buffer_growth() {
        let flow_control = SubscriptionFlowControl::new();

        for seq in 1..=(MAX_BUFFERED_NOTIFICATIONS_PER_SUBSCRIPTION as i64) {
            flow_control.buffer_notification(
                make_notification("sub-1"),
                Some(VersionId::try_from_i64(seq).unwrap()),
            );
        }
        flow_control.buffer_notification(
            make_notification("sub-1"),
            Some(
                VersionId::try_from_i64(MAX_BUFFERED_NOTIFICATIONS_PER_SUBSCRIPTION as i64 + 1)
                    .unwrap(),
            ),
        );

        assert!(
            flow_control.is_gapped(),
            "overflow must not keep a prefix that skips older changes"
        );
        assert!(
            flow_control.drain_buffered_notifications().is_empty(),
            "a gapped buffer is dropped instead of delivered with a hole"
        );
    }

    #[test]
    fn test_finish_sends_catchup_before_later_live_changes() {
        let flow_control = SubscriptionFlowControl::new();
        let (tx, mut rx) = mpsc::channel(4);
        flow_control.buffer_notification(
            make_notification("sub-ordered"),
            Some(VersionId::try_from_i64(3).unwrap()),
        );
        assert_eq!(flow_control.finish(&tx).expect("handoff"), 1);
        assert_eq!(
            flow_control.accept(
                make_notification("sub-ordered"),
                Some(VersionId::try_from_i64(4).unwrap()),
                &tx,
            ),
            Accept::Sent
        );
        assert_eq!(rx.try_recv().expect("prefix").subscription_id.as_ref(), "sub-ordered");
        assert!(rx.try_recv().is_ok(), "live change follows the prefix");
    }

    #[test]
    fn test_subscription_flow_control_drains_in_seq_order() {
        let flow_control = SubscriptionFlowControl::new();

        flow_control.buffer_notification(
            make_notification("sub-ordered"),
            Some(VersionId::try_from_i64(9).unwrap()),
        );
        flow_control.buffer_notification(
            make_notification("sub-ordered"),
            Some(VersionId::try_from_i64(3).unwrap()),
        );
        flow_control.buffer_notification(
            make_notification("sub-ordered"),
            Some(VersionId::try_from_i64(6).unwrap()),
        );

        let buffered = flow_control.drain_buffered_notifications();
        let seqs: Vec<_> = buffered
            .into_iter()
            .map(|item| item.seq.expect("seq should be present").as_i64())
            .collect();

        assert_eq!(seqs, vec![3, 6, 9]);
    }

    #[test]
    fn test_subscription_flow_control_release_buffer_drops_capacity() {
        let flow_control = SubscriptionFlowControl::new();
        for seq in 1..=32 {
            flow_control.buffer_notification(
                make_notification("sub-release"),
                Some(VersionId::try_from_i64(seq).unwrap()),
            );
        }

        flow_control.release_buffer();
        assert!(flow_control.drain_buffered_notifications().is_empty());
    }

    #[test]
    fn test_subscription_runtime_metadata_owns_query_and_options() {
        let first = SubscriptionRuntimeMetadata::new(
            "SELECT * FROM shared.events",
            Some(r#"{"batch_size":100}"#),
            1,
        );
        let second = SubscriptionRuntimeMetadata::new(
            "SELECT * FROM shared.events",
            Some(r#"{"batch_size":100}"#),
            2,
        );

        assert_eq!(first.query(), second.query());
        assert_eq!(first.options_json(), second.options_json());
        assert!(!Arc::ptr_eq(&first.query, &second.query));
        assert!(!Arc::ptr_eq(
            first.options_json.as_ref().expect("options should exist"),
            second.options_json.as_ref().expect("options should exist")
        ));
    }
}
