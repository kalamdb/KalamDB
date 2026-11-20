use std::{
    cell::RefCell,
    collections::HashMap,
    rc::Rc,
    sync::atomic::{AtomicU64, Ordering},
};

static SUBSCRIPTION_COUNTER: AtomicU64 = AtomicU64::new(0);

use link_common::{
    models::{
        ChangeEvent, ClientMessage, SerializationType, ServerMessage, SubscriptionOptions,
        SubscriptionRequest,
    },
    realtime::{resume_acknowledged, SubscriptionRecovery},
    VersionDomain,
};

const MAX_LEARNED_DOMAINS: usize = 1_024;
use serde::Serialize;
use wasm_bindgen::prelude::*;
use web_sys::WebSocket;

use super::{
    helpers::{create_promise, send_ws_message, serialize_json_to_js_value, subscription_hash},
    reconnect::BrowserConnection,
    state::{
        callback_payload, filter_subscription_event, track_subscription_checkpoint,
        SubscriptionCallbackMode, SubscriptionState, WasmLiveRowsOptions,
    },
    validation::{
        quote_table_name, validate_column_name, validate_row_id, validate_sql_identifier,
    },
    wasm_auth::{perform_basic_login, resolve_auth_provider, WasmAuthProvider},
    wasm_debug_log,
};

#[derive(Serialize)]
struct BorrowedQueryRequest<'a> {
    sql:          &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    params:       Option<&'a [serde_json::Value]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    namespace_id: Option<&'a str>,
}

/// WASM-compatible KalamDB client with auto-reconnection support
///
/// Supports multiple authentication methods:
/// - Basic Auth: `new KalamClient(url, username, password)`
/// - JWT Token: `KalamClient.withJwt(url, token)`
/// - Anonymous: `KalamClient.anonymous(url)`
/// - Dynamic Auth: `KalamClient.anonymous(url)` + `setAuthProvider(async () => ({ jwt: { token }
///   }))`
///
/// # Example (JavaScript)
/// ```js
/// import init, { KalamClient, KalamClientWithJwt, KalamClientAnonymous } from './pkg/kalam_client.js';
///
/// await init();
///
/// // Basic Auth (username/password)
/// const client = new KalamClient(
///   "http://localhost:2900",
///   "username",
///   "password"
/// );
///
/// // JWT Token Auth
/// const jwtClient = KalamClient.withJwt(
///   "http://localhost:2900",
///   "eyJhbGciOiJIUzI1NiIs..."
/// );
///
/// // Anonymous (localhost bypass)
/// const anonClient = KalamClient.anonymous("http://localhost:2900");
///
/// // Dynamic async auth provider (e.g. refresh token flow)
/// const dynClient = KalamClient.anonymous("http://localhost:2900");
/// dynClient.setAuthProvider(async () => {
///   const token = await myApp.getOrRefreshToken();
///   return { jwt: { token } };
/// });
///
/// // Configure auto-reconnect (enabled by default)
/// client.setAutoReconnect(true);
/// client.setReconnectDelay(1000, 30000);
///
/// // WebSocket connects automatically on first live call (wsLazyConnect=true by default)
/// const subId = await client.liveEvents(
///   "SELECT * FROM chat.messages",
///   JSON.stringify({
///     batch_size: 100,
///     include_old_values: true
///   }),
///   (event) => console.log('Change:', event)
/// );
/// ```
#[wasm_bindgen]
pub struct KalamClient {
    realtime:          Rc<BrowserConnection>,
    default_namespace: RefCell<Option<String>>,
}

impl Drop for KalamClient {
    fn drop(&mut self) {
        self.realtime.disconnect(false);
    }
}

impl KalamClient {
    async fn register_subscription(
        &self,
        sql: String,
        subscription_options: SubscriptionOptions,
        callback: js_sys::Function,
        callback_mode: SubscriptionCallbackMode,
    ) -> Result<String, JsValue> {
        self.realtime.connect().await?;

        let mut subscription_options = subscription_options;
        if subscription_options.from.is_some() && subscription_options.version_domain.is_none() {
            if let Some(domain) = self.realtime.learned_domains.borrow().get(&sql).cloned() {
                subscription_options.version_domain = Some(domain);
            }
        }

        let subscription_id = format!(
            "sub-{:x}-{}",
            subscription_hash(&sql),
            SUBSCRIPTION_COUNTER.fetch_add(1, Ordering::Relaxed),
        );
        let (subscribe_promise, subscribe_resolve, subscribe_reject) = create_promise();

        self.realtime.subscriptions.borrow_mut().insert(
            subscription_id.clone(),
            SubscriptionState {
                sql: sql.clone(),
                options: subscription_options.clone(),
                callback,
                last_seq_id: None,
                batch_seq_id: None,
                resume_pending: false,
                pending_subscribe_resolve: Some(subscribe_resolve),
                pending_subscribe_reject: Some(subscribe_reject),
                awaiting_initial_response: true,
                callback_mode,
            },
        );

        if let Some(ws) = self.realtime.socket().as_ref() {
            let subscribe_msg = ClientMessage::Subscribe {
                subscription: SubscriptionRequest {
                    id:      subscription_id.clone(),
                    sql:     sql.clone(),
                    options: Some(subscription_options),
                },
            };
            wasm_debug_log!(&format!(
                "KalamClient: Sending subscribe request - id: {}, sql: {}",
                subscription_id, sql
            ));
            if let Err(error) = send_ws_message_traced(
                ws,
                &subscribe_msg,
                self.realtime.serialization.get(),
                &self.realtime.on_send,
            ) {
                self.realtime.subscriptions.borrow_mut().remove(&subscription_id);
                return Err(error);
            }
        } else {
            self.realtime.subscriptions.borrow_mut().remove(&subscription_id);
            return Err(JsValue::from_str(
                "WebSocket connection is unavailable for subscription registration",
            ));
        }

        if let Err(error) = self.realtime.await_subscription(subscribe_promise).await {
            let _ = self.unsubscribe(subscription_id.clone()).await;
            return Err(error);
        }
        wasm_debug_log!(&format!("KalamClient: Subscribed with ID: {}", subscription_id));
        Ok(subscription_id)
    }

    fn new_with_auth(url: String, auth: WasmAuthProvider) -> KalamClient {
        KalamClient {
            realtime:          Rc::new(BrowserConnection::new(url, auth)),
            default_namespace: RefCell::default(),
        }
    }
}

fn emit_ws_send(on_send_cb: &Rc<RefCell<Option<js_sys::Function>>>, msg: &ClientMessage) {
    let Some(cb) = on_send_cb.borrow().as_ref().cloned() else {
        return;
    };
    if let Ok(json) = serde_json::to_string(msg) {
        let _ = cb.call1(&JsValue::NULL, &JsValue::from_str(&json));
    }
}

pub(crate) fn send_ws_message_traced(
    ws: &WebSocket,
    msg: &ClientMessage,
    serialization: SerializationType,
    on_send_cb: &Rc<RefCell<Option<js_sys::Function>>>,
) -> Result<(), JsValue> {
    send_ws_message(ws, msg, serialization)?;
    emit_ws_send(on_send_cb, msg);
    Ok(())
}

fn next_batch_message(
    subscription_id: &str,
    last_seq_id: Option<link_common::VersionId>,
) -> ClientMessage {
    ClientMessage::NextBatch {
        subscription_id: subscription_id.to_string(),
        last_seq_id,
    }
}

