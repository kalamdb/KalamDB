//! Public API surface for flutter_rust_bridge codegen.
//!
//! Every `pub` function and type in this module is picked up by FRB and
//! gets a corresponding Dart binding generated automatically.
//!
//! ## Streaming pattern
//!
//! FRB v2 generates `StreamSink` in user boilerplate code rather than
//! exporting it from the crate.  To avoid depending on generated code,
//! live streams use an **async pull model**: call `dart_live_events_next`
//! in a loop from Dart, which internally awaits the next event from the
//! underlying `SubscriptionManager`.
//!
//! ## Connection events
//!
//! Connection lifecycle events (connect, disconnect, error, receive, send)
//! also follow the async-pull model. When `enable_connection_events` is
//! `true` in [`dart_create_client`], events are queued internally and
//! retrieved via [`dart_next_connection_event`].

use std::{
    collections::VecDeque,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};

use flutter_rust_bridge::frb;
use kalam_client::{query::models::query_param::params_from_json_values, FileRef, QueryParam};
use tokio::sync::{Mutex, Notify};

use crate::models::{
    DartAuthProvider, DartChangeEvent, DartConnectionError, DartConnectionEvent,
    DartDisconnectReason, DartFileDownload, DartFileRef, DartFileUpload, DartLiveRowsConfig,
    DartLiveRowsEvent, DartLoginResponse, DartQueryResponse, DartSubscriptionConfig,
};

const DART_INITIAL_RECONNECT_DELAY_MS: u64 = 200;
const DART_MAX_RECONNECT_DELAY_MS: u64 = 2_000;
const DART_CONNECTION_EVENT_QUEUE_CAPACITY: usize = 256;

fn parse_dart_file_ref(file_ref_json: &str) -> anyhow::Result<FileRef> {
    FileRef::from_json(file_ref_json).ok_or_else(|| anyhow::anyhow!("invalid FileRef JSON"))
}

fn parse_dart_query_params(params_json: Option<String>) -> anyhow::Result<Option<Vec<QueryParam>>> {
    let Some(json) = params_json else {
        return Ok(None);
    };
    let values: Vec<serde_json::Value> = serde_json::from_str(&json)?;
    Ok(Some(params_from_json_values(values)))
}

/// Build a FILE download URL using the canonical `link-common` FileRef model.
#[frb(sync)]
pub fn dart_file_ref_download_url(
    file_ref_json: String,
    base_url: String,
    namespace: String,
    table: String,
) -> anyhow::Result<String> {
    Ok(parse_dart_file_ref(&file_ref_json)?.download_url(&base_url, &namespace, &table))
}

/// Build a FILE download path using the canonical `link-common` FileRef model.
#[frb(sync)]
pub fn dart_file_ref_relative_url(
    file_ref_json: String,
    namespace: String,
    table: String,
) -> anyhow::Result<String> {
    Ok(parse_dart_file_ref(&file_ref_json)?.relative_url(&namespace, &table))
}

/// Return the stored filename using the canonical `link-common` FileRef model.
#[frb(sync)]
pub fn dart_file_ref_stored_name(file_ref_json: String) -> anyhow::Result<String> {
    Ok(parse_dart_file_ref(&file_ref_json)?.stored_name())
}

/// Return the storage-relative path using the canonical `link-common` FileRef model.
#[frb(sync)]
pub fn dart_file_ref_relative_path(file_ref_json: String) -> anyhow::Result<String> {
    Ok(parse_dart_file_ref(&file_ref_json)?.relative_path())
}

/// Parse a FILE column JSON value into a typed [`DartFileRef`].
#[frb(sync)]
pub fn dart_parse_file_ref(file_ref_json: String) -> anyhow::Result<DartFileRef> {
    Ok(parse_dart_file_ref(&file_ref_json)?.into())
}

/// Parse a FILE column JSON value, returning `None` for invalid input.
#[frb(sync)]
pub fn dart_try_parse_file_ref(file_ref_json: String) -> Option<DartFileRef> {
    FileRef::from_json(&file_ref_json).map(DartFileRef::from)
}

// ---------------------------------------------------------------------------
// Client wrapper
// ---------------------------------------------------------------------------

