use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    rc::Rc,
    time::Duration,
};

use futures_util::future::{AbortHandle, Abortable};
use js_sys::{Function, Promise};
use link_common::{
    models::{
        ClientMessage, ConnectionOptions, SerializationType, ServerMessage, SubscriptionRequest,
    },
    realtime::{resume_from, resume_timeout, RetryBackoff},
    KalamLinkTimeouts, VersionDomain,
};
use wasm_bindgen::{prelude::*, JsCast};
use wasm_bindgen_futures::{future_to_promise, spawn_local, JsFuture};
use web_sys::{MessageEvent, WebSocket};

use crate::{
    helpers::{
        create_promise, decode_ws_binary_payload, decode_ws_message, jwt_uses_upgrade_auth,
        open_websocket, ws_url_from_http_opts,
    },
    state::SubscriptionState,
    wasm_auth::{perform_basic_login, resolve_auth_provider, WasmAuthProvider},
    wasm_client::{
        auth_user_from_wasm_auth, build_runtime_connection_error,
        dispatch_subscription_server_message, emit_runtime_js_error, emit_runtime_ws_error,
        js_error_message, reject_pending_subscriptions, send_next_batch_traced,
        send_ws_message_traced,
    },
};

mod socket;
mod timer;

use self::{
    socket::{matches_socket, SocketSession},
    timer::{within, Timer},
};

struct Flight {
    promise:   Promise,
    abort:     AbortHandle,
    automatic: bool,
}

pub(crate) struct BrowserConnection {
    pub(crate) url: String,
    pub(crate) auth: RefCell<WasmAuthProvider>,
    pub(crate) subscriptions: Rc<RefCell<HashMap<String, SubscriptionState>>>,
    pub(crate) options: RefCell<ConnectionOptions>,
    pub(crate) learned_domains: RefCell<HashMap<String, VersionDomain>>,
    pub(crate) auth_provider: RefCell<Option<Function>>,
    pub(crate) on_connect: RefCell<Option<Function>>,
    pub(crate) on_disconnect: RefCell<Option<Function>>,
    pub(crate) on_error: Rc<RefCell<Option<Function>>>,
    pub(crate) on_receive: RefCell<Option<Function>>,
    pub(crate) on_send: Rc<RefCell<Option<Function>>>,
    pub(crate) serialization: Cell<SerializationType>,
    timeouts: KalamLinkTimeouts,
    session: RefCell<Option<SocketSession>>,
    flight: RefCell<Option<Flight>>,
    generation: Cell<u64>,
    retry: RefCell<RetryBackoff>,
    ready: Cell<bool>,
}

impl BrowserConnection {
    pub(crate) fn new(url: String, auth: WasmAuthProvider) -> Self {
        Self {
            url,
            auth: RefCell::new(auth),
            subscriptions: Rc::default(),
            options: RefCell::new(ConnectionOptions::default()),
            learned_domains: RefCell::default(),
            auth_provider: RefCell::default(),
            on_connect: RefCell::default(),
            on_disconnect: RefCell::default(),
            on_error: Rc::default(),
            on_receive: RefCell::default(),
            on_send: Rc::default(),
            serialization: Cell::new(SerializationType::Json),
            timeouts: KalamLinkTimeouts::default(),
            session: RefCell::default(),
            flight: RefCell::default(),
            generation: Cell::new(0),
            retry: RefCell::default(),
            ready: Cell::new(false),
        }
    }

    pub(crate) fn socket(&self) -> Option<WebSocket> {
        self.session
            .borrow()
            .as_ref()
            .filter(|session| session.authenticated)
            .map(|session| session.socket.clone())
    }

    pub(crate) fn is_connected(&self) -> bool {
        self.ready.get()
            && self.socket().is_some_and(|socket| socket.ready_state() == WebSocket::OPEN)
    }

    pub(crate) fn is_reconnecting(&self) -> bool {
        self.flight.borrow().as_ref().is_some_and(|flight| flight.automatic)
    }

    pub(crate) fn attempts(&self) -> u32 {
        self.retry.borrow().attempts()
    }

    pub(crate) fn set_auto_reconnect(&self, enabled: bool) {
        self.options.borrow_mut().auto_reconnect = enabled;
        if !enabled && self.is_reconnecting() {
            self.cancel_flight();
            self.close_socket(&JsValue::from_str("Automatic reconnect disabled"));
            reject_pending_subscriptions(&self.subscriptions, "Automatic reconnect disabled");
        }
    }