pub(crate) fn send_next_batch_traced(
    ws: &WebSocket,
    subscription_id: &str,
    last_seq_id: Option<link_common::VersionId>,
    serialization: SerializationType,
    on_send_cb: &Rc<RefCell<Option<js_sys::Function>>>,
) -> Result<(), JsValue> {
    let msg = next_batch_message(subscription_id, last_seq_id);
    send_ws_message_traced(ws, &msg, serialization, on_send_cb)
}

fn normalize_default_namespace(namespace: Option<String>) -> Option<String> {
    namespace.and_then(|value| {
        let trimmed = value.trim();
        (!trimmed.is_empty()).then(|| trimmed.to_string())
    })
}

fn quote_identifier(identifier: &str) -> String {
    format!("\"{}\"", identifier.replace('"', "\"\""))
}

fn find_subscription_from_clause(sql: &str) -> Option<usize> {
    let chars: Vec<(usize, char)> = sql.char_indices().collect();
    let mut in_single_quotes = false;
    let mut in_double_quotes = false;
    let mut i = 0;

    while i < chars.len() {
        let (index, ch) = chars[i];
        match ch {
            '\'' if !in_double_quotes => in_single_quotes = !in_single_quotes,
            '"' if !in_single_quotes => in_double_quotes = !in_double_quotes,
            _ if !in_single_quotes && !in_double_quotes => {
                let candidate = &sql[index..];
                if candidate.len() >= 4
                    && candidate[..4].eq_ignore_ascii_case("from")
                    && candidate[4..].chars().next().map_or(true, char::is_whitespace)
                {
                    return Some(index + 4);
                }
            },
            _ => {},
        }
        i += 1;
    }

    None
}

fn find_subscription_relation_end(sql: &str, relation_start: usize) -> usize {
    let mut in_single_quotes = false;
    let mut in_double_quotes = false;

    for (index, ch) in sql[relation_start..].char_indices() {
        match ch {
            '\'' if !in_double_quotes => in_single_quotes = !in_single_quotes,
            '"' if !in_single_quotes => in_double_quotes = !in_double_quotes,
            _ if !in_single_quotes && !in_double_quotes && ch.is_whitespace() => {
                return relation_start + index;
            },
            _ => {},
        }
    }

    sql.len()
}

fn split_identifier_parts(identifier: &str) -> Result<Vec<String>, ()> {
    let mut parts = Vec::new();
    let mut current = String::new();
    let mut in_double_quotes = false;

    for ch in identifier.chars() {
        match ch {
            '"' => {
                in_double_quotes = !in_double_quotes;
                current.push(ch);
            },
            '.' if !in_double_quotes => {
                if current.trim().is_empty() {
                    return Err(());
                }
                parts.push(current.trim().to_string());
                current.clear();
            },
            _ => current.push(ch),
        }
    }

    if in_double_quotes || current.trim().is_empty() {
        return Err(());
    }

    parts.push(current.trim().to_string());
    Ok(parts)
}

fn qualify_subscription_sql(sql: &str, default_namespace: Option<&str>) -> String {
    let Some(default_namespace) = default_namespace else {
        return sql.trim().trim_end_matches(';').trim().to_string();
    };

    let trimmed = sql.trim().trim_end_matches(';').trim();
    if trimmed.is_empty() {
        return String::new();
    }

    let Some(from_idx) = find_subscription_from_clause(trimmed) else {
        return trimmed.to_string();
    };

    let relation_start = trimmed[from_idx..]
        .char_indices()
        .find_map(|(offset, ch)| (!ch.is_whitespace()).then_some(from_idx + offset));
    let Some(relation_start) = relation_start else {
        return trimmed.to_string();
    };

    let relation_end = find_subscription_relation_end(trimmed, relation_start);
    let relation = trimmed[relation_start..relation_end].trim();
    let Ok(parts) = split_identifier_parts(relation) else {
        return trimmed.to_string();
    };

    if parts.len() != 1 {
        return trimmed.to_string();
    }

    let qualified_relation = format!("{}.{}", quote_identifier(default_namespace), relation);
    format!(
        "{}{}{}",
        &trimmed[..relation_start],
        qualified_relation,
        &trimmed[relation_end..]
    )
}

pub(crate) fn reject_pending_subscriptions(
    subscriptions: &Rc<RefCell<HashMap<String, SubscriptionState>>>,
    message: &str,
) {
    let mut pending_rejects = Vec::new();

    {
        let mut subs = subscriptions.borrow_mut();
        let pending_ids: Vec<String> = subs
            .iter()
            .filter_map(|(id, state)| {
                if state.awaiting_initial_response {
                    Some(id.clone())
                } else {
                    None
                }
            })
            .collect();

        for id in pending_ids {
            if let Some(mut state) = subs.remove(&id) {
                state.awaiting_initial_response = false;
                state.pending_subscribe_resolve = None;
                if let Some(reject) = state.pending_subscribe_reject.take() {
                    pending_rejects.push(reject);
                }
            }
        }
    }

    for reject in pending_rejects {
        let _ = reject.call1(&JsValue::NULL, &JsValue::from_str(message));
    }
}

pub(crate) struct SubscriptionDispatch {
    pub(crate) recovery: Option<SubscriptionRequest>,
    callback:            Option<js_sys::Function>,
    payload:             Option<String>,
    resolve_subscribe:   Option<js_sys::Function>,
    reject_subscribe:    Option<(js_sys::Function, String)>,
    next_batch:          Option<(String, Option<link_common::VersionId>)>,
}

impl SubscriptionDispatch {
    pub(crate) fn invoke(self) -> Option<(String, Option<link_common::VersionId>)> {
        if let Some(cb) = self.callback {
            if let Some(payload) = self.payload {
                let _ = cb.call1(&JsValue::NULL, &JsValue::from_str(&payload));
            }
        }
        if let Some(resolve) = self.resolve_subscribe {
            let _ = resolve.call0(&JsValue::NULL);
        }
        if let Some((reject, reason)) = self.reject_subscribe {
            let _ = reject.call1(&JsValue::NULL, &JsValue::from_str(&reason));
        }
        self.next_batch
    }
}

fn subscription_id_from_server_message(event: &ServerMessage) -> Option<&str> {
    match event {
        ServerMessage::SubscriptionAck {
            subscription_id,
            total_rows: _total_rows,
            ..
        } => {
            wasm_debug_log!(&format!(
                "KalamClient: Parsed SubscriptionAck - id: {}, total_rows: {}",
                subscription_id, _total_rows
            ));
            Some(subscription_id.as_str())
        },
        ServerMessage::InitialDataBatch {
            subscription_id,
            batch_control: _batch_control,
            rows: _rows,
        } => {
            wasm_debug_log!(&format!(
                "KalamClient: Parsed InitialDataBatch - id: {}, rows: {}, status: {:?}",
                subscription_id,
                _rows.len(),
                _batch_control.status
            ));
            Some(subscription_id.as_str())
        },
        ServerMessage::Change {
            subscription_id,
            change_type: _change_type,
            rows: _rows,
            old_values: _,
        } => {
            wasm_debug_log!(&format!(
                "KalamClient: Parsed Change - id: {}, type: {:?}, rows: {:?}",
                subscription_id,
                _change_type,
                _rows.as_ref().map(|value| value.len())
            ));
            Some(subscription_id.as_str())
        },
        ServerMessage::Error {
            subscription_id,
            code: _code,
            message: _message,
            ..
        } => {
            wasm_debug_log!(&format!(
                "KalamClient: Parsed Error - id: {}, code: {}, msg: {}",
                subscription_id, _code, _message
            ));
            Some(subscription_id.as_str())
        },
        _ => None,
    }
}