/// Opaque handle to a connected KalamDB client.
///
/// Create one via [`dart_create_client`] and pass it to query/subscribe helpers.
pub struct DartKalamClient {
    inner:          kalam_client::KalamLinkClient,
    /// Queue of connection lifecycle events (populated when event handlers are
    /// enabled). Dart pulls from this via [`dart_next_connection_event`].
    ///
    /// Uses `std::sync::Mutex` (not tokio) because the event handler callbacks
    /// are synchronous closures; tokio Mutex requires `.await` or has borrow
    /// issues with `try_lock()` inside `#[frb(sync)]` functions.
    event_queue:    Arc<std::sync::Mutex<VecDeque<DartConnectionEvent>>>,
    /// Notifier so the pull-side can await without busy-looping.
    event_notify:   Arc<Notify>,
    /// Set to `true` when [`dart_signal_dispose`] is called, causing
    /// [`dart_next_connection_event`] to return `None` immediately.
    disposed:       Arc<AtomicBool>,
    /// Whether connection event collection is enabled.
    events_enabled: bool,
}

/// Create a new KalamDB client.
///
/// * `base_url` — server URL, e.g. `"https://db.example.com"` or `"http://localhost:3000"`.
/// * `auth` — authentication method (basic, JWT, or none).
/// * `timeout_ms` — optional HTTP request timeout in milliseconds (default 30 000).
/// * `max_retries` — optional retry count for idempotent queries (default 3).
/// * `enable_connection_events` — when `true`, connection lifecycle events (connect, disconnect,
///   error, receive, send) are queued internally and can be retrieved via
///   [`dart_next_connection_event`].
/// * `disable_compression` — when `true`, the WebSocket URL includes `?compress=false` so the
///   server sends plain-text JSON frames instead of gzip-compressed binary frames. Useful during
///   development.
/// * `keepalive_interval_ms` — optional WebSocket keep-alive ping interval in milliseconds (default
///   10 000). Set to 0 to disable keep-alive pings.
/// * `ws_lazy_connect` — controls when the WebSocket connection is established. When `true` (the
///   default), the connection is deferred until the first `subscribe()` call. When `false`, the
///   connection is established eagerly. Authentication uses the same provider configured for HTTP
///   queries.
///
/// **Note:** This function intentionally omits `#[frb(sync)]` so that FRB
/// dispatches it to a worker thread via `executeNormal`. The client
/// builder may perform TLS initialisation, certificate loading, or other
/// CPU-bound work that would otherwise freeze the Flutter UI thread.
pub fn dart_create_client(
    base_url: String,
    auth: DartAuthProvider,
    timeout_ms: Option<i64>,
    max_retries: Option<i32>,
    enable_connection_events: Option<bool>,
    disable_compression: Option<bool>,
    keepalive_interval_ms: Option<i64>,
    ws_lazy_connect: Option<bool>,
) -> anyhow::Result<DartKalamClient> {
    create_client_inner(
        base_url,
        auth,
        timeout_ms,
        max_retries,
        enable_connection_events,
        disable_compression,
        keepalive_interval_ms,
        ws_lazy_connect,
    )
}

/// Internal helper that performs the actual client construction.
///
/// Extracted from [`dart_create_client`] so that the `#[frb(sync)]` macro
/// expansion does not interfere with borrow lifetimes inside the event
/// handler closures.
fn create_client_inner(
    base_url: String,
    auth: DartAuthProvider,
    timeout_ms: Option<i64>,
    max_retries: Option<i32>,
    enable_connection_events: Option<bool>,
    disable_compression: Option<bool>,
    keepalive_interval_ms: Option<i64>,
    ws_lazy_connect: Option<bool>,
) -> anyhow::Result<DartKalamClient> {
    let event_queue: Arc<std::sync::Mutex<VecDeque<DartConnectionEvent>>> =
        Arc::new(std::sync::Mutex::new(VecDeque::new()));
    let event_notify = Arc::new(Notify::new());
    let events_enabled = enable_connection_events.unwrap_or(false);

    let mut builder = kalam_client::KalamLinkClient::builder()
        .base_url(base_url)
        .auth(auth.into_native());

    if let Some(ms) = timeout_ms {
        builder = builder.timeout(std::time::Duration::from_millis(ms as u64));
    }
    if let Some(r) = max_retries {
        builder = builder.max_retries(r as u32);
    }

    builder = builder
        .connection_options(build_dart_connection_options(disable_compression, ws_lazy_connect));

    if let Some(ms) = keepalive_interval_ms {
        let mut timeouts = kalam_client::KalamLinkTimeouts::default();
        timeouts.keepalive_interval = std::time::Duration::from_millis(ms as u64);
        builder = builder.timeouts(timeouts);
    }

    // Wire connection lifecycle event handlers that push into the queue.
    if events_enabled {
        builder =
            builder.event_handlers(build_event_handlers(event_queue.clone(), event_notify.clone()));
    }

    let client = builder.build()?;
    Ok(DartKalamClient {
        inner: client,
        event_queue,
        event_notify,
        disposed: Arc::new(AtomicBool::new(false)),
        events_enabled,
    })
}

