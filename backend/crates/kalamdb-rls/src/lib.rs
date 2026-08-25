//! Compile-time and runtime row-level security for shared tables.
//!
//! Policy SQL is compiled into user-independent [`PolicyProgram`] values. At query time,
//! programs are bound to a principal and evaluated via [`BoundTablePolicies`] and
//! [`AuthorizationSet`]. Live subscriptions hold [`BoundLiveAuthorization`] with epoch
//! guards that fail closed when membership or policy catalogs move.

mod authorization_cache;
mod authorization_cache_key;
mod authorization_constraint;
mod authorization_dependency_guard;
mod authorization_mutation_guard;
mod authorization_policy_guard;
mod authorization_set;
mod authorization_strategy;
mod bound_authorization;
mod bound_live_authorization;
mod bound_table_policies;
mod epochs;
mod policy_compiler;
mod row_access;

#[cfg(test)]
mod policy_compiler_tests;

#[cfg(test)]
mod security_regression_tests;

pub use authorization_cache::{AuthorizationCache, AuthorizationCacheMetrics};
pub use authorization_cache_key::AuthorizationCacheKey;
pub use authorization_constraint::{extract_authorization_constraint, AuthorizationConstraint};
pub use authorization_dependency_guard::AuthorizationDependencyGuard;
pub use authorization_mutation_guard::AuthorizationMutationGuard;
pub use authorization_policy_guard::AuthorizationPolicyGuard;
pub use authorization_set::AuthorizationSet;
pub use authorization_strategy::AuthorizationStrategy;
pub use bound_authorization::BoundAuthorization;
pub use bound_live_authorization::BoundLiveAuthorization;
pub use bound_table_policies::BoundTablePolicies;
pub use epochs::{MembershipEpoch, PolicyCatalogEpoch};
pub use policy_compiler::{PolicyCompiler, PolicyTableResolver};
