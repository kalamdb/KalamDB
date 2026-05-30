use std::{str::FromStr, sync::Arc};

use kalamdb_commons::AuthType as KalamAuthType;
use moka::future::Cache;
use openidconnect::{
    core::{
        CoreAuthDisplay, CoreClaimName, CoreClaimType, CoreClient, CoreClientAuthMethod,
        CoreGrantType, CoreIdToken, CoreIdTokenVerifier, CoreJsonWebKey,
        CoreJweContentEncryptionAlgorithm, CoreJweKeyManagementAlgorithm, CoreResponseMode,
        CoreResponseType, CoreSubjectIdentifierType,
    },
    AdditionalProviderMetadata, AuthType, AuthUrl, ClientId, ClientSecret, DeviceAuthorizationUrl,
    EndpointMaybeSet, EndpointNotSet, EndpointSet, IssuerUrl, ProviderMetadata, Scope, TokenUrl,
};
use serde::{Deserialize, Serialize};

use crate::providers::jwt_auth::JwtClaims;

use super::{http::OidcHttpClient, OidcError};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct OidcPublicMetadata {
    pub issuer: String,
    pub client_id: String,
    pub scopes: Vec<String>,
    pub authorization_endpoint: Option<String>,
    pub token_endpoint: Option<String>,
    pub device_authorization_endpoint: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub(crate) struct DeviceEndpointProviderMetadata {
    #[serde(default)]
    pub device_authorization_endpoint: Option<DeviceAuthorizationUrl>,
}

impl AdditionalProviderMetadata for DeviceEndpointProviderMetadata {}

pub(crate) type DeviceProviderMetadata = ProviderMetadata<
    DeviceEndpointProviderMetadata,
    CoreAuthDisplay,
    CoreClientAuthMethod,
    CoreClaimName,
    CoreClaimType,
    CoreGrantType,
    CoreJweContentEncryptionAlgorithm,
    CoreJweKeyManagementAlgorithm,
    CoreJsonWebKey,
    CoreResponseMode,
    CoreResponseType,
    CoreSubjectIdentifierType,
>;

pub(crate) type OidcCoreClient = CoreClient<
    EndpointSet,
    EndpointNotSet,
    EndpointNotSet,
    EndpointNotSet,
    EndpointMaybeSet,
    EndpointMaybeSet,
>;

pub(crate) type OidcDeviceClient = CoreClient<
    EndpointSet,
    EndpointSet,
    EndpointNotSet,
    EndpointNotSet,
    EndpointMaybeSet,
    EndpointMaybeSet,
>;

pub(crate) type OidcClientCache = Cache<String, Arc<OidcClientHandle>>;

#[derive(Clone)]
pub(crate) struct OidcClientHandle {
    pub issuer: IssuerUrl,
    pub client_id: ClientId,
    pub audience: ClientId,
    pub client_secret: Option<ClientSecret>,
    pub provider_metadata: DeviceProviderMetadata,
    pub device_authorization_endpoint: Option<DeviceAuthorizationUrl>,
    pub public_metadata: OidcPublicMetadata,
}

impl OidcClientHandle {
    pub(crate) async fn discover(
        settings: &kalamdb_configs::AuthOidcSettings,
        http_client: &OidcHttpClient,
    ) -> Result<Self, OidcError> {
        let issuer = issuer_url(settings)?;
        let provider_metadata = DeviceProviderMetadata::discover_async(issuer.clone(), http_client)
            .await
            .map_err(|error| OidcError::DiscoveryFailed(error.to_string()))?;

        Self::from_metadata(settings, issuer, provider_metadata)
    }

    pub(crate) fn from_metadata(
        settings: &kalamdb_configs::AuthOidcSettings,
        issuer: IssuerUrl,
        provider_metadata: DeviceProviderMetadata,
    ) -> Result<Self, OidcError> {
        let client_id = client_id(settings)?;
        let audience = audience(settings)?;
        let client_secret = client_secret(settings);
        let scopes = scopes(settings);
        let device_authorization_endpoint =
            device_authorization_endpoint(settings)?.or_else(|| {
                provider_metadata.additional_metadata().device_authorization_endpoint.clone()
            });

        let public_metadata = OidcPublicMetadata::from_metadata(
            &client_id,
            &scopes,
            &provider_metadata,
            device_authorization_endpoint.as_ref(),
        );

        Ok(Self {
            issuer,
            client_id,
            audience,
            client_secret,
            provider_metadata,
            device_authorization_endpoint,
            public_metadata,
        })
    }

    pub(crate) fn core_client(&self) -> OidcCoreClient {
        CoreClient::from_provider_metadata(
            self.provider_metadata.clone(),
            self.client_id.clone(),
            self.client_secret.clone(),
        )
    }

    pub(crate) fn device_client(&self) -> Result<OidcDeviceClient, OidcError> {
        let endpoint = self.device_authorization_endpoint.clone().ok_or_else(|| {
            OidcError::Configuration(
                "OIDC provider does not advertise a device authorization endpoint".to_string(),
            )
        })?;

        Ok(self
            .core_client()
            .set_device_authorization_url(endpoint)
            .set_auth_type(AuthType::RequestBody))
    }

    pub(crate) fn id_token_verifier(&self) -> CoreIdTokenVerifier<'static> {
        let verifier = if let Some(client_secret) = self.client_secret.clone() {
            CoreIdTokenVerifier::new_confidential_client(
                self.audience.clone(),
                client_secret,
                self.issuer.clone(),
                self.provider_metadata.jwks().clone(),
            )
        } else {
            CoreIdTokenVerifier::new_public_client(
                self.audience.clone(),
                self.issuer.clone(),
                self.provider_metadata.jwks().clone(),
            )
        };

        verifier.allow_all_jose_types()
    }

    pub(crate) fn validate_id_token(&self, token: &str) -> Result<JwtClaims, OidcError> {
        let id_token = CoreIdToken::from_str(token).map_err(|error| {
            OidcError::JwtValidationFailed(format!("Invalid OIDC ID token: {error}"))
        })?;
        let verifier = self.id_token_verifier();
        let claims = id_token
            .into_claims(&verifier, accept_optional_nonce)
            .map_err(|error| OidcError::JwtValidationFailed(error.to_string()))?;

        Ok(JwtClaims {
            sub: claims.subject().as_str().to_string(),
            iss: claims.issuer().as_str().to_string(),
            exp: claims.expiration().timestamp() as usize,
            iat: claims.issue_time().timestamp() as usize,
            email: claims.email().map(|email| email.as_str().to_string()),
            role: None,
            auth_type: Some(KalamAuthType::Oidc),
            token_type: None,
        })
    }
}

