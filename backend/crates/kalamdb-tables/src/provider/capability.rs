//! Capability profile for each table kind.
//!
//! Stream tables stay on the append log: TTL and hard-delete are stream
//! capabilities, and explicit stream transactions stay rejected.

use kalamdb_commons::models::schemas::TableType;

/// How a table scopes row ownership.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScopeMode {
    /// Reads and writes require a user id.
    UserRequired,
    /// Rows are global to the table.
    Shared,
}

/// How a table records mutations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MutationMode {
    /// User and shared MVCC versions.
    VersionedMvcc,
    /// Stream append, TTL, and hard-delete.
    AppendAndHardDelete,
    /// Typed `IndexedEntityStore` writes. SQL DML stays rejected.
    DirectApiOnly,
    /// No store. Each scan recomputes rows.
    ReadOnly,
}

/// Cold storage for a table kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColdTierMode {
    /// Parquet segments published through the manifest.
    ParquetManifest,
    /// Hot storage only.
    Absent,
}

/// Which transaction forms a table accepts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransactionMode {
    /// Explicit `BEGIN` and autocommit statements.
    ExplicitAndAutocommit,
    /// Autocommit only. Explicit transactions stay rejected.
    AutocommitOnly,
    /// No SQL transaction.
    Disabled,
}

/// Where a row version comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VersionSource {
    /// Raft log index packed with an entry ordinal.
    RaftEntryOrdinal,
    /// Local sequence plus a timestamp.
    LocalMonotonicWithTimestamp,
    /// The stored key is replaced in place.
    DirectKeyInPlace,
    /// No stored version.
    Absent,
}

/// Row-level security for a table kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RlsMode {
    /// User tables restrict rows to the session user.
    OwnerScopePrefix,
    /// Shared tables use `SharedTableAuthorization`.
    ForcePolicyCheck,
    /// System and catalog relations require an admin role.
    SystemAdminOnly,
}

/// Which scan implementation serves the table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScanEngineKind {
    /// User and shared MVCC.
    Versioned,
    /// Stream append log.
    Stream,
    /// Persisted system `IndexedEntityStore`.
    Keyed,
    /// In-memory catalog projection.
    Computed,
}

/// Declarative capabilities for one table kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TableCapabilityProfile {
    pub scope_mode: ScopeMode,
    pub mutation_mode: MutationMode,
    pub cold_tier_mode: ColdTierMode,
    pub transaction_mode: TransactionMode,
    pub version_source: VersionSource,
    pub rls_mode: RlsMode,
    pub scan_engine: ScanEngineKind,
}

impl TableCapabilityProfile {
    /// Profile for a stored user, shared, or stream table.
    pub fn for_table_type(table_type: TableType) -> Self {
        match table_type {
            TableType::User => Self::user(),
            TableType::Shared => Self::shared(),
            TableType::Stream => Self::stream(),
            TableType::System => Self::persisted_system(),
        }
    }

    pub fn user() -> Self {
        Self {
            scope_mode: ScopeMode::UserRequired,
            mutation_mode: MutationMode::VersionedMvcc,
            cold_tier_mode: ColdTierMode::ParquetManifest,
            transaction_mode: TransactionMode::ExplicitAndAutocommit,
            version_source: VersionSource::RaftEntryOrdinal,
            rls_mode: RlsMode::OwnerScopePrefix,
            scan_engine: ScanEngineKind::Versioned,
        }
    }

    pub fn shared() -> Self {
        Self {
            scope_mode: ScopeMode::Shared,
            mutation_mode: MutationMode::VersionedMvcc,
            cold_tier_mode: ColdTierMode::ParquetManifest,
            transaction_mode: TransactionMode::ExplicitAndAutocommit,
            version_source: VersionSource::RaftEntryOrdinal,
            rls_mode: RlsMode::ForcePolicyCheck,
            scan_engine: ScanEngineKind::Versioned,
        }
    }

    pub fn stream() -> Self {
        Self {
            scope_mode: ScopeMode::UserRequired,
            mutation_mode: MutationMode::AppendAndHardDelete,
            cold_tier_mode: ColdTierMode::Absent,
            transaction_mode: TransactionMode::AutocommitOnly,
            version_source: VersionSource::LocalMonotonicWithTimestamp,
            rls_mode: RlsMode::OwnerScopePrefix,
            scan_engine: ScanEngineKind::Stream,
        }
    }

    pub fn persisted_system() -> Self {
        Self {
            scope_mode: ScopeMode::Shared,
            mutation_mode: MutationMode::DirectApiOnly,
            cold_tier_mode: ColdTierMode::Absent,
            transaction_mode: TransactionMode::Disabled,
            version_source: VersionSource::DirectKeyInPlace,
            rls_mode: RlsMode::SystemAdminOnly,
            scan_engine: ScanEngineKind::Keyed,
        }
    }

    pub fn computed_catalog() -> Self {
        Self {
            scope_mode: ScopeMode::Shared,
            mutation_mode: MutationMode::ReadOnly,
            cold_tier_mode: ColdTierMode::Absent,
            transaction_mode: TransactionMode::Disabled,
            version_source: VersionSource::Absent,
            rls_mode: RlsMode::SystemAdminOnly,
            scan_engine: ScanEngineKind::Computed,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shared_tables_force_policy_checks_and_streams_stay_autocommit() {
        let shared = TableCapabilityProfile::shared();
        assert_eq!(shared.rls_mode, RlsMode::ForcePolicyCheck);
        assert_eq!(shared.scan_engine, ScanEngineKind::Versioned);

        let stream = TableCapabilityProfile::for_table_type(TableType::Stream);
        assert_eq!(stream.mutation_mode, MutationMode::AppendAndHardDelete);
        assert_eq!(stream.transaction_mode, TransactionMode::AutocommitOnly);
        assert_eq!(stream.scan_engine, ScanEngineKind::Stream);
    }
}
