//! COMMENT ON TYPE / PROCEDURE handler.

use std::sync::Arc;

use kalamdb_commons::models::{RoutineId, TypeId};
use kalamdb_core::{
    app_context::AppContext,
    error::KalamDbError,
    sql::{
        context::{ExecutionContext, ExecutionResult, ScalarValue},
        executor::handlers::TypedStatementHandler,
    },
};
use kalamdb_sql::ddl::{CommentOnStatement, CommentOnTarget};
use kalamdb_system::{CatalogStores, CatalogType};

use crate::helpers::{async_blocking::run_blocking, guards::require_admin};

pub struct CommentOnHandler {
    app_context: Arc<AppContext>,
}

impl CommentOnHandler {
    pub fn new(app_context: Arc<AppContext>) -> Self {
        Self { app_context }
    }
}

impl TypedStatementHandler<CommentOnStatement> for CommentOnHandler {
    async fn execute(
        &self,
        statement: CommentOnStatement,
        _params: Vec<ScalarValue>,
        context: &ExecutionContext,
    ) -> Result<ExecutionResult, KalamDbError> {
        require_admin(context, "comment on")?;
        let app = Arc::clone(&self.app_context);
        run_blocking(move || persist_comment(&app.system_tables().catalog_stores(), statement))
            .await
    }
}

fn persist_comment(
    stores: &CatalogStores,
    statement: CommentOnStatement,
) -> Result<ExecutionResult, KalamDbError> {
    match statement.target {
        CommentOnTarget::Type(type_id) => persist_type_comment(
            stores,
            &type_id,
            statement.comment.as_deref(),
            statement.if_exists,
        ),
        CommentOnTarget::Procedure(routine_id) => persist_procedure_comment(
            stores,
            &routine_id,
            statement.comment.as_deref(),
            statement.if_exists,
        ),
    }
}

fn persist_type_comment(
    stores: &CatalogStores,
    type_id: &TypeId,
    comment: Option<&str>,
    if_exists: bool,
) -> Result<ExecutionResult, KalamDbError> {
    let Some(mut catalog_type) = lookup_type(stores, type_id)? else {
        if if_exists {
            return Ok(ExecutionResult::Success {
                message: format!("Type {type_id} does not exist, skipping comment"),
            });
        }
        return Err(KalamDbError::NotFound(format!("type {type_id} not found")));
    };
    catalog_type.comment = comment.map(str::to_string);
    upsert_type(stores, catalog_type)?;
    Ok(comment_message("Type", type_id.as_str(), comment))
}

fn persist_procedure_comment(
    stores: &CatalogStores,
    routine_id: &RoutineId,
    comment: Option<&str>,
    if_exists: bool,
) -> Result<ExecutionResult, KalamDbError> {
    let Some(mut routine) = stores
        .get_routine(routine_id)
        .map_err(|error| KalamDbError::ExecutionError(error.to_string()))?
    else {
        if if_exists {
            return Ok(ExecutionResult::Success {
                message: format!("Procedure {routine_id} does not exist, skipping comment"),
            });
        }
        return Err(KalamDbError::NotFound(format!("procedure {routine_id} not found")));
    };
    routine.comment = comment.map(str::to_string);
    stores
        .upsert_routine(routine)
        .map_err(|error| KalamDbError::ExecutionError(error.to_string()))?;
    Ok(comment_message("Procedure", routine_id.as_str(), comment))
}

fn lookup_type(
    stores: &CatalogStores,
    type_id: &TypeId,
) -> Result<Option<CatalogType>, KalamDbError> {
    if let Some(catalog_type) = stores
        .get_type(type_id)
        .map_err(|error| KalamDbError::ExecutionError(error.to_string()))?
    {
        return Ok(Some(catalog_type));
    }
    let folded = TypeId::new(type_id.as_str().to_ascii_lowercase());
    if folded.as_str() == type_id.as_str() {
        return Ok(None);
    }
    stores
        .get_type(&folded)
        .map_err(|error| KalamDbError::ExecutionError(error.to_string()))
}

fn upsert_type(stores: &CatalogStores, catalog_type: CatalogType) -> Result<(), KalamDbError> {
    stores
        .upsert_type(catalog_type)
        .map_err(|error| KalamDbError::ExecutionError(error.to_string()))
}

fn comment_message(kind: &str, id: &str, comment: Option<&str>) -> ExecutionResult {
    ExecutionResult::Success {
        message: match comment {
            Some(_) => format!("{kind} {id} comment set"),
            None => format!("{kind} {id} comment cleared"),
        },
    }
}