fn resolve_subscription_key<'a>(
    subscription_id: &'a str,
    subscriptions: &HashMap<String, SubscriptionState>,
) -> Option<&'a str> {
    // The server echoes the exact subscription_id the client sent, so an exact
    // match is always correct. The previous ends_with() fallback was unsafe with
    // multiple concurrent subscriptions because it could match the wrong entry.
    if subscriptions.contains_key(subscription_id) {
        Some(subscription_id)
    } else {
        None
    }
}

fn remember_learned_domain(
    learned_domains: &RefCell<HashMap<String, VersionDomain>>,
    sql: &str,
    domain: VersionDomain,
) {
    let mut domains = learned_domains.borrow_mut();
    if !domains.contains_key(sql) && domains.len() >= MAX_LEARNED_DOMAINS {
        if let Some(evicted) = domains.keys().next().cloned() {
            domains.remove(&evicted);
        }
    }
    domains.insert(sql.to_string(), domain);
}

pub(crate) fn dispatch_subscription_server_message(
    subscriptions: &Rc<RefCell<HashMap<String, SubscriptionState>>>,
    learned_domains: &RefCell<HashMap<String, VersionDomain>>,
    event: &ServerMessage,
) -> Option<SubscriptionDispatch> {
    let subscription_id = subscription_id_from_server_message(event)?;
    let matched_key = {
        let subs = subscriptions.borrow();
        wasm_debug_log!(&format!(
            "KalamClient: Looking for callback for subscription_id: {} (registered subs: {})",
            subscription_id,
            subs.len()
        ));
        resolve_subscription_key(subscription_id, &subs)
    };

    let Some(client_id) = matched_key else {
        wasm_debug_log!(&format!(
            "KalamClient: No callback found for subscription_id: {}",
            subscription_id
        ));
        return None;
    };

    let mut callback = None;
    let mut payload = None;
    let mut resolve_subscribe = None;
    let mut reject_subscribe = None;
    let mut next_batch = None;
    let mut remove_state = false;
    let mut recovery = None;

    {
        let mut subs = subscriptions.borrow_mut();
        if let Some(state) = subs.get_mut(client_id) {
            let recovery_action = match event {
                ServerMessage::Error { code, message, .. } if !state.awaiting_initial_response => {
                    SubscriptionRecovery::from_error(code, message)
                },
                _ => None,
            };
            if let Some(action) = recovery_action {
                action.apply(&mut state.options, &mut state.last_seq_id);
                state.batch_seq_id = None;
                if action == SubscriptionRecovery::Snapshot {
                    learned_domains.borrow_mut().remove(&state.sql);
                    state.callback_mode.reset_snapshot();
                }
                recovery = Some(state.request(client_id));
            } else if let Some(filtered_event) = filter_subscription_event(&state.options, event) {
                callback = Some(state.callback.clone());
                if resume_acknowledged(&filtered_event) {
                    state.resume_pending = false;
                }
                track_subscription_checkpoint(&mut state.last_seq_id, &filtered_event);
                payload = callback_payload(&mut state.callback_mode, &filtered_event);
                if let ChangeEvent::InitialDataBatch { batch_control, .. } = filtered_event {
                    state.batch_seq_id = batch_control.last_seq_id;
                    if batch_control.has_more && state.options.auto_fetch_batches.unwrap_or(false) {
                        next_batch =
                            Some((client_id.to_string(), state.batch_seq_id.or(state.last_seq_id)));
                    }
                }
            }

            match event {
                ServerMessage::SubscriptionAck { batch_control, .. } => {
                    state.batch_seq_id = batch_control.last_seq_id;
                    if let Some(domain) = batch_control.version_domain.clone() {
                        let sql = state.sql.clone();
                        state.options.version_domain = Some(domain.clone());
                        remember_learned_domain(learned_domains, &sql, domain);
                    }
                    if state.awaiting_initial_response {
                        state.awaiting_initial_response = false;
                        state.pending_subscribe_reject = None;
                        resolve_subscribe = state.pending_subscribe_resolve.take();
                    }
                },
                ServerMessage::Error { code, message, .. } => {
                    if state.awaiting_initial_response {
                        state.awaiting_initial_response = false;
                        state.pending_subscribe_resolve = None;
                        if let Some(reject) = state.pending_subscribe_reject.take() {
                            reject_subscribe = Some((
                                reject,
                                format!("Subscription failed ({}): {}", code, message),
                            ));
                        }
                        remove_state = true;
                    } else if recovery.is_none() && state.resume_pending {
                        remove_state = true;
                    }
                },
                _ => {},
            }
        }

        if remove_state {
            subs.remove(client_id);
        }
    }

    Some(SubscriptionDispatch {
        recovery,
        callback,
        payload,
        resolve_subscribe,
        reject_subscribe,
        next_batch,
    })
}

pub(crate) fn emit_runtime_ws_error(
    on_error_cb: &Rc<RefCell<Option<js_sys::Function>>>,
    error: &link_common::event_handlers::ConnectionError,
) {
    let callback = on_error_cb.borrow().clone();
    if let Some(cb) = callback {
        let err_obj = js_sys::Object::new();
        let _ =
            js_sys::Reflect::set(&err_obj, &"message".into(), &JsValue::from_str(&error.message));
        let _ = js_sys::Reflect::set(
            &err_obj,
            &"recoverable".into(),
            &JsValue::from_bool(error.recoverable),
        );
        if let Some(url) = &error.url {
            let _ = js_sys::Reflect::set(&err_obj, &"url".into(), &JsValue::from_str(url));
        }
        if let Some(auth_user) = &error.auth_user {
            let _ =
                js_sys::Reflect::set(&err_obj, &"authUser".into(), &JsValue::from_str(auth_user));
        }
        if let Some(hint) = &error.hint {
            let _ = js_sys::Reflect::set(&err_obj, &"hint".into(), &JsValue::from_str(hint));
        }
        let _ = cb.call1(&JsValue::NULL, &err_obj);
    }
}

pub(crate) fn auth_user_from_wasm_auth(auth: &WasmAuthProvider) -> Option<&str> {
    match auth {
        WasmAuthProvider::Basic { username, .. } => Some(username.as_str()),
        _ => None,
    }
}

fn connection_hint(detail: &str, recoverable: bool, auth_user: Option<&str>) -> &'static str {
    let normalized = detail.to_lowercase();
    if normalized.contains("invalid url")
        || normalized.contains("relative url")
        || normalized.contains("base url")
        || normalized.contains("url parse")
    {
        return "Check the configured KalamDB URL. Use an absolute http:// or https:// base URL \
                that the client can reach.";
    }
    if normalized.contains("401")
        || normalized.contains("403")
        || normalized.contains("unauthorized")
        || normalized.contains("authentication")
        || normalized.contains("token")
        || normalized.contains("invalid credentials")
    {
        return if auth_user.is_some() {
            "Verify the configured auth user and password or JWT token. Basic auth must login() \
             successfully before opening realtime connections."
        } else {
            "Verify the configured JWT token or auth provider. Realtime connections require valid \
             authentication before subscribing."
        };
    }
    if recoverable {
        return "Verify KalamDB is running and reachable at the configured URL from this client, \
                then retry.";
    }
    "Review the connection configuration and authentication settings for this client."
}

