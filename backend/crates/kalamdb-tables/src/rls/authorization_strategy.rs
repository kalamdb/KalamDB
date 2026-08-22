/// Execution strategy selected for a bound shared-table policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthorizationStrategy {
    PointGuard,
    MultiGuard,
    DataFusionSemiJoin,
    CachedAuthorizationSet,
}
