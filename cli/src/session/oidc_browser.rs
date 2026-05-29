use std::{
    io::{BufRead, BufReader, Write},
    net::TcpListener,
    process::Command,
    time::Duration,
};

use indicatif::{ProgressBar, ProgressStyle};
use openidconnect::{
    core::{CoreAuthenticationFlow, CoreClient, CoreProviderMetadata},
    reqwest::Client as OidcHttpClient,
    AuthType, ClientId, CsrfToken, EndpointMaybeSet, EndpointNotSet, EndpointSet, EndpointState,
    IssuerUrl, Nonce, PkceCodeChallenge, PkceCodeVerifier, RedirectUrl, Scope,
};
use url::Url;

use crate::{
    error::{CLIError, Result},
    session::auth_options::{
        exchange_oidc_authorization_code, ExternalLoginSession, OidcLoginOptions,
    },
    terminal_ui,
};

const DEFAULT_CLI_REDIRECT_URI: &str = "http://localhost:8787/callback";
const CALLBACK_TIMEOUT_SECS: u64 = 120;
const PROGRESS_TICK_MS: u64 = 80;

#[derive(Debug)]
struct ResolvedRedirectUri {
    redirect_uri: String,
    listener_addr: String,
}

pub type CliOidcClient = CoreClient<
    EndpointSet,
    EndpointNotSet,
    EndpointNotSet,
    EndpointNotSet,
    EndpointMaybeSet,
    EndpointMaybeSet,
>;

pub struct BrowserAuthorizationRequest {
    pub authorization_url: String,
    pub csrf_state: CsrfToken,
    pub nonce: Nonce,
    pub pkce_verifier: PkceCodeVerifier,
}

struct CallbackParams {
    code: String,
    state: String,
}

pub async fn login_with_browser(
    http_client: &OidcHttpClient,
    api_client: &reqwest::Client,
    server_url: &str,
    options: &OidcLoginOptions,
    redirect_uri_override: Option<&str>,
    color: bool,
    show_progress: bool,
) -> Result<ExternalLoginSession> {
    let redirect = resolve_redirect_uri(redirect_uri_override)?;
    let redirect_url = RedirectUrl::new(redirect.redirect_uri.clone()).map_err(|error| {
        CLIError::ConfigurationError(format!("invalid CLI OIDC redirect URL: {error}"))
    })?;
    let client = discover_core_client(http_client, options).await?.set_redirect_uri(redirect_url);
    let request = build_authorization_request(&client, &options.scopes);
    let listener = TcpListener::bind(&redirect.listener_addr).map_err(|error| {
        CLIError::ConfigurationError(format!(
            "failed to listen for OIDC callback on {}: {error}",
            redirect.listener_addr
        ))
    })?;

    terminal_ui::print_oidc_browser_prompt(
        &options.display_name,
        &request.authorization_url,
        color,
    );
    try_open_browser(&request.authorization_url);

    let wait_spinner = if show_progress {
        let progress_bar = ProgressBar::new_spinner();
        progress_bar.set_style(
            ProgressStyle::default_spinner()
                .template("{spinner:.cyan} {msg} {elapsed_precise}")
                .expect("oidc spinner template should be valid"),
        );
        progress_bar.set_message("Waiting for OIDC browser verification...".to_string());
        progress_bar.enable_steady_tick(Duration::from_millis(PROGRESS_TICK_MS));
        Some(progress_bar)
    } else {
        None
    };

    let callback_result = tokio::time::timeout(
        Duration::from_secs(CALLBACK_TIMEOUT_SECS),
        tokio::task::spawn_blocking(move || wait_for_callback(listener)),
    )
    .await;

    let callback = match callback_result {
        Ok(task_result) => task_result.map_err(|error| {
            if let Some(progress_bar) = &wait_spinner {
                progress_bar.finish_and_clear();
            }
            CLIError::ConfigurationError(format!("OIDC callback task failed: {error}"))
        })??,
        Err(_) => {
            if let Some(progress_bar) = &wait_spinner {
                progress_bar.finish_and_clear();
            }
            return Err(CLIError::ConfigurationError("OIDC browser login timed out".to_string()));
        },
    };

    if let Some(progress_bar) = wait_spinner {
        progress_bar.finish_with_message("OIDC browser verification received".to_string());
    }

    if callback.state != request.csrf_state.secret().as_str() {
        return Err(CLIError::ConfigurationError("OIDC callback state did not match".to_string()));
    }

    exchange_oidc_authorization_code(
        api_client,
        server_url,
        callback.code,
        redirect.redirect_uri,
        request.pkce_verifier.secret().to_string(),
    )
    .await
}

