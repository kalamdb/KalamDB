//! # kalamdb-system
//!
//! System table providers and metadata management for KalamDB.
//!
//! This crate contains all system table implementations:
//! - UsersTableProvider: System users and authentication
//! - JobsTableProvider: Background job status and history
//! - NamespacesTableProvider: Database namespace catalog
//! - SchemasTableProvider: Table metadata and schema registry
//! - StoragesTableProvider: Storage backend configuration
//! - AuditLogsTableProvider: System audit log entries
//!
//! ## Architecture
//!
//! System tables use `SecuredSystemTableProvider` from `kalamdb-session-datafusion` to
//! enforce permission checks at scan time. This provides defense-in-depth
//! security even for nested queries or subqueries.
//!
//! ## Example Usage
//!
//! ```rust,ignore
//! use kalamdb_system::{SystemTablesRegistry, UsersTableProvider};
//!
//! // Register all system tables
//! let registry = SystemTablesRegistry::new(app_context);
//! registry.register_all(session_state)?;
//!
//! // Query system tables via SQL
//! // SELECT * FROM system.users WHERE role = 'dba';
//! ```

#[macro_use]
pub mod macros;

pub mod error;
pub mod impls;
pub mod providers;
pub mod registry;
pub mod services;
pub(crate) mod system_row_mapper;

// Re-export main types
pub use error::{Result, SystemError};
pub use impls::{
    classify_live_history, live_history_gap_message, live_notify_scope, publish_live_table_change,
    ClusterCoordinator, LiveHistoryDecision, LiveReadContract, ManifestService,
    NotificationService, SchemaRegistry, TopicPublisher,
};
// Re-export from kalamdb-commons for convenience
pub use kalamdb_commons::models::{AuthType, Role};
// Re-export SystemTable and StoragePartition from kalamdb_commons for consistent usage
pub use kalamdb_commons::{schemas, SchemaId, StoragePartition, SystemTable, TableName};
// Re-export DataFusion session security adapters for convenience
pub use kalamdb_session_datafusion::{
    check_system_table_access, secure_provider, SecuredSystemTableProvider, SessionUserContext,
};
// Re-export other system table models for convenience
pub use providers::audit_logs::models::AuditLogEntry;
// Re-export job models for convenience
pub use providers::jobs::models::{
    Job, JobFilter, JobOptions, JobSortField, JobStatus, JobType, SortOrder,
};
// Re-export live query models for convenience
pub use providers::live::models::{LiveQuery, LiveQueryStatus};
pub use providers::{
    catalog::{
        ActivateFunctionOutcome, CatalogFunctionArtifact, CatalogFunctionModule,
        CatalogFunctionRevision, CatalogRoutine, CatalogRoutineGrant, CatalogRoutineParameter,
        CatalogSchedule, CatalogStores, CatalogTrigger, CatalogTriggerAttempt, CatalogType,
        CatalogTypeField,
    },
    job_nodes::models::JobNode,
    manifest::models::{
        ColumnStats, FileRef, FileSubfolderState, Manifest, ManifestCacheEntry, SegmentMetadata,
        SegmentStatus, SyncState, VectorEngine, VectorIndexMetadata, VectorIndexState,
        VectorMetric,
    },
    migrations::models::Migration,
    schemas::models::Namespace,
    storages::models::{Storage, StorageType},
    users::models::{
        AuthData, User, DEFAULT_LOCKOUT_DURATION_MINUTES, DEFAULT_MAX_FAILED_ATTEMPTS,
    },
};
// Re-export all providers
pub use providers::{
    AuditLogsTableProvider, FunctionArtifactsTableProvider, FunctionModulesTableProvider,
    FunctionRevisionsTableProvider, InMemoryChecker, JobNodesTableProvider, JobsTableProvider,
    ManifestTableProvider, MigrationsTableProvider, NamespacesTableProvider,
    RoutineGrantsTableProvider, RoutineParametersTableProvider, RoutinesTableProvider,
    SchedulesTableProvider, SchemasTableProvider, StoragesTableProvider,
    TablePoliciesTableProvider, TriggerAttemptsTableProvider, TriggersTableProvider,
    TypeFieldsTableProvider, TypesTableProvider, UsersTableProvider,
};
pub use registry::SystemTablesRegistry;
pub use services::SystemColumnsService;
