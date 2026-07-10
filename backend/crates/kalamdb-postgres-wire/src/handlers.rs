use std::sync::Arc;

use kalamdb_auth::{
    authenticate_wire_password, repository::user_repo::UserRepository, AuthResult, WireAuthResult,
    WirePasswordAuthRequest,
};
use kalamdb_backend::session::BackendAuth;

const DEFAULT_WIRE_SESSION_LEASE_MS: i64 = 30 * 60 * 1_000;

pub async fn authenticate_startup_password(
    request: WirePasswordAuthRequest,
    repo: &Arc<dyn UserRepository>,
    now_ms: i64,
) -> AuthResult<BackendAuth> {
    let auth_result = authenticate_wire_password(request, repo).await?;
    Ok(backend_auth_from_wire_result(auth_result, now_ms))
}

pub fn backend_auth_from_wire_result(result: WireAuthResult, now_ms: i64) -> BackendAuth {
    BackendAuth::new(
        result.user_id,
        result.role,
        format!("{:?}", result.method).to_lowercase(),
        now_ms + DEFAULT_WIRE_SESSION_LEASE_MS,
    )
}
