//! WebSocket handler for live query subscriptions
//!
//! This module provides the HTTP endpoint for establishing WebSocket connections
//! and managing live query subscriptions using actix-ws (non-actor based).
//!
//! Connection lifecycle and heartbeat management is handled by the shared
//! ConnectionsManager from kalamdb-core.
//!
//! Architecture:
//! - Connection created in ConnectionsManager on WebSocket open
//! - Subscriptions stored in ConnectionState.subscriptions
//! - No local tracking needed - everything is in ConnectionState

use std::sync::Arc;

use actix_web::{get, web, Error, HttpRequest, HttpResponse};
use kalamdb_auth::UserRepository;
use kalamdb_core::app_context::AppContext;
use kalamdb_live::{ConnectionId, ConnectionsManager, LiveQueryManager};
use log::{debug, warn};

use super::{
    context::WsHandlerContext,
    events::auth::spawn_upgrade_auth,
    protocol::{compression_enabled_from_query, parse_upgrade_auth, validate_origin},
    runtime::run_websocket,
};
use crate::limiter::RateLimiter;

/// GET /v1/ws - Establish WebSocket connection
///
/// Accepts unauthenticated WebSocket connections.
/// JWT can be supplied on the upgrade (`Authorization` or `kalamdb.jwt.*`
/// subprotocol) so AuthSuccess can be sent without an extra round-trip.
/// Otherwise authentication happens via a post-connection Authenticate message.
/// Uses ConnectionsManager for consolidated connection state management.
///
/// Security:
/// - Origin header validation (if configured)
/// - Message size limits enforced
/// - Rate limiting per connection
#[get("/ws")]
pub async fn websocket_handler(
    req: HttpRequest,
    stream: web::Payload,
    app_context: web::Data<Arc<AppContext>>,
    rate_limiter: web::Data<Arc<RateLimiter>>,
    live_query_manager: web::Data<Arc<LiveQueryManager>>,
    user_repo: web::Data<Arc<dyn UserRepository>>,
    connection_registry: web::Data<Arc<ConnectionsManager>>,
) -> Result<HttpResponse, Error> {
    if connection_registry.is_shutting_down() {
        return Ok(HttpResponse::ServiceUnavailable().body("Server is shutting down"));
    }

    if let Err(response) = validate_origin(&req, app_context.get_ref()) {
        return Ok(response);
    }

    let compression_enabled = compression_enabled_from_query(&req);
    let pre_auth = parse_upgrade_auth(&req);
    let echo_subprotocol = pre_auth.as_ref().and_then(|auth| auth.echo_subprotocol.clone());

    let connection_id = ConnectionId::new(uuid::Uuid::new_v4().simple().to_string());
    let client_ip = kalamdb_auth::extract_client_ip_secure(&req);
    let _connect_span = tracing::debug_span!(
        "ws.connect",
        connection_id = %connection_id,
        client_ip = ?client_ip,
        compression_enabled
    )
    .entered();

    debug!(
        "New WebSocket connection: {} (pre_auth={})",
        connection_id,
        if pre_auth.as_ref().is_some_and(|auth| auth.echo_subprotocol.is_some()) {
            "protocol"
        } else if pre_auth.is_some() {
            "header"
        } else {
            "pending"
        }
    );

    let pending_auth = pre_auth.map(|auth| {
        spawn_upgrade_auth(
            client_ip.clone(),
            auth,
            Arc::clone(rate_limiter.get_ref()),
            Arc::clone(user_repo.get_ref()),
        )
    });

    let registration =
        match connection_registry.register_connection(connection_id.clone(), client_ip.clone()) {
            Some(reg) => reg,
            None => {
                if let Some(pending) = pending_auth {
                    pending.auth_task.abort();
                }
                warn!("Rejecting WebSocket during shutdown: {}", connection_id);
                return Ok(HttpResponse::ServiceUnavailable().body("Server shutting down"));
            },
        };

    let handshake = if let Some(ref protocol) = echo_subprotocol {
        actix_ws::handle_with_protocols(&req, stream, &[protocol.as_str()])
    } else {
        actix_ws::handle(&req, stream)
    };
    let (response, session, msg_stream) = match handshake {
        Ok(parts) => parts,
        Err(error) => {
            if let Some(pending) = pending_auth {
                pending.auth_task.abort();
            }
            return Err(error);
        },
    };

    let handler_context = WsHandlerContext::new(
        Arc::clone(app_context.get_ref()),
        Arc::clone(rate_limiter.get_ref()),
        Arc::clone(live_query_manager.get_ref()),
        Arc::clone(user_repo.get_ref()),
        Arc::clone(connection_registry.get_ref()),
        app_context.config().security.max_ws_message_size,
        compression_enabled,
    );

    actix_web::rt::spawn(async move {
        run_websocket(client_ip, session, msg_stream, registration, handler_context, pending_auth)
            .await;
    });

    Ok(response)
}
