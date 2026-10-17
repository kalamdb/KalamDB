//! GRANT / REVOKE EXECUTE handlers.

use std::sync::Arc;

use kalamdb_commons::models::{RoutineGrantId, RoutineGrantee};
use kalamdb_core::{
    app_context::AppContext,
    error::KalamDbError,
    sql::{
        context::{ExecutionContext, ExecutionResult, ScalarValue},
        executor::handlers::TypedStatementHandler,
    },
};
use kalamdb_sql::ddl::{ExecuteGrantee, ExecuteTarget, GrantExecuteStatement, RevokeExecuteStatement};
use kalamdb_system::CatalogRoutineGrant;

use crate::helpers::{async_blocking::run_blocking, guards::require_admin};

pub struct GrantExecuteHandler {
    app_context: Arc<AppContext>,
}

impl GrantExecuteHandler {
    pub fn new(app_context: Arc<AppContext>) -> Self {
        Self { app_context }
    }
}

impl TypedStatementHandler<GrantExecuteStatement> for GrantExecuteHandler {
    async fn execute(
        &self,
        statement: GrantExecuteStatement,
        _params: Vec<ScalarValue>,
        context: &ExecutionContext,
    ) -> Result<ExecutionResult, KalamDbError> {
        require_admin(context, "grant execute")?;
        let app = Arc::clone(&self.app_context);
        run_blocking(move || {
            let stores = app.system_tables().catalog_stores();
            let grantee = map_grantee(&statement.grantee);
            match &statement.target {
                ExecuteTarget::Routine {
                    routine_id,
                    expected_kind,
                } => {
                    let Some(existing) = stores
                        .get_routine(routine_id)
                        .map_err(|error| KalamDbError::ExecutionError(error.to_string()))?
                    else {
                        return Err(KalamDbError::NotFound(format!(
                            "{} {} not found",
                            expected_kind.as_str().to_ascii_lowercase(),
                            routine_id
                        )));
                    };
                    if existing.kind != *expected_kind {
                        return Err(KalamDbError::InvalidSql(format!(
                            "{} is not a {}",
                            routine_id,
                            expected_kind.as_str().to_ascii_lowercase()
                        )));
                    }
                    let grant = CatalogRoutineGrant {
                        grant_id: RoutineGrantId::new(routine_id, &grantee),
                        routine_id: routine_id.clone(),
                        grantee,
                    };
                    stores
                        .upsert_grant(grant)
                        .map_err(|error| KalamDbError::ExecutionError(error.to_string()))?;
                    Ok::<ExecutionResult, KalamDbError>(ExecutionResult::Success {
                        message: format!(
                            "Granted EXECUTE on {} to {}",
                            routine_id,
                            statement.grantee.as_sql()
                        ),
                    })
                },
                ExecuteTarget::AllInSchema {
                    namespace_id,
                    expected_kind,
                } => {
                    let routines = stores
                        .list_routines()
                        .map_err(|error| KalamDbError::ExecutionError(error.to_string()))?;
                    for routine in routines {
                        if routine.namespace_id == *namespace_id
                            && routine.kind == *expected_kind
                            && routine.language.as_deref() != Some("INTERNAL")
                        {
                            let grant = CatalogRoutineGrant {
                                grant_id: RoutineGrantId::new(&routine.routine_id, &grantee),
                                routine_id: routine.routine_id.clone(),
                                grantee: grantee.clone(),
                            };
                            stores
                                .upsert_grant(grant)
                                .map_err(|error| KalamDbError::ExecutionError(error.to_string()))?;
                        }
                    }
                    Ok::<ExecutionResult, KalamDbError>(ExecutionResult::Success {
                        message: format!(
                            "Granted EXECUTE on all {}s in schema {} to {}",
                            expected_kind.as_str().to_ascii_lowercase(),
                            namespace_id,
                            statement.grantee.as_sql()
                        ),
                    })
                },
            }
        })
        .await
    }
}

pub struct RevokeExecuteHandler {
    app_context: Arc<AppContext>,
}

impl RevokeExecuteHandler {
    pub fn new(app_context: Arc<AppContext>) -> Self {
        Self { app_context }
    }
}

impl TypedStatementHandler<RevokeExecuteStatement> for RevokeExecuteHandler {
    async fn execute(
        &self,
        statement: RevokeExecuteStatement,
        _params: Vec<ScalarValue>,
        context: &ExecutionContext,
    ) -> Result<ExecutionResult, KalamDbError> {
        require_admin(context, "revoke execute")?;
        let app = Arc::clone(&self.app_context);
        run_blocking(move || {
            let stores = app.system_tables().catalog_stores();
            let grantee = map_grantee(&statement.grantee);
            match &statement.target {
                ExecuteTarget::Routine {
                    routine_id,
                    expected_kind,
                } => {
                    if let Some(existing) = stores
                        .get_routine(routine_id)
                        .map_err(|error| KalamDbError::ExecutionError(error.to_string()))?
                    {
                        if existing.kind != *expected_kind {
                            return Err(KalamDbError::InvalidSql(format!(
                                "{} is not a {}",
                                routine_id,
                                expected_kind.as_str().to_ascii_lowercase()
                            )));
                        }
                    }
                    let grant_id = RoutineGrantId::new(routine_id, &grantee);
                    stores
                        .delete_grant(&grant_id)
                        .map_err(|error| KalamDbError::ExecutionError(error.to_string()))?;
                    Ok::<ExecutionResult, KalamDbError>(ExecutionResult::Success {
                        message: format!(
                            "Revoked EXECUTE on {} from {}",
                            routine_id,
                            statement.grantee.as_sql()
                        ),
                    })
                },
                ExecuteTarget::AllInSchema {
                    namespace_id,
                    expected_kind,
                } => {
                    let routines = stores
                        .list_routines()
                        .map_err(|error| KalamDbError::ExecutionError(error.to_string()))?;
                    for routine in routines {
                        if routine.namespace_id == *namespace_id
                            && routine.kind == *expected_kind
                            && routine.language.as_deref() != Some("INTERNAL")
                        {
                            let grant_id = RoutineGrantId::new(&routine.routine_id, &grantee);
                            stores
                                .delete_grant(&grant_id)
                                .map_err(|error| KalamDbError::ExecutionError(error.to_string()))?;
                        }
                    }
                    Ok::<ExecutionResult, KalamDbError>(ExecutionResult::Success {
                        message: format!(
                            "Revoked EXECUTE on all {}s in schema {} from {}",
                            expected_kind.as_str().to_ascii_lowercase(),
                            namespace_id,
                            statement.grantee.as_sql()
                        ),
                    })
                },
            }
        })
        .await
    }
}

fn map_grantee(grantee: &ExecuteGrantee) -> RoutineGrantee {
    match grantee {
        ExecuteGrantee::Public => RoutineGrantee::Public,
        ExecuteGrantee::User => RoutineGrantee::User,
        ExecuteGrantee::Service => RoutineGrantee::Service,
        ExecuteGrantee::Anonymous => RoutineGrantee::Anonymous,
        ExecuteGrantee::Role(name) => RoutineGrantee::Role(name.clone()),
    }
}