    pub(crate) async fn connect(self: &Rc<Self>) -> Result<(), JsValue> {
        if !self.is_connected() {
            JsFuture::from(self.start(false)).await?;
        }
        Ok(())
    }

    fn start(self: &Rc<Self>, automatic: bool) -> Promise {
        if let Some(flight) = self.flight.borrow().as_ref() {
            return flight.promise.clone();
        }
        let generation = self.generation.get();
        let (abort, registration) = AbortHandle::new_pair();
        let connection = Rc::clone(self);
        let promise = future_to_promise(async move {
            Abortable::new(connection.run(automatic, generation), registration)
                .await
                .map_err(|_| JsValue::from_str("WebSocket connection cancelled"))??;
            Ok(JsValue::UNDEFINED)
        });
        *self.flight.borrow_mut() = Some(Flight {
            promise: promise.clone(),
            abort,
            automatic,
        });
        promise
    }

    async fn run(self: &Rc<Self>, automatic: bool, generation: u64) -> Result<(), JsValue> {
        let mut last_error = JsValue::from_str("Max reconnection attempts reached");
        while self.generation.get() == generation {
            if automatic {
                let delay = self.retry.borrow_mut().next_delay(&self.options.borrow());
                let Some(delay) = delay else {
                    self.flight.borrow_mut().take();
                    let message = "Max reconnection attempts reached";
                    reject_pending_subscriptions(&self.subscriptions, message);
                    let error = build_runtime_connection_error(
                        message,
                        &js_error_message(&last_error),
                        false,
                        &self.url,
                        auth_user_from_wasm_auth(&self.auth.borrow()),
                    );
                    emit_runtime_ws_error(&self.on_error, &error);
                    return Err(JsValue::from_str(&error.message));
                };
                Timer::sleep(delay).await;
            }

            let result = self.dial(generation).await;
            if self.generation.get() != generation {
                break;
            }
            match result {
                Ok(()) => {
                    self.ready.set(true);
                    self.retry.borrow_mut().reset();
                    self.flight.borrow_mut().take();
                    let callback = self.on_connect.borrow().clone();
                    if let Some(callback) = callback {
                        let _ = callback.call0(&JsValue::NULL);
                    }
                    return Ok(());
                },
                Err(error) => {
                    self.close_socket(&error);
                    if !automatic {
                        self.flight.borrow_mut().take();
                        self.report_error("Failed to connect to KalamDB", &error);
                        return Err(error);
                    }
                    last_error = error;
                },
            }
        }
        Err(JsValue::from_str("WebSocket connection cancelled"))
    }

    async fn dial(self: &Rc<Self>, generation: u64) -> Result<(), JsValue> {
        let callback = self.auth_provider.borrow().clone();
        let fallback = self.auth.borrow().clone();
        let auth = within(
            async {
                match resolve_auth_provider(callback, fallback).await? {
                    WasmAuthProvider::Basic { username, password } => {
                        let response = perform_basic_login(&self.url, &username, &password).await?;
                        Ok(WasmAuthProvider::Jwt {
                            token: response.access_token,
                        })
                    },
                    auth => Ok(auth),
                }
            },
            Some((self.timeouts.auth_timeout, "WebSocket credential resolution timed out")),
        )
        .await?;

        if self.generation.get() != generation {
            return Err(JsValue::from_str("WebSocket connection cancelled"));
        }
        let options = self.options.borrow().clone();
        let protocol = options.protocol;
        let url = ws_url_from_http_opts(&self.url, options.disable_compression, protocol)?;
        let jwt = match &auth {
            WasmAuthProvider::Jwt { token } => Some(token.as_str()),
            _ => None,
        };
        let requires_auth = jwt.is_some();
        let authenticate = (!jwt.is_some_and(jwt_uses_upgrade_auth))
            .then(|| auth.to_ws_auth_message(protocol))
            .flatten();
        let socket = open_websocket(&url, jwt)?;
        socket.set_binary_type(web_sys::BinaryType::Arraybuffer);
        self.serialization.set(SerializationType::Json);
        let (authenticated, resolve, reject) = create_promise();
        *self.session.borrow_mut() = Some(SocketSession::new(
            socket.clone(),
            Rc::downgrade(self),
            authenticate,
            requires_auth,
            resolve,
            reject,
        ));
        within(
            JsFuture::from(authenticated),
            Some((
                self.timeouts.connection_timeout.saturating_add(self.timeouts.auth_timeout),
                "WebSocket connection or authentication timed out",
            )),
        )
        .await?;

        self.resume_subscriptions(&socket).await?;
        self.start_keepalive(&socket);
        Ok(())
    }

