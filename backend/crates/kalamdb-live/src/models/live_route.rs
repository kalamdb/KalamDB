use std::{collections::HashSet, sync::Arc};

use datafusion_common::ScalarValue;

/// Principal-bound candidate route for one shared-table live subscription.
///
/// `Keyed` stores conversation/owner values behind an `Arc` so indexing and unindexing
/// share the set instead of cloning every scalar on each registry update.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LiveRoute {
    Broadcast,
    Deny,
    Keyed(Arc<HashSet<(Arc<str>, ScalarValue)>>),
}
