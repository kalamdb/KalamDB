use anyhow::Result;
use kalamdb_commons::Role;
use reqwest::StatusCode;
use serial_test::serial;

use super::test_oidc_auto_provision::{
    post_sql_raw, with_configured_oidc_server, TestOidcProvider, TEST_OIDC_CLIENT_ID,
};

#[actix_web::test]
#[ntest::timeout(45000)]
#[serial]
async fn test_oidc_invalid_issuer_returns_401() -> Result<()> {
    let provider = TestOidcProvider::start(TEST_OIDC_CLIENT_ID).await?;
    let issuer = provider.issuer().to_string();

    with_configured_oidc_server(&issuer, TEST_OIDC_CLIENT_ID, Role::User, |server| {
        Box::pin(async move {
            let token = provider.issue_token(
                Some("invalid-issuer-user"),
                Some("invalid-issuer@test.local"),
                Some(Role::User),
                Some("https://untrusted-issuer.example"),
                3600,
            )?;
            let (status, body) =
                post_sql_raw(server, &format!("Bearer {token}"), "SELECT 1 AS ok").await?;

            assert_eq!(status, StatusCode::UNAUTHORIZED);
            let body_lower = body.to_lowercase();
            assert!(
                body_lower.contains("invalid_credentials")
                    || body_lower.contains("invalid credentials")
                    || body_lower.contains("untrusted issuer"),
                "unexpected response body: {body}"
            );

            Ok(())
        })
    })
    .await
}

#[actix_web::test]
#[ntest::timeout(45000)]
#[serial]
async fn test_oidc_invalid_audience_returns_401() -> Result<()> {
    let provider = TestOidcProvider::start(TEST_OIDC_CLIENT_ID).await?;
    let issuer = provider.issuer().to_string();

    with_configured_oidc_server(&issuer, TEST_OIDC_CLIENT_ID, Role::User, |server| {
        Box::pin(async move {
            let token = provider.issue_token_for_audience(
                "unexpected-client",
                Some("invalid-audience-user"),
                Some("invalid-audience@test.local"),
                Some(Role::User),
                3600,
            )?;
            let (status, body) =
                post_sql_raw(server, &format!("Bearer {token}"), "SELECT 1 AS ok").await?;

            assert_eq!(status, StatusCode::UNAUTHORIZED);
            let body_lower = body.to_lowercase();
            assert!(
                body_lower.contains("invalid_credentials")
                    || body_lower.contains("invalid credentials")
                    || body_lower.contains("audience"),
                "unexpected response body: {body}"
            );

            Ok(())
        })
    })
    .await
}

#[actix_web::test]
#[ntest::timeout(45000)]
#[serial]
async fn test_oidc_expired_token_returns_401() -> Result<()> {
    let provider = TestOidcProvider::start(TEST_OIDC_CLIENT_ID).await?;
    let issuer = provider.issuer().to_string();

    with_configured_oidc_server(&issuer, TEST_OIDC_CLIENT_ID, Role::User, |server| {
        Box::pin(async move {
            let token = provider.issue_token(
                Some("expired-user"),
                Some("expired@test.local"),
                Some(Role::User),
                None,
                -3600,
            )?;
            let (status, body) =
                post_sql_raw(server, &format!("Bearer {token}"), "SELECT 1 AS ok").await?;

            assert_eq!(status, StatusCode::UNAUTHORIZED);
            let body_lower = body.to_lowercase();
            assert!(
                body_lower.contains("token expired")
                    || body_lower.contains("invalid_credentials")
                    || body_lower.contains("invalid credentials"),
                "unexpected response body: {body}"
            );

            Ok(())
        })
    })
    .await
}