fn resolve_redirect_uri(redirect_uri_override: Option<&str>) -> Result<ResolvedRedirectUri> {
    let redirect_uri = redirect_uri_override
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(DEFAULT_CLI_REDIRECT_URI);
    let parsed = Url::parse(redirect_uri).map_err(|error| {
        CLIError::ConfigurationError(format!("invalid CLI OIDC redirect URL: {error}"))
    })?;

    if parsed.scheme() != "http" {
        return Err(CLIError::ConfigurationError(
            "OIDC browser login redirect URI must use http:// and a loopback host".to_string(),
        ));
    }

    let host = parsed.host_str().ok_or_else(|| {
        CLIError::ConfigurationError(
            "OIDC browser login redirect URI must include a loopback host".to_string(),
        )
    })?;
    if !is_loopback_host(host) {
        return Err(CLIError::ConfigurationError(
            "OIDC browser login redirect URI must use localhost, 127.0.0.1, or ::1".to_string(),
        ));
    }

    let port = parsed.port_or_known_default().ok_or_else(|| {
        CLIError::ConfigurationError(
            "OIDC browser login redirect URI must include a port".to_string(),
        )
    })?;
    let listener_addr = if host.contains(':') {
        format!("[{}]:{}", host, port)
    } else {
        format!("{}:{}", host, port)
    };

    Ok(ResolvedRedirectUri {
        redirect_uri: parsed.to_string(),
        listener_addr,
    })
}

fn is_loopback_host(host: &str) -> bool {
    if host.eq_ignore_ascii_case("localhost") {
        return true;
    }

    host.parse::<std::net::IpAddr>().is_ok_and(|ip| ip.is_loopback())
}

pub async fn discover_core_client(
    http_client: &OidcHttpClient,
    options: &OidcLoginOptions,
) -> Result<CliOidcClient> {
    let issuer = IssuerUrl::new(options.issuer.clone()).map_err(|error| {
        CLIError::ConfigurationError(format!("invalid OIDC issuer URL: {error}"))
    })?;
    let provider_metadata = CoreProviderMetadata::discover_async(issuer, http_client)
        .await
        .map_err(|error| {
            CLIError::ConfigurationError(format!("failed to discover OIDC provider: {error}"))
        })?;

    Ok(CoreClient::from_provider_metadata(
        provider_metadata,
        ClientId::new(options.client_id.clone()),
        None,
    )
    .set_auth_type(AuthType::RequestBody))
}

pub fn build_authorization_request<
    HasDeviceAuthUrl,
    HasIntrospectionUrl,
    HasRevocationUrl,
    HasTokenUrl,
    HasUserInfoUrl,
>(
    client: &CoreClient<
        EndpointSet,
        HasDeviceAuthUrl,
        HasIntrospectionUrl,
        HasRevocationUrl,
        HasTokenUrl,
        HasUserInfoUrl,
    >,
    scopes: &[String],
) -> BrowserAuthorizationRequest
where
    HasDeviceAuthUrl: EndpointState,
    HasIntrospectionUrl: EndpointState,
    HasRevocationUrl: EndpointState,
    HasTokenUrl: EndpointState,
    HasUserInfoUrl: EndpointState,
{
    let (pkce_challenge, pkce_verifier) = PkceCodeChallenge::new_random_sha256();
    let mut request = client
        .authorize_url(
            CoreAuthenticationFlow::AuthorizationCode,
            CsrfToken::new_random,
            Nonce::new_random,
        )
        .set_pkce_challenge(pkce_challenge);

    for scope in scopes.iter().filter(|scope| scope.as_str() != "openid") {
        request = request.add_scope(Scope::new(scope.clone()));
    }

    let (authorization_url, csrf_state, nonce) = request.url();
    BrowserAuthorizationRequest {
        authorization_url: authorization_url.to_string(),
        csrf_state,
        nonce,
        pkce_verifier,
    }
}

