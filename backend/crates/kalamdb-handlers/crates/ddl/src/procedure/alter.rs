//! Typed handler for ALTER FUNCTION / ALTER PROCEDURE.

use std::sync::Arc;

use kalamdb_commons::models::{RoutineGrantId, RoutineId, RoutineParameterId};
use kalamdb_core::{
    app_context::AppContext,
    error::KalamDbError,
    sql::{
        context::{ExecutionContext, ExecutionResult, ScalarValue},
        executor::handlers::TypedStatementHandler,
    },
};
use kalamdb_sql::ddl::{AlterRoutineAction, AlterRoutineStatement};
use kalamdb_system::CatalogStores;

use crate::helpers::{
    async_blocking::run_blocking,
    guards::{require_admin, require_existing_namespace},
};

pub struct AlterRoutineHandler {
    app_context: Arc<AppContext>,
}

impl AlterRoutineHandler {
    pub fn new(app_context: Arc<AppContext>) -> Self {
        Self { app_context }
    }
}

impl TypedStatementHandler<AlterRoutineStatement> for AlterRoutineHandler {
    async fn execute(
        &self,
        statement: AlterRoutineStatement,
        _params: Vec<ScalarValue>,
        context: &ExecutionContext,
    ) -> Result<ExecutionResult, KalamDbError> {
        require_admin(context, "alter routine")?;
        if let AlterRoutineAction::SetSchema { new_namespace_id } = &statement.action {
            require_existing_namespace(&self.app_context, new_namespace_id)?;
        }
        let app = Arc::clone(&self.app_context);
        let result = run_blocking(move || {
            let stores = app.system_tables().catalog_stores();
            apply_alter_routine(&stores, &statement)
        })
        .await?;

        kalamdb_core::functions::rebuild_active_function_set(&self.app_context).await?;

        Ok(result)
    }
}

fn apply_alter_routine(
    stores: &CatalogStores,
    statement: &AlterRoutineStatement,
) -> Result<ExecutionResult, KalamDbError> {
    let label = statement.expected_kind.as_str();
    let Some(mut routine) = stores
        .get_routine(&statement.routine_id)
        .map_err(|error| KalamDbError::ExecutionError(error.to_string()))?
    else {
        return Err(KalamDbError::NotFound(format!(
            "{} {} not found",
            label.to_ascii_lowercase(),
            statement.routine_id
        )));
    };
    if routine.namespace_id.is_system_namespace() || routine.language.as_deref() == Some("INTERNAL")
    {
        return Err(KalamDbError::InvalidSql(format!(
            "cannot alter built-in system routine {}",
            routine.routine_id
        )));
    }
    if routine.kind != statement.expected_kind {
        return Err(KalamDbError::InvalidSql(format!(
            "{} is not a {}",
            statement.routine_id,
            label.to_ascii_lowercase()
        )));
    }

    let old_id = routine.routine_id.clone();
    let mut new_namespace = routine.namespace_id.clone();
    let mut new_name = routine.name.clone();

    match &statement.action {
        AlterRoutineAction::RenameTo {
            new_name: target_name,
        } => {
            new_name.clone_from(target_name);
        },
        AlterRoutineAction::SetSchema { new_namespace_id } => {
            new_namespace.clone_from(new_namespace_id);
        },
        AlterRoutineAction::OwnerTo { new_owner } => {
            routine.owner = new_owner.clone();
        },
        AlterRoutineAction::Options {
            security,
            volatility,
            null_safety,
            parallel_safety,
            cost,
            rows,
            config_changes,
        } => {
            if let Some(security) = security {
                routine.security = *security;
            }
            if let Some(volatility) = volatility {
                routine.volatility = *volatility;
            }
            if let Some(null_safety) = null_safety {
                routine.null_safety = *null_safety;
            }
            if let Some(parallel_safety) = parallel_safety {
                routine.parallel_safety = *parallel_safety;
            }
            if let Some(cost) = cost {
                routine.cost = Some(*cost);
            }
            if let Some(rows) = rows {
                routine.rows = Some(*rows);
            }
            if !config_changes.is_empty() {
                routine.config_params = kalamdb_sql::ddl::create_procedure::encode_config_changes(
                    config_changes,
                    routine.config_params.as_deref(),
                );
            }
        },
        AlterRoutineAction::DependsOnExtension { .. } => {},
    }

    let new_id = RoutineId::from_parts(Some(&new_namespace), &new_name);
    if new_id != old_id {
        if stores
            .get_routine(&new_id)
            .map_err(|error| KalamDbError::ExecutionError(error.to_string()))?
            .is_some()
        {
            return Err(KalamDbError::AlreadyExists(format!(
                "{} {} already exists",
                label.to_ascii_lowercase(),
                new_id
            )));
        }
        let mut params = stores
            .list_parameters(&old_id)
            .map_err(|error| KalamDbError::ExecutionError(error.to_string()))?;
        for param in &mut params {
            param.routine_id = new_id.clone();
            param.parameter_id = RoutineParameterId::new(&new_id, param.ordinal)
                .map_err(KalamDbError::InvalidSql)?;
        }
        let mut grants = stores
            .list_grants(&old_id)
            .map_err(|error| KalamDbError::ExecutionError(error.to_string()))?;
        for grant in &mut grants {
            grant.routine_id = new_id.clone();
            grant.grant_id = RoutineGrantId::new(&new_id, &grant.grantee);
        }
        stores
            .drop_routine(&old_id)
            .map_err(|error| KalamDbError::ExecutionError(error.to_string()))?;
        routine.routine_id = new_id.clone();
        routine.namespace_id = new_namespace;
        routine.name = new_name;
        stores
            .upsert_routine(routine)
            .map_err(|error| KalamDbError::ExecutionError(error.to_string()))?;
        stores
            .replace_parameters(&new_id, params)
            .map_err(|error| KalamDbError::ExecutionError(error.to_string()))?;
        for grant in grants {
            stores
                .upsert_grant(grant)
                .map_err(|error| KalamDbError::ExecutionError(error.to_string()))?;
        }
    } else {
        stores
            .upsert_routine(routine)
            .map_err(|error| KalamDbError::ExecutionError(error.to_string()))?;
    }

    Ok(ExecutionResult::Success {
        message: format!("{label} {} altered", new_id),
    })
}
