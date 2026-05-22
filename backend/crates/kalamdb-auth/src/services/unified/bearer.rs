use std::sync::Arc;

use jsonwebtoken::Algorithm;
use kalamdb_commons::{
    models::{ConnectionInfo, UserId},
    AuthType, Role,
};
use kalamdb_system::providers::storages::models::StorageMode;
use kalamdb_system::{AuthData, User};
use sha2::{Digest, Sha256};
use tracing::Instrument;

use crate::{
    errors::error::{AuthError, AuthResult},
    models::context::AuthenticatedUser,
    oidc::OidcError,
    providers::{jwt_auth, jwt_config, jwt_config::JwtConfig},
    repository::user_repo::UserRepository,
};

pub(super) async fn authenticate_bearer(
    token: &str,
    connection_info: &ConnectionInfo,
    repo: &Arc<dyn UserRepository>,
) -> AuthResult<AuthenticatedUser> {
    let span = tracing::info_span!(
        "auth.bearer",
        token_len = token.len(),
        is_localhost = connection_info.is_localhost()
    );

    async move {
        let config = jwt_config::get_jwt_config();

        let alg = jwt_auth::extract_algorithm_unverified(token)?;
        let issuer = jwt_auth::extract_issuer_unverified(token)?;

        jwt_auth::verify_issuer(&issuer, &config.trusted_issuers)?;

        tracing::debug!(
            issuer = %issuer,
            algorithm = ?alg,
            oauth_auto_provision = config.oauth_auto_provision,
            trusted_issuer_count = config.trusted_issuers.len(),
            "Bearer JWT issuer accepted"
        );

        let claims = validate_bearer_token(token, &alg, &issuer, &config).await?;

        let is_internal = jwt_auth::is_internal_issuer(&claims.iss);

        if is_internal {
            match claims.token_type {
                Some(jwt_auth::TokenType::Access) => { /* OK */ },
                Some(jwt_auth::TokenType::Refresh) => {
                    log::warn!("Refresh token used as access token for user={}", claims.sub);
                    return Err(AuthError::InvalidCredentials(
                        "Refresh tokens cannot be used for API authentication".to_string(),
                    ));
                },
                None => {
                    log::warn!("Token missing token_type claim for user={}", claims.sub);
                    return Err(AuthError::InvalidCredentials(
                        "Token missing required token_type claim".to_string(),
                    ));
                },
            }
        } else {
            let header = jsonwebtoken::decode_header(token).map_err(|e| {
                AuthError::MalformedAuthorization(format!("Invalid JWT header: {}", e))
            })?;

            if let Some(typ) = header.typ {
                let typ_lower = typ.to_lowercase();
                if typ_lower.contains("refresh") {
                    log::warn!(
                        "External refresh token used as access token for user={}",
                        claims.sub
                    );
                    return Err(AuthError::InvalidCredentials(
                        "Refresh tokens cannot be used for API authentication".to_string(),
                    ));
                }
            }
        }

        let user = if is_internal {
            let token_user_id = UserId::try_new(claims.sub.clone()).map_err(|_| {
                AuthError::MalformedAuthorization(
                    "Token contains an invalid user claim".to_string(),
                )
            })?;
            repo.get_user_by_id(&token_user_id).await?
        } else {
            resolve_external_user(repo, &config, &claims).await?
        };

        if user.deleted_at.is_some() {
            return Err(AuthError::InvalidCredentials("Invalid credentials".to_string()));
        }

        if !is_internal && user.auth_type != AuthType::OAuth {
            return Err(AuthError::AuthenticationFailed(
                "Account is not configured for OAuth authentication".to_string(),
            ));
        }

        if let Some(claimed_role) = claims.role {
            if claimed_role != user.role {
                log::warn!(
                    "Bearer token role mismatch: claimed={:?}, actual={:?} for user={}",
                    claimed_role,
                    user.role,
                    user.user_id.as_str()
                );
                return Err(AuthError::InvalidCredentials(
                    "Token role does not match user role".to_string(),
                ));
            }
        }

        let role = user.role;

        tracing::trace!(user_id = %user.user_id, role = ?role, "Bearer authentication succeeded");

        Ok(AuthenticatedUser::new(
            user.user_id.clone(),
            role,
            user.email.clone(),
            user.created_at,
            user.updated_at,
            connection_info.clone(),
        ))
    }
    .instrument(span)
    .await
}

