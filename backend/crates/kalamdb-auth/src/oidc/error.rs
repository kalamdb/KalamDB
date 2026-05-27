/// Errors produced by the OIDC validator.
#[derive(Debug, thiserror::Error)]
pub(crate) enum OidcError {
    #[error("OIDC configuration error: {0}")]
    Configuration(String),

    #[error("OIDC discovery failed: {0}")]
    DiscoveryFailed(String),

    #[error("JWT validation failed: {0}")]
    JwtValidationFailed(String),
}

impl From<jsonwebtoken::errors::Error> for OidcError {
    fn from(error: jsonwebtoken::errors::Error) -> Self {
        use jsonwebtoken::errors::ErrorKind;

        match error.kind() {
            ErrorKind::ExpiredSignature => OidcError::JwtValidationFailed("Token expired".into()),
            ErrorKind::InvalidSignature => {
                OidcError::JwtValidationFailed("Invalid signature".into())
            },
            ErrorKind::InvalidToken => OidcError::JwtValidationFailed("Invalid token".into()),
            _ => OidcError::JwtValidationFailed(error.to_string()),
        }
    }
}
