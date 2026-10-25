use actix_ws::{AggregatedMessage, CloseCode, CloseReason, Session};
use futures_util::StreamExt;
use kalamdb_commons::{websocket::SerializationType, WebSocketMessage};
use kalamdb_jobs::health_monitor::{
    decrement_websocket_sessions, increment_websocket_sessions, record_activity_now,
};
use kalamdb_live::{ConnectionEvent, ConnectionRegistration};
use log::{debug, error, warn};

use super::{
    context::{PendingUpgradeAuth, WsHandlerContext},
    events::{
        auth::apply_pending_upgrade_auth, cleanup::cleanup_connection, send_error, send_json,
        send_wire_notification,
    },
    messages::{handle_binary_message, handle_text_message},
    models::WsErrorCode,
    protocol::{is_expected_ws_disconnect, is_ws_message_too_large},
};

pub(super) async fn run_websocket(
    client_ip: kalamdb_commons::models::ConnectionInfo,
    session: Session,
    msg_stream: actix_ws::MessageStream,
    registration: ConnectionRegistration,
    handler_context: WsHandlerContext,
    pending_auth: Option<PendingUpgradeAuth>,
) {
    record_activity_now();
    increment_websocket_sessions();

    let mut event_rx = registration.event_rx;
    let mut notification_rx = registration.notification_rx;
    let connection_state = registration.state;
    let connection_id = connection_state.connection_id().clone();

    {
        let mut session = session;
        let max_message_size = handler_context.max_message_size;
        let mut msg_stream = msg_stream
            .max_frame_size(max_message_size)
            .aggregate_continuations()
            .max_continuation_size(max_message_size);

        if let Some(pending) = pending_auth {
            connection_state.mark_auth_started();
            if apply_pending_upgrade_auth(
                &connection_state,
                &mut session,
                pending,
                handler_context.compression_enabled,
            )
            .await
            .is_err()
            {
                let _ = session
                    .close(Some(CloseReason {
                        code: CloseCode::Policy,
                        description: Some("Authentication failed".into()),
                    }))
                    .await;
                cleanup_connection(
                    &connection_state,
                    &handler_context.connection_registry,
                    &handler_context.rate_limiter,
                    &handler_context.live_query_manager,
                )
                .await;
                decrement_websocket_sessions();
                return;
            }
            debug!("WebSocket authenticated from upgrade header: {}", connection_id);
        }

        let mut pending_gaps: Vec<std::sync::Arc<str>> = Vec::new();
        loop {
            // A full notification buffer is what caused the gap. Deliver those
            // rows before asking the client to resume, so the resume starts
            // from an empty buffer and the replay log can fill it.
            if notification_rx.is_empty() {
                for subscription_id in pending_gaps.drain(..) {
                    warn!(
                        "Live subscription {} lapsed on connection {}",
                        subscription_id, connection_id
                    );
                    let _ = send_error(
                        &mut session,
                        &subscription_id,
                        WsErrorCode::DeliveryGap,
                        "live delivery gap; resubscribe from the last cursor",
                        handler_context.compression_enabled,
                    )
                    .await;
                }
            }
            tokio::select! {
                biased;

                event = event_rx.recv() => {
                    match event {
                        Some(ConnectionEvent::AuthTimeout) => {
                            error!("WebSocket auth timeout: {}", connection_id);
                            let msg = WebSocketMessage::AuthError {
                                message: "Authentication timeout - no auth message received within 3 seconds".to_string(),
                            };
                            let _ = send_json(&mut session, &msg, handler_context.compression_enabled).await;
                            let _ = session.close(Some(CloseReason {
                                code: CloseCode::Policy,
                                description: Some("Authentication timeout".into()),
                            })).await;
                            break;
                        }
                        Some(ConnectionEvent::HeartbeatTimeout) => {
                            debug!("WebSocket heartbeat timeout: {}", connection_id);
                            let _ = session.close(Some(CloseReason {
                                code: CloseCode::Normal,
                                description: Some("Heartbeat timeout".into()),
                            })).await;
                            break;
                        }
                        Some(ConnectionEvent::Shutdown) => {
                            debug!("WebSocket shutdown requested: {}", connection_id);
                            let _ = session.close(Some(CloseReason {
                                code: CloseCode::Away,
                                description: Some("Server shutting down".into()),
                            })).await;
                            break;
                        }
                        Some(ConnectionEvent::SubscriptionLapsed { subscription_id }) => {
                            pending_gaps.push(subscription_id);
                        }
                        None => break,
                    }
                }

                msg = msg_stream.next() => {
                    match msg {
                        Some(Ok(AggregatedMessage::Ping(bytes))) => {
                            connection_state.update_heartbeat();
                            if session.pong(&bytes).await.is_err() {
                                break;
                            }
                        }
                        Some(Ok(AggregatedMessage::Pong(_))) => {
                            connection_state.update_heartbeat();
                        }
                        Some(Ok(AggregatedMessage::Text(text))) => {
                            record_activity_now();
                            connection_state.update_heartbeat();

                            if text.len() > max_message_size {
                                warn!(
                                    "Message too large from {}: {} bytes (max {})",
                                    connection_id,
                                    text.len(),
                                    max_message_size
                                );
                                let _ = send_error(
                                    &mut session,
                                    "protocol",
                                    WsErrorCode::MessageTooLarge,
                                    &format!(
                                        "Message exceeds maximum size of {} bytes",
                                        max_message_size
                                    ),
                                    handler_context.compression_enabled,
                                )
                                .await;
                                continue;
                            }

                            if !handler_context
                                .rate_limiter
                                .check_message_rate(&connection_id)
                            {
                                warn!("Message rate limit exceeded: {}", connection_id);
                                let _ = send_error(
                                    &mut session,
                                    "rate_limit",
                                    WsErrorCode::RateLimitExceeded,
                                    "Too many messages",
                                    handler_context.compression_enabled,
                                )
                                .await;
                                continue;
                            }

                            if let Err(err) = handle_text_message(
                                &connection_state,
                                &client_ip,
                                &text,
                                &mut session,
                                &handler_context.app_context,
                                &handler_context.rate_limiter,
                                &handler_context.live_query_manager,
                                &handler_context.user_repo,
                                handler_context.compression_enabled,
                            )
                            .await
                            {
                                warn!("Error handling message: {}", err);
                                let _ = session.close(Some(CloseReason {
                                    code: CloseCode::Error,
                                    description: Some(err),
                                })).await;
                                break;
                            }
                        }
                        Some(Ok(AggregatedMessage::Binary(data))) => {
                            record_activity_now();
                            connection_state.update_heartbeat();

                            if data.len() > max_message_size {
                                warn!(
                                    "Binary message too large from {}: {} bytes (max {})",
                                    connection_id,
                                    data.len(),
                                    max_message_size
                                );
                                let _ = send_error(
                                    &mut session,
                                    "protocol",
                                    WsErrorCode::MessageTooLarge,
                                    &format!(
                                        "Message exceeds maximum size of {} bytes",
                                        max_message_size
                                    ),
                                    handler_context.compression_enabled,
                                )
                                .await;
                                continue;
                            }

                            if let Err(err) = handle_binary_message(
                                &connection_state,
                                &client_ip,
                                data,
                                &mut session,
                                &handler_context.app_context,
                                &handler_context.rate_limiter,
                                &handler_context.live_query_manager,
                                &handler_context.user_repo,
                                handler_context.compression_enabled,
                                max_message_size,
                            )
                            .await
                            {
                                if connection_state.serialization_type() == SerializationType::Domain {
                                    warn!("Invalid domain frame from {}: {}", connection_id, err);
                                    let _ = send_error(
                                        &mut session,
                                        "protocol",
                                        WsErrorCode::UnsupportedData,
                                        &err,
                                        handler_context.compression_enabled,
                                    )
                                    .await;
                                    continue;
                                }

                                warn!("Error handling binary message: {}", err);
                                let _ = session.close(Some(CloseReason {
                                    code: CloseCode::Error,
                                    description: Some(err),
                                })).await;
                                break;
                            }
                        }
                        Some(Ok(AggregatedMessage::Close(reason))) => {
                            debug!("Client requested close: {:?}", reason);
                            let _ = session.close(reason).await;
                            break;
                        }
                        Some(Err(err)) => {
                            if is_ws_message_too_large(&err) {
                                warn!(
                                    "WebSocket message exceeded {} bytes ({})",
                                    max_message_size, connection_id
                                );
                                let _ = send_error(
                                    &mut session,
                                    "protocol",
                                    WsErrorCode::MessageTooLarge,
                                    &format!(
                                        "Message exceeds maximum size of {} bytes",
                                        max_message_size
                                    ),
                                    handler_context.compression_enabled,
                                )
                                .await;
                                let _ = session
                                    .close(Some(CloseReason {
                                        code: CloseCode::Size,
                                        description: Some("Message too large".into()),
                                    }))
                                    .await;
                                break;
                            }
                            if is_expected_ws_disconnect(&err) {
                                debug!("WebSocket client disconnected ({}): {}", connection_id, err);
                            } else {
                                warn!("WebSocket error ({}): {}", connection_id, err);
                            }
                            break;
                        }
                        None => {
                            debug!("WebSocket stream ended: {}", connection_id);
                            break;
                        }
                    }
                }

                notification = notification_rx.recv() => {
                    let Some(first) = notification else {
                        break;
                    };
                    let ser = connection_state.serialization_type();
                    if send_wire_notification(&mut session, first.as_ref(), ser, handler_context.compression_enabled).await.is_err() {
                        break;
                    }
                    // A burst of changes for this connection is already queued.
                    // Write them before returning to select so a pending ping
                    // does not sit between every notification.
                    let mut disconnected = false;
                    // Drain a short burst, then return to select so heartbeats
                    // still run when the buffer is large.
                    for _ in 1..128 {
                        match notification_rx.try_recv() {
                            Ok(next) => {
                                if send_wire_notification(&mut session, next.as_ref(), ser, handler_context.compression_enabled).await.is_err() {
                                    disconnected = true;
                                    break;
                                }
                            }
                            Err(tokio::sync::mpsc::error::TryRecvError::Empty) => break,
                            Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => {
                                disconnected = true;
                                break;
                            }
                        }
                    }
                    if disconnected {
                        break;
                    }
                }
            }
        }
    }

    cleanup_connection(
        &connection_state,
        &handler_context.connection_registry,
        &handler_context.rate_limiter,
        &handler_context.live_query_manager,
    )
    .await;

    decrement_websocket_sessions();
}
