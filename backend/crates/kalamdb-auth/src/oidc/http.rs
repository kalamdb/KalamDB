use std::time::Duration;

pub(crate) type OidcHttpClient = openidconnect::reqwest::Client;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct OidcHttpClientConfig {
    pub timeout: Duration,
}

impl Default for OidcHttpClientConfig {
    fn default() -> Self {
        Self {
            // Keep discovery bounded so a down IdP cannot stall actix workers.
            timeout: Duration::from_secs(3),
        }
    }
}

pub(crate) fn redirect_disabled_reqwest_client(
    config: OidcHttpClientConfig,
) -> Result<OidcHttpClient, openidconnect::reqwest::Error> {
    openidconnect::reqwest::ClientBuilder::new()
        .timeout(config.timeout)
        .connect_timeout(Duration::from_secs(1))
        .redirect(openidconnect::reqwest::redirect::Policy::none())
        .build()
}

pub(crate) fn default_oidc_http_client() -> Result<OidcHttpClient, openidconnect::reqwest::Error> {
    redirect_disabled_reqwest_client(OidcHttpClientConfig::default())
}
