//! Typed handler for CREATE PROCEDURE.

use std::sync::Arc;

use kalamdb_commons::{
    models::{ArtifactId, RoutineParameterId, UserId},
    FunctionRuntime,
};
use kalamdb_core::{
    app_context::AppContext,
    error::KalamDbError,
    sql::{
        context::{ExecutionContext, ExecutionResult, ScalarValue},
        executor::handlers::TypedStatementHandler,
    },
};
use kalamdb_functions::{
    hash_artifact_bytes, prepare_inline_javascript, FunctionActivation, ImplementationRef,
};
use kalamdb_sql::ddl::CreateProcedureStatement;
use kalamdb_system::{CatalogRoutine, CatalogRoutineParameter};

use crate::helpers::{
    async_blocking::run_blocking,
    guards::{require_admin, require_existing_namespace},
};

pub struct CreateProcedureHandler {
    app_context: Arc<AppContext>,
}

impl CreateProcedureHandler {
    pub fn new(app_context: Arc<AppContext>) -> Self {
        Self { app_context }
    }
}

impl TypedStatementHandler<CreateProcedureStatement> for CreateProcedureHandler {
    async fn execute(
        &self,
        statement: CreateProcedureStatement,
        _params: Vec<ScalarValue>,
        context: &ExecutionContext,
    ) -> Result<ExecutionResult, KalamDbError> {
        require_admin(context, "create procedure")?;
        require_existing_namespace(&self.app_context, &statement.namespace_id)?;
        {
            let stores = self.app_context.system_tables().catalog_stores();
            crate::catalog_type::require_procedure_types(&stores, &statement)?;
        }
        let app = Arc::clone(&self.app_context);
        let owner = context.user_id().clone();
        let mut routine = catalog_routine(&statement, owner);
        if let Some(body) = statement.body.as_deref() {
            routine.inline_source_hash =
                Some(hash_artifact_bytes(body.as_bytes()).as_str().to_string());
        }
        if should_compile_javascript(statement.language.as_deref()) {
            routine.inline_artifact_id = Some(compile_inline_javascript(&app, &statement).await?);
        }
        let parameters = catalog_parameters(&statement)?;
        let existing = {
            let stores = app.system_tables().catalog_stores();
            stores
                .get_routine(&statement.routine_id)
                .map_err(|error| KalamDbError::ExecutionError(error.to_string()))?
        };
        if existing.is_some() && !statement.or_replace {
            return Err(KalamDbError::AlreadyExists(format!(
                "procedure {} already exists",
                statement.routine_id
            )));
        }
        let replaced = existing.is_some();
        if routine.comment.is_none() {
            if let Some(previous) = existing.as_ref() {
                routine.comment.clone_from(&previous.comment);
            }
        }
        let source_unchanged = existing.as_ref().is_some_and(|previous| {
            previous.body == routine.body && previous.language == routine.language
        });

        let routine_id = statement.routine_id.clone();
        let app_for_catalog = Arc::clone(&app);
        let routine_for_catalog = routine.clone();
        let parameters_for_catalog = parameters.clone();
        run_blocking(move || {
            let stores = app_for_catalog.system_tables().catalog_stores();
            stores
                .upsert_routine(routine_for_catalog)
                .map_err(|error| KalamDbError::ExecutionError(error.to_string()))?;
            stores
                .replace_parameters(&routine_id, parameters_for_catalog)
                .map_err(|error| KalamDbError::ExecutionError(error.to_string()))?;
            Ok::<(), KalamDbError>(())
        })
        .await?;

        kalamdb_core::functions::rebuild_active_function_set(&app).await?;

        Ok(ExecutionResult::Success {
            message: procedure_ddl_message(&app, &routine, replaced, source_unchanged),
        })
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use kalamdb_commons::{
        models::{NamespaceId, UserId},
        Role,
    };
    use kalamdb_core::test_helpers::{create_test_session_for, test_app_context_simple};

    use super::*;

    #[test]
    fn bodyless_contract_does_not_compile_javascript() {
        assert!(!should_compile_javascript(None));
        assert!(!should_compile_javascript(Some("SQL")));
        assert!(!should_compile_javascript(Some("TYPESCRIPT")));
        assert!(should_compile_javascript(Some("JAVASCRIPT")));
    }

    #[test]
    fn ddl_message_distinguishes_create_and_replace() {
        let created = format_procedure_ddl_message(
            "api.health",
            false,
            false,
            Some("JAVASCRIPT"),
            Some("abc"),
            Some("def"),
            "inline javascript",
            None,
            1,
        );
        assert!(created.starts_with("Procedure api.health created\n"), "{created}");
        assert!(created.contains("inline javascript"), "{created}");
        assert!(!created.contains("module_revision"), "{created}");

        let replaced = format_procedure_ddl_message(
            "api.health",
            true,
            false,
            Some("JAVASCRIPT"),
            Some("abc"),
            Some("def"),
            "inline javascript",
            Some("backend:rev1"),
            4,
        );
        assert!(replaced.starts_with("Procedure api.health replaced\n"), "{replaced}");
        assert!(replaced.contains("source: changed"), "{replaced}");
        assert!(
            replaced.contains("module_revision: backend:rev1 (not created by this statement)"),
            "{replaced}"
        );
        assert!(replaced.contains("active_set_generation: 4"), "{replaced}");

        let unchanged = format_procedure_ddl_message(
            "api.health",
            true,
            true,
            Some("JAVASCRIPT"),
            Some("abc"),
            Some("def"),
            "inline javascript",
            None,
            2,
        );
        assert!(
            unchanged.starts_with("Procedure api.health replaced (source unchanged)\n"),
            "{unchanged}"
        );
    }

    #[tokio::test]
    async fn missing_namespace_returns_error() {
        let app_ctx = test_app_context_simple();
        let handler = CreateProcedureHandler::new(Arc::clone(&app_ctx));
        let ctx = ExecutionContext::new(
            UserId::new("test_user"),
            Role::Dba,
            create_test_session_for(&app_ctx),
        );
        let statement = CreateProcedureStatement::parse(
            "CREATE PROCEDURE missing_ns.echo(msg TEXT) LANGUAGE JAVASCRIPT AS $$ return input; $$",
            &NamespaceId::default(),
        )
        .expect("parse create procedure");
        let error = handler.execute(statement, vec![], &ctx).await.unwrap_err();
        let message = error.to_string();
        assert!(message.contains("does not exist"), "{message}");
        assert!(message.contains("CREATE NAMESPACE missing_ns"), "{message}");
    }

    #[tokio::test]
    async fn unknown_parameter_type_is_rejected() {
        let app_ctx = test_app_context_simple();
        let namespaces = app_ctx.system_tables().namespaces();
        let ns = NamespaceId::new("app");
        if namespaces.get_namespace(&ns).unwrap().is_none() {
            namespaces.create_namespace(kalamdb_system::Namespace::new("app")).unwrap();
        }
        let handler = CreateProcedureHandler::new(Arc::clone(&app_ctx));
        let ctx = ExecutionContext::new(
            UserId::new("test_user"),
            Role::Dba,
            create_test_session_for(&app_ctx),
        );
        let statement = CreateProcedureStatement::parse(
            "CREATE PROCEDURE app.echo_status(s app.missing_status) LANGUAGE JAVASCRIPT AS $$ \
             return input; $$",
            &ns,
        )
        .expect("parse create procedure");
        let error = handler.execute(statement, vec![], &ctx).await.unwrap_err();
        let message = error.to_string();
        assert!(message.contains("app.missing_status"), "{message}");
        assert!(message.contains("not found"), "{message}");
    }

    #[tokio::test]
    async fn topic_payload_type_can_be_used_as_return_type() {
        let app_ctx = test_app_context_simple();
        let namespaces = app_ctx.system_tables().namespaces();
        let ns = NamespaceId::new("app");
        if namespaces.get_namespace(&ns).unwrap().is_none() {
            namespaces.create_namespace(kalamdb_system::Namespace::new("app")).unwrap();
        }
        app_ctx
            .system_tables()
            .catalog_stores()
            .ensure_implicit_topic_payload_type(&kalamdb_commons::models::TopicId::new("app.inbox"))
            .unwrap();
        let handler = CreateProcedureHandler::new(Arc::clone(&app_ctx));
        let ctx = ExecutionContext::new(
            UserId::new("test_user"),
            Role::Dba,
            create_test_session_for(&app_ctx),
        );
        let statement = CreateProcedureStatement::parse(
            "CREATE PROCEDURE app.send() RETURNS app.inbox SECURITY INVOKER",
            &ns,
        )
        .expect("parse create procedure");
        handler.execute(statement, vec![], &ctx).await.expect("create procedure");
    }
}

fn catalog_routine(statement: &CreateProcedureStatement, owner: UserId) -> CatalogRoutine {
    CatalogRoutine {
        routine_id: statement.routine_id.clone(),
        namespace_id: statement.namespace_id.clone(),
        name: statement.name.clone(),
        owner,
        security: statement.security,
        language: statement.language.clone(),
        body: statement.body.clone(),
        return_type_id: statement
            .return_type
            .as_ref()
            .and_then(|ty| ty.resolved_type_id(&statement.namespace_id)),
        return_type_name: statement
            .return_type
            .as_ref()
            .map(|ty| ty.resolved_type_name(&statement.namespace_id)),
        return_is_array: statement.return_type.as_ref().is_some_and(|ty| ty.is_array),
        return_not_null: statement.return_type.as_ref().is_some_and(|ty| ty.not_null),
        comment: statement.comment.clone(),
        return_data_type: statement.return_type.as_ref().and_then(|ty| ty.builtin_data_type()),
        inline_source_hash: None,
        inline_artifact_id: None,
    }
}

fn catalog_parameters(
    statement: &CreateProcedureStatement,
) -> Result<Vec<CatalogRoutineParameter>, KalamDbError> {
    let mut parameters = Vec::with_capacity(statement.parameters.len());
    for (index, parameter) in statement.parameters.iter().enumerate() {
        let ordinal = (index + 1) as i32;
        parameters.push(CatalogRoutineParameter {
            parameter_id: RoutineParameterId::new(&statement.routine_id, ordinal)
                .map_err(|error| KalamDbError::InvalidSql(error))?,
            routine_id: statement.routine_id.clone(),
            name: parameter.name.clone(),
            ordinal,
            type_id: parameter.type_ref.resolved_type_id(&statement.namespace_id),
            type_name: parameter.type_ref.resolved_type_name(&statement.namespace_id),
            is_array: parameter.type_ref.is_array,
            not_null: parameter.type_ref.not_null,
            nonempty: parameter.type_ref.nonempty,
            data_type: parameter.type_ref.builtin_data_type(),
        });
    }
    Ok(parameters)
}

fn should_compile_javascript(language: Option<&str>) -> bool {
    matches!(
        language.map(|value| value.to_ascii_uppercase()).as_deref(),
        Some("JAVASCRIPT" | "JS")
    )
}

fn procedure_ddl_message(
    app: &AppContext,
    routine: &CatalogRoutine,
    replaced: bool,
    source_unchanged: bool,
) -> String {
    let set = app.function_runtime().active_set();
    let implementation = match set.lookup(&routine.routine_id) {
        ImplementationRef::Inline { artifact } => {
            format!("inline javascript artifact={}", artifact.artifact_id)
        },
        ImplementationRef::Module { revision, .. } => {
            format!(
                "module override revision={} (inline body stored, not executed)",
                revision.revision_id
            )
        },
        ImplementationRef::Missing => "not implemented".to_string(),
    };
    let module_revision =
        set.module_revision.as_ref().map(|revision| revision.revision_id.to_string());
    format_procedure_ddl_message(
        routine.routine_id.as_str(),
        replaced,
        source_unchanged,
        routine.language.as_deref(),
        routine.inline_source_hash.as_deref(),
        routine.inline_artifact_id.as_ref().map(|id| id.as_str()),
        &implementation,
        module_revision.as_deref(),
        set.generation,
    )
}

fn format_procedure_ddl_message(
    routine_id: &str,
    replaced: bool,
    source_unchanged: bool,
    language: Option<&str>,
    source_hash: Option<&str>,
    inline_artifact: Option<&str>,
    implementation: &str,
    module_revision: Option<&str>,
    generation: u64,
) -> String {
    let mut lines = Vec::with_capacity(8);
    if replaced && source_unchanged {
        lines.push(format!("Procedure {routine_id} replaced (source unchanged)"));
    } else if replaced {
        lines.push(format!("Procedure {routine_id} replaced"));
    } else {
        lines.push(format!("Procedure {routine_id} created"));
    }
    if let Some(language) = language {
        lines.push(format!("language: {language}"));
    }
    lines.push(format!("implementation: {implementation}"));
    if let Some(source_hash) = source_hash {
        lines.push(format!("source_hash: {source_hash}"));
    }
    if let Some(inline_artifact) = inline_artifact {
        lines.push(format!("inline_artifact: {inline_artifact}"));
    }
    if replaced {
        lines.push(format!(
            "source: {}",
            if source_unchanged {
                "unchanged"
            } else {
                "changed"
            }
        ));
    }
    if let Some(module_revision) = module_revision {
        lines.push(format!("module_revision: {module_revision} (not created by this statement)"));
    }
    lines.push(format!("active_set_generation: {generation}"));
    lines.join("\n")
}

async fn compile_inline_javascript(
    app: &Arc<AppContext>,
    statement: &CreateProcedureStatement,
) -> Result<ArtifactId, KalamDbError> {
    let body = statement.body.as_deref().ok_or_else(|| {
        KalamDbError::InvalidSql(format!(
            "javascript procedure {} requires an AS $$ ... $$ body",
            statement.routine_id
        ))
    })?;
    let source = {
        let body = body.to_string();
        run_blocking(move || {
            prepare_inline_javascript(&body)
                .map_err(|error| KalamDbError::InvalidSql(error.to_string()))
        })
        .await?
    };
    let storage = kalamdb_core::functions::function_storage(app)?;
    let stores = app.system_tables().catalog_stores();
    let activation = FunctionActivation::new(stores);
    let artifact = activation
        .upload(storage.as_ref(), source.as_bytes(), FunctionRuntime::Typescript)
        .await
        .map_err(|error| KalamDbError::ExecutionError(error.to_string()))?;
    Ok(artifact.artifact_id)
}