fn build_dart_connection_options(
    disable_compression: Option<bool>,
    ws_lazy_connect: Option<bool>,
) -> kalam_client::ConnectionOptions {
    let mut conn_opts = kalam_client::ConnectionOptions::default();

    // Favor the smaller binary wire format for Dart subscriptions by default.
    conn_opts.protocol.serialization = kalam_client::models::SerializationType::MessagePack;
    // Mobile apps resume and reconnect frequently. Favor a faster first retry
    // than the generic SDK defaults while keeping exponential backoff.
    conn_opts.reconnect_delay_ms = DART_INITIAL_RECONNECT_DELAY_MS;
    conn_opts.max_reconnect_delay_ms = DART_MAX_RECONNECT_DELAY_MS;

    if disable_compression.unwrap_or(false) {
        conn_opts.disable_compression = true;
    }

    // ws_lazy_connect defaults to true in ConnectionOptions::default().
    // Only override when the caller explicitly passes false.
    if let Some(lazy) = ws_lazy_connect {
        conn_opts.ws_lazy_connect = lazy;
    }

    conn_opts
}

/// Update the authentication credentials on a live client.
///
/// This is used to implement refresh-token flows from Dart:
/// before re-subscribing or on a schedule, Dart calls the user's
/// `authProvider` callback, receives a new [Auth], and calls this
/// function to push updated credentials into the Rust client.
///
/// The new credentials take effect on the next `subscribe()` call.
///
/// Uses `&self` (read lock) instead of `&mut self` (write lock) to avoid
/// deadlocking with [`dart_next_connection_event`], which holds a long-lived
/// read lock on the `RustAutoOpaque<DartKalamClient>` while awaiting events.
///
/// **Note:** `#[frb(sync)]` is intentionally removed so the lock acquisition
/// runs on a worker thread instead of the Flutter UI thread.
pub fn dart_update_auth(client: &DartKalamClient, auth: DartAuthProvider) -> anyhow::Result<()> {
    client.inner.update_shared_auth(auth.into_native());
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::{collections::VecDeque, sync::Arc};

    use kalam_client::models::SerializationType;
    use tokio::sync::Notify;

    use super::{
        build_dart_connection_options, dart_file_ref_download_url, dart_file_ref_relative_path,
        dart_file_ref_relative_url, dart_file_ref_stored_name, push_connection_event,
        push_debug_connection_event, DART_CONNECTION_EVENT_QUEUE_CAPACITY,
        DART_INITIAL_RECONNECT_DELAY_MS, DART_MAX_RECONNECT_DELAY_MS,
    };
    use crate::models::DartConnectionEvent;

    #[test]
    fn dart_connection_options_default_to_msgpack() {
        let options = build_dart_connection_options(None, None);

        assert_eq!(options.protocol.serialization, SerializationType::MessagePack);
        assert!(options.ws_lazy_connect);
        assert!(!options.disable_compression);
        assert_eq!(options.reconnect_delay_ms, DART_INITIAL_RECONNECT_DELAY_MS);
        assert_eq!(options.max_reconnect_delay_ms, DART_MAX_RECONNECT_DELAY_MS);
    }

    #[test]
    fn dart_connection_options_preserve_explicit_flags() {
        let options = build_dart_connection_options(Some(true), Some(false));

        assert_eq!(options.protocol.serialization, SerializationType::MessagePack);
        assert!(!options.ws_lazy_connect);
        assert!(options.disable_compression);
        assert_eq!(options.reconnect_delay_ms, DART_INITIAL_RECONNECT_DELAY_MS);
        assert_eq!(options.max_reconnect_delay_ms, DART_MAX_RECONNECT_DELAY_MS);
    }

    #[test]
    fn dart_debug_connection_events_are_bounded() {
        let queue = Arc::new(std::sync::Mutex::new(VecDeque::new()));
        let notify = Arc::new(Notify::new());

        for index in 0..(DART_CONNECTION_EVENT_QUEUE_CAPACITY + 10) {
            push_debug_connection_event(&queue, &notify, || DartConnectionEvent::Receive {
                message: index.to_string(),
            });
        }

        assert_eq!(queue.lock().unwrap().len(), DART_CONNECTION_EVENT_QUEUE_CAPACITY);
    }

    #[test]
    fn dart_lifecycle_connection_events_keep_latest_when_full() {
        let queue = Arc::new(std::sync::Mutex::new(VecDeque::new()));
        let notify = Arc::new(Notify::new());

        for _ in 0..DART_CONNECTION_EVENT_QUEUE_CAPACITY {
            push_connection_event(&queue, &notify, DartConnectionEvent::Connect);
        }
        push_connection_event(
            &queue,
            &notify,
            DartConnectionEvent::Error {
                error: crate::models::DartConnectionError {
                    message:     "latest".to_string(),
                    recoverable: true,
                },
            },
        );

        let guard = queue.lock().unwrap();
        assert_eq!(guard.len(), DART_CONNECTION_EVENT_QUEUE_CAPACITY);
        assert!(matches!(guard.back(), Some(DartConnectionEvent::Error { .. })));
    }

    #[test]
    fn dart_file_ref_helpers_delegate_to_link_common() {
        let file_ref_json = r#"{"id":"12345","sub":"f0001","name":"My Avatar.png","size":4096,"mime":"image/png","sha256":"abc123"}"#.to_string();

        assert_eq!(
            dart_file_ref_download_url(
                file_ref_json.clone(),
                "http://localhost:2900/".to_string(),
                "app".to_string(),
                "users".to_string(),
            )
            .unwrap(),
            "http://localhost:2900/v1/files/app/users/f0001/12345-my-avatar.png"
        );
        assert_eq!(
            dart_file_ref_relative_url(
                file_ref_json.clone(),
                "app".to_string(),
                "users".to_string(),
            )
            .unwrap(),
            "/v1/files/app/users/f0001/12345-my-avatar.png"
        );
        assert_eq!(
            dart_file_ref_stored_name(file_ref_json.clone()).unwrap(),
            "12345-my-avatar.png"
        );
        assert_eq!(
            dart_file_ref_relative_path(file_ref_json).unwrap(),
            "f0001/12345-my-avatar.png"
        );
    }
}

