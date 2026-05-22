use std::time::Duration;

use jsonwebtoken::{
    decode, decode_header,
    jwk::{Jwk, JwkSet},
    Algorithm, DecodingKey, Validation,
};
use moka::sync::Cache;
use serde::Deserialize;

use crate::oidc::{OidcConfig, OidcError};

const JWKS_CACHE_MAX_KEYS: u64 = 128;
const JWKS_CACHE_TTL_SECS: u64 = 60 * 60;

/// OIDC JWT validator with per-issuer JWKS caching.
#[derive(Clone)]
pub(crate) struct OidcValidator {
    config: OidcConfig,
    jwks_cache: Cache<String, Jwk>,
}

impl OidcValidator {
    pub fn new(config: OidcConfig) -> Self {
        Self {
            config,
            jwks_cache: Cache::builder()
                .max_capacity(JWKS_CACHE_MAX_KEYS)
                .time_to_live(Duration::from_secs(JWKS_CACHE_TTL_SECS))
                .build(),
        }
    }

    pub async fn validate<T>(&self, token: &str) -> Result<T, OidcError>
    where
        T: for<'de> Deserialize<'de>,
    {
        let mut validation = Validation::new(Algorithm::RS256);
        validation.algorithms = vec![
            Algorithm::RS256,
            Algorithm::RS384,
            Algorithm::RS512,
            Algorithm::PS256,
            Algorithm::PS384,
            Algorithm::PS512,
            Algorithm::ES256,
            Algorithm::ES384,
        ];
        validation.set_issuer(&[&self.config.issuer_url]);
        validation.validate_exp = true;
        validation.validate_nbf = true;
        validation.leeway = 60;

        if let Some(client_id) = &self.config.client_id {
            validation.set_audience(&[client_id]);
            validation.required_spec_claims.insert("aud".to_string());
        } else {
            validation.validate_aud = false;
        }

        self.validate_custom(token, &validation).await
    }

    pub async fn validate_custom<T>(
        &self,
        token: &str,
        validation: &Validation,
    ) -> Result<T, OidcError>
    where
        T: for<'de> Deserialize<'de>,
    {
        log::debug!("Validating JWT token");

        let header = decode_header(token)?;
        let kid = header.kid.ok_or(OidcError::MissingKid)?;
        log::debug!("Token kid: {}, alg: {:?}", kid, header.alg);

        let jwk = self.get_jwk(&kid).await?;
        let decoding_key = DecodingKey::from_jwk(&jwk)
            .map_err(|error| OidcError::InvalidKeyFormat(error.to_string()))?;

        if !validation.algorithms.contains(&header.alg) {
            return Err(OidcError::JwtValidationFailed(format!(
                "Algorithm {:?} is not allowed",
                header.alg
            )));
        }

        let mut pinned = validation.clone();
        pinned.algorithms = vec![header.alg];

        let token_data = decode::<T>(token, &decoding_key, &pinned).map_err(|error| {
            log::warn!("JWT decode failed: kind={:?} msg={}", error.kind(), error);
            OidcError::from(error)
        })?;

        let parts: Vec<&str> = token.splitn(3, '.').collect();
        if parts.len() >= 2 {
            use base64::Engine as _;

            if let Ok(payload_bytes) =
                base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(parts[1])
            {
                if let Ok(payload) = serde_json::from_slice::<serde_json::Value>(&payload_bytes) {
                    if let Some(iat) = payload.get("iat").and_then(|value| value.as_u64()) {
                        let now = chrono::Utc::now().timestamp() as u64;
                        let leeway = pinned.leeway;
                        if iat > now + leeway {
                            return Err(OidcError::JwtValidationFailed(format!(
                                "Token issued in the future (iat: {}, now: {})",
                                iat, now
                            )));
                        }
                    }
                }
            }
        }

        Ok(token_data.claims)
    }

    async fn get_jwk(&self, kid: &str) -> Result<Jwk, OidcError> {
        if let Some(jwk) = self.jwks_cache.get(kid) {
            return Ok(jwk);
        }

        self.refresh_jwks_cache().await?;

        self.jwks_cache.get(kid).ok_or_else(|| OidcError::KeyNotFound(kid.to_string()))
    }

    pub async fn refresh_jwks_cache(&self) -> Result<(), OidcError> {
        log::info!("Refreshing JWKS cache for {}", self.config.issuer_url);

        let new_jwks = self.fetch_jwks().await?;

        if new_jwks.keys.len() > JWKS_CACHE_MAX_KEYS as usize {
            return Err(OidcError::JwksFetchFailed(format!(
                "JWKS response from '{}' contained {} keys, exceeding the configured limit of {}",
                self.config.jwks_uri,
                new_jwks.keys.len(),
                JWKS_CACHE_MAX_KEYS
            )));
        }

        let mut cached_keys = 0usize;
        self.jwks_cache.invalidate_all();
        for jwk in new_jwks.keys {
            if let Some(kid) = jwk.common.key_id.clone() {
                log::debug!("Caching key: {}", kid);
                self.jwks_cache.insert(kid, jwk);
                cached_keys += 1;
            }
        }

        log::info!("JWKS cache refreshed with {} keys for {}", cached_keys, self.config.issuer_url);

        Ok(())
    }

    async fn fetch_jwks(&self) -> Result<JwkSet, OidcError> {
        log::debug!("Fetching JWKS from: {}", self.config.jwks_uri);

        let response = reqwest::get(&self.config.jwks_uri).await.map_err(|error| {
            OidcError::JwksFetchFailed(format!(
                "Failed to fetch JWKS from '{}': {}",
                self.config.jwks_uri, error
            ))
        })?;

        if !response.status().is_success() {
            return Err(OidcError::JwksFetchFailed(format!(
                "JWKS request to '{}' returned status {}",
                self.config.jwks_uri,
                response.status()
            )));
        }

        let jwks: JwkSet = response.json().await.map_err(|error| {
            OidcError::JwksFetchFailed(format!(
                "Failed to parse JWKS JSON from '{}': {}",
                self.config.jwks_uri, error
            ))
        })?;

        log::debug!("Fetched {} keys from JWKS", jwks.keys.len());
        Ok(jwks)
    }
}