pub(crate) fn build_runtime_connection_error(
    context: &str,
    detail: &str,
    recoverable: bool,
    url: &str,
    auth_user: Option<&str>,
) -> link_common::event_handlers::ConnectionError {
    let auth_fragment = auth_user.map(|user| format!(" for user \"{}\"", user)).unwrap_or_default();
    let message = if context.is_empty() {
        format!(
            "{}{} at {}. Hint: {}",
            detail,
            auth_fragment,
            url,
            connection_hint(detail, recoverable, auth_user)
        )
    } else {
        format!(
            "{}{} at {}: {}. Hint: {}",
            context,
            auth_fragment,
            url,
            detail,
            connection_hint(detail, recoverable, auth_user)
        )
    };

    let mut error =
        link_common::event_handlers::ConnectionError::new(message, recoverable).with_url(url);
    if let Some(user) = auth_user {
        error = error.with_auth_user(user);
    }
    error.with_hint(connection_hint(detail, recoverable, auth_user))
}

pub(crate) fn js_error_message(error: &JsValue) -> String {
    error
        .as_string()
        .or_else(|| {
            js_sys::Reflect::get(error, &"message".into())
                .ok()
                .and_then(|value| value.as_string())
        })
        .unwrap_or_else(|| format!("{:?}", error))
}

fn is_recoverable_preconnect_error(message: &str) -> bool {
    let normalized = message.to_lowercase();
    !(normalized.contains("401")
        || normalized.contains("403")
        || normalized.contains("unauthorized")
        || normalized.contains("authentication")
        || normalized.contains("token")
        || normalized.contains("invalid credentials")
        || normalized.contains("invalid url")
        || normalized.contains("relative url")
        || normalized.contains("configuration error")
        || normalized.contains("base url"))
}

pub(crate) fn emit_runtime_js_error(
    on_error_cb: &Rc<RefCell<Option<js_sys::Function>>>,
    error: &JsValue,
    context: &str,
    url: &str,
    auth_user: Option<&str>,
) {
    let detail = js_error_message(error);
    let recoverable = is_recoverable_preconnect_error(&detail);
    let event = build_runtime_connection_error(context, &detail, recoverable, url, auth_user);
    emit_runtime_ws_error(on_error_cb, &event);
}

#[wasm_bindgen]
impl KalamClient {
    /// Create a new KalamDB client with HTTP Basic Authentication (T042, T043, T044)
    ///
    /// # Arguments
    /// * `url` - KalamDB server URL (required, e.g., "http://localhost:2900")
    /// * `username` - Username for authentication (required)
    /// * `password` - Password for authentication (required)
    ///
    /// # Errors
    /// Returns JsValue error if url, username, or password is empty
    #[wasm_bindgen(constructor)]
    pub fn new(url: String, username: String, password: String) -> Result<KalamClient, JsValue> {
        // T044: Validate required parameters with clear error messages
        if url.is_empty() {
            return Err(JsValue::from_str(
                "KalamClient: 'url' parameter is required and cannot be empty",
            ));
        }
        if username.is_empty() {
            return Err(JsValue::from_str(
                "KalamClient: 'username' parameter is required and cannot be empty",
            ));
        }
        if password.is_empty() {
            return Err(JsValue::from_str(
                "KalamClient: 'password' parameter is required and cannot be empty",
            ));
        }

        Ok(KalamClient::new_with_auth(url, WasmAuthProvider::Basic { username, password }))
    }

    /// Create a new KalamDB client with JWT Token Authentication
    ///
    /// # Arguments
    /// * `url` - KalamDB server URL (required, e.g., "http://localhost:2900")
    /// * `token` - JWT token for authentication (required)
    ///
    /// # Errors
    /// Returns JsValue error if url or token is empty
    ///
    /// # Example (JavaScript)
    /// ```js
    /// const client = KalamClient.withJwt(
    ///   "http://localhost:2900",
    ///   "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9..."
    /// );
    /// await client.connect();
    /// ```
    #[wasm_bindgen(js_name = withJwt)]
    pub fn with_jwt(url: String, token: String) -> Result<KalamClient, JsValue> {
        if url.is_empty() {
            return Err(JsValue::from_str(
                "KalamClient.withJwt: 'url' parameter is required and cannot be empty",
            ));
        }
        if token.is_empty() {
            return Err(JsValue::from_str(
                "KalamClient.withJwt: 'token' parameter is required and cannot be empty",
            ));
        }

        Ok(KalamClient::new_with_auth(url, WasmAuthProvider::Jwt { token }))
    }

    /// Create a new KalamDB client with no authentication
    ///
    /// Useful for localhost connections where the server allows
    /// unauthenticated access, or for development/testing scenarios.
    ///
    /// # Arguments
    /// * `url` - KalamDB server URL (required, e.g., "http://localhost:2900")
    ///
    /// # Errors
    /// Returns JsValue error if url is empty
    ///
    /// # Example (JavaScript)
    /// ```js
    /// const client = KalamClient.anonymous("http://localhost:2900");
    /// await client.connect();
    /// ```
    #[wasm_bindgen(js_name = anonymous)]
    pub fn anonymous(url: String) -> Result<KalamClient, JsValue> {
        if url.is_empty() {
            return Err(JsValue::from_str(
                "KalamClient.anonymous: 'url' parameter is required and cannot be empty",
            ));
        }

        Ok(KalamClient::new_with_auth(url, WasmAuthProvider::None))
    }

    /// Get the current authentication type
    ///
    /// Returns one of: "basic", "jwt", or "none"
    #[wasm_bindgen(js_name = getAuthType)]
    pub fn get_auth_type(&self) -> String {
        match &*self.realtime.auth.borrow() {
            WasmAuthProvider::Basic { .. } => "basic".to_string(),
            WasmAuthProvider::Jwt { .. } => "jwt".to_string(),
            WasmAuthProvider::None => "none".to_string(),
        }
    }

    /// Enable or disable automatic reconnection
    ///
    /// # Arguments
    /// * `enabled` - Whether to automatically reconnect on connection loss
    #[wasm_bindgen(js_name = setAutoReconnect)]
    pub fn set_auto_reconnect(&self, enabled: bool) {
        self.realtime.set_auto_reconnect(enabled);
    }

    /// Set reconnection delay parameters
    ///
    /// # Arguments
    /// * `initial_delay_ms` - Initial delay in milliseconds between reconnection attempts
    /// * `max_delay_ms` - Maximum delay (for exponential backoff)
    #[wasm_bindgen(js_name = setReconnectDelay)]
    pub fn set_reconnect_delay(&self, initial_delay_ms: u64, max_delay_ms: u64) {
        let mut opts = self.realtime.options.borrow_mut();
        opts.reconnect_delay_ms = initial_delay_ms;
        opts.max_reconnect_delay_ms = max_delay_ms;
    }

    /// Set maximum reconnection attempts
    ///
    /// # Arguments
    /// * `max_attempts` - Maximum number of attempts (0 = infinite)
    #[wasm_bindgen(js_name = setMaxReconnectAttempts)]
    pub fn set_max_reconnect_attempts(&self, max_attempts: u32) {
        self.realtime.options.borrow_mut().max_reconnect_attempts = if max_attempts == 0 {
            None
        } else {
            Some(max_attempts)
        };
    }

    /// Get the current reconnection attempt count
    #[wasm_bindgen(js_name = getReconnectAttempts)]
    pub fn get_reconnect_attempts(&self) -> u32 {
        self.realtime.attempts()
    }

    /// Check if currently reconnecting
    #[wasm_bindgen(js_name = isReconnecting)]
    pub fn is_reconnecting_flag(&self) -> bool {
        self.realtime.is_reconnecting()
    }