/// Build [`EventHandlers`](kalam_client::EventHandlers) that push events into
/// a shared queue and notify waiters.
fn build_event_handlers(
    queue: Arc<std::sync::Mutex<VecDeque<DartConnectionEvent>>>,
    notify: Arc<Notify>,
) -> kalam_client::EventHandlers {
    let mut handlers = kalam_client::EventHandlers::new();

    // on_connect
    {
        let q = queue.clone();
        let n = notify.clone();
        handlers = handlers.on_connect(move || {
            push_connection_event(&q, &n, DartConnectionEvent::Connect);
        });
    }

    // on_disconnect
    {
        let q = queue.clone();
        let n = notify.clone();
        handlers = handlers.on_disconnect(move |reason| {
            let event = DartConnectionEvent::Disconnect {
                reason: DartDisconnectReason {
                    message: reason.message,
                    code:    reason.code.map(|c| c as i32),
                },
            };
            push_connection_event(&q, &n, event);
        });
    }

    // on_error
    {
        let q = queue.clone();
        let n = notify.clone();
        handlers = handlers.on_error(move |error| {
            let event = DartConnectionEvent::Error {
                error: DartConnectionError {
                    message:     error.message,
                    recoverable: error.recoverable,
                },
            };
            push_connection_event(&q, &n, event);
        });
    }

    // on_receive (debug)
    {
        let q = queue.clone();
        let n = notify.clone();
        handlers = handlers.on_receive(move |msg: &str| {
            push_debug_connection_event(&q, &n, || DartConnectionEvent::Receive {
                message: msg.to_owned(),
            });
        });
    }

    // on_send (debug)
    {
        let q = queue;
        let n = notify;
        handlers = handlers.on_send(move |msg: &str| {
            push_debug_connection_event(&q, &n, || DartConnectionEvent::Send {
                message: msg.to_owned(),
            });
        });
    }

    handlers
}

