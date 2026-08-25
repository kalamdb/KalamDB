//! Security regression tests
//!
//! Tests covering authentication boundary bypass attempts over the real HTTP API.

pub(super) use super::test_support;

mod test_auth_bypass_http;
mod test_shared_table_rls_http;