    async fn resume_subscriptions(&self, socket: &WebSocket) -> Result<(), JsValue> {
        let requests: Vec<SubscriptionRequest> = self
            .subscriptions
            .borrow_mut()
            .iter_mut()
            .map(|(id, state)| {
                resume_from(&mut state.options, state.last_seq_id);
                state.resume_pending = true;
                state.batch_seq_id = None;
                state.request(id)
            })
            .collect();
        if requests.is_empty() {
            return Ok(());
        }
        let (resumed, resolve, reject) = create_promise();
        let resumed = JsFuture::from(resumed);
        if let Some(session) = self.session.borrow_mut().as_mut() {
            session.resume = Some((resolve, reject));
        } else {
            return Err(JsValue::from_str("WebSocket closed before subscriptions could resume"));
        }
        for subscription in requests {
            if self.subscriptions.borrow().contains_key(&subscription.id) {
                self.send(socket, &ClientMessage::Subscribe { subscription })?;
            }
        }
        within(
            resumed,
            resume_timeout(&self.timeouts)
                .map(|timeout| (timeout, "WebSocket subscription resume timed out")),
        )
        .await?;
        if !matches_socket(&self.session, socket) {
            return Err(JsValue::from_str("WebSocket closed while resuming subscriptions"));
        }
        Ok(())
    }

    pub(crate) fn check_resumed(&self) {
        if self.subscriptions.borrow().values().any(|entry| entry.resume_pending) {
            return;
        }
        let signal = self.session.borrow_mut().as_mut().and_then(|session| session.resume.take());
        if let Some((resolve, _)) = signal {
            let _ = resolve.call0(&JsValue::NULL);
        }
    }

    pub(crate) async fn await_subscription(&self, promise: Promise) -> Result<(), JsValue> {
        within(
            JsFuture::from(promise),
            Some((self.timeouts.initial_data_timeout, "Subscription acknowledgement timed out")),
        )
        .await?;
        Ok(())
    }

    fn authenticated(
        &self,
        socket: &WebSocket,
        serialization: Option<SerializationType>,
        resolve: &Function,
    ) {
        if !matches_socket(&self.session, socket) {
            return;
        }
        if let Some(serialization) = serialization {
            self.serialization.set(serialization);
        }
        if let Some(session) = self.session.borrow_mut().as_mut() {
            session.authenticated = true;
        }
        let _ = resolve.call0(&JsValue::NULL);
    }

    fn on_frame(self: &Rc<Self>, socket: &WebSocket, event: &MessageEvent, resolve: &Function) {
        if !matches_socket(&self.session, socket) {
            return;
        }
        let event = self.decode_message(event);
        if !matches_socket(&self.session, socket) {
            return;
        }
        let Some(event) = event else { return };
        if self.socket().is_none() {
            match event {
                ServerMessage::AuthSuccess { protocol, .. } => {
                    self.authenticated(socket, Some(protocol.serialization), resolve);
                },
                ServerMessage::AuthError { message } => {
                    self.socket_failed(
                        socket,
                        &JsValue::from_str(&format!("Authentication failed: {message}")),
                        None,
                        false,
                    );
                },
                _ => {},
            }
            return;
        }
        if let Some(mut dispatch) =
            dispatch_subscription_server_message(&self.subscriptions, &self.learned_domains, &event)
        {
            let recovery = dispatch.recovery.take();
            let next = dispatch.invoke();
            if matches_socket(&self.session, socket) {
                if let Some(subscription) = recovery {
                    // Close the old server-side registration before reopening its cursor.
                    let unsubscribe = ClientMessage::Unsubscribe {
                        subscription_id: subscription.id.clone(),
                    };
                    let result = self.send(socket, &unsubscribe).and_then(|()| {
                        self.send(socket, &ClientMessage::Subscribe { subscription })
                    });
                    if let Err(error) = result {
                        self.socket_failed(socket, &error, None, true);
                    }
                }
                if let Some((id, last_seq_id)) = next {
                    if let Err(error) = send_next_batch_traced(
                        socket,
                        &id,
                        last_seq_id,
                        self.serialization.get(),
                        &self.on_send,
                    ) {
                        self.socket_failed(socket, &error, None, true);
                    }
                }
            }
        }
        self.check_resumed();
    }