fn push_connection_event(
    queue: &Arc<std::sync::Mutex<VecDeque<DartConnectionEvent>>>,
    notify: &Arc<Notify>,
    event: DartConnectionEvent,
) {
    if let Ok(mut guard) = queue.lock() {
        if guard.len() >= DART_CONNECTION_EVENT_QUEUE_CAPACITY {
            guard.pop_front();
        }
        guard.push_back(event);
        notify.notify_one();
    }
}

fn push_debug_connection_event(
    queue: &Arc<std::sync::Mutex<VecDeque<DartConnectionEvent>>>,
    notify: &Arc<Notify>,
    event: impl FnOnce() -> DartConnectionEvent,
) {
    if let Ok(mut guard) = queue.lock() {
        if guard.len() >= DART_CONNECTION_EVENT_QUEUE_CAPACITY {
            return;
        }
        guard.push_back(event());
        notify.notify_one();
    }
}

// ---------------------------------------------------------------------------
// Query
// ---------------------------------------------------------------------------

/// Execute a SQL query, optionally with parameters and namespace.
///
/// * `params_json` — JSON-encoded array of parameter values, e.g. `'["val1", 42]'`. Pass `null` for
///   no parameters.
/// * `namespace` — optional namespace context for unqualified table names.
pub async fn dart_execute_query(
    client: &DartKalamClient,
    sql: String,
    params_json: Option<String>,
    namespace: Option<String>,
) -> anyhow::Result<DartQueryResponse> {
    let params = parse_dart_query_params(params_json)?;
    let response = client.inner.execute_query(&sql, None, params, namespace.as_deref()).await?;
    Ok(DartQueryResponse::from(response))
}

/// Execute a SQL query with multipart file uploads.
///
/// Use `FILE("placeholder")` in SQL and pass matching [`DartFileUpload`] items.
pub async fn dart_execute_query_with_files(
    client: &DartKalamClient,
    sql: String,
    files: Vec<DartFileUpload>,
    params_json: Option<String>,
    namespace: Option<String>,
) -> anyhow::Result<DartQueryResponse> {
    let params = parse_dart_query_params(params_json)?;
    let uploads = files.into_iter().map(Into::into).collect();
    let response = client
        .inner
        .execute_query(&sql, Some(uploads), params, namespace.as_deref())
        .await?;
    Ok(DartQueryResponse::from(response))
}

/// Download file bytes for a [`DartFileRef`] returned from a query row.
pub async fn dart_download_file(
    client: &DartKalamClient,
    file_ref: DartFileRef,
    namespace: String,
    table: String,
    target_user_id: Option<String>,
) -> anyhow::Result<DartFileDownload> {
    let file_ref = FileRef::from(file_ref);
    let download = client
        .inner
        .download_file(&file_ref, &namespace, &table, target_user_id.as_deref())
        .await?;
    Ok(download.into())
}

// ---------------------------------------------------------------------------
// Auth endpoints
// ---------------------------------------------------------------------------

/// Log in with user and password. Returns tokens and user info.
pub async fn dart_login(
    client: &DartKalamClient,
    user: String,
    password: String,
) -> anyhow::Result<DartLoginResponse> {
    let response = client.inner.login(&user, &password).await?;
    Ok(DartLoginResponse::from(response))
}

/// Refresh an access token using a refresh token.
pub async fn dart_refresh_token(
    client: &DartKalamClient,
    refresh_token: String,
) -> anyhow::Result<DartLoginResponse> {
    let response = client.inner.refresh_access_token(&refresh_token).await?;
    Ok(DartLoginResponse::from(response))
}

// ---------------------------------------------------------------------------
// Connection events (async pull model)
// ---------------------------------------------------------------------------

/// Pull the next connection lifecycle event.
///
/// Returns `None` when event collection is disabled or the client is dropped.
/// Dart should call this in a loop:
///
/// ```dart
/// while (true) {
///   final event = await dartNextConnectionEvent(client: client);
///   if (event == null) break;
///   // handle event ...
/// }
/// ```
pub async fn dart_next_connection_event(
    client: &DartKalamClient,
) -> anyhow::Result<Option<DartConnectionEvent>> {
    if !client.events_enabled || client.disposed.load(Ordering::Relaxed) {
        return Ok(None);
    }

    loop {
        // Fast path: check queue first
        {
            let mut queue = client
                .event_queue
                .lock()
                .map_err(|e| anyhow::anyhow!("event queue lock poisoned: {e}"))?;
            if let Some(event) = queue.pop_front() {
                return Ok(Some(event));
            }
        }

        // Slow path: wait for notification, then drain one event
        client.event_notify.notified().await;

        // Re-check disposed after wake-up — dart_signal_dispose may have
        // notified us to unblock.
        if client.disposed.load(Ordering::Relaxed) {
            return Ok(None);
        }

        // Re-check queue; if empty (spurious wake / permit consumed with no
        // matching push), loop back to wait again rather than returning None
        // (which would stop the Dart event pump).
        {
            let mut queue = client
                .event_queue
                .lock()
                .map_err(|e| anyhow::anyhow!("event queue lock poisoned: {e}"))?;
            if let Some(event) = queue.pop_front() {
                return Ok(Some(event));
            }
        }
        // Queue was empty after wake-up (spurious permit) — loop back
    }
}

