//! Public OAuth/OIDC provider metadata for browser login.

use std::{sync::Arc, time::Duration};

use actix_web::{web, HttpResponse};
use kalamdb_configs::OAuthProviderConfig;
use kalamdb_core::app_context::AppContext;
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize)]
struct OidcDiscoveryResponse {
    authorization_endpoint: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct OAuthProviderInfo {
    id: &'static str,
    display_name: &'static str,
    issuer: String,
    client_id: String,
    authorization_endpoint: Option<String>,
    scopes: &'static [&'static str],
}

/// GET /v1/api/auth/oauth/providers
///
/// Returns only public provider metadata needed by the Admin UI to start an
/// OIDC browser redirect. Secrets and disabled/incomplete providers are never
/// exposed.
pub async fn oauth_providers_handler(app_context: web::Data<Arc<AppContext>>) -> HttpResponse {
    let config = app_context.config();
    if !config.oauth.enabled {
        return HttpResponse::Ok().json(Vec::<OAuthProviderInfo>::new());
    }

    let client = match reqwest::Client::builder().timeout(Duration::from_secs(5)).build() {
        Ok(client) => client,
        Err(error) => {
            log::warn!("failed to build OAuth discovery client: {}", error);
            return HttpResponse::Ok().json(Vec::<OAuthProviderInfo>::new());
        },
    };

    let candidates = [
        ("google", "Google", &config.oauth.providers.google),
        ("github", "GitHub", &config.oauth.providers.github),
        ("azure", "Microsoft", &config.oauth.providers.azure),
        ("openid", "OpenID", &config.oauth.providers.openid),
        ("firebase", "Firebase", &config.oauth.providers.firebase),
    ];

    let mut providers = Vec::with_capacity(candidates.len());
    for (id, display_name, provider) in candidates {
        if let Some(info) = provider_info(&client, id, display_name, provider).await {
            providers.push(info);
        }
    }

    HttpResponse::Ok().json(providers)
}

async fn provider_info(
    client: &reqwest::Client,
    id: &'static str,
    display_name: &'static str,
    provider: &OAuthProviderConfig,
) -> Option<OAuthProviderInfo> {
    if !provider.enabled || provider.issuer.is_empty() {
        return None;
    }

    let client_id = provider.client_id.as_ref()?.trim();
    if client_id.is_empty() {
        return None;
    }

    let authorization_endpoint = discover_authorization_endpoint(client, &provider.issuer).await;
    Some(OAuthProviderInfo {
        id,
        display_name,
        issuer: provider.issuer.clone(),
        client_id: client_id.to_string(),
        authorization_endpoint,
        scopes: &["openid", "email", "profile"],
    })
}

async fn discover_authorization_endpoint(client: &reqwest::Client, issuer: &str) -> Option<String> {
    let discovery_url =
        format!("{}/.well-known/openid-configuration", issuer.trim_end_matches('/'));

    let response = match client.get(&discovery_url).send().await {
        Ok(response) => response,
        Err(error) => {
            log::warn!("OIDC discovery failed for issuer '{}': {}", issuer, error);
            return None;
        },
    };

    if !response.status().is_success() {
        log::warn!("OIDC discovery for issuer '{}' returned status {}", issuer, response.status());
        return None;
    }

    match response.json::<OidcDiscoveryResponse>().await {
        Ok(discovery) => discovery.authorization_endpoint,
        Err(error) => {
            log::warn!("failed to parse OIDC discovery for issuer '{}': {}", issuer, error);
            None
        },
    }
}