    fn decode_message(&self, event: &MessageEvent) -> Option<ServerMessage> {
        let data = event.data();
        if !data.is_string() && self.serialization.get() == SerializationType::Domain {
            return link_common::ws_domain::decode_server_message(&decode_ws_binary_payload(
                event,
            )?)
            .ok();
        }
        let text = decode_ws_message(event)?;
        let callback = self.on_receive.borrow().clone();
        if let Some(callback) = callback {
            let _ = callback.call1(&JsValue::NULL, &JsValue::from_str(&text));
        }
        serde_json::from_str(&text).ok()
    }

    pub(crate) fn send(&self, socket: &WebSocket, message: &ClientMessage) -> Result<(), JsValue> {
        if socket.ready_state() != WebSocket::OPEN {
            return Err(JsValue::from_str("WebSocket connection is not open"));
        }
        send_ws_message_traced(socket, message, self.serialization.get(), &self.on_send)
    }

    fn start_keepalive(self: &Rc<Self>, socket: &WebSocket) {
        let interval = self.options.borrow().ping_interval_ms;
        if interval == 0 || !matches_socket(&self.session, socket) {
            return;
        }
        let weak = Rc::downgrade(self);
        let callback = Closure::new(move || {
            if let Some(connection) = weak.upgrade() {
                if let Some(socket) = connection.socket() {
                    if let Err(error) = connection.send(&socket, &ClientMessage::Ping) {
                        connection.socket_failed(&socket, &error, None, true);
                    }
                }
            }
        });
        let timer =
            Timer::interval(callback.as_ref().unchecked_ref(), Duration::from_millis(interval));
        if let Some(session) = self.session.borrow_mut().as_mut() {
            session.keepalive = Some((timer, callback));
        }
    }

    fn socket_failed(
        self: &Rc<Self>,
        socket: &WebSocket,
        reason: &JsValue,
        code: Option<u16>,
        report: bool,
    ) {
        if !matches_socket(&self.session, socket) {
            return;
        }
        let was_ready = self.ready.replace(false);
        self.close_socket(reason);
        if was_ready {
            if self.options.borrow().auto_reconnect {
                let promise = self.start(true);
                spawn_local(async move {
                    let _ = JsFuture::from(promise).await;
                });
            } else {
                reject_pending_subscriptions(&self.subscriptions, "WebSocket disconnected");
            }
            if report {
                self.report_error("WebSocket disconnected", reason);
            }
            self.emit_disconnect(&js_error_message(reason), code);
        }
    }

    fn report_error(&self, context: &str, error: &JsValue) {
        let auth_user = auth_user_from_wasm_auth(&self.auth.borrow()).map(str::to_owned);
        emit_runtime_js_error(&self.on_error, error, context, &self.url, auth_user.as_deref());
    }

    fn emit_disconnect(&self, message: &str, code: Option<u16>) {
        let callback = self.on_disconnect.borrow().clone();
        if let Some(callback) = callback {
            let reason = js_sys::Object::new();
            let _ = js_sys::Reflect::set(&reason, &"message".into(), &message.into());
            if let Some(code) = code {
                let _ = js_sys::Reflect::set(&reason, &"code".into(), &JsValue::from(code));
            }
            let _ = callback.call1(&JsValue::NULL, &reason);
        }
    }

    fn close_socket(&self, reason: &JsValue) {
        let session = self.session.borrow_mut().take();
        if let Some(session) = session {
            session.reject(reason);
        }
    }

    fn cancel_flight(&self) {
        self.generation.set(self.generation.get().wrapping_add(1));
        let flight = self.flight.borrow_mut().take();
        if let Some(flight) = flight {
            flight.abort.abort();
        }
        self.retry.borrow_mut().reset();
    }

    pub(crate) fn disconnect(&self, notify: bool) {
        let was_ready = self.ready.replace(false);
        self.cancel_flight();
        self.close_socket(&JsValue::from_str("Client disconnected"));
        reject_pending_subscriptions(&self.subscriptions, "Client disconnected");
        self.subscriptions.borrow_mut().clear();
        if was_ready && notify {
            self.emit_disconnect("manual disconnect", Some(1000));
        }
    }
}