/// Signal the event pump to stop.
///
/// Call this before awaiting the pump future in `dispose()` so that
/// [`dart_next_connection_event`] returns `None` immediately instead
/// of blocking forever when the event queue is empty.
#[frb(sync)]
pub fn dart_signal_dispose(client: &DartKalamClient) {
    client.disposed.store(true, Ordering::Relaxed);
    client.event_notify.notify_one();
}

/// Check whether connection event collection is enabled for this client.
#[frb(sync)]
pub fn dart_connection_events_enabled(client: &DartKalamClient) -> bool {
    client.events_enabled
}

// ---------------------------------------------------------------------------
// Live streams (async pull model)
// ---------------------------------------------------------------------------

/// Opaque handle to an active low-level live event stream.
///
/// On the Dart side, call [`dart_live_events_next`] in a loop to pull events.
/// The loop ends when `None` is returned (stream closed).
pub struct DartLiveEventsSubscription {
    inner:  Arc<Mutex<kalam_client::SubscriptionManager>>,
    sub_id: String,
}

/// Opaque handle to an active high-level live-row subscription.
pub struct DartLiveRowsSubscription {
    inner:  Arc<Mutex<kalam_client::LiveRowsSubscription>>,
    sub_id: String,
}

// ---------------------------------------------------------------------------
// Shared connection lifecycle
// ---------------------------------------------------------------------------

/// Establish a shared WebSocket connection.
///
/// After this call, subsequent live calls will multiplex over a single
/// WebSocket instead of opening one per stream. The connection handles
/// auto-reconnection and re-subscription.
///
/// Calling this when already connected is a no-op.
pub async fn dart_connect(client: &DartKalamClient) -> anyhow::Result<()> {
    client.inner.connect().await?;
    Ok(())
}

/// Disconnect the shared WebSocket connection.
///
/// All active subscriptions will be unsubscribed and the background
/// task shut down.  New subscriptions will fall back to per-subscription
/// connections until [`dart_connect`] is called again.
pub async fn dart_disconnect(client: &DartKalamClient) -> anyhow::Result<()> {
    client.inner.disconnect().await;
    Ok(())
}

/// Check whether a shared connection is currently open and authenticated.
pub async fn dart_is_connected(client: &DartKalamClient) -> anyhow::Result<bool> {
    Ok(client.inner.is_connected().await)
}

/// Cancel a subscription by ID on the shared connection.
///
/// This sends an explicit unsubscribe command that:
/// 1. Removes the subscription from the client-side map
/// 2. Sends an unsubscribe message to the server
/// 3. Drops the event channel, causing any blocking [`dart_live_events_next`] call to return `None`
///
/// Unlike [`dart_live_events_close`], this does **not** require the
/// `DartLiveEventsSubscription` mutex, so it can be called safely even while
/// [`dart_live_events_next`] is blocked.  Use this from Dart's
/// `StreamController.onCancel` to immediately release server-side
/// resources.
pub async fn dart_cancel_subscription(
    client: &DartKalamClient,
    subscription_id: String,
) -> anyhow::Result<()> {
    client.inner.cancel_subscription(&subscription_id).await?;
    Ok(())
}