    /// Get the last received version for a subscription
    ///
    /// Useful for debugging or manual resumption tracking
    #[wasm_bindgen(js_name = getLastVersionId)]
    pub fn get_last_seq_id(&self, subscription_id: String) -> Option<String> {
        self.realtime
            .subscriptions
            .borrow()
            .get(&subscription_id)
            .and_then(|state| state.last_seq_id.map(|seq| seq.to_string()))
    }

    /// Register a callback invoked when the WebSocket connection is established.
    ///
    /// The callback receives no arguments.
    ///
    /// # Example (JavaScript)
    /// ```js
    /// client.onConnect(() => console.log('Connected!'));
    /// ```
    #[wasm_bindgen(js_name = onConnect)]
    pub fn on_connect(&self, callback: js_sys::Function) {
        *self.realtime.on_connect.borrow_mut() = Some(callback);
    }

    /// Register a callback invoked when the WebSocket connection is closed.
    ///
    /// The callback receives an object: `{ message: string, code?: number }`.
    ///
    /// # Example (JavaScript)
    /// ```js
    /// client.onDisconnect((reason) => console.log('Disconnected:', reason.message));
    /// ```
    #[wasm_bindgen(js_name = onDisconnect)]
    pub fn on_disconnect(&self, callback: js_sys::Function) {
        *self.realtime.on_disconnect.borrow_mut() = Some(callback);
    }

    /// Register a callback invoked when a connection error occurs.
    ///
    /// The callback receives an object: `{ message: string, recoverable: boolean }`.
    ///
    /// # Example (JavaScript)
    /// ```js
    /// client.onError((err) => console.error('Error:', err.message, 'recoverable:', err.recoverable));
    /// ```
    #[wasm_bindgen(js_name = onError)]
    pub fn on_error(&self, callback: js_sys::Function) {
        *self.realtime.on_error.borrow_mut() = Some(callback);
    }

    /// Register a callback invoked when a connection or pre-connection error occurs.
    ///
    /// Alias for `onError` so higher-level SDKs can use a more explicit name
    /// without re-implementing connection lifecycle semantics.
    #[wasm_bindgen(js_name = onConnectionError)]
    pub fn on_connection_error(&self, callback: js_sys::Function) {
        *self.realtime.on_error.borrow_mut() = Some(callback);
    }

    /// Register a callback invoked for every raw message received from the server.
    ///
    /// This is a debug/tracing hook. The callback receives the raw JSON string.
    ///
    /// # Example (JavaScript)
    /// ```js
    /// client.onReceive((msg) => console.log('[RECV]', msg));
    /// ```
    #[wasm_bindgen(js_name = onReceive)]
    pub fn on_receive(&self, callback: js_sys::Function) {
        *self.realtime.on_receive.borrow_mut() = Some(callback);
    }

    /// Register a callback invoked for every raw message sent to the server.
    ///
    /// This is a debug/tracing hook. The callback receives the raw JSON string.
    ///
    /// # Example (JavaScript)
    /// ```js
    /// client.onSend((msg) => console.log('[SEND]', msg));
    /// ```
    #[wasm_bindgen(js_name = onSend)]
    pub fn on_send(&self, callback: js_sys::Function) {
        *self.realtime.on_send.borrow_mut() = Some(callback);
    }

    /// Set an async authentication provider callback.
    ///
    /// When set, this callback is invoked before each (re-)connection attempt
    /// to obtain a fresh JWT token.  This is the recommended approach for
    /// applications that implement refresh-token flows.
    ///
    /// The callback must be an `async function` (or any function returning a
    /// `Promise`) that resolves to **either**:
    /// - `{ jwt: { token: "eyJ..." } }` — authenticates with the given JWT
    /// - `null` / `undefined` — treated as anonymous (no authentication)
    ///
    /// The static `auth` set at construction time is ignored once a provider
    /// is registered.
    ///
    /// # Example (JavaScript)
    /// ```js
    /// client.setAuthProvider(async () => {
    ///   const token = await myApp.getOrRefreshJwt();
    ///   return { jwt: { token } };
    /// });
    /// ```
    #[wasm_bindgen(js_name = setAuthProvider)]
    pub fn set_auth_provider(&self, callback: js_sys::Function) {
        *self.realtime.auth_provider.borrow_mut() = Some(callback);
    }

    /// Set the default namespace for unqualified SQL queries and live subscriptions.
    #[wasm_bindgen(js_name = setDefaultNamespace)]
    pub fn set_default_namespace(&self, namespace: Option<String>) {
        *self.default_namespace.borrow_mut() = normalize_default_namespace(namespace);
    }

    /// Clear a previously set auth provider, reverting to the static auth
    /// configured at construction time.
    #[wasm_bindgen(js_name = clearAuthProvider)]
    pub fn clear_auth_provider(&self) {
        *self.realtime.auth_provider.borrow_mut() = None;
    }

    /// Enable or disable compression for WebSocket messages.
    ///
    /// When set to `true` (default) the server sends gzip-compressed binary
    /// frames for large payloads.  Set to `false` during development to receive
    /// plain-text JSON frames that are easier to inspect.
    ///
    /// Takes effect on the **next** `connect()` call.
    ///
    /// # Example (JavaScript)
    /// ```js
    /// client.setDisableCompression(true); // plain-text frames
    /// await client.connect();
    /// ```
    #[wasm_bindgen(js_name = setDisableCompression)]
    pub fn set_disable_compression(&self, disable: bool) {
        self.realtime.options.borrow_mut().disable_compression = disable;
    }

    /// Control lazy WebSocket connections.
    ///
    /// When `true` (the default), the WebSocket connection is deferred until
    /// the first `live()`, `liveTable()`, or `liveEvents()` call. The SDK manages
    /// the connection lifecycle automatically.
    ///
    /// When `false`, the caller should call `connect()` before opening live streams.
    ///
    /// Default: `true`.
    ///
    /// # Example (JavaScript)
    /// ```js
    /// // Eager connection (override the default lazy behaviour)
    /// client.setWsLazyConnect(false);
    /// await client.connect();
    /// const subId = await client.liveEvents('SELECT * FROM messages', null, cb);
    /// ```
    #[wasm_bindgen(js_name = setWsLazyConnect)]
    pub fn set_ws_lazy_connect(&self, lazy: bool) {
        self.realtime.options.borrow_mut().ws_lazy_connect = lazy;
    }

    /// # Returns
    /// Promise that resolves when connection is established and authenticated
    pub async fn connect(&self) -> Result<(), JsValue> {
        self.realtime.connect().await
    }

    /// Disconnect the WebSocket and end its subscriptions.
    pub async fn disconnect(&self) -> Result<(), JsValue> {
        self.realtime.disconnect(true);
        Ok(())
    }

    /// Whether the socket is authenticated and its subscriptions have resumed.
    #[wasm_bindgen(js_name = isConnected)]
    pub fn is_connected(&self) -> bool {
        self.realtime.is_connected()
    }

    /// Set the application-level keepalive ping interval in milliseconds.
    ///
    /// Browser WebSocket APIs do not expose protocol-level Ping frames, so
    /// the WASM client sends a JSON `{"type":"ping"}` message at this
    /// interval. Set to `0` to disable. Default: 5 000 ms.
    ///
    /// The change takes effect on the next `connect()` or reconnect.
    ///
    /// # Note
    /// Takes `u32` (maps to TypeScript `number`); the internal store is `u64`.
    #[wasm_bindgen(js_name = setPingInterval)]
    pub fn set_ping_interval(&self, ms: u32) {
        self.realtime.options.borrow_mut().ping_interval_ms = ms as u64;
    }

