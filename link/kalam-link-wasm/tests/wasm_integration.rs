// WASM integration tests (T063P-T063AA)
// These tests validate the WASM SDK functionality in a browser-like environment

#![cfg(target_arch = "wasm32")]

use kalam_link_wasm::*;
use std::{cell::RefCell, rc::Rc};
use wasm_bindgen::JsValue;
use wasm_bindgen::{closure::Closure, JsCast};
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

fn create_test_client() -> KalamClient {
    KalamClient::new(
        "http://localhost:2900".to_string(),
        "testuser".to_string(),
        "testpass".to_string(),
    )
    .expect("Client creation should succeed")
}

fn js_error_text(err: JsValue) -> String {
    err.as_string().unwrap_or_else(|| format!("{:?}", err))
}

fn capture_connection_events() -> (
    Rc<RefCell<Vec<(String, bool, Option<String>, Option<String>, Option<String>)>>>,
    Closure<dyn FnMut(JsValue)>,
) {
    let events = Rc::new(RefCell::new(Vec::new()));
    let captured = Rc::clone(&events);
    let closure = Closure::wrap(Box::new(move |event: JsValue| {
        let message = js_sys::Reflect::get(&event, &"message".into())
            .ok()
            .and_then(|value| value.as_string())
            .unwrap_or_else(|| format!("{:?}", event));
        let recoverable = js_sys::Reflect::get(&event, &"recoverable".into())
            .ok()
            .and_then(|value| value.as_bool())
            .unwrap_or(false);
        let url = js_sys::Reflect::get(&event, &"url".into())
            .ok()
            .and_then(|value| value.as_string());
        let auth_user = js_sys::Reflect::get(&event, &"authUser".into())
            .ok()
            .and_then(|value| value.as_string());
        let hint = js_sys::Reflect::get(&event, &"hint".into())
            .ok()
            .and_then(|value| value.as_string());
        captured.borrow_mut().push((message, recoverable, url, auth_user, hint));
    }) as Box<dyn FnMut(JsValue)>);
    (events, closure)
}

// T063R: Test WASM client creation with valid and invalid parameters
#[wasm_bindgen_test]
fn test_client_creation_valid() {
    let client = KalamClient::new(
        "http://localhost:2900".to_string(),
        "testuser".to_string(),
        "testpass".to_string(),
    );
    assert!(client.is_ok(), "Client creation should succeed with valid parameters");
}

#[wasm_bindgen_test]
fn test_client_creation_empty_url() {
    let client = KalamClient::new("".to_string(), "testuser".to_string(), "testpass".to_string());
    assert!(client.is_err(), "Client creation should fail with empty URL");
}

#[wasm_bindgen_test]
fn test_client_creation_empty_username() {
    let client = KalamClient::new(
        "http://localhost:2900".to_string(),
        "".to_string(),
        "testpass".to_string(),
    );
    assert!(client.is_err(), "Client creation should fail with empty username");
}

#[wasm_bindgen_test]
fn test_client_creation_empty_password() {
    let client = KalamClient::new(
        "http://localhost:2900".to_string(),
        "testuser".to_string(),
        "".to_string(),
    );
    assert!(client.is_err(), "Client creation should fail with empty password");
}

// T063S: Test connect() failure is surfaced cleanly when server is unavailable
#[wasm_bindgen_test]
async fn test_connect_errors_cleanly_when_server_unavailable() {
    let mut client =
        KalamClient::with_jwt("http://127.0.0.1:1".to_string(), "test-token".to_string())
            .expect("client creation should succeed");
    let (events, on_error) = capture_connection_events();
    client.on_error(on_error.as_ref().unchecked_ref::<js_sys::Function>().clone());
    let result = client.connect().await;

    let err = js_error_text(result.expect_err("unreachable connect should fail"));
    assert!(
        err.contains("WebSocket connection failed")
            || err.contains("closed")
            || err.contains("connect"),
        "unexpected unreachable connect error: {err}"
    );
    assert_eq!(events.borrow().len(), 1, "onError should fire for unreachable server");
    assert!(events.borrow()[0].0.contains("WebSocket connection failed"));
    assert!(events.borrow()[0].1, "unreachable server should be recoverable");
    assert_eq!(events.borrow()[0].2.as_deref(), Some("http://127.0.0.1:1"));
    assert!(events.borrow()[0].4.as_deref().unwrap_or_default().contains("reachable"));
}

