use std::{cell::RefCell, rc::Weak};

use js_sys::Function;
use wasm_bindgen::{closure::Closure, JsCast, JsValue};
use web_sys::{CloseEvent, Event, MessageEvent, WebSocket};

use super::{BrowserConnection, Timer};

pub(super) struct SocketSession {
    pub(super) socket:        WebSocket,
    pub(super) resume:        Option<(Function, Function)>,
    pub(super) keepalive:     Option<(Timer, Closure<dyn FnMut()>)>,
    pub(super) authenticated: bool,
    handshake_reject:         Function,
    _open:                    Closure<dyn FnMut()>,
    _error:                   Closure<dyn FnMut(Event)>,
    _close:                   Closure<dyn FnMut(CloseEvent)>,
    _message:                 Closure<dyn FnMut(MessageEvent)>,
}

impl SocketSession {
    pub(super) fn new(
        socket: WebSocket,
        connection: Weak<BrowserConnection>,
        authenticate: Option<link_common::models::ClientMessage>,
        requires_auth: bool,
        resolve: Function,
        reject: Function,
    ) -> Self {
        let on_open = connection.clone();
        let open_socket = socket.clone();
        let open_resolve = resolve.clone();
        let open = Closure::new(move || {
            let Some(connection) = on_open.upgrade() else {
                return;
            };
            if let Some(message) = &authenticate {
                if let Err(error) = connection.send(&open_socket, message) {
                    connection.socket_failed(&open_socket, &error, None, true);
                }
            } else if !requires_auth {
                connection.authenticated(&open_socket, None, &open_resolve);
            }
        });

        let on_error = connection.clone();
        let error_socket = socket.clone();
        let error = Closure::new(move |_: Event| {
            if let Some(connection) = on_error.upgrade() {
                connection.socket_failed(
                    &error_socket,
                    &JsValue::from_str("WebSocket connection failed"),
                    None,
                    true,
                );
            }
        });

        let on_close = connection.clone();
        let close_socket = socket.clone();
        let close = Closure::new(move |event: CloseEvent| {
            if let Some(connection) = on_close.upgrade() {
                let reason = if event.reason().is_empty() {
                    format!("WebSocket closed (code {})", event.code())
                } else {
                    event.reason()
                };
                connection.socket_failed(
                    &close_socket,
                    &JsValue::from_str(&reason),
                    Some(event.code()),
                    false,
                );
            }
        });

        let message_socket = socket.clone();
        let message = Closure::new(move |event: MessageEvent| {
            if let Some(connection) = connection.upgrade() {
                connection.on_frame(&message_socket, &event, &resolve);
            }
        });

        socket.set_onopen(Some(open.as_ref().unchecked_ref()));
        socket.set_onerror(Some(error.as_ref().unchecked_ref()));
        socket.set_onclose(Some(close.as_ref().unchecked_ref()));
        socket.set_onmessage(Some(message.as_ref().unchecked_ref()));
        Self {
            socket,
            resume: None,
            keepalive: None,
            authenticated: false,
            handshake_reject: reject,
            _open: open,
            _error: error,
            _close: close,
            _message: message,
        }
    }

    pub(super) fn reject(&self, error: &JsValue) {
        let _ = self.handshake_reject.call1(&JsValue::NULL, error);
        if let Some((_, reject)) = &self.resume {
            let _ = reject.call1(&JsValue::NULL, error);
        }
    }
}

impl Drop for SocketSession {
    fn drop(&mut self) {
        self.socket.set_onopen(None);
        self.socket.set_onerror(None);
        self.socket.set_onclose(None);
        self.socket.set_onmessage(None);
        let _ = self.socket.close();
    }
}

pub(super) fn matches_socket(session: &RefCell<Option<SocketSession>>, socket: &WebSocket) -> bool {
    session
        .borrow()
        .as_ref()
        .is_some_and(|active| js_sys::Object::is(active.socket.as_ref(), socket.as_ref()))
}
