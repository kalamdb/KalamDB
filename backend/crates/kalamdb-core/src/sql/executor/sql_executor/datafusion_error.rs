//! Map DataFusion planner/execution errors onto `KalamDbError`.

use crate::{error::KalamDbError, sql::executor::SqlExecutor};

impl SqlExecutor {
    /// Try to extract a typed `KalamDbError::NotLeader` from a `DataFusionError`.
    ///
    /// Table providers wrap [`kalamdb_commons::NotLeaderError`] inside
    /// `DataFusionError::External(...)`. This method downcasts back to the
    /// concrete type — no string parsing required.
    pub(super) fn try_not_leader_error(
        e: &datafusion::error::DataFusionError,
    ) -> Option<KalamDbError> {
        if let datafusion::error::DataFusionError::External(inner) = e {
            if let Some(nle) = inner.downcast_ref::<kalamdb_commons::NotLeaderError>() {
                return Some(KalamDbError::NotLeader {
                    leader_addr: nle.leader_addr.clone(),
                });
            }
        }
        None
    }

    /// Convert a `DataFusionError` into a `KalamDbError`, preserving
    /// `NotLeader` semantics when the error wraps a [`kalamdb_commons::NotLeaderError`].
    pub(super) fn datafusion_to_execution_error(
        e: datafusion::error::DataFusionError,
    ) -> KalamDbError {
        Self::classify_datafusion_error(&e)
    }

    pub(super) fn is_table_not_found_error(e: &datafusion::error::DataFusionError) -> bool {
        is_table_not_found_msg(&e.to_string().to_lowercase())
    }

    /// Classify a DataFusionError into a KalamDbError while preserving the
    /// underlying user-facing parser/planner/execution message when available.
    pub(super) fn classify_datafusion_error(
        e: &datafusion::error::DataFusionError,
    ) -> KalamDbError {
        if let Some(not_leader) = Self::try_not_leader_error(e) {
            return not_leader;
        }

        let error_msg = datafusion_user_message(e);
        let lower = error_msg.to_lowercase();

        if is_table_not_found_msg(&lower) {
            return KalamDbError::TableNotFound(error_msg);
        }

        if is_permission_msg(&lower) {
            return KalamDbError::PermissionDenied(error_msg);
        }

        if is_column_not_found_msg(&lower) {
            return KalamDbError::InvalidOperation(error_msg);
        }

        if is_constraint_violation_msg(&lower) {
            return KalamDbError::AlreadyExists(error_msg);
        }

        KalamDbError::ExecutionError(error_msg)
    }
}

fn is_table_not_found_msg(msg: &str) -> bool {
    (msg.contains("table") && msg.contains("not found"))
        || (msg.contains("relation") && msg.contains("does not exist"))
        || msg.contains("unknown table")
}

fn is_permission_msg(msg: &str) -> bool {
    msg.contains("access denied")
        || msg.contains("permission denied")
        || msg.contains("unauthorized")
        || msg.contains("not authorized")
        || msg.contains("forbidden")
        || msg.contains("insufficient privileges")
}

fn is_column_not_found_msg(msg: &str) -> bool {
    (msg.contains("column") && msg.contains("not found"))
        || (msg.contains("field") && msg.contains("not found"))
        || msg.contains("no field named")
        || msg.contains("schema error: no field named")
}

fn is_constraint_violation_msg(msg: &str) -> bool {
    msg.contains("primary key")
        || msg.contains("constraint violation")
        || msg.contains("already exists")
        || msg.contains("duplicate")
        || msg.contains("unique constraint")
        || msg.contains("unique index")
}

fn datafusion_user_message(e: &datafusion::error::DataFusionError) -> String {
    match e {
        datafusion::error::DataFusionError::SQL(parser_error, _) => parser_error.to_string(),
        datafusion::error::DataFusionError::SchemaError(schema_error, _) => {
            schema_error.to_string()
        },
        datafusion::error::DataFusionError::Plan(message)
        | datafusion::error::DataFusionError::Execution(message)
        | datafusion::error::DataFusionError::Internal(message)
        | datafusion::error::DataFusionError::ResourcesExhausted(message)
        | datafusion::error::DataFusionError::NotImplemented(message) => message.clone(),
        datafusion::error::DataFusionError::Context(context, source) => {
            format!("{context}: {}", datafusion_user_message(source))
        },
        datafusion::error::DataFusionError::ArrowError(error, _) => error.to_string(),
        datafusion::error::DataFusionError::External(error) => error.to_string(),
        _ => e.to_string(),
    }
}