impl OidcPublicMetadata {
    fn from_metadata(
        client_id: &ClientId,
        scopes: &[Scope],
        provider_metadata: &DeviceProviderMetadata,
        device_authorization_endpoint: Option<&DeviceAuthorizationUrl>,
    ) -> Self {
        Self {
            issuer: provider_metadata.issuer().as_str().to_string(),
            client_id: client_id.as_str().to_string(),
            scopes: scopes.iter().map(|scope| scope.as_str().to_string()).collect(),
            authorization_endpoint: Some(auth_url_to_string(
                provider_metadata.authorization_endpoint(),
            )),
            token_endpoint: provider_metadata.token_endpoint().map(token_url_to_string),
            device_authorization_endpoint: device_authorization_endpoint
                .map(|url| url.as_str().to_string()),
        }
    }
}

pub(crate) fn oidc_client_cache(max_capacity: u64) -> OidcClientCache {
    Cache::builder().max_capacity(max_capacity).build()
}

pub(crate) async fn get_or_discover_oidc_client(
    cache: &OidcClientCache,
    settings: kalamdb_configs::AuthOidcSettings,
    http_client: &OidcHttpClient,
) -> Result<Arc<OidcClientHandle>, OidcError> {
    let cache_key = settings_cache_key(&settings)?;
    if let Some(handle) = cache.get(&cache_key).await {
        return Ok(handle);
    }

    let handle = Arc::new(OidcClientHandle::discover(&settings, http_client).await?);
    cache.insert(cache_key, handle.clone()).await;
    Ok(handle)
}

fn issuer_url(settings: &kalamdb_configs::AuthOidcSettings) -> Result<IssuerUrl, OidcError> {
    let issuer = settings
        .issuer_str()
        .ok_or_else(|| OidcError::Configuration("auth.oidc.issuer is required".to_string()))?;

    IssuerUrl::new(issuer.to_string())
        .map_err(|error| OidcError::Configuration(format!("invalid auth.oidc.issuer: {error}")))
}

fn client_id(settings: &kalamdb_configs::AuthOidcSettings) -> Result<ClientId, OidcError> {
    let client_id = settings
        .client_id_str()
        .ok_or_else(|| OidcError::Configuration("auth.oidc.client_id is required".to_string()))?;

    Ok(ClientId::new(client_id.to_string()))
}

