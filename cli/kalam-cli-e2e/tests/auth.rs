// Aggregator for authentication tests to ensure Cargo picks them up
//
// Run these tests with:
//   cargo test --test auth
//
// Run individual test files:
//   cargo test --test auth test_auth

mod common;

#[path = "auth/test_auth.rs"]
mod test_auth;

#[path = "auth/test_keycloak_auth.rs"]
mod test_keycloak_auth;

#[path = "auth/test_oidc_browser_login.rs"]
mod test_oidc_browser_login;

#[path = "auth/test_oidc_device_login.rs"]
mod test_oidc_device_login;

#[path = "auth/test_oidc_dex_e2e.rs"]
mod test_oidc_dex_e2e;

#[path = "auth/test_oidc_token_validation.rs"]
mod test_oidc_token_validation;

#[path = "auth/test_oidc_local_policy.rs"]
mod test_oidc_local_policy;
