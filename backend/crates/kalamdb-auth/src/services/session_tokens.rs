use chrono::{DateTime, Duration, Utc};
use kalamdb_commons::{AuthType, Role, UserId};
use kalamdb_configs::AuthSettings;

#[cfg(feature = "http")]
use crate::helpers::cookie::CookieConfig;
use crate::{
    errors::error::{AuthError, AuthResult},
    providers::jwt_auth::{
        create_and_sign_refresh_token_with_auth_type, create_and_sign_token_with_auth_type,
    },
};

#[derive(Debug, Clone)]
pub struct IssuedAuthTokens {
    pub access_token:       String,
    pub refresh_token:      String,
    pub access_expires_in:  Duration,
    pub refresh_expires_in: Duration,
    pub expires_at:         DateTime<Utc>,
    pub refresh_expires_at: DateTime<Utc>,
}

pub fn issue_auth_tokens(
    user_id: &UserId,
    role: &Role,
    email: Option<&str>,
    auth_type: AuthType,
    settings: &AuthSettings,
) -> AuthResult<IssuedAuthTokens> {
    let access_expiry_hours = settings.jwt_expiry_hours;
    let refresh_expiry_hours = access_expiry_hours.checked_mul(7).ok_or_else(|| {
        AuthError::AuthenticationFailed("JWT refresh expiry configuration is too large".to_string())
    })?;

    let (access_token, access_claims) = create_and_sign_token_with_auth_type(
        user_id,
        role,
        email,
        Some(access_expiry_hours),
        &settings.jwt_secret,
        auth_type,
    )?;
    let (refresh_token, refresh_claims) = create_and_sign_refresh_token_with_auth_type(
        user_id,
        role,
        email,
        Some(refresh_expiry_hours),
        &settings.jwt_secret,
        auth_type,
    )?;

    Ok(IssuedAuthTokens {
        access_token,
        refresh_token,
        access_expires_in: Duration::hours(access_expiry_hours),
        refresh_expires_in: Duration::hours(refresh_expiry_hours),
        expires_at: DateTime::from_timestamp(access_claims.exp as i64, 0).ok_or_else(|| {
            AuthError::AuthenticationFailed("JWT access expiry timestamp is invalid".to_string())
        })?,
        refresh_expires_at: DateTime::from_timestamp(refresh_claims.exp as i64, 0).ok_or_else(
            || {
                AuthError::AuthenticationFailed(
                    "JWT refresh expiry timestamp is invalid".to_string(),
                )
            },
        )?,
    })
}

#[cfg(feature = "http")]
pub fn auth_cookie_config(settings: &AuthSettings) -> CookieConfig {
    CookieConfig {
        secure: settings.cookie_secure,
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::jwt_auth::{validate_jwt_token, TokenType, KALAMDB_ISSUER};

    #[test]
    fn issue_auth_tokens_sets_distinct_token_types_and_expiry_windows() {
        let mut settings = AuthSettings::default();
        settings.jwt_secret = "test-secret-with-enough-length-123456".to_string();
        settings.jwt_expiry_hours = 2;

        let user_id = UserId::new("user_issued");
        let issued = issue_auth_tokens(
            &user_id,
            &Role::User,
            Some("user@example.com"),
            AuthType::Password,
            &settings,
        )
        .expect("tokens should be issued");

        let trusted = vec![KALAMDB_ISSUER.to_string()];
        let access_claims =
            validate_jwt_token(&issued.access_token, &settings.jwt_secret, &trusted)
                .expect("access token should validate");
        let refresh_claims =
            validate_jwt_token(&issued.refresh_token, &settings.jwt_secret, &trusted)
                .expect("refresh token should validate");

        assert_eq!(access_claims.token_type, Some(TokenType::Access));
        assert_eq!(refresh_claims.token_type, Some(TokenType::Refresh));
        assert_eq!(issued.access_expires_in, Duration::hours(2));
        assert_eq!(issued.refresh_expires_in, Duration::hours(14));
    }

    #[cfg(feature = "http")]
    #[test]
    fn auth_cookie_config_uses_configured_secure_flag_without_request_scheme_downgrade() {
        let mut settings = AuthSettings::default();
        settings.cookie_secure = true;

        assert!(auth_cookie_config(&settings).secure);

        settings.cookie_secure = false;
        assert!(!auth_cookie_config(&settings).secure);
    }
}