fn wait_for_callback(listener: TcpListener) -> Result<CallbackParams> {
    let (mut stream, _) = listener.accept().map_err(|error| {
        CLIError::ConfigurationError(format!("failed to accept OIDC callback: {error}"))
    })?;
    let mut reader = BufReader::new(stream.try_clone().map_err(|error| {
        CLIError::ConfigurationError(format!("failed to read OIDC callback: {error}"))
    })?);
    let mut request_line = String::new();
    reader.read_line(&mut request_line).map_err(|error| {
        CLIError::ConfigurationError(format!("failed to read OIDC callback: {error}"))
    })?;

    let target = request_line
        .split_whitespace()
        .nth(1)
        .ok_or_else(|| CLIError::ConfigurationError("OIDC callback was malformed".to_string()))?;
    let callback_url = Url::parse(&format!("http://127.0.0.1{target}")).map_err(|error| {
        CLIError::ConfigurationError(format!("OIDC callback URL was invalid: {error}"))
    })?;
    let query = callback_url.query_pairs().collect::<std::collections::HashMap<_, _>>();

    let response = "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nConnection: close\r\n\r\nOIDC login complete. You can return to the terminal.\n";
    stream.write_all(response.as_bytes()).map_err(|error| {
        CLIError::ConfigurationError(format!("failed to write OIDC callback response: {error}"))
    })?;

    let code = query
        .get("code")
        .ok_or_else(|| {
            CLIError::ConfigurationError("OIDC callback did not include a code".to_string())
        })?
        .to_string();
    let state = query
        .get("state")
        .ok_or_else(|| {
            CLIError::ConfigurationError("OIDC callback did not include state".to_string())
        })?
        .to_string();
    Ok(CallbackParams { code, state })
}

fn try_open_browser(url: &str) {
    if matches!(
        std::env::var("KALAMDB_CLI_OIDC_OPEN_BROWSER").ok().as_deref(),
        Some("0" | "false" | "FALSE" | "no" | "NO")
    ) {
        return;
    }

    let command = if cfg!(target_os = "macos") {
        ("open", vec![url])
    } else if cfg!(target_os = "windows") {
        ("rundll32", vec!["url.dll,FileProtocolHandler", url])
    } else {
        ("xdg-open", vec![url])
    };
    let _ = Command::new(command.0).args(command.1).spawn();
}

#[cfg(test)]
mod tests {
    use super::{is_loopback_host, resolve_redirect_uri, DEFAULT_CLI_REDIRECT_URI};

    #[test]
    fn resolves_default_redirect_uri() {
        let redirect = resolve_redirect_uri(None).expect("default redirect uri should resolve");

        assert_eq!(redirect.redirect_uri, DEFAULT_CLI_REDIRECT_URI);
        assert_eq!(redirect.listener_addr, "localhost:8787");
    }

    #[test]
    fn accepts_localhost_redirect_uri() {
        let redirect = resolve_redirect_uri(Some("http://localhost:9999/callback"))
            .expect("localhost redirect uri should resolve");

        assert_eq!(redirect.redirect_uri, "http://localhost:9999/callback");
        assert_eq!(redirect.listener_addr, "localhost:9999");
    }

    #[test]
    fn rejects_non_loopback_redirect_uri() {
        let error = resolve_redirect_uri(Some("http://example.com/callback")).unwrap_err();
        assert!(error.to_string().contains("localhost, 127.0.0.1, or ::1"));
    }

    #[test]
    fn detects_loopback_hosts() {
        assert!(is_loopback_host("localhost"));
        assert!(is_loopback_host("127.0.0.1"));
        assert!(is_loopback_host("::1"));
        assert!(!is_loopback_host("example.com"));
    }
}
