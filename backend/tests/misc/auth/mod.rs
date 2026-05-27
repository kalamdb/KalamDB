//! Authentication and Authorization Tests
//!
//! Tests covering:
//! - Bearer token authentication
//! - JWT Authentication
//! - OAuth 2.0 Integration (Google, GitHub, Azure AD)
//! - Role-Based Access Control (RBAC)
//! - Password Security & Complexity
//! - User Impersonation
//! - Session Management
//! - Soft Deletes

// Re-export test_support from crate root
pub(super) use crate::test_support;

// Auth Tests
mod test_as_user_impersonation;
mod test_auth_performance;
mod test_basic_auth;
mod test_cli_auth;
mod test_e2e_auth_flow;
mod test_jwt_auth;
mod test_last_seen;
mod test_live_queries_auth_expiry;
mod test_local_auth_policy;
mod test_oauth;
mod test_oidc_auto_provision;
mod test_oidc_token_validation;
mod test_password_complexity;
mod test_rbac;
mod test_soft_delete;