fn audience(settings: &kalamdb_configs::AuthOidcSettings) -> Result<ClientId, OidcError> {
    let audience = settings
        .audience_str()
        .ok_or_else(|| OidcError::Configuration("auth.oidc.client_id is required".to_string()))?;

    Ok(ClientId::new(audience.to_string()))
}

fn client_secret(settings: &kalamdb_configs::AuthOidcSettings) -> Option<ClientSecret> {
    settings
        .client_secret
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| ClientSecret::new(value.to_string()))
}

fn scopes(settings: &kalamdb_configs::AuthOidcSettings) -> Vec<Scope> {
    settings.scopes.iter().map(|scope| Scope::new(scope.clone())).collect()
}

fn device_authorization_endpoint(
    settings: &kalamdb_configs::AuthOidcSettings,
) -> Result<Option<DeviceAuthorizationUrl>, OidcError> {
    settings
        .device_authorization_endpoint
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| {
            DeviceAuthorizationUrl::new(value.to_string()).map_err(|error| {
                OidcError::Configuration(format!(
                    "invalid auth.oidc.device_authorization_endpoint: {error}"
                ))
            })
        })
        .transpose()
}

fn settings_cache_key(settings: &kalamdb_configs::AuthOidcSettings) -> Result<String, OidcError> {
    let issuer = issuer_url(settings)?;
    let client_id = client_id(settings)?;
    let audience = audience(settings)?;
    let device_endpoint = device_authorization_endpoint(settings)?
        .as_ref()
        .map(|url| url.as_str())
        .unwrap_or_default()
        .to_string();

    Ok(format!(
        "{}|{}|{}|{}",
        issuer.as_str(),
        client_id.as_str(),
        audience.as_str(),
        device_endpoint
    ))
}

fn auth_url_to_string(url: &AuthUrl) -> String {
    url.as_str().to_string()
}

fn token_url_to_string(url: &TokenUrl) -> String {
    url.as_str().to_string()
}

