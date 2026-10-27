//! Canonical SQL contract compiler: snapshot, Arrow resolution, hashing, and diff.

mod arrow;
mod compile;
mod diff;
mod hash;
mod identity;
mod query_plan;
pub mod scalar_mapping;
mod snapshot;
mod table_access;

use std::fmt;

pub use arrow::{is_builtin_type, resolve_arrow_type};
pub use compile::{compile_contract, compile_contract_sql, ContractSource};
pub use diff::{diff_contracts, ContractDiff};
pub use hash::{canonical_contract_hash, scoped_contract_hashes, ScopedContractHashes};
pub use identity::{apply_identity_manifest, TypeIdentityEntry, TypeIdentityManifest};
pub use query_plan::{
    analyze_named_query, LiveEligibility, QueryCancellation, QueryCardinality, QueryColumn,
    QueryFeatures, QueryOrderKey, QueryPagination, QueryParameter, QueryPlanError, QueryPredicate,
    QuerySourceTable, TypedQueryPlan,
};
pub use scalar_mapping::{
    check_json_scalar, parse_checked_decimal128, CheckedContractScalar, ContractFieldError,
    ValuePresence,
};
pub use snapshot::{
    table_payload_tag, ContractField, ContractRoutine, ContractSnapshot, ContractTable,
    ContractTableKind, ContractTrigger, ContractType, ContractTypeKind, RoutineCardinality,
    CURRENT_CONTRACT_EPOCH,
};
pub use table_access::{
    reject_forbidden_write, table_row_contract, ColumnContract, TableRowContract, WriteShape,
};

/// Error produced while compiling or diffing a SQL contract.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContractError {
    pub message: String,
}

impl ContractError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for ContractError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for ContractError {}