    /// Send a single application-level keepalive ping to the server.
    ///
    /// Usually called automatically by the internal ping timer; exposed so
    /// callers can send an ad-hoc ping if needed.
    #[wasm_bindgen(js_name = sendPing)]
    pub fn send_ping(&self) -> Result<(), JsValue> {
        if let Some(ws) = self.realtime.socket().as_ref() {
            if ws.ready_state() == WebSocket::OPEN {
                send_ws_message_traced(
                    ws,
                    &ClientMessage::Ping,
                    self.realtime.serialization.get(),
                    &self.realtime.on_send,
                )?;
            }
        }
        Ok(())
    }

    /// Request the next initial-data batch for a subscription.
    #[wasm_bindgen(js_name = requestNextBatch)]
    pub fn request_next_batch(&self, subscription_id: String) -> Result<(), JsValue> {
        let last_seq_id = self
            .realtime
            .subscriptions
            .borrow()
            .get(&subscription_id)
            .map(|state| state.batch_seq_id.or(state.last_seq_id))
            .ok_or_else(|| JsValue::from_str("Unknown subscription id"))?;

        let Some(ws) = self.realtime.socket() else {
            return Err(JsValue::from_str("WebSocket connection is unavailable"));
        };

        if ws.ready_state() != WebSocket::OPEN {
            return Err(JsValue::from_str("WebSocket connection is not open"));
        }

        send_next_batch_traced(
            &ws,
            &subscription_id,
            last_seq_id,
            self.realtime.serialization.get(),
            &self.realtime.on_send,
        )
    }

    /// Insert data into a table (T048, T063G)
    ///
    /// # Arguments
    /// * `table_name` - Name of the table to insert into
    /// * `data` - JSON string representing the row data
    ///
    /// # Example (JavaScript)
    /// ```js
    /// await client.insert("todos", JSON.stringify({
    ///   title: "Buy groceries",
    ///   completed: false
    /// }));
    /// ```
    pub async fn insert(&self, table_name: String, data: String) -> Result<String, JsValue> {
        // Security: Validate table name to prevent SQL injection
        validate_sql_identifier(&table_name, "Table name")?;

        // Parse JSON data to build proper SQL INSERT statement
        let parsed: serde_json::Value = serde_json::from_str(&data)
            .map_err(|e| JsValue::from_str(&format!("Invalid JSON data: {}", e)))?;

        let obj = parsed
            .as_object()
            .ok_or_else(|| JsValue::from_str("Data must be a JSON object"))?;

        if obj.is_empty() {
            return Err(JsValue::from_str("Cannot insert empty object"));
        }

        // Security: Validate all column names
        for key in obj.keys() {
            validate_column_name(key)?;
        }

        // Build column names and values
        // Security: Quote identifiers with double quotes (SQL standard)
        let columns: Vec<String> = obj.keys().map(|k| format!("\"{}\"", k)).collect();
        let values: Vec<String> = obj
            .values()
            .map(|v| match v {
                serde_json::Value::Null => "NULL".to_string(),
                serde_json::Value::Bool(b) => b.to_string(),
                serde_json::Value::Number(n) => n.to_string(),
                serde_json::Value::String(s) => format!("'{}'", s.replace('\'', "''")),
                _ => format!("'{}'", v.to_string().replace('\'', "''")),
            })
            .collect();

        // Security: Quote table name with double quotes, handling namespace.table format
        let sql = format!(
            "INSERT INTO {} ({}) VALUES ({})",
            quote_table_name(&table_name),
            columns.join(", "),
            values.join(", ")
        );

        let namespace_id = self.default_namespace.borrow().clone();
        self.execute_sql_internal(&sql, None, namespace_id).await
    }

    /// Delete a row from a table (T049, T063H)
    ///
    /// # Arguments
    /// * `table_name` - Name of the table
    /// * `row_id` - ID of the row to delete
    pub async fn delete(&self, table_name: String, row_id: String) -> Result<(), JsValue> {
        // Security: Validate inputs to prevent SQL injection
        validate_sql_identifier(&table_name, "Table name")?;
        validate_row_id(&row_id)?;

        // T063H: Implement using fetch API to execute DELETE statement via /v1/api/sql
        // Security: Quote table name (handling namespace.table format) and use parameterized-style
        // value
        let sql = format!(
            "DELETE FROM {} WHERE id = '{}'",
            quote_table_name(&table_name),
            row_id.replace('\'', "''")
        );
        let namespace_id = self.default_namespace.borrow().clone();
        self.execute_sql_internal(&sql, None, namespace_id).await?;
        Ok(())
    }

    /// Execute a SQL query (T050, T063F)
    ///
    /// # Arguments
    /// * `sql` - SQL query string
    ///
    /// # Returns
    /// JSON string with query results
    ///
    /// # Example (JavaScript)
    /// ```js
    /// const result = await client.query("SELECT * FROM todos WHERE completed = false");
    /// const data = JSON.parse(result);
    /// ```
    pub async fn query(&self, sql: String) -> Result<String, JsValue> {
        let namespace_id = self.default_namespace.borrow().clone();
        self.execute_sql_internal(&sql, None, namespace_id).await
    }

    /// Execute a SQL query with parameters
    ///
    /// # Arguments
    /// * `sql` - SQL query string with placeholders ($1, $2, ...)
    /// * `params` - JSON array string of parameter values
    ///
    /// # Returns
    /// JSON string with query results
    ///
    /// # Example (JavaScript)
    /// ```js
    /// const result = await client.queryWithParams(
    ///   "SELECT * FROM users WHERE id = $1 AND age > $2",
    ///   JSON.stringify([42, 18])
    /// );
    /// const data = JSON.parse(result);
    /// ```
    #[wasm_bindgen(js_name = queryWithParams)]
    pub async fn query_with_params(
        &self,
        sql: String,
        params: Option<String>,
    ) -> Result<String, JsValue> {
        let parsed_params: Option<Vec<serde_json::Value>> = match params {
            Some(p) if !p.is_empty() => Some(
                serde_json::from_str(&p)
                    .map_err(|e| JsValue::from_str(&format!("Invalid params JSON: {}", e)))?,
            ),
            _ => None,
        };
        let namespace_id = self.default_namespace.borrow().clone();
        self.execute_sql_internal(&sql, parsed_params, namespace_id).await
    }

    /// Subscribe to table changes (T051, T063I-T063J)
    ///
    /// # Arguments
    /// * `table_name` - Name of the table to subscribe to
    /// * `callback` - JavaScript function to call when changes occur
    ///
    /// # Returns
    /// Subscription ID for later unsubscribe
    #[wasm_bindgen(js_name = liveTable)]
    pub async fn live_table(
        &self,
        table_name: String,
        options: Option<String>,
        callback: js_sys::Function,
    ) -> Result<String, JsValue> {
        // Security: Validate table name to prevent SQL injection
        validate_sql_identifier(&table_name, "Table name")?;

        // Default: SELECT * FROM table with default options
        // Security: Quote table name properly (handles namespace.table format)
        let sql = format!("SELECT * FROM {}", quote_table_name(&table_name));
        self.live(sql, options, callback).await
    }

