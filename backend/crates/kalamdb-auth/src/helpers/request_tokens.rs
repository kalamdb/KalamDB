use actix_web::{http::header, HttpRequest};

use crate::{
    errors::error::{AuthError, AuthResult},
    helpers::{authorization_header::extract_bearer_token, cookie},
};

pub fn extract_bearer_or_cookie_token(req: &HttpRequest) -> AuthResult<String> {
    if let Some(auth_header) = authorization_header(req)? {
        return extract_bearer_token(auth_header).map(str::to_owned);
    }

    cookie::extract_auth_token(req.cookies().ok().iter().flat_map(|c| c.iter().cloned()))
        .ok_or_else(|| AuthError::MissingAuthorization("No auth token found".to_string()))
}

pub fn extract_refresh_or_bearer_token(req: &HttpRequest) -> AuthResult<String> {
    if let Some(token) =
        cookie::extract_refresh_token(req.cookies().ok().iter().flat_map(|c| c.iter().cloned()))
    {
        return Ok(token);
    }

    if let Some(auth_header) = authorization_header(req)? {
        return extract_bearer_token(auth_header).map(str::to_owned);
    }

    Err(AuthError::MissingAuthorization("No refresh token found".to_string()))
}

fn authorization_header(req: &HttpRequest) -> AuthResult<Option<&str>> {
    req.headers()
        .get(header::AUTHORIZATION)
        .map(|raw_header| {
            raw_header.to_str().map_err(|_| {
                AuthError::MalformedAuthorization(
                    "Authorization header contains invalid characters".to_string(),
                )
            })
        })
        .transpose()
}
