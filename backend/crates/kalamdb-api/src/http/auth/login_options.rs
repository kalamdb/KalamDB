use std::sync::Arc;

use actix_web::{web, HttpResponse};
use kalamdb_auth::providers::jwt_config;
use kalamdb_core::app_context::AppContext;

use super::models::{
    AuthLoginOptionsResponse, LocalLoginOptions, OidcDeviceFlowOptions, OidcLoginOptions,
};

pub async fn login_options_handler(app_context: web::Data<Arc<AppContext>>) -> HttpResponse {
    let config = app_context.config();
    let oidc = if config.auth.oidc.enabled {
        match (config.auth.oidc.issuer_str(), config.auth.oidc.client_id_str()) {
            (Some(issuer), Some(client_id)) => {
                let public_metadata =
                    match jwt_config::get_jwt_config().oidc_public_metadata(issuer).await {
                        Ok(metadata) => Some(metadata),
                        Err(error) => {
                            log::warn!("OIDC login-options metadata discovery failed: {}", error);
                            None
                        },
                    };
                let device_authorization_endpoint = public_metadata
                    .as_ref()
                    .and_then(|metadata| metadata.device_authorization_endpoint.clone())
                    .or_else(|| config.auth.oidc.device_authorization_endpoint.clone());

                Some(OidcLoginOptions {
                    enabled: true,
                    display_name: config.auth.oidc.display_name.clone(),
                    issuer: issuer.to_string(),
                    client_id: client_id.to_string(),
                    authorization_endpoint: public_metadata
                        .as_ref()
                        .and_then(|metadata| metadata.authorization_endpoint.clone()),
                    token_endpoint: public_metadata
                        .as_ref()
                        .and_then(|metadata| metadata.token_endpoint.clone()),
                    device_authorization_endpoint: device_authorization_endpoint.clone(),
                    scopes: public_metadata
                        .as_ref()
                        .map(|metadata| metadata.scopes.clone())
                        .unwrap_or_else(|| config.auth.oidc.scopes.clone()),
                    device_flow: Some(OidcDeviceFlowOptions {
                        direct_supported: device_authorization_endpoint.is_some(),
                        broker_supported: config.auth.oidc.broker_device_flow_enabled,
                        device_authorization_endpoint,
                        broker_start_endpoint: Some("/v1/api/auth/oidc/device/start".to_string()),
                        broker_poll_endpoint: Some("/v1/api/auth/oidc/device/poll".to_string()),
                    }),
                })
            },
            _ => None,
        }
    } else {
        None
    };

    HttpResponse::Ok().json(AuthLoginOptionsResponse {
        local: LocalLoginOptions {
            enabled: config.auth.local.enabled,
        },
        oidc,
    })
}
