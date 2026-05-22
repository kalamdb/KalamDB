use crate::errors::error::{AuthError, AuthResult};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthorizationScheme {
    Basic,
    Bearer,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ParsedAuthorizationHeader<'a> {
    pub scheme: AuthorizationScheme,
    pub credentials: &'a str,
}

impl AuthorizationScheme {
    fn parse(scheme: &str) -> Self {
        if scheme.eq_ignore_ascii_case("Basic") {
            Self::Basic
        } else if scheme.eq_ignore_ascii_case("Bearer") {
            Self::Bearer
        } else {
            Self::Other
        }
    }
}

pub fn parse_authorization_header(auth_header: &str) -> AuthResult<ParsedAuthorizationHeader<'_>> {
    let trimmed = auth_header.trim();
    if trimmed.is_empty() {
        return Err(AuthError::MissingAuthorization("Authorization header is empty".to_string()));
    }

    let mut parts = trimmed.splitn(2, char::is_whitespace);
    let scheme = parts.next().unwrap_or_default().trim();
    let credentials = parts.next().unwrap_or_default().trim();

    if scheme.is_empty() || credentials.is_empty() {
        return Err(AuthError::MalformedAuthorization(
            "Authorization header must contain scheme and credentials".to_string(),
        ));
    }

    Ok(ParsedAuthorizationHeader {
        scheme: AuthorizationScheme::parse(scheme),
        credentials,
    })
}

pub fn extract_bearer_token(auth_header: &str) -> AuthResult<&str> {
    let parsed = parse_authorization_header(auth_header)?;
    match parsed.scheme {
        AuthorizationScheme::Bearer => Ok(parsed.credentials),
        AuthorizationScheme::Basic => Err(AuthError::InvalidCredentials(
            "This endpoint requires a Bearer token. Basic authentication is not supported."
                .to_string(),
        )),
        AuthorizationScheme::Other => Err(AuthError::MalformedAuthorization(
            "Authorization header must use Bearer token".to_string(),
        )),
    }
}

pub fn is_basic_authorization(auth_header: &str) -> bool {
    auth_header
        .trim_start()
        .split_whitespace()
        .next()
        .is_some_and(|scheme| scheme.eq_ignore_ascii_case("Basic"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_bearer_with_extra_whitespace() {
        let parsed = parse_authorization_header("  bearer   abc.def.ghi  ").unwrap();
        assert_eq!(parsed.scheme, AuthorizationScheme::Bearer);
        assert_eq!(parsed.credentials, "abc.def.ghi");
    }

    #[test]
    fn extracts_bearer_token_case_insensitively() {
        assert_eq!(extract_bearer_token("bEaReR token").unwrap(), "token");
    }

    #[test]
    fn rejects_basic_for_bearer_only_paths() {
        let result = extract_bearer_token("Basic dXNlcjpwYXNz");
        assert!(matches!(result, Err(AuthError::InvalidCredentials(_))));
    }

    #[test]
    fn detects_basic_scheme_without_allocating_lowercase_copy() {
        assert!(is_basic_authorization("  basic dXNlcjpwYXNz"));
        assert!(!is_basic_authorization("Bearer token"));
        assert!(!is_basic_authorization("Basicish token"));
    }
}