async fn validate_bearer_token(
    token: &str,
    alg: &Algorithm,
    issuer: &str,
    config: &JwtConfig,
) -> AuthResult<jwt_auth::JwtClaims> {
    match alg {
        Algorithm::HS256 | Algorithm::HS384 | Algorithm::HS512 => {
            if !jwt_auth::is_internal_issuer(issuer) {
                return Err(AuthError::MalformedAuthorization(format!(
                    "HS256 tokens are only accepted for internally-issued tokens (iss='{}'). \
                     External provider tokens must use an asymmetric algorithm (RS256 / ES256). \
                     Received iss='{}'.",
                    jwt_auth::KALAMDB_ISSUER,
                    issuer
                )));
            }
            jwt_auth::validate_jwt_token(token, &config.secret, &config.trusted_issuers)
        },
        Algorithm::RS256
        | Algorithm::RS384
        | Algorithm::RS512
        | Algorithm::PS256
        | Algorithm::PS384
        | Algorithm::PS512
        | Algorithm::ES256
        | Algorithm::ES384 => {
            if jwt_auth::is_internal_issuer(issuer) {
                return Err(AuthError::MalformedAuthorization(
                    "Internal 'kalamdb' tokens must use HS256, not an asymmetric algorithm."
                        .to_string(),
                ));
            }
            let validator = config.get_oidc_validator(issuer).await?;
            let claims: jwt_auth::JwtClaims =
                validator.validate(token).await.map_err(|e| match e {
                    OidcError::JwtValidationFailed(ref msg) if msg.contains("expired") => {
                        AuthError::TokenExpired
                    },
                    OidcError::JwtValidationFailed(ref msg) if msg.contains("signature") => {
                        AuthError::InvalidSignature
                    },
                    _ => AuthError::MalformedAuthorization(format!(
                        "External token validation failed: {}",
                        e
                    )),
                })?;
            jwt_auth::verify_issuer(&claims.iss, &config.trusted_issuers)?;
            if claims.sub.is_empty() {
                return Err(AuthError::MalformedAuthorization(
                    "Token is missing a required user claim".to_string(),
                ));
            }
            Ok(claims)
        },
        _ => Err(AuthError::MalformedAuthorization(format!(
            "Unsupported JWT algorithm: {:?}",
            alg
        ))),
    }
}

async fn resolve_external_user(
    repo: &Arc<dyn UserRepository>,
    config: &JwtConfig,
    claims: &jwt_auth::JwtClaims,
) -> AuthResult<User> {
    let user_id = oidc_user_id(&claims.iss, &claims.sub)?;

    match repo.get_user_by_id(&user_id).await {
        Ok(user) => Ok(user),
        Err(AuthError::UserNotFound(_)) if config.oauth_auto_provision => {
            auto_provision_external_user(repo, config.oauth_default_role, claims, user_id).await
        },
        Err(AuthError::UserNotFound(_)) => {
            Err(AuthError::InvalidCredentials("Invalid credentials".to_string()))
        },
        Err(err) => Err(err),
    }
}

async fn auto_provision_external_user(
    repo: &Arc<dyn UserRepository>,
    default_role: Role,
    claims: &jwt_auth::JwtClaims,
    user_id: UserId,
) -> AuthResult<User> {
    let now = chrono::Utc::now().timestamp_millis();
    let user = User {
        created_at: now,
        updated_at: now,
        locked_until: None,
        last_login_at: None,
        last_seen: None,
        deleted_at: None,
        user_id: user_id.clone(),
        password_hash: String::new(),
        email: claims.email.clone(),
        auth_data: Some(AuthData::new(claims.iss.clone(), claims.sub.clone())),
        storage_id: None,
        failed_login_attempts: 0,
        role: default_role,
        auth_type: AuthType::OAuth,
        storage_mode: StorageMode::Table,
    };

    match repo.create_user(user).await {
        Ok(()) => repo.get_user_by_id(&user_id).await,
        Err(create_err) => match repo.get_user_by_id(&user_id).await {
            Ok(user) => Ok(user),
            Err(_) => Err(create_err),
        },
    }
}

fn oidc_user_id(issuer: &str, subject: &str) -> AuthResult<UserId> {
    let digest = Sha256::digest(format!("{issuer}:{subject}").as_bytes());
    let candidate = format!("u_oidc_{}", &hex::encode(digest)[..16]);
    UserId::try_new(candidate).map_err(|_| {
        AuthError::MalformedAuthorization(
            "Token contains an invalid external identity claim".to_string(),
        )
    })
}

#[cfg(test)]
mod tests {
    use super::oidc_user_id;

    #[test]
    fn test_oidc_user_id_is_deterministic() {
        let first = oidc_user_id("https://issuer.example", "subject-123").unwrap();
        let second = oidc_user_id("https://issuer.example", "subject-123").unwrap();
        let different = oidc_user_id("https://issuer.example", "subject-456").unwrap();

        assert_eq!(first, second);
        assert_ne!(first, different);
        assert!(first.as_str().starts_with("u_oidc_"));
    }
}