    /// Subscribe to a SQL query and receive low-level live events.
    ///
    /// # Arguments
    /// * `sql` - SQL SELECT query to subscribe to
    /// * `options` - Optional JSON string with subscription options:
    ///   - `batch_size`: Number of rows per batch (default: server-configured)
    ///   - `auto_reconnect`: Override client auto-reconnect for this subscription (default: true)
    ///   - `include_old_values`: Include old values in UPDATE/DELETE events (default: false)
    ///   - `from`: Resume from a specific sequence ID (internal use)
    /// * `callback` - JavaScript function to call when changes occur
    ///
    /// # Returns
    /// Subscription ID for later unsubscribe
    ///
    /// # Example (JavaScript)
    /// ```js
    /// // Subscribe with options
    /// const subId = await client.liveEvents(
    ///   "SELECT * FROM chat.messages WHERE conversation_id = 1",
    ///   JSON.stringify({ batch_size: 50, from: 42 }),
    ///   (event) => console.log('Change:', event)
    /// );
    /// ```
    #[wasm_bindgen(js_name = liveEvents)]
    pub async fn live_events(
        &self,
        sql: String,
        options: Option<String>,
        callback: js_sys::Function,
    ) -> Result<String, JsValue> {
        let subscription_options: SubscriptionOptions = if let Some(opts_json) = options {
            serde_json::from_str(&opts_json)
                .map_err(|e| JsValue::from_str(&format!("Invalid options JSON: {}", e)))?
        } else {
            SubscriptionOptions::default()
        };
        let default_namespace = self.default_namespace.borrow().clone();
        let qualified_sql = qualify_subscription_sql(&sql, default_namespace.as_deref());

        self.register_subscription(
            qualified_sql,
            subscription_options,
            callback,
            SubscriptionCallbackMode::raw(),
        )
        .await
    }

    /// Subscribe to a SQL query and receive materialized live rows.
    ///
    /// The callback receives JSON strings with one of these shapes:
    /// - `{ type: "rows", subscription_id, rows }`
    /// - `{ type: "error", subscription_id, code, message }`
    #[wasm_bindgen(js_name = live)]
    pub async fn live(
        &self,
        sql: String,
        options: Option<String>,
        callback: js_sys::Function,
    ) -> Result<String, JsValue> {
        let parsed_options = if let Some(opts_json) = options {
            serde_json::from_str::<WasmLiveRowsOptions>(&opts_json)
                .map_err(|e| JsValue::from_str(&format!("Invalid live rows options JSON: {}", e)))?
        } else {
            WasmLiveRowsOptions::default()
        };
        let default_namespace = self.default_namespace.borrow().clone();
        let qualified_sql = qualify_subscription_sql(&sql, default_namespace.as_deref());

        self.register_subscription(
            qualified_sql,
            parsed_options.subscription_options.unwrap_or_default(),
            callback,
            SubscriptionCallbackMode::live_rows(link_common::subscription::LiveRowsConfig {
                limit:       parsed_options.limit,
                key_columns: parsed_options.key_columns,
            }),
        )
        .await
    }

    /// Unsubscribe from table changes (T052, T063M)
    ///
    /// # Arguments
    /// * `subscription_id` - ID returned from subscribe()
    pub async fn unsubscribe(&self, subscription_id: String) -> Result<(), JsValue> {
        let removed = self.realtime.subscriptions.borrow_mut().remove(&subscription_id);
        if let Some(reject) = removed.and_then(|state| state.pending_subscribe_reject) {
            let _ = reject.call1(&JsValue::NULL, &JsValue::from_str("Subscription cancelled"));
        }
        self.realtime.check_resumed();

        // Send unsubscribe message via WebSocket
        if let Some(ws) = self.realtime.socket().as_ref() {
            let unsubscribe_msg = ClientMessage::Unsubscribe {
                subscription_id: subscription_id.clone(),
            };
            send_ws_message_traced(
                ws,
                &unsubscribe_msg,
                self.realtime.serialization.get(),
                &self.realtime.on_send,
            )?;
        }

        wasm_debug_log!(&format!("KalamClient: Unsubscribed from: {}", subscription_id));
        Ok(())
    }

    /// Return a JSON array describing all active subscriptions.
    ///
    /// Each element contains `id`, `query`, `lastSeqId`, `lastEventTimeMs`,
    /// `createdAtMs`, and `closed`.  The WASM layer surfaces its own
    /// reconnection state, so `lastSeqId` reflects the latest seq received.
    ///
    /// # Example (JavaScript)
    /// ```js
    /// const subs = client.getSubscriptions();
    /// // subs = [{ id: "sub-abc", query: "SELECT ...", lastSeqId: "123", ... }]
    /// ```
    #[wasm_bindgen(js_name = getSubscriptions)]
    pub fn get_subscriptions(&self) -> JsValue {
        let state = self.realtime.subscriptions.borrow();
        let list: Vec<serde_json::Value> = state
            .iter()
            .map(|(id, entry)| {
                serde_json::json!({
                    "id": id,
                    "query": entry.sql,
                    "lastSeqId": entry.last_seq_id.map(|s| s.as_i64().to_string()),
                    "versionDomain": entry.options.version_domain,
                    "closed": false,
                })
            })
            .collect();
        JsValue::from_str(&serde_json::to_string(&list).unwrap_or_else(|_| "[]".to_string()))
    }

    /// Login with current Basic Auth credentials and switch to JWT authentication
    ///
    /// Sends a POST request to `/v1/api/auth/login` with the stored user/password
    /// and updates the client to use JWT authentication on success.
    ///
    /// # Returns
    /// The full LoginResponse as a JsValue (includes access_token, refresh_token, user info, etc.)
    ///
    /// # Errors
    /// - If the client doesn't use Basic Auth
    /// - If login request fails
    /// - If the response doesn't contain an access_token
    ///
    /// # Example (JavaScript)
    /// ```js
    /// const client = new KalamClient("http://localhost:2900", "user", "pass");
    /// const response = await client.login();
    /// console.log(response.access_token, response.refresh_token);
    /// await client.connect(); // Now uses JWT for WebSocket
    /// ```
    pub async fn login(&mut self) -> Result<JsValue, JsValue> {
        let (user, password) = match &*self.realtime.auth.borrow() {
            WasmAuthProvider::Basic { username, password } => (username.clone(), password.clone()),
            _ => {
                return Err(JsValue::from_str(
                    "login() requires user/password credentials. Create client with new \
                     KalamClient(url, user, password)",
                ));
            },
        };

        let login_response = perform_basic_login(&self.realtime.url, &user, &password).await?;

        // Switch to JWT auth
        *self.realtime.auth.borrow_mut() = WasmAuthProvider::Jwt {
            token: login_response.access_token.clone(),
        };

        wasm_debug_log!("KalamClient: Login successful, switched to JWT authentication");

        // Return full LoginResponse as JsValue
        serialize_json_to_js_value(&login_response, "login response")
    }

    /// Refresh the access token using a refresh token
    ///
    /// Sends a POST request to `/v1/api/auth/refresh` with the refresh token
    /// in the Authorization Bearer header, and updates the client to use the new JWT.
    ///
    /// # Arguments
    /// * `refresh_token` - The refresh token obtained from a previous login
    ///
    /// # Returns
    /// The full LoginResponse as a JsValue (includes new access_token, refresh_token, etc.)
    ///
    /// # Errors
    /// - If the refresh request fails
    /// - If the response doesn't contain a valid token
    ///
    /// # Example (JavaScript)
    /// ```js
    /// const client = new KalamClient("http://localhost:2900", "user", "pass");
    /// const loginResp = await client.login();
    /// // Later, when access_token expires:
    /// const refreshResp = await client.refresh_access_token(loginResp.refresh_token);
    /// console.log(refreshResp.access_token);
    /// ```
    pub async fn refresh_access_token(&mut self, refresh_token: &str) -> Result<JsValue, JsValue> {
        let url = format!("{}/v1/api/auth/refresh", self.realtime.url);
        let auth_header = format!("Bearer {}", refresh_token);
        let json_str = super::helpers::wasm_fetch(
            &url,
            "POST",
            None,
            &[("Authorization", &auth_header)],
            "Token refresh failed",
        )
        .await?;

        let login_response: link_common::models::LoginResponse = serde_json::from_str(&json_str)
            .map_err(|e| JsValue::from_str(&format!("Failed to parse refresh response: {}", e)))?;

        // Update to new JWT
        *self.realtime.auth.borrow_mut() = WasmAuthProvider::Jwt {
            token: login_response.access_token.clone(),
        };

        wasm_debug_log!("KalamClient: Token refreshed, updated JWT authentication");

        // Return full LoginResponse as JsValue
        serialize_json_to_js_value(&login_response, "refresh response")
    }

