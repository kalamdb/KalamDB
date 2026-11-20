//! Authentication provider and models for KalamDB client.
//!
//! Handles JWT tokens, HTTP Basic Auth, async dynamic auth providers,
//! and authentication-related data models.

pub mod models;

#[cfg(feature = "tokio-runtime")]
mod provider;
#[cfg(feature = "tokio-runtime")]
pub use provider::{ArcDynAuthProvider, AuthProvider, DynamicAuthProvider, ResolvedAuth};

#[cfg(feature = "tokio-runtime")]
pub(crate) async fn exchange_login_credentials(
    client: &reqwest::Client,
    base_url: &str,
    user: &str,
    password: &str,
) -> crate::Result<crate::LoginResponse> {
    let url = format!("{base_url}/v1/api/auth/login");
    let body = serde_json::json!({ "user": user, "password": password });
    let response = client.post(url).json(&body).send().await?;
    let status = response.status();
    if !status.is_success() {
        let detail = response.text().await.unwrap_or_default();
        return Err(crate::KalamLinkError::AuthenticationError(format!(
            "Login failed during auth exchange ({status}): {detail}"
        )));
    }
    Ok(response.json().await?)
}
