//! System Table Tests
//!
//! Tests covering:
//! - System table queries
//! - System metadata access

// Re-export test_support from parent
pub(super) use super::test_support;

// System Tests
mod test_system_tables_http;
mod test_namespace_drop_cleanup_http;
mod test_namespace_drop_cascade_topics_http;
