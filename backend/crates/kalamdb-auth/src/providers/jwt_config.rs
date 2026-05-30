// JWT configuration cache, trusted issuer parsing, and OIDC client registry.

use std::{
    collections::HashMap,
    sync::{Arc, RwLock as StdRwLock},
};

use kalamdb_commons::Role;
use once_cell::sync::Lazy;

use crate::{
    errors::error::{AuthError, AuthResult},
    oidc::{
        default_oidc_http_client, get_or_discover_oidc_client, oidc_client_cache, OidcClientCache,
        OidcClientHandle, OidcError,
    },
    providers::jwt_auth,
};

/// Cached JWT configuration for performance.
///
/// Holds the HS256 shared secret for internal tokens, the list of trusted
/// issuers, and an openidconnect-backed client/verifier cache for external
/// ID-token verification.
pub struct JwtConfig {
    pub secret: String,
    pub trusted_issuers: Vec<String>,
    pub issuer_audiences: HashMap<String, String>,
    pub oidc_auto_provision: bool,
    pub oidc_default_role: Role,

    /// Per-issuer OIDC provider client configuration.
    oidc_configs: HashMap<String, kalamdb_configs::AuthOidcSettings>,

    /// Bounded openidconnect provider metadata/JWKS client cache.
    oidc_clients: OidcClientCache,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OidcPublicClientMetadata {
    pub issuer: String,
    pub client_id: String,
    pub scopes: Vec<String>,
    pub authorization_endpoint: Option<String>,
    pub token_endpoint: Option<String>,
    pub device_authorization_endpoint: Option<String>,
}

static JWT_CONFIG: Lazy<StdRwLock<Arc<JwtConfig>>> =
    Lazy::new(|| StdRwLock::new(Arc::new(default_jwt_config())));

/// Initialize JWT configuration from server settings.
///
/// This should be called once at startup after loading server.toml and applying
/// environment overrides. If not called, defaults are used.
pub fn init_jwt_config(
    secret: &str,
    trusted_issuers: &str,
    oidc_settings: Option<&kalamdb_configs::AuthOidcSettings>,
    oidc_auto_provision: bool,
    oidc_default_role: Role,
) {
    let (issuer_audiences, oidc_configs) = build_oidc_config_maps(oidc_settings);
    let config = Arc::new(JwtConfig {
        secret: secret.to_string(),
        trusted_issuers: parse_trusted_issuers(trusted_issuers),
        issuer_audiences,
        oidc_auto_provision,
        oidc_default_role,
        oidc_configs,
        oidc_clients: oidc_client_cache(16),
    });

    *JWT_CONFIG.write().expect("JWT config lock poisoned") = config;
}

pub fn get_jwt_config() -> Arc<JwtConfig> {
    JWT_CONFIG.read().expect("JWT config lock poisoned").clone()
}

fn default_jwt_config() -> JwtConfig {
    JwtConfig {
        secret: kalamdb_configs::defaults::default_auth_jwt_secret(),
        trusted_issuers: parse_trusted_issuers(
            &kalamdb_configs::defaults::default_auth_jwt_trusted_issuers(),
        ),
        issuer_audiences: HashMap::new(),
        oidc_auto_provision: false,
        oidc_default_role: Role::User,
        oidc_configs: HashMap::new(),
        oidc_clients: oidc_client_cache(16),
    }
}

impl JwtConfig {
    #[cfg(test)]
    pub(crate) fn for_tests(oidc_auto_provision: bool, oidc_default_role: Role) -> Self {
        Self {
            secret: "test-secret".to_string(),
            trusted_issuers: vec![jwt_auth::KALAMDB_ISSUER.to_string()],
            issuer_audiences: HashMap::new(),
            oidc_auto_provision,
            oidc_default_role,
            oidc_configs: HashMap::new(),
            oidc_clients: oidc_client_cache(16),
        }
    }

    /// Get or discover the openidconnect provider client for the given issuer.
    pub(crate) async fn get_oidc_client(&self, issuer: &str) -> AuthResult<Arc<OidcClientHandle>> {
        let oidc_config = self.oidc_configs.get(issuer).cloned().ok_or_else(|| {
            AuthError::MalformedAuthorization(format!(
                "No OIDC client configuration is registered for issuer '{}'",
                issuer
            ))
        })?;
        let http_client = default_oidc_http_client().map_err(|error| {
            AuthError::MalformedAuthorization(format!("Failed to build OIDC HTTP client: {error}"))
        })?;

        get_or_discover_oidc_client(&self.oidc_clients, oidc_config, &http_client)
            .await
            .map_err(map_oidc_error)
    }

    /// Get public, non-secret metadata for client login options.
    pub async fn oidc_public_metadata(&self, issuer: &str) -> AuthResult<OidcPublicClientMetadata> {
        let client = self.get_oidc_client(issuer).await?;
        Ok(OidcPublicClientMetadata {
            issuer: client.public_metadata.issuer.clone(),
            client_id: client.public_metadata.client_id.clone(),
            scopes: client.public_metadata.scopes.clone(),
            authorization_endpoint: client.public_metadata.authorization_endpoint.clone(),
            token_endpoint: client.public_metadata.token_endpoint.clone(),
            device_authorization_endpoint: client
                .public_metadata
                .device_authorization_endpoint
                .clone(),
        })
    }

    /// Validate an external OIDC ID token with the cached openidconnect verifier.
    pub(crate) async fn validate_oidc_id_token(
        &self,
        issuer: &str,
        token: &str,
    ) -> AuthResult<jwt_auth::JwtClaims> {
        let client = self.get_oidc_client(issuer).await?;
        client.validate_id_token(token).map_err(map_oidc_error)
    }
}

fn build_oidc_config_maps(
    oidc_settings: Option<&kalamdb_configs::AuthOidcSettings>,
) -> (HashMap<String, String>, HashMap<String, kalamdb_configs::AuthOidcSettings>) {
    let mut issuer_audiences = HashMap::new();
    let mut oidc_configs = HashMap::new();

    let Some(settings) = oidc_settings.filter(|settings| settings.enabled) else {
        return (issuer_audiences, oidc_configs);
    };

    let Some(issuer) = settings.issuer_str() else {
        log::warn!("OIDC auth configuration was not registered: auth.oidc.issuer is required");
        return (issuer_audiences, oidc_configs);
    };
    let Some(audience) = settings.audience_str() else {
        log::warn!("OIDC auth configuration was not registered: auth.oidc.client_id is required");
        return (issuer_audiences, oidc_configs);
    };

    issuer_audiences.insert(issuer.to_string(), audience.to_string());
    oidc_configs.insert(issuer.to_string(), settings.clone());

    (issuer_audiences, oidc_configs)
}

fn map_oidc_error(error: OidcError) -> AuthError {
    match error {
        OidcError::JwtValidationFailed(ref msg) if msg.to_lowercase().contains("expired") => {
            AuthError::TokenExpired
        },
        OidcError::JwtValidationFailed(ref msg) if msg.to_lowercase().contains("signature") => {
            AuthError::InvalidSignature
        },
        _ => AuthError::MalformedAuthorization(format!(
            "External token validation failed: {}",
            error
        )),
    }
}

fn parse_trusted_issuers(input: &str) -> Vec<String> {
    let mut issuers: Vec<String> = input
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();

    // Always include the internal issuer so local accounts (admin etc.) keep working
    if !issuers.contains(&jwt_auth::KALAMDB_ISSUER.to_string()) {
        issuers.insert(0, jwt_auth::KALAMDB_ISSUER.to_string());
    }
    issuers
}
