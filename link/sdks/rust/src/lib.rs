//! Shared helpers for Rust SDK integration tests.

use std::time::Duration;

use kalam_client::{AuthProvider, KalamLinkClient, KalamLinkError};
use reqwest::Client;

pub fn server_url() -> String {
    std::env::var("KALAMDB_SERVER_URL")
        .or_else(|_| std::env::var("KALAMDB_URL"))
        .unwrap_or_else(|_| "http://localhost:2900".to_string())
}

pub fn root_password() -> String {
    std::env::var("KALAMDB_ROOT_PASSWORD").unwrap_or_else(|_| "kalamdb123".to_string())
}

pub fn auth() -> AuthProvider {
    AuthProvider::system_user_auth(root_password())
}

pub async fn is_server_running() -> bool {
    let client = match Client::builder().timeout(Duration::from_secs(3)).build() {
        Ok(client) => client,
        Err(_) => return false,
    };

    let url = server_url();
    for path in ["/health", "/v1/api/healthcheck"] {
        if client
            .get(format!("{url}{path}"))
            .send()
            .await
            .map(|response| response.status().is_success())
            .unwrap_or(false)
        {
            return true;
        }
    }

    false
}

pub fn build_client() -> Result<KalamLinkClient, KalamLinkError> {
    KalamLinkClient::builder()
        .base_url(server_url())
        .auth(auth())
        .timeout(Duration::from_secs(30))
        .build()
}

pub fn unique_ident(prefix: &str) -> String {
    format!(
        "{prefix}_{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    )
}
