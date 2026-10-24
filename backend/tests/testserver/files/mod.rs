//! File permission tests
//!
//! Tests covering:
//! - File download permissions
//! - File uploads with insufficient permissions
//! - Cleanup of staged files on failure

// Re-export test_support from parent
pub(super) use super::test_support;

mod test_file_permissions_http;
mod test_shared_file_rls_http;
