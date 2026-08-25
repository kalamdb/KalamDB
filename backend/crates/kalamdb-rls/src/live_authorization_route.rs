use std::collections::HashSet;

use datafusion::scalar::ScalarValue;

/// Subscription routing derived from a principal-bound SELECT authorization program.
///
/// Keyed routes are candidate selectors only. The bound authorization evaluator remains the
/// final, fail-closed visibility check before delivery.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LiveAuthorizationRoute {
    /// Every subscription bound to this policy set is a candidate for each table change.
    Broadcast,
    /// No row can be visible to this subscription.
    Deny,
    /// Candidate rows are selected by stable protected-column IDs and typed values.
    Keyed(HashSet<(u64, ScalarValue)>),
}