/// Create a low-level live event stream.
///
/// * `sql` — the SELECT query to subscribe to.
/// * `config` — optional advanced configuration (batch size, etc.).
///
/// Returns an opaque [`DartLiveEventsSubscription`] handle. Use
/// [`dart_live_events_next`] to pull events and
/// [`dart_live_events_close`] to tear down.
pub async fn dart_live_events_subscribe(
    client: &DartKalamClient,
    sql: String,
    config: Option<DartSubscriptionConfig>,
) -> anyhow::Result<DartLiveEventsSubscription> {
    let sub = if let Some(cfg) = config {
        let mut native_cfg = cfg.into_native();
        native_cfg.sql = sql;
        client.inner.live_events_with_config(native_cfg).await?
    } else {
        client.inner.live_events(&sql).await?
    };

    let sub_id = sub.subscription_id().to_owned();
    Ok(DartLiveEventsSubscription {
        inner: Arc::new(Mutex::new(sub)),
        sub_id,
    })
}

/// Pull the next change event from a subscription.
///
/// Returns `None` when the subscription has ended (server closed or
/// [`dart_live_events_close`] was called).
pub async fn dart_live_events_next(
    subscription: &DartLiveEventsSubscription,
) -> anyhow::Result<Option<DartChangeEvent>> {
    let mut sub = subscription.inner.lock().await;
    match sub.next().await {
        Some(Ok(event)) => Ok(Some(DartChangeEvent::from(event))),
        Some(Err(e)) => Err(e.into()),
        None => Ok(None),
    }
}

/// Acknowledge progress after the consumer durably commits an event.
pub async fn dart_live_events_ack(
    subscription: &DartLiveEventsSubscription,
    seq_id: i64,
) -> anyhow::Result<()> {
    let mut sub = subscription.inner.lock().await;
    sub.acknowledge(kalam_client::SeqId::from_i64(seq_id)).await?;
    Ok(())
}

/// Close a subscription and release server-side resources.
pub async fn dart_live_events_close(
    subscription: &DartLiveEventsSubscription,
) -> anyhow::Result<()> {
    let mut sub = subscription.inner.lock().await;
    sub.close().await?;
    Ok(())
}

/// Get the server-assigned subscription ID.
#[frb(sync)]
pub fn dart_live_events_id(subscription: &DartLiveEventsSubscription) -> String {
    subscription.sub_id.clone()
}

/// Create a materialized live-query subscription.
pub async fn dart_live_subscribe(
    client: &DartKalamClient,
    sql: String,
    config: Option<DartSubscriptionConfig>,
    live_config: Option<DartLiveRowsConfig>,
) -> anyhow::Result<DartLiveRowsSubscription> {
    let native_live_config = live_config
        .unwrap_or(DartLiveRowsConfig {
            limit:       None,
            key_columns: None,
        })
        .into_native();
    let sub = if let Some(cfg) = config {
        let mut native_cfg = cfg.into_native();
        native_cfg.sql = sql;
        client.inner.live_with_config(native_cfg, native_live_config).await?
    } else {
        let native_cfg = kalam_client::SubscriptionConfig::new(
            format!(
                "dart-live-{}",
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_nanos()
            ),
            sql,
        );
        client.inner.live_with_config(native_cfg, native_live_config).await?
    };

    let sub_id = sub.subscription_id().to_owned();
    Ok(DartLiveRowsSubscription {
        inner: Arc::new(Mutex::new(sub)),
        sub_id,
    })
}

/// Pull the next materialized live-row event from a subscription.
pub async fn dart_live_next(
    subscription: &DartLiveRowsSubscription,
) -> anyhow::Result<Option<DartLiveRowsEvent>> {
    let mut sub = subscription.inner.lock().await;
    match sub.next().await {
        Some(Ok(event)) => Ok(Some(DartLiveRowsEvent::from(event))),
        Some(Err(e)) => Err(e.into()),
        None => Ok(None),
    }
}

/// Close a materialized live-row subscription.
pub async fn dart_live_close(subscription: &DartLiveRowsSubscription) -> anyhow::Result<()> {
    let mut sub = subscription.inner.lock().await;
    sub.close().await?;
    Ok(())
}

/// Get the server-assigned subscription ID for a live-row subscription.
#[frb(sync)]
pub fn dart_live_id(subscription: &DartLiveRowsSubscription) -> String {
    subscription.sub_id.clone()
}

/// List all active subscriptions on the shared connection.
///
/// Returns a snapshot of each subscription's metadata including the
/// subscription ID, SQL query, last received sequence ID, and timestamps.
pub async fn dart_list_subscriptions(
    client: &DartKalamClient,
) -> anyhow::Result<Vec<crate::models::DartSubscriptionInfo>> {
    let infos = client.inner.subscriptions().await;
    Ok(infos.into_iter().map(Into::into).collect())
}
