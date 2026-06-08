use std::sync::Arc;

use jsonwebtoken::Algorithm;
use kalamdb_commons::{
    models::{ConnectionInfo, UserId},
    AuthType, Role,
};
use kalamdb_system::providers::storages::models::StorageMode;
use kalamdb_system::{AuthData, User};
use tracing::Instrument;

use crate::{
    errors::error::{AuthError, AuthResult},
    models::context::AuthenticatedUser,
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

        // Single pass: extract alg, header typ, and issuer together — avoids three separate
        // decode_header / base64 / JSON parses that the old two-call approach incurred.
        let (alg, header_typ, issuer) = jwt_auth::extract_header_claims_unverified(token)?;

        jwt_auth::verify_issuer(&issuer, &config.trusted_issuers)?;

        tracing::debug!(
            issuer = %issuer,
            algorithm = ?alg,
            oidc_auto_provision = config.oidc_auto_provision,
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
            // Use the typ already extracted above — no additional decode_header needed.
            if let Some(typ) = header_typ {
                if typ.to_lowercase().contains("refresh") {
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
            resolve_internal_authenticated_user(repo, &config, &claims, connection_info).await?
        } else {
            resolve_external_authenticated_user(repo, &config, &claims, connection_info).await?
        };

        let role = user.role;

        tracing::trace!(user_id = %user.user_id, role = ?role, "Bearer authentication succeeded");

        Ok(user)
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
            let claims = config.validate_oidc_id_token(issuer, token).await?;
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

pub(super) async fn resolve_internal_authenticated_user(
    repo: &Arc<dyn UserRepository>,
    config: &JwtConfig,
    claims: &jwt_auth::JwtClaims,
    connection_info: &ConnectionInfo,
) -> AuthResult<AuthenticatedUser> {
    let user_id = user_id_from_token_subject(&claims.sub)?;

    if matches!(claims.auth_type, Some(AuthType::Oidc)) {
        return resolve_internal_oauth_session_user(repo, config, claims, user_id, connection_info)
            .await;
    }

    let user = repo.get_user_by_id(&user_id).await?;
    if user.deleted_at.is_some() {
        return Err(AuthError::InvalidCredentials("Invalid credentials".to_string()));
    }
    verify_claimed_role(&user.user_id, claims.role.as_ref(), user.role)?;
    Ok(authenticated_user_from_persisted_user(user, connection_info))
}

async fn resolve_internal_oauth_session_user(
    repo: &Arc<dyn UserRepository>,
    config: &JwtConfig,
    claims: &jwt_auth::JwtClaims,
    user_id: UserId,
    connection_info: &ConnectionInfo,
) -> AuthResult<AuthenticatedUser> {
    match repo.get_user_by_id(&user_id).await {
        Ok(user) => {
            if user.deleted_at.is_some() {
                return Err(AuthError::InvalidCredentials("Invalid credentials".to_string()));
            }
            if user.auth_type != AuthType::Oidc {
                return Err(AuthError::AuthenticationFailed(
                    "Account is not configured for OIDC authentication".to_string(),
                ));
            }
            verify_claimed_role(&user.user_id, claims.role.as_ref(), user.role)?;
            Ok(authenticated_user_from_persisted_user(user, connection_info))
        },
        Err(AuthError::UserNotFound(_))
            if config.oidc_auto_provision && config.oidc_default_role == Role::User =>
        {
            verify_claimed_role(&user_id, claims.role.as_ref(), Role::User)?;
            Ok(authenticated_user_from_claims(
                user_id,
                Role::User,
                AuthType::Oidc,
                claims,
                connection_info,
            ))
        },
        Err(AuthError::UserNotFound(_)) => {
            Err(AuthError::InvalidCredentials("Invalid credentials".to_string()))
        },
        Err(err) => Err(err),
    }
}

async fn resolve_external_authenticated_user(
    repo: &Arc<dyn UserRepository>,
    config: &JwtConfig,
    claims: &jwt_auth::JwtClaims,
    connection_info: &ConnectionInfo,
) -> AuthResult<AuthenticatedUser> {
    let user_id = user_id_from_oidc_subject(&claims.sub)?;

    match repo.get_user_by_id(&user_id).await {
        Ok(user) => {
            if user.deleted_at.is_some() {
                if let Some(user) =
                    try_accept_external_oidc_invite(repo, claims, user_id.clone()).await?
                {
                    return authenticate_persisted_external_user(user, claims, connection_info);
                }

                return Err(AuthError::InvalidCredentials("Invalid credentials".to_string()));
            }

            authenticate_persisted_external_user(user, claims, connection_info)
        },
        Err(AuthError::UserNotFound(_)) => {
            if let Some(user) =
                try_accept_external_oidc_invite(repo, claims, user_id.clone()).await?
            {
                return authenticate_persisted_external_user(user, claims, connection_info);
            }

            if config.oidc_auto_provision && config.oidc_default_role == Role::User {
                verify_claimed_role(&user_id, claims.role.as_ref(), Role::User)?;
                return Ok(authenticated_user_from_claims(
                    user_id,
                    Role::User,
                    AuthType::Oidc,
                    claims,
                    connection_info,
                ));
            }

            if config.oidc_auto_provision {
                let user =
                    auto_provision_external_user(repo, config.oidc_default_role, claims, user_id)
                        .await?;
                return authenticate_persisted_external_user(user, claims, connection_info);
            }

            Err(AuthError::InvalidCredentials("Invalid credentials".to_string()))
        },
        Err(err) => Err(err),
    }
}

async fn try_accept_external_oidc_invite(
    repo: &Arc<dyn UserRepository>,
    claims: &jwt_auth::JwtClaims,
    user_id: UserId,
) -> AuthResult<Option<User>> {
    let Some(email) = claims.email.as_deref().map(str::trim).filter(|email| !email.is_empty())
    else {
        return Ok(None);
    };

    let now = chrono::Utc::now().timestamp_millis();
    let Some(invite) = repo.get_active_oidc_invite_by_email(email, now).await? else {
        return Ok(None);
    };

    verify_claimed_role(&user_id, claims.role.as_ref(), invite.role)?;
    let now = chrono::Utc::now().timestamp_millis();
    let user = User {
        created_at: now,
        updated_at: now,
        locked_until: None,
        last_login_at: None,
        last_seen: None,
        deleted_at: None,
        invite_expires_at: None,
        invited_by: None,
        user_id: user_id.clone(),
        password_hash: String::new(),
        name: claims.name.clone(),
        email: Some(email.to_ascii_lowercase()),
        auth_data: Some(AuthData::new(claims.iss.clone(), claims.sub.clone())),
        storage_id: invite.storage_id.clone(),
        failed_login_attempts: 0,
        role: invite.role,
        auth_type: AuthType::Oidc,
        storage_mode: invite.storage_mode,
    };

    repo.accept_oidc_invite(&invite.user_id, user).await?;
    match repo.get_user_by_id(&user_id).await {
        Ok(user) => Ok(Some(user)),
        Err(AuthError::UserNotFound(_)) => Err(AuthError::DatabaseError(
            "accepted OIDC invite did not create a user".to_string(),
        )),
        Err(error) => Err(error),
    }
}

fn authenticate_persisted_external_user(
    user: User,
    claims: &jwt_auth::JwtClaims,
    connection_info: &ConnectionInfo,
) -> AuthResult<AuthenticatedUser> {
    if user.deleted_at.is_some() {
        return Err(AuthError::InvalidCredentials("Invalid credentials".to_string()));
    }
    verify_persisted_external_identity(&user, claims)?;
    verify_claimed_role(&user.user_id, claims.role.as_ref(), user.role)?;

    Ok(authenticated_user_from_persisted_user(user, connection_info))
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
        invite_expires_at: None,
        user_id: user_id.clone(),
        password_hash: String::new(),
        name: claims.name.clone(),
        email: claims.email.clone(),
        auth_data: Some(AuthData::new(claims.iss.clone(), claims.sub.clone())),
        storage_id: None,
        invited_by: None,
        failed_login_attempts: 0,
        role: default_role,
        auth_type: AuthType::Oidc,
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

fn user_id_from_oidc_subject(subject: &str) -> AuthResult<UserId> {
    UserId::try_new(subject.to_string()).map_err(|error| {
        AuthError::MalformedAuthorization(format!(
            "OIDC subject is not a valid KalamDB user id: {error}"
        ))
    })
}

fn user_id_from_token_subject(subject: &str) -> AuthResult<UserId> {
    UserId::try_new(subject.to_string()).map_err(|_| {
        AuthError::MalformedAuthorization("Token contains an invalid user claim".to_string())
    })
}

fn verify_persisted_external_identity(user: &User, claims: &jwt_auth::JwtClaims) -> AuthResult<()> {
    if user.auth_type != AuthType::Oidc {
        return Err(AuthError::AuthenticationFailed(
            "Account is not configured for OIDC authentication".to_string(),
        ));
    }

    let auth_data = user.auth_data.as_ref().ok_or_else(|| {
        AuthError::AuthenticationFailed("OIDC account is missing identity link".to_string())
    })?;

    if auth_data.issuer != claims.iss || auth_data.subject != claims.sub {
        log::warn!(
            "OIDC token identity mismatch for user={}: token issuer={}, token subject={}",
            user.user_id,
            claims.iss,
            claims.sub
        );
        return Err(AuthError::AuthenticationFailed(
            "OIDC token does not match the linked identity".to_string(),
        ));
    }

    Ok(())
}

fn verify_claimed_role(
    user_id: &UserId,
    claimed_role: Option<&Role>,
    actual_role: Role,
) -> AuthResult<()> {
    if let Some(claimed_role) = claimed_role {
        if *claimed_role != actual_role {
            log::warn!(
                "Bearer token role mismatch: claimed={:?}, actual={:?} for user={}",
                claimed_role,
                actual_role,
                user_id.as_str()
            );
            return Err(AuthError::InvalidCredentials(
                "Token role does not match user role".to_string(),
            ));
        }
    }

    Ok(())
}

fn authenticated_user_from_persisted_user(
    user: User,
    connection_info: &ConnectionInfo,
) -> AuthenticatedUser {
    AuthenticatedUser::with_auth_type(
        user.user_id,
        user.role,
        user.auth_type,
        user.email,
        user.name,
        user.created_at,
        user.updated_at,
        connection_info.clone(),
    )
}

fn authenticated_user_from_claims(
    user_id: UserId,
    role: Role,
    auth_type: AuthType,
    claims: &jwt_auth::JwtClaims,
    connection_info: &ConnectionInfo,
) -> AuthenticatedUser {
    let issued_at = token_timestamp_millis(claims.iat);
    AuthenticatedUser::with_auth_type(
        user_id,
        role,
        auth_type,
        claims.email.clone(),
        claims.name.clone(),
        issued_at,
        issued_at,
        connection_info.clone(),
    )
}

fn token_timestamp_millis(timestamp_secs: usize) -> i64 {
    timestamp_secs.saturating_mul(1000).min(i64::MAX as usize) as i64
}

#[cfg(test)]
mod tests {
    use std::{
        collections::HashMap,
        sync::{
            atomic::{AtomicUsize, Ordering},
            Arc, Mutex,
        },
    };

    use super::*;

    #[derive(Default)]
    struct CountingRepo {
        lookups: AtomicUsize,
        users: Mutex<HashMap<UserId, User>>,
    }

    impl CountingRepo {
        fn with_user(user: User) -> Self {
            Self {
                lookups: AtomicUsize::new(0),
                users: Mutex::new(HashMap::from([(user.user_id.clone(), user)])),
            }
        }

        fn with_users(users: Vec<User>) -> Self {
            Self {
                lookups: AtomicUsize::new(0),
                users: Mutex::new(
                    users.into_iter().map(|user| (user.user_id.clone(), user)).collect(),
                ),
            }
        }

        fn lookup_count(&self) -> usize {
            self.lookups.load(Ordering::SeqCst)
        }
    }

    #[async_trait::async_trait]
    impl UserRepository for CountingRepo {
        async fn get_user_by_id(&self, user_id: &UserId) -> AuthResult<User> {
            self.lookups.fetch_add(1, Ordering::SeqCst);
            self.users
                .lock()
                .expect("test repo users lock should not be poisoned")
                .get(user_id)
                .cloned()
                .ok_or_else(|| AuthError::UserNotFound(format!("User '{}' not found", user_id)))
        }

        async fn get_active_oidc_invite_by_email(
            &self,
            email: &str,
            now_millis: i64,
        ) -> AuthResult<Option<User>> {
            let normalized_email = email.trim().to_ascii_lowercase();
            Ok(self
                .users
                .lock()
                .expect("test repo users lock should not be poisoned")
                .values()
                .find(|user| {
                    user.auth_type == AuthType::OidcInvite
                        && user.deleted_at.is_none()
                        && user.email.as_deref().map(str::to_ascii_lowercase)
                            == Some(normalized_email.clone())
                        && user.invite_expires_at.is_some_and(|expires_at| expires_at >= now_millis)
                })
                .cloned())
        }

        async fn update_user(&self, _user: &User) -> AuthResult<()> {
            Ok(())
        }

        async fn create_user(&self, _user: User) -> AuthResult<()> {
            self.users
                .lock()
                .expect("test repo users lock should not be poisoned")
                .insert(_user.user_id.clone(), _user);
            Ok(())
        }

        async fn accept_oidc_invite(&self, invite_user_id: &UserId, user: User) -> AuthResult<()> {
            let mut users = self.users.lock().expect("test repo users lock should not be poisoned");
            users.insert(user.user_id.clone(), user);
            if let Some(invite) = users.get_mut(invite_user_id) {
                invite.deleted_at = Some(chrono::Utc::now().timestamp_millis());
            }
            Ok(())
        }
    }

    fn external_claims(role: Option<Role>) -> jwt_auth::JwtClaims {
        jwt_auth::JwtClaims {
            sub: "subject-123".to_string(),
            iss: "https://issuer.example".to_string(),
            exp: 99_999_999,
            iat: 1_700_000_000,
            name: None,
            email: Some("user@example.com".to_string()),
            role,
            auth_type: Some(AuthType::Oidc),
            token_type: None,
        }
    }

    fn internal_claims(user_id: &UserId, role: Role, auth_type: AuthType) -> jwt_auth::JwtClaims {
        jwt_auth::JwtClaims {
            sub: user_id.as_str().to_string(),
            iss: jwt_auth::KALAMDB_ISSUER.to_string(),
            exp: 99_999_999,
            iat: 1_700_000_000,
            name: None,
            email: Some("user@example.com".to_string()),
            role: Some(role),
            auth_type: Some(auth_type),
            token_type: Some(jwt_auth::TokenType::Access),
        }
    }

    fn test_user(user_id: UserId, role: Role, auth_type: AuthType) -> User {
        let auth_data = if auth_type == AuthType::Oidc {
            Some(AuthData::new("https://issuer.example", user_id.as_str()))
        } else {
            None
        };

        User {
            created_at: 1_700_000_000_000,
            updated_at: 1_700_000_000_000,
            locked_until: None,
            last_login_at: None,
            last_seen: None,
            deleted_at: None,
            invite_expires_at: None,
            invited_by: None,
            user_id,
            password_hash: String::new(),
            name: None,
            email: Some("stored@example.com".to_string()),
            auth_data,
            storage_id: None,
            failed_login_attempts: 0,
            role,
            auth_type,
            storage_mode: StorageMode::Table,
        }
    }

    fn connection_info() -> ConnectionInfo {
        ConnectionInfo::new(Some("127.0.0.1".to_string()))
    }

    #[test]
    fn oidc_user_id_uses_subject_directly() {
        let user_id = user_id_from_oidc_subject("subject-123").unwrap();

        assert_eq!(user_id.as_str(), "subject-123");
        assert!(user_id_from_oidc_subject("subject/123").is_err());
    }

    #[tokio::test]
    async fn regular_external_user_uses_subject_without_persisting_when_default_role_is_user() {
        let repo = Arc::new(CountingRepo::default());
        let repo_dyn: Arc<dyn UserRepository> = repo.clone();
        let config = JwtConfig::for_tests(true, Role::User);
        let claims = external_claims(None);

        let user =
            resolve_external_authenticated_user(&repo_dyn, &config, &claims, &connection_info())
                .await
                .expect("regular external user should resolve from token claims");

        assert_eq!(repo.lookup_count(), 1);
        assert_eq!(user.user_id.as_str(), "subject-123");
        assert_eq!(user.role, Role::User);
        assert_eq!(user.auth_type, AuthType::Oidc);
        assert_eq!(user.email.as_deref(), Some("user@example.com"));
    }

    #[tokio::test]
    async fn external_subject_does_not_authenticate_password_user_collision() {
        let claims = external_claims(None);
        let user_id = UserId::new(claims.sub.clone());
        let repo =
            Arc::new(CountingRepo::with_user(test_user(user_id, Role::User, AuthType::Password)));
        let repo_dyn: Arc<dyn UserRepository> = repo.clone();
        let config = JwtConfig::for_tests(true, Role::User);

        let error =
            resolve_external_authenticated_user(&repo_dyn, &config, &claims, &connection_info())
                .await
                .expect_err("OIDC subject must not authenticate a password user with the same id");

        assert!(matches!(error, AuthError::AuthenticationFailed(_)));
        assert_eq!(repo.lookup_count(), 1);
    }

    #[tokio::test]
    async fn elevated_external_oauth_user_uses_persisted_role() {
        let claims = external_claims(None);
        let user_id = UserId::new(claims.sub.clone());
        let repo = Arc::new(CountingRepo::with_user(test_user(
            user_id.clone(),
            Role::Dba,
            AuthType::Oidc,
        )));
        let repo_dyn: Arc<dyn UserRepository> = repo.clone();
        let config = JwtConfig::for_tests(true, Role::User);

        let user =
            resolve_external_authenticated_user(&repo_dyn, &config, &claims, &connection_info())
                .await
                .expect("persisted OIDC user should resolve from system.users");

        assert_eq!(repo.lookup_count(), 1);
        assert_eq!(user.user_id, user_id);
        assert_eq!(user.role, Role::Dba);
        assert_eq!(user.auth_type, AuthType::Oidc);
    }

    #[tokio::test]
    #[ntest::timeout(100)]
    async fn external_oauth_user_accepts_active_email_invite_when_auto_provision_is_disabled() {
        let claims = external_claims(None);
        let invite_id = UserId::new("invite_alice");
        let mut invite = test_user(invite_id.clone(), Role::Dba, AuthType::OidcInvite);
        invite.email = Some("USER@example.com".to_string());
        invite.auth_data = None;
        invite.invite_expires_at = Some(chrono::Utc::now().timestamp_millis() + 60_000);

        let repo = Arc::new(CountingRepo::with_user(invite));
        let repo_dyn: Arc<dyn UserRepository> = repo.clone();
        let config = JwtConfig::for_tests(false, Role::User);

        let user =
            resolve_external_authenticated_user(&repo_dyn, &config, &claims, &connection_info())
                .await
                .expect("active invite should create a persisted OIDC user");

        assert_eq!(user.user_id.as_str(), "subject-123");
        assert_eq!(user.role, Role::Dba);
        assert_eq!(user.auth_type, AuthType::Oidc);

        let accepted = repo
            .get_user_by_id(&UserId::new("subject-123"))
            .await
            .expect("accepted user should be persisted");
        assert_eq!(
            accepted.auth_data.as_ref().map(|data| data.subject.as_str()),
            Some("subject-123")
        );

        let invite = repo
            .get_user_by_id(&invite_id)
            .await
            .expect("invite row should remain as soft-deleted history");
        assert!(invite.deleted_at.is_some());
    }

    #[tokio::test]
    #[ntest::timeout(100)]
    async fn external_oauth_user_accepts_active_invite_when_same_subject_was_deleted() {
        let claims = external_claims(None);
        let user_id = UserId::new(claims.sub.clone());
        let mut deleted_user = test_user(user_id.clone(), Role::User, AuthType::Oidc);
        deleted_user.email = Some("user@example.com".to_string());
        deleted_user.deleted_at = Some(chrono::Utc::now().timestamp_millis());

        let invite_id = UserId::new("invite_retry");
        let mut invite = test_user(invite_id.clone(), Role::Dba, AuthType::OidcInvite);
        invite.email = Some("user@example.com".to_string());
        invite.auth_data = None;
        invite.invite_expires_at = Some(chrono::Utc::now().timestamp_millis() + 60_000);

        let repo = Arc::new(CountingRepo::with_users(vec![deleted_user, invite]));
        let repo_dyn: Arc<dyn UserRepository> = repo.clone();
        let config = JwtConfig::for_tests(false, Role::User);

        let user =
            resolve_external_authenticated_user(&repo_dyn, &config, &claims, &connection_info())
                .await
                .expect("active invite should recreate a previously deleted OIDC subject");

        assert_eq!(user.user_id, user_id);
        assert_eq!(user.role, Role::Dba);
        assert_eq!(user.auth_type, AuthType::Oidc);

        let invite = repo
            .get_user_by_id(&invite_id)
            .await
            .expect("invite row should remain as soft-deleted history");
        assert!(invite.deleted_at.is_some());
    }

    #[tokio::test]
    async fn exchanged_regular_oauth_session_token_uses_subject_without_persisting() {
        let user_id = UserId::new("subject-123");
        let claims = internal_claims(&user_id, Role::User, AuthType::Oidc);
        let repo = Arc::new(CountingRepo::default());
        let repo_dyn: Arc<dyn UserRepository> = repo.clone();
        let config = JwtConfig::for_tests(true, Role::User);

        let user =
            resolve_internal_authenticated_user(&repo_dyn, &config, &claims, &connection_info())
                .await
                .expect("OAuth session token should resolve without a stored regular user");

        assert_eq!(repo.lookup_count(), 1);
        assert_eq!(user.user_id, user_id);
        assert_eq!(user.role, Role::User);
        assert_eq!(user.auth_type, AuthType::Oidc);
    }
}
