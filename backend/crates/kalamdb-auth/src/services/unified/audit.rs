use kalamdb_commons::UserId;

use super::types::AuthRequest;

/// Extract a user id from an auth request for audit logging.
///
/// Bearer and raw JWT values are not decoded. An unverified `sub` claim is
/// attacker-controlled and must not be written as the actor.
pub fn extract_user_id_for_audit(request: &AuthRequest) -> UserId {
    match request {
        AuthRequest::Header(_) | AuthRequest::Jwt { .. } => UserId::anonymous(),
        AuthRequest::Credentials { user, .. } => {
            UserId::try_new(user.clone()).unwrap_or_else(|_| UserId::anonymous())
        },
    }
}