fn accept_optional_nonce(_nonce: Option<&openidconnect::Nonce>) -> Result<(), String> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_RSA_PRIVATE_KEY_DER_BASE64: &str = "MIIEowIBAAKCAQEAwwphfTBE9LBOHdPUXacsmeuDad+rjh81B5MH/74eelEcs3Z+jUxFa7CqqMm7432It9joUO0mULUXfpUBnwFCgGIHEvTWHDOcR+Wgnc07LfYMGxqxlifCEK6RUdfSAVAj97a5DSuIpQ6iAvGp54iBRrf5vgmD/z38fisRa6YrBagWyMOFerPPQP94WhvNRN9Lt7NO+3jgf1N8reh0KMo2KynJDyZ3y/xQWcIrPc/g/FqqRkj8/WrOgpaPzW5Q/Nqcd5GIAEj6cDELk76XL9whbk6ixhnu2mkvIJ/cZenBd2AGM8BbU7XxIi6GzuS2v+PeKjRlGQx8TkGqtjZ4KibY+wIDAQABAoIBAA2qGwVpzdL0zSxCzISZM0M/YFAZFwxYfF8g+nT87Wa1axTZrukYWF7AnFxB8fNwtpTm0fPlgYMzBMfeCaSJso6LD6LQ23VTWlYhLN0RZV2FePinKJz0ASEpEc5RmAl2g2aV+yYEkEi8GzaolrY9do0tU4ZwZTqLLbbrLofDtwzox9K1LXZOdYK2+UZlKXKRJFu06wAd4Pvq3LUP4MmstfaKBklAsGf7hgwt+uREPd3YzLpaWn/5F4gI03sJA1oB+zHS3FAexo8Yxwuy10ATQ4ERdRPc7/86CS3n+XKpoj+IzBjDYDqtM2qcH2YAP3wcU1B8nRGpxJY0pqKPpdFz79kCgYEA++0JY3bFhCAClcwDMlyfzev/LVEV3JqoWNeH5ryQ4qJ2HiXwBcfTqtO4yoTCU7UpSXKI32aBlEhpn4rpZgM8trbivUHkagmoIaSJxGhpdLv35W456kvF7pLWckIziq0g9i+EGGhW0cpCmfFipfjgPUzeZKKovL6QhHiVEVOqnSkCgYEAxjHXSIsHsz30GqbwRmW/e0uQEwABcCActNVB3hj7Q1nePxfFB8sDe+s7FGFXsuhtemkHzA6j5UbzkMlrVSG98geZrlZniXdThS+jRvpEncqfUyO+POqhx6blWyldyo9PgMcvWsB4yuKG3lWKdR3kL8aQX9gcFsOPF4JxyyLFpYMCgYEAqJNu6t25QbZhxHcl1Hdif9rhgCN4K4xaBkkDKYUYtm7b90SPnm6e1vqh9vJrTrQ1Em7P5B2lq+Hgu9+qWpbj86fhhZ8oB0S6+vgtL/5mQrTdJuthWcSmiAQ992sRLkS3f8U/8U0we2WKt5Rs3H7zHlHnpxOpMdOaxOojZdrEmjECgYBKCNozdgPVV+I0hoGgumdRxkM2Zb0jxksS3cqyDUDmws47YUSviY1un8s87LPW1+31WQCZoCpm/h8Dycm3Tlhm7aHhttMMTa+8Q7RJUjmJe+QSKXrpxHfUXaq1Z/lqLihzoXQ2AUnd98qLiQakgxr3IcRSmSa89iYgkRCy4fVUwwKBgHBxSEFAcRyEGvgWVL1Ti8KoiMfTyj8HofGUGON9PmP5yyGabZrd4TdcRozoUYh9jrI3FvwepbAKxnyuGDKAsEIXnAvUgqGZF0AEy0CFrSTHW8WynGzDTEslzWG4Ha1xdGlwv+SpjwThP5Un9k1ei99y/rd0bS1zBKAEUC4gvWOP";
    const TEST_RSA_JWK_N: &str = "wwphfTBE9LBOHdPUXacsmeuDad-rjh81B5MH_74eelEcs3Z-jUxFa7CqqMm7432It9joUO0mULUXfpUBnwFCgGIHEvTWHDOcR-Wgnc07LfYMGxqxlifCEK6RUdfSAVAj97a5DSuIpQ6iAvGp54iBRrf5vgmD_z38fisRa6YrBagWyMOFerPPQP94WhvNRN9Lt7NO-3jgf1N8reh0KMo2KynJDyZ3y_xQWcIrPc_g_FqqRkj8_WrOgpaPzW5Q_Nqcd5GIAEj6cDELk76XL9whbk6ixhnu2mkvIJ_cZenBd2AGM8BbU7XxIi6GzuS2v-PeKjRlGQx8TkGqtjZ4KibY-w";
    const TEST_RSA_JWK_E: &str = "AQAB";
    const TEST_KEY_ID: &str = "test-key-1";
    const TEST_CLIENT_ID: &str = "kalamdb-client";

    #[derive(serde::Serialize)]
    struct TestIdTokenClaims {
        sub: String,
        iss: String,
        aud: String,
        exp: usize,
        iat: usize,
        email: String,
    }

    struct TestProvider {
        issuer: String,
        handle: actix_web::dev::ServerHandle,
    }

    impl Drop for TestProvider {
        fn drop(&mut self) {
            let handle = self.handle.clone();
            actix_web::rt::spawn(async move {
                handle.stop(true).await;
            });
        }
    }

    #[test]
    fn public_metadata_is_non_secret() {
        let metadata = OidcPublicMetadata {
            issuer: "https://idp.example.com".to_string(),
            client_id: "kalamdb".to_string(),
            scopes: vec!["openid".to_string()],
            authorization_endpoint: Some("https://idp.example.com/auth".to_string()),
            token_endpoint: Some("https://idp.example.com/token".to_string()),
            device_authorization_endpoint: None,
        };

        assert_eq!(metadata.client_id, "kalamdb");
        assert!(metadata.scopes.iter().any(|scope| scope == "openid"));
    }

    #[test]
    fn custom_device_endpoint_metadata_deserializes() {
        let metadata: DeviceEndpointProviderMetadata = serde_json::from_value(serde_json::json!({
            "device_authorization_endpoint": "https://idp.example.com/device"
        }))
        .expect("device endpoint metadata should deserialize");

        assert_eq!(
            metadata.device_authorization_endpoint.unwrap().as_str(),
            "https://idp.example.com/device"
        );
    }

    #[test]
    fn client_cache_is_bounded() {
        let cache = oidc_client_cache(8);
        assert_eq!(cache.policy().max_capacity(), Some(8));
    }

    #[actix_web::test]
    async fn discovery_builds_verifier_and_validates_issuer_and_audience() {
        let provider = start_test_provider().await;
        let http_client =
            super::super::http::default_oidc_http_client().expect("OIDC HTTP client should build");
        let settings = kalamdb_configs::AuthOidcSettings {
            enabled: true,
            display_name: "Test OIDC".to_string(),
            issuer: Some(provider.issuer.clone()),
            client_id: Some(TEST_CLIENT_ID.to_string()),
            client_secret: None,
            scopes: vec!["openid".to_string(), "email".to_string()],
            device_authorization_endpoint: None,
            broker_device_flow_enabled: false,
            auto_provision: false,
            default_role: "user".to_string(),
            audience: None,
        };
        let handle = OidcClientHandle::discover(&settings, &http_client)
            .await
            .expect("provider discovery should succeed");

        assert_eq!(handle.public_metadata.issuer, provider.issuer);
        assert_eq!(
            handle.public_metadata.device_authorization_endpoint.as_deref(),
            Some(format!("{}/device/code", provider.issuer).as_str())
        );

        let valid = issue_test_id_token(&provider.issuer, TEST_CLIENT_ID, "subject-1");
        let claims = handle.validate_id_token(&valid).expect("valid ID token should verify");
        assert_eq!(claims.sub, "subject-1");
        assert_eq!(claims.iss, provider.issuer);
        assert_eq!(claims.email.as_deref(), Some("subject-1@example.org"));

        let wrong_audience = issue_test_id_token(&provider.issuer, "other-client", "subject-1");
        let error = handle
            .validate_id_token(&wrong_audience)
            .expect_err("wrong audience should fail verifier");
        assert!(error.to_string().contains("audience"), "unexpected error: {error}");
    }

    async fn start_test_provider() -> TestProvider {
        use actix_web::{http::StatusCode, web, App, HttpResponse, HttpServer};
        use std::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").expect("test provider should bind");
        let address = listener.local_addr().expect("test provider address should be available");
        let issuer = format!("http://{}", address);
        let discovery = serde_json::json!({
            "issuer": issuer.clone(),
            "authorization_endpoint": format!("{}/authorize", issuer),
            "token_endpoint": format!("{}/token", issuer),
            "jwks_uri": format!("{}/jwks", issuer),
            "response_types_supported": ["code"],
            "subject_types_supported": ["public"],
            "id_token_signing_alg_values_supported": ["RS256"],
            "device_authorization_endpoint": format!("{}/device/code", issuer),
        });
        let jwks = serde_json::json!({
            "keys": [{
                "kty": "RSA",
                "alg": "RS256",
                "use": "sig",
                "kid": TEST_KEY_ID,
                "n": TEST_RSA_JWK_N,
                "e": TEST_RSA_JWK_E,
            }]
        });

        let server = HttpServer::new(move || {
            let discovery = discovery.clone();
            let jwks = jwks.clone();
            App::new()
                .route(
                    "/.well-known/openid-configuration",
                    web::get().to(move || {
                        let discovery = discovery.clone();
                        async move {
                            HttpResponse::build(StatusCode::OK)
                                .content_type("application/json")
                                .body(discovery.to_string())
                        }
                    }),
                )
                .route(
                    "/jwks",
                    web::get().to(move || {
                        let jwks = jwks.clone();
                        async move { HttpResponse::Ok().json(jwks) }
                    }),
                )
        })
        .listen(listener)
        .expect("test provider should listen")
        .run();
        let handle = server.handle();
        actix_web::rt::spawn(server);

        TestProvider { issuer, handle }
    }

    fn issue_test_id_token(issuer: &str, audience: &str, subject: &str) -> String {
        use base64::Engine as _;
        use jsonwebtoken::{Algorithm, EncodingKey, Header};

        let now = chrono::Utc::now().timestamp() as usize;
        let claims = TestIdTokenClaims {
            sub: subject.to_string(),
            iss: issuer.to_string(),
            aud: audience.to_string(),
            exp: now + 3600,
            iat: now.saturating_sub(1),
            email: format!("{}@example.org", subject),
        };
        let mut header = Header::new(Algorithm::RS256);
        header.kid = Some(TEST_KEY_ID.to_string());
        let der = base64::engine::general_purpose::STANDARD
            .decode(TEST_RSA_PRIVATE_KEY_DER_BASE64)
            .expect("test RSA key should decode");
        jsonwebtoken::encode(&header, &claims, &EncodingKey::from_rsa_der(&der))
            .expect("test ID token should sign")
    }
}