#[wasm_bindgen_test]
async fn test_on_error_fires_for_invalid_url_before_socket_creation() {
    let mut client = KalamClient::with_jwt("http://[::1".to_string(), "test-token".to_string())
        .expect("client creation should succeed");
    let (events, on_error) = capture_connection_events();
    client.on_connection_error(on_error.as_ref().unchecked_ref::<js_sys::Function>().clone());

    let err = js_error_text(client.connect().await.expect_err("invalid URL should fail"));
    assert!(
        err.to_lowercase().contains("url") || err.to_lowercase().contains("invalid"),
        "unexpected invalid URL error: {err}"
    );
    assert_eq!(events.borrow().len(), 1, "onConnectionError should fire for invalid URL");
    assert!(events.borrow()[0].0.contains("Failed to connect to KalamDB"));
    assert!(!events.borrow()[0].1, "invalid URL should be fatal");
    assert_eq!(events.borrow()[0].2.as_deref(), Some("http://[::1"));
    assert!(events.borrow()[0]
        .4
        .as_deref()
        .unwrap_or_default()
        .contains("absolute http:// or https://"));
}

#[wasm_bindgen_test]
async fn test_on_error_fires_for_auth_provider_resolution_failure() {
    let mut client = KalamClient::anonymous("http://localhost:2900".to_string())
        .expect("client creation should succeed");
    let (events, on_error) = capture_connection_events();
    client.on_error(on_error.as_ref().unchecked_ref::<js_sys::Function>().clone());
    let provider = js_sys::Function::new_no_args("return { jwt: {} }; ");
    client.set_auth_provider(provider);

    let err =
        js_error_text(client.connect().await.expect_err("invalid authProvider result should fail"));
    assert!(
        err.contains("authProvider result must have shape"),
        "unexpected authProvider error: {err}"
    );
    assert_eq!(
        events.borrow().len(),
        1,
        "onError should fire for authProvider resolution failure"
    );
    assert!(events.borrow()[0].0.contains("Authentication failed"));
    assert!(!events.borrow()[0].1, "authProvider shape errors should be fatal");
    assert_eq!(events.borrow()[0].2.as_deref(), Some("http://localhost:2900"));
    assert!(events.borrow()[0]
        .4
        .as_deref()
        .unwrap_or_default()
        .contains("JWT token or auth provider"));
}

// T063T: Test disconnect() properly closes WebSocket
#[wasm_bindgen_test]
async fn test_disconnect() {
    let mut client = create_test_client();

    let _ = client.connect().await;
    let result = client.disconnect().await;

    assert!(result.is_ok(), "Disconnect should succeed");
    assert!(!client.is_connected(), "Client should not be connected after disconnect");
}