    /// Internal: Execute SQL via HTTP POST to /v1/api/sql (T063F)
    ///
    /// On TOKEN_EXPIRED, the method resolves fresh credentials from the
    /// auth_provider_cb (if set) or re-uses stored basic-auth to login,
    /// then retries the request exactly once.
    ///
    /// TOKEN_EXPIRED may arrive as either:
    ///   (a) HTTP 200 with `{"status":"error","error":{"code":"TOKEN_EXPIRED",...}}`
    ///   (b) HTTP 401 with the same JSON body (wasm_fetch returns Err)
    async fn execute_sql_internal(
        &self,
        sql: &str,
        params: Option<Vec<serde_json::Value>>,
        namespace_id: Option<String>,
    ) -> Result<String, JsValue> {
        if matches!(&*self.realtime.auth.borrow(), WasmAuthProvider::Basic { .. }) {
            self.reauthenticate_for_http().await?;
        }

        let result = self.execute_sql_http(sql, &params, namespace_id.as_deref()).await;

        match result {
            Ok(result_str) => {
                // Case (a): HTTP 200 with TOKEN_EXPIRED in the JSON body.
                if let Ok(query_resp) =
                    serde_json::from_str::<link_common::query::models::QueryResponse>(&result_str)
                {
                    if query_resp.is_token_expired() {
                        wasm_debug_log!(
                            "KalamClient: TOKEN_EXPIRED detected — reauthenticating and retrying \
                             query",
                        );
                        self.reauthenticate_for_http().await?;
                        return self.execute_sql_http(sql, &params, namespace_id.as_deref()).await;
                    }
                }
                Ok(result_str)
            },
            Err(err) => {
                // Case (b): HTTP 401 — wasm_fetch returned the JSON body as an Err string.
                if let Some(err_str) = err.as_string() {
                    if let Ok(query_resp) =
                        serde_json::from_str::<link_common::query::models::QueryResponse>(&err_str)
                    {
                        if query_resp.is_token_expired() {
                            wasm_debug_log!(
                                "KalamClient: TOKEN_EXPIRED detected in HTTP error — \
                                 reauthenticating and retrying query",
                            );
                            self.reauthenticate_for_http().await?;
                            return self
                                .execute_sql_http(sql, &params, namespace_id.as_deref())
                                .await;
                        }
                    }
                }
                Err(err)
            },
        }
    }

    /// Low-level HTTP POST to /v1/api/sql.  Returns the raw JSON response
    /// string with `named_rows` populated.
    async fn execute_sql_http(
        &self,
        sql: &str,
        params: &Option<Vec<serde_json::Value>>,
        namespace_id: Option<&str>,
    ) -> Result<String, JsValue> {
        let body = BorrowedQueryRequest {
            sql,
            params: params.as_deref(),
            namespace_id,
        };
        let body_str = serde_json::to_string(&body)
            .map_err(|e| JsValue::from_str(&format!("Serialization error: {}", e)))?;

        let url = format!("{}/v1/api/sql", self.realtime.url);
        let auth_header = self.realtime.auth.borrow().to_http_header();
        let mut hdrs: Vec<(&str, &str)> = vec![("Content-Type", "application/json")];
        if let Some(ref ah) = auth_header {
            hdrs.push(("Authorization", ah));
        }
        let raw =
            super::helpers::wasm_fetch(&url, "POST", Some(&body_str), &hdrs, "HTTP error").await?;

        // Deserialize, populate named_rows (schema → map), re-serialize.
        // This moves the transformation into Rust so every SDK gets it for free.
        match serde_json::from_str::<link_common::query::models::QueryResponse>(&raw) {
            Ok(mut query_resp) => {
                for result in &mut query_resp.results {
                    result.populate_named_rows();
                }
                serde_json::to_string(&query_resp)
                    .map_err(|e| JsValue::from_str(&format!("Serialization error: {}", e)))
            },
            // If deserialization fails, return raw response unchanged
            Err(_) => Ok(raw),
        }
    }

    /// Resolve fresh credentials from authProvider callback or stored basic
    /// credentials, obtain a JWT, and update `self.realtime.auth`.
    async fn reauthenticate_for_http(&self) -> Result<(), JsValue> {
        // Try the dynamic auth provider callback first (set by TS/Dart SDK).
        let provider = self.realtime.auth_provider.borrow().clone();
        if let Some(cb) = provider {
            let resolved_auth = resolve_auth_provider(Some(cb), WasmAuthProvider::None).await?;
            let _log_message = match &resolved_auth {
                WasmAuthProvider::Jwt { .. } => {
                    "KalamClient: Reauthenticated via authProvider (JWT)"
                },
                WasmAuthProvider::None => "KalamClient: authProvider returned no credentials",
                WasmAuthProvider::Basic { .. } => unreachable!(),
            };
            *self.realtime.auth.borrow_mut() = resolved_auth;
            wasm_debug_log!(_log_message);
            return Ok(());
        }

        // Fallback: if we have stored basic credentials, perform a login.
        let basic = {
            let auth = self.realtime.auth.borrow();
            match &*auth {
                WasmAuthProvider::Basic { username, password } => {
                    Some((username.clone(), password.clone()))
                },
                _ => None,
            }
        };

        if let Some((username, password)) = basic {
            let login_resp = perform_basic_login(&self.realtime.url, &username, &password).await?;
            *self.realtime.auth.borrow_mut() = WasmAuthProvider::Jwt {
                token: login_resp.access_token,
            };
            wasm_debug_log!("KalamClient: Reauthenticated via basic login");
            return Ok(());
        }

        Err(JsValue::from_str(
            "Cannot reauthenticate: no authProvider callback or basic credentials available",
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::{normalize_default_namespace, qualify_subscription_sql};

    #[test]
    fn normalize_default_namespace_trims_and_drops_empty_values() {
        assert_eq!(
            normalize_default_namespace(Some(" chat ".to_string())),
            Some("chat".to_string())
        );
        assert_eq!(normalize_default_namespace(Some("   ".to_string())), None);
        assert_eq!(normalize_default_namespace(None), None);
    }

    #[test]
    fn qualify_subscription_sql_uses_default_namespace_for_unqualified_relations() {
        let sql = qualify_subscription_sql("SELECT * FROM messages WHERE id = 1", Some("chat"));
        assert_eq!(sql, "SELECT * FROM \"chat\".messages WHERE id = 1");
    }

    #[test]
    fn qualify_subscription_sql_preserves_explicit_namespace() {
        let sql = qualify_subscription_sql("SELECT * FROM billing.messages", Some("chat"));
        assert_eq!(sql, "SELECT * FROM billing.messages");
    }
}