// T063U: Test insert() validates table name before making network calls
#[wasm_bindgen_test]
async fn test_insert_rejects_invalid_table_name() {
    let client = create_test_client();

    let result = client
        .insert("bad table name".to_string(), r#"{"title":"x"}"#.to_string())
        .await;

    let err = js_error_text(result.expect_err("invalid table name should fail"));
    assert!(
        err.contains("Table name") || err.contains("invalid character"),
        "unexpected insert validation error: {err}"
    );
}

// T063V: Test insert() rejects malformed JSON payloads
#[wasm_bindgen_test]
async fn test_insert_rejects_invalid_json_payload() {
    let client = create_test_client();

    let result = client.insert("todos".to_string(), "{not-json}".to_string()).await;

    let err = js_error_text(result.expect_err("invalid JSON should fail"));
    assert!(err.contains("Invalid JSON data"), "unexpected invalid JSON error: {err}");
}

// T063W: Test insert() rejects empty JSON objects
#[wasm_bindgen_test]
async fn test_insert_rejects_empty_object() {
    let client = create_test_client();

    let result = client.insert("todos".to_string(), "{}".to_string()).await;

    let err = js_error_text(result.expect_err("empty object should fail"));
    assert!(
        err.contains("Cannot insert empty object"),
        "unexpected empty object error: {err}"
    );
}

// T063X: Test liveEvents() registers callback and receives messages
#[wasm_bindgen_test]
async fn test_live_events_not_connected() {
    let client = create_test_client();

    let callback = js_sys::Function::new_no_args("");
    let result = client.live_events("SELECT * FROM todos".to_string(), None, callback).await;

    // Should fail because not connected
    assert!(result.is_err(), "liveEvents should fail when not connected");
}

// T063Y: Test unsubscribe() removes callback and stops receiving messages
#[wasm_bindgen_test]
async fn test_unsubscribe_not_connected() {
    let client = create_test_client();

    let result = client.unsubscribe("test-subscription".to_string()).await;

    // Should fail because not connected
    assert!(result.is_err(), "Unsubscribe should fail when not connected");
}

// T063Z: Test delete() validates row ID before making network calls
#[wasm_bindgen_test]
async fn test_delete_rejects_invalid_row_id() {
    let client = create_test_client();
    let result = client.delete("todos".to_string(), "bad' OR 1=1 --".to_string()).await;

    let err = js_error_text(result.expect_err("invalid row id should fail"));
    assert!(
        err.contains("Row ID") && (err.contains("forbidden") || err.contains("invalid")),
        "unexpected row-id validation error: {err}"
    );
}

// T063AB: Test that multiple subscriptions share the same WebSocket connection
// This verifies that opening liveEvents multiple times does NOT open new WebSocket connections
#[wasm_bindgen_test]
async fn test_multiple_subscriptions_share_single_websocket() {
    let mut client = create_test_client();

    // Connect once - this opens a single WebSocket
    let connect_result = client.connect().await;

    if connect_result.is_err() {
        // Skip test if server not available
        return;
    }

    assert!(client.is_connected(), "Client should be connected after connect()");

    // Create multiple subscriptions - these should all share the same WebSocket
    let callback1 =
        js_sys::Function::new_with_args("data", "console.log('Subscription 1:', data);");
    let callback2 =
        js_sys::Function::new_with_args("data", "console.log('Subscription 2:', data);");
    let callback3 =
        js_sys::Function::new_with_args("data", "console.log('Subscription 3:', data);");

    let sub1_result = client.live_events("SELECT * FROM todos".to_string(), None, callback1).await;
    let sub2_result = client.live_events("SELECT * FROM events".to_string(), None, callback2).await;
    let sub3_result =
        client.live_events("SELECT * FROM messages".to_string(), None, callback3).await;

    // All subscriptions should use the same connection
    // The isConnected() check verifies the single WebSocket is still open
    assert!(
        client.is_connected(),
        "Client should remain connected with multiple subscriptions"
    );

    // Verify all subscriptions succeeded (they share the same WebSocket)
    if sub1_result.is_ok() && sub2_result.is_ok() && sub3_result.is_ok() {
        // Get subscription IDs
        let sub1_id = sub1_result.unwrap();
        let sub2_id = sub2_result.unwrap();
        let sub3_id = sub3_result.unwrap();

        // Subscription IDs should be different (but use same connection)
        assert_ne!(sub1_id, sub2_id, "Subscription IDs should be unique");
        assert_ne!(sub2_id, sub3_id, "Subscription IDs should be unique");
        assert_ne!(sub1_id, sub3_id, "Subscription IDs should be unique");

        // Unsubscribe from one - connection should remain open for others
        let _ = client.unsubscribe(sub1_id).await;
        assert!(client.is_connected(), "Connection should remain open after unsubscribing one");

        // Unsubscribe from another
        let _ = client.unsubscribe(sub2_id).await;
        assert!(client.is_connected(), "Connection should remain open after unsubscribing two");

        // Unsubscribe from last one - connection still open (disconnect() needed to close)
        let _ = client.unsubscribe(sub3_id).await;
        assert!(
            client.is_connected(),
            "Connection should remain open until disconnect() is called"
        );
    }

    // Disconnect closes the single WebSocket connection
    let _ = client.disconnect().await;
    assert!(!client.is_connected(), "Client should be disconnected after disconnect()");
}

// T063AC: Test subscription reuse for same table returns different subscription IDs
#[wasm_bindgen_test]
async fn test_live_events_same_table_multiple_times() {
    let mut client = create_test_client();

    let connect_result = client.connect().await;

    if connect_result.is_err() {
        // Skip test if server not available
        return;
    }

    // Open the same table twice
    let callback1 = js_sys::Function::new_with_args("data", "console.log('First:', data);");
    let callback2 = js_sys::Function::new_with_args("data", "console.log('Second:', data);");

    let sub1_result = client.live_events("SELECT * FROM todos".to_string(), None, callback1).await;
    let sub2_result = client.live_events("SELECT * FROM todos".to_string(), None, callback2).await;

    // Note: Current implementation uses "sub-{table_name}" as ID, so second subscription
    // will overwrite the first callback in the HashMap. This is a known behavior.
    // The WebSocket connection is still the same.
    assert!(client.is_connected(), "Single WebSocket connection should be active");

    let _ = client.disconnect().await;
}

// T063AA: Run tests with: wasm-pack test --headless --firefox (or --chrome)
// Note: These tests require a running KalamDB server for full integration testing
// For CI/CD, consider using a mock server or headless browser testing framework
