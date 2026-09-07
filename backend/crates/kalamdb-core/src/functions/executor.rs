//! Root and nested procedure invocation.

use std::sync::Arc;

use arrow::{
    array::RecordBatch,
    datatypes::{Field, Schema},
};
use datafusion::scalar::ScalarValue;
use kalamdb_commons::{
    conversions::arrow_json_conversion::{scalar_value_to_js_json, scalar_value_to_json},
    models::{RoutineCall, RoutineId, TopicId, TransactionId},
    FunctionModuleId, FunctionRevisionId, FunctionRuntime, Role, UserId,
};
use kalamdb_filestore::StorageCached;
use kalamdb_functions::{
    now_ms, ActiveFunctionSet, FunctionActivation, FunctionCallOrigin, FunctionCallResult,
    FunctionExecutionRoot, FunctionsError, ImplementationRef, InlineArtifact, Invocation,
    InvocationScope, ModuleRevision, ProcedureFrame, RoutineValue, StagedTopicPublish, ABI_VERSION,
};
use kalamdb_sql::ddl::CallStatement;
use kalamdb_system::{ActivateFunctionOutcome, CatalogRoutine};
use kalamdb_transactions::RequestTransactionState;
use kalamdb_views::{
    active_function_runs::ActiveFunctionRunSnapshot, function_errors::FunctionErrorSnapshot,
};
use parking_lot::Mutex;
use smallvec::SmallVec;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use super::{
    acl,
    convert::bind_call_arguments,
    host::{frame_principal, push_frame, CoreFunctionHost, HostSession},
};
use crate::{
    app_context::AppContext,
    error::KalamDbError,
    sql::{
        context::{ExecutionContext, ExecutionResult},
        executor::request_transaction_state::{
            map_request_transaction_error, AppContextRequestTransactionCoordinator,
        },
    },
};

pub struct FunctionService;

impl FunctionService {
    pub async fn execute_call(
        app: Arc<AppContext>,
        exec_ctx: &ExecutionContext,
        statement: &CallStatement,
        params: &[ScalarValue],
    ) -> Result<ExecutionResult, KalamDbError> {
        let result = Self::invoke_sql_call(app, exec_ctx, &statement.call, params).await?;
        routine_value_to_execution_result(&result.value)
    }

    pub async fn execute_routine_call(
        app: Arc<AppContext>,
        exec_ctx: &ExecutionContext,
        call: &RoutineCall,
    ) -> Result<ScalarValue, KalamDbError> {
        if call.has_placeholder() {
            return Err(KalamDbError::InvalidSql(
                "DEFAULT procedure arguments cannot use placeholders".to_string(),
            ));
        }
        let result = Self::invoke_sql_call(app, exec_ctx, call, &[]).await?;
        Ok(result.value.value)
    }

    async fn invoke_sql_call(
        app: Arc<AppContext>,
        exec_ctx: &ExecutionContext,
        call: &RoutineCall,
        params: &[ScalarValue],
    ) -> Result<FunctionCallResult, KalamDbError> {
        let args = bind_call_arguments(&call.arguments, params)?;
        Self::invoke(app, exec_ctx, FunctionCallOrigin::Sql, call.routine_id.clone(), args).await
    }

    pub async fn invoke(
        app: Arc<AppContext>,
        exec_ctx: &ExecutionContext,
        origin: FunctionCallOrigin,
        routine_id: RoutineId,
        args: Vec<RoutineValue>,
    ) -> Result<FunctionCallResult, KalamDbError> {
        let request_id = exec_ctx
            .request_id()
            .map(|id| id.to_string())
            .unwrap_or_else(|| Uuid::now_v7().to_string());
        let exec_ctx = exec_ctx.clone().with_request_id(request_id.clone());

        let coordinator = AppContextRequestTransactionCoordinator::new(app.as_ref());
        let mut request_state = RequestTransactionState::from_request_id(Some(request_id.as_str()))
            .expect("request id is present");
        request_state.sync(&coordinator);
        let owned_tx = !request_state.is_active();
        if owned_tx {
            request_state.begin(&coordinator).map_err(map_request_transaction_error)?;
        }

        let actor = exec_ctx.user_id().clone();
        let origin_kind = match &origin {
            FunctionCallOrigin::Sql => "sql",
            FunctionCallOrigin::Http { .. } => "http",
            FunctionCallOrigin::Topic { .. } => "topic",
        };
        kalamdb_observability::begin_function_run();
        let started = std::time::Instant::now();
        let invoke_result =
            invoke_root(Arc::clone(&app), exec_ctx, origin, routine_id.clone(), args).await;
        match &invoke_result {
            Err(error) => {
                if error.function_error_code()
                    == Some(kalamdb_functions::FunctionErrorCode::ProcedureTimeout)
                {
                    kalamdb_observability::record_function_timeout();
                }
                log::error!(
                    target: "kalamdb::functions",
                    "execution_id={request_id} request_id={request_id} routine={routine_id} actor={actor} principal={actor} source={origin_kind} code={:?} {error}",
                    error.function_error_code()
                );
                app.function_runtime().record_error(FunctionErrorSnapshot {
                    execution_id: request_id.clone(),
                    request_id:   request_id.clone(),
                    routine_id:   routine_id.to_string(),
                    actor:        actor.to_string(),
                    origin:       origin_kind.to_string(),
                    code:         error
                        .function_error_code()
                        .map(|code| code.as_str().to_string())
                        .unwrap_or_else(|| "INTERNAL_RUNTIME_ERROR".into()),
                    message:      sanitize_function_error(&error.to_string()),
                    recorded_at:  now_ms(),
                });
            },
            Ok(_) => {},
        }
        kalamdb_observability::finish_function_run(started.elapsed(), invoke_result.is_err());
        log::debug!(
            target: "kalamdb::functions",
            "routine={routine_id} origin={origin_kind} duration_ms={} ok={}",
            started.elapsed().as_millis(),
            invoke_result.is_ok()
        );

        match invoke_result {
            Ok(result) => {
                if owned_tx {
                    request_state
                        .commit(&coordinator)
                        .await
                        .map_err(map_request_transaction_error)?;
                }
                Ok(result)
            },
            Err(error) => {
                if owned_tx {
                    let _ = request_state.rollback(&coordinator);
                }
                Err(error)
            },
        }
    }
}

async fn invoke_root(
    app: Arc<AppContext>,
    exec_ctx: ExecutionContext,
    origin: FunctionCallOrigin,
    routine_id: RoutineId,
    args: Vec<RoutineValue>,
) -> Result<FunctionCallResult, KalamDbError> {
    if app.function_runtime().active_set().generation == 0 {
        rebuild_active_function_set(&app).await?;
    }
    let deployment = app.function_runtime().active_set();
    let execution_id = exec_ctx
        .request_id()
        .map(|id| id.to_string())
        .unwrap_or_else(|| Uuid::now_v7().to_string());
    let revision_id = deployment
        .module_revision
        .as_ref()
        .map(|revision| revision.revision_id.to_string());
    let runtime = app.function_runtime();
    runtime.begin_run(ActiveFunctionRunSnapshot {
        execution_id: execution_id.clone(),
        request_id: execution_id.clone(),
        routine_id: routine_id.to_string(),
        revision_id,
        actor: exec_ctx.user_id().to_string(),
        principal: exec_ctx.user_id().to_string(),
        origin: match &origin {
            FunctionCallOrigin::Sql => "sql".into(),
            FunctionCallOrigin::Http { .. } => "http".into(),
            FunctionCallOrigin::Topic { .. } => "topic".into(),
        },
        started_at: now_ms(),
        depth: 0,
    });
    let _run_guard = ActiveRunGuard {
        runtime,
        execution_id,
    };
    let root = FunctionExecutionRoot {
        actor_user: exec_ctx.user_id().clone(),
        actor_role: exec_ctx.user_role(),
        origin:     origin.clone(),
        deployment: Arc::clone(&deployment),
    };
    let host = CoreFunctionHost {
        app:      Arc::clone(&app),
        handle:   tokio::runtime::Handle::current(),
        session:  Arc::new(HostSession {
            exec_ctx,
            stack: SmallVec::new(),
            root,
            principal_ctx: Arc::new(Mutex::new(SmallVec::new())),
        }),
        origin:   origin.clone(),
        scope:    InvocationScope {
            deadline: std::time::Instant::now()
                + app.function_runtime().engine().map_err(map_functions)?.config().timeout,
            cancel:   CancellationToken::new(),
            depth:    0,
        },
        sql_gate: Arc::new(tokio::sync::Mutex::new(())),
    };
    let value = invoke_on_host(&host, routine_id, &args).await?;

    let (http_status, http_headers) = match origin {
        FunctionCallOrigin::Http { response, .. } => {
            let overrides = response.lock().clone();
            (overrides.status, overrides.headers)
        },
        FunctionCallOrigin::Sql | FunctionCallOrigin::Topic { .. } => {
            (None, std::collections::HashMap::new())
        },
    };
    Ok(FunctionCallResult {
        value,
        http_status,
        http_headers,
    })
}

pub(super) async fn invoke_nested(
    host: &CoreFunctionHost,
    routine_id: RoutineId,
    args: &[RoutineValue],
) -> Result<RoutineValue, KalamDbError> {
    let mut child = host.clone();
    child.scope = host
        .scope
        .child(host.app.function_runtime().engine().map_err(map_functions)?.config().max_depth)
        .map_err(map_functions)?;
    invoke_on_host(&child, routine_id, args).await
}

async fn invoke_on_host(
    host: &CoreFunctionHost,
    routine_id: RoutineId,
    args: &[RoutineValue],
) -> Result<RoutineValue, KalamDbError> {
    let (invocation, child) = prepare_call(host, routine_id, args)?;
    let engine = host.app.function_runtime().engine().map_err(map_functions)?;
    engine.invoke(invocation, child).await.map_err(map_functions)
}

pub(super) fn prepare_call(
    host: &CoreFunctionHost,
    routine_id: RoutineId,
    args: &[RoutineValue],
) -> Result<(Invocation, Arc<dyn kalamdb_functions::FunctionHost>), KalamDbError> {
    let stores = host.app.system_tables().catalog_stores();
    let routine = stores.get_routine(&routine_id).map_err(|error| {
        KalamDbError::ExecutionError(format!("failed to load procedure {routine_id}: {error}"))
    })?;
    let Some(routine) = routine else {
        return Err(KalamDbError::NotFound(format!("procedure {routine_id} not found")));
    };

    let (caller_user, caller_role) = {
        let session = host.session.as_ref();
        match session.stack.last() {
            Some(frame) => (frame.principal_user.clone(), frame.principal_role),
            None => (session.exec_ctx.user_id().clone(), session.exec_ctx.user_role()),
        }
    };
    acl::require_execute(&stores, &routine, &caller_user, caller_role)?;

    let owner_role = owner_role(&host.app, &routine.owner)?;
    let (principal_user, principal_role) = frame_principal(
        routine.security,
        caller_user,
        caller_role,
        routine.owner.clone(),
        owner_role,
    );

    let revision = revision_for_routine(host, &routine)?;
    let args = attach_transfer(args, &revision.contract_hash);
    let frame = ProcedureFrame {
        routine_id: routine.routine_id.clone(),
        revision_id: revision.revision_id.clone(),
        namespace_id: routine.namespace_id.clone(),
        principal_user,
        principal_role,
        security: routine.security,
    };

    let mut child = host.clone();
    child.session = Arc::new(push_frame(host.session.as_ref(), frame)?);
    Ok((
        Invocation {
            routine_id,
            revision,
            args,
            scope: host.scope.clone(),
            return_template: None,
        },
        Arc::new(child),
    ))
}

fn revision_for_routine(
    host: &CoreFunctionHost,
    routine: &CatalogRoutine,
) -> Result<Arc<ModuleRevision>, KalamDbError> {
    let language = routine.language.as_deref().unwrap_or("");
    if is_sql_language(language) {
        return Err(KalamDbError::InvalidOperation(format!(
            "CALL of LANGUAGE SQL procedure {} is not supported",
            routine.routine_id
        )));
    }
    if is_uncompiled_inline_typescript(routine) {
        return Err(KalamDbError::from(FunctionsError::Invalid(format!(
            "inline TypeScript procedure {} is stored but not executed until compiled JavaScript \
             exists; run `kalam functions build` or use LANGUAGE JAVASCRIPT",
            routine.routine_id
        ))));
    }
    match host.session.root.deployment.lookup(&routine.routine_id) {
        ImplementationRef::Module { revision, .. } => Ok(revision),
        ImplementationRef::Inline { artifact } => Ok(inline_revision(&artifact)),
        ImplementationRef::Missing => Err(KalamDbError::from(FunctionsError::NotImplemented(
            routine.routine_id.to_string(),
        ))),
    }
}

fn inline_revision(artifact: &InlineArtifact) -> Arc<ModuleRevision> {
    let module_id = FunctionModuleId::new("inline");
    Arc::new(ModuleRevision {
        revision_id: FunctionRevisionId::from_module_artifact(&module_id, &artifact.artifact_id),
        module_id,
        artifact_id: artifact.artifact_id.clone(),
        runtime: FunctionRuntime::Typescript,
        abi_version: ABI_VERSION,
        contract_hash: String::new(),
        source: Arc::clone(&artifact.source),
    })
}

pub async fn rebuild_active_function_set(app: &AppContext) -> Result<(), KalamDbError> {
    let stores = app.system_tables().catalog_stores();
    let module_id = FunctionModuleId::new(app.config().functions.module.as_str());
    let activation = FunctionActivation::new(stores.clone());
    let module_revision = match activation.active_module(&module_id).map_err(map_functions)? {
        Some(module) if module.active_revision_id.is_some() => {
            let storage = function_storage(app)?;
            let revision_id = module.active_revision_id.expect("checked");
            Some(
                app.function_runtime()
                    .engine()
                    .map_err(map_functions)?
                    .load_revision(
                        revision_id.clone(),
                        activation.load_revision(&storage, &revision_id),
                    )
                    .await
                    .map_err(map_functions)?,
            )
        },
        _ => None,
    };

    let routines = stores
        .list_routines()
        .map_err(|error| KalamDbError::ExecutionError(error.to_string()))?;
    let mut rows = Vec::with_capacity(routines.len());
    for routine in routines {
        let inline = if let Some(artifact_id) = routine.inline_artifact_id.as_ref() {
            let storage = function_storage(app)?;
            let bytes = storage
                .get_function_artifact(artifact_id)
                .await
                .map_err(|error| KalamDbError::ExecutionError(error.to_string()))?;
            let source = String::from_utf8(bytes.data.to_vec()).map_err(|error| {
                KalamDbError::ExecutionError(format!("inline artifact is not utf8: {error}"))
            })?;
            Some(Arc::new(InlineArtifact {
                artifact_id: artifact_id.clone(),
                source:      source.into(),
                language:    routine.language.clone().unwrap_or_default(),
            }))
        } else {
            None
        };
        rows.push((routine, inline));
    }

    let previous = app.function_runtime().active_set();
    let set = ActiveFunctionSet::resolve(
        previous.generation.saturating_add(1).max(1),
        module_revision
            .as_ref()
            .map(|revision| revision.contract_hash.clone())
            .unwrap_or_default(),
        module_revision,
        &std::collections::HashSet::new(),
        rows,
    );
    app.function_runtime().publish_active_set(set);
    Ok(())
}

pub async fn activate_module_artifact(
    app: &AppContext,
    module_id: FunctionModuleId,
    bytes: &[u8],
    contract_hash: &str,
    abi_version: u32,
    exports: &[String],
) -> Result<ActivateFunctionOutcome, KalamDbError> {
    let max_bytes = app.config().functions.runtime.max_artifact_bytes;
    if bytes.len() > max_bytes {
        return Err(KalamDbError::InvalidOperation(format!(
            "function artifact exceeds {max_bytes} bytes"
        )));
    }
    let stores = app.system_tables().catalog_stores();
    let catalog_exports: std::collections::HashSet<String> = stores
        .list_routines()
        .map_err(|error| KalamDbError::ExecutionError(error.to_string()))?
        .into_iter()
        .filter(|routine| routine.body.is_none())
        .map(|routine| routine.routine_id.to_string())
        .collect();
    let requested: std::collections::HashSet<String> = exports.iter().cloned().collect();
    let exports_match =
        requested.is_empty() || requested.iter().all(|name| catalog_exports.contains(name));
    FunctionActivation::validate_activation(
        abi_version,
        contract_hash,
        contract_hash,
        exports_match,
    )
    .map_err(map_functions)?;
    let storage = function_storage(app)?;
    let activation = FunctionActivation::new(stores);
    let expected = activation
        .active_module(&module_id)
        .map_err(map_functions)?
        .and_then(|module| module.active_revision_id);
    let artifact = activation
        .upload(storage.as_ref(), bytes, FunctionRuntime::Typescript)
        .await
        .map_err(map_functions)?;
    let outcome = activation
        .activate(module_id, artifact, contract_hash.to_string(), expected.as_ref())
        .map_err(map_functions)?;
    rebuild_active_function_set(app).await?;
    Ok(outcome)
}

pub async fn rollback_module_revision(
    app: &AppContext,
    module_id: FunctionModuleId,
    revision_id: FunctionRevisionId,
) -> Result<ActivateFunctionOutcome, KalamDbError> {
    let stores = app.system_tables().catalog_stores();
    let revision = stores
        .get_function_revision(&revision_id)
        .map_err(|error| KalamDbError::ExecutionError(error.to_string()))?
        .ok_or_else(|| {
            KalamDbError::NotFound(format!("function revision {revision_id} not found"))
        })?;
    if revision.module_id != module_id {
        return Err(KalamDbError::InvalidOperation(format!(
            "revision {revision_id} does not belong to module {module_id}"
        )));
    }
    let activation = FunctionActivation::new(stores);
    let current = activation.active_module(&module_id).map_err(map_functions)?;
    let expected = current.as_ref().and_then(|module| module.active_revision_id.clone());
    let catalog_hash = current
        .as_ref()
        .and_then(|module| module.contract_hash.clone())
        .unwrap_or_else(|| revision.contract_hash.clone());
    let outcome = activation
        .rollback(
            module_id,
            revision.artifact_id,
            revision.contract_hash,
            expected.as_ref(),
            revision.runtime,
            revision.abi_version as u32,
            &catalog_hash,
        )
        .map_err(map_functions)?;
    rebuild_active_function_set(app).await?;
    Ok(outcome)
}

struct ActiveRunGuard {
    runtime:      Arc<crate::functions::FunctionRuntimeState>,
    execution_id: String,
}

impl Drop for ActiveRunGuard {
    fn drop(&mut self) {
        self.runtime.end_run(&self.execution_id);
    }
}

fn sanitize_function_error(message: &str) -> String {
    let mut sanitized = message.to_string();
    for secret in ["Authorization", "Bearer ", "Cookie:", "cookie="] {
        if sanitized.contains(secret) {
            sanitized = sanitized.replace(secret, "[redacted]");
        }
    }
    sanitized
}

fn is_uncompiled_inline_typescript(routine: &CatalogRoutine) -> bool {
    let language = routine.language.as_deref().unwrap_or("");
    matches!(language.to_ascii_uppercase().as_str(), "TYPESCRIPT" | "TS")
        && routine.inline_artifact_id.is_none()
}

pub fn function_storage(app: &AppContext) -> Result<Arc<StorageCached>, KalamDbError> {
    let registry = app.storage_registry();
    let storages = registry.list_storages().map_err(|error| {
        KalamDbError::ExecutionError(format!("failed to list storages: {error}"))
    })?;
    let preferred = storages
        .iter()
        .find(|storage| storage.storage_id.as_str() == "local")
        .or_else(|| storages.first())
        .ok_or_else(|| {
            KalamDbError::NotFound("no storage configured for function artifacts".to_string())
        })?;
    registry
        .get_cached(&preferred.storage_id)
        .map_err(|error| KalamDbError::ExecutionError(format!("failed to load storage: {error}")))?
        .ok_or_else(|| {
            KalamDbError::NotFound(format!("storage {} is not cached", preferred.storage_id))
        })
}

fn owner_role(app: &AppContext, owner: &UserId) -> Result<Role, KalamDbError> {
    let user = app.system_tables().users().get_user_by_id(owner).map_err(|error| {
        KalamDbError::ExecutionError(format!("failed to load procedure owner: {error}"))
    })?;
    Ok(user.map(|user| user.role).unwrap_or(Role::User))
}

fn is_sql_language(language: &str) -> bool {
    language.eq_ignore_ascii_case("SQL")
}

pub(super) fn stage_topic_publish(
    app: &AppContext,
    exec_ctx: &ExecutionContext,
    topic: &str,
    payload: &RoutineValue,
) -> Result<(), KalamDbError> {
    let request_id = exec_ctx.request_id().ok_or_else(|| {
        KalamDbError::InvalidOperation(
            "topic publish requires an active request transaction".to_string(),
        )
    })?;
    let coordinator = AppContextRequestTransactionCoordinator::new(app);
    let mut request_state =
        RequestTransactionState::from_request_id(Some(request_id)).expect("request id is present");
    request_state.sync(&coordinator);
    let transaction_id = request_state.active_transaction_id().cloned().ok_or_else(|| {
        KalamDbError::InvalidOperation(
            "topic publish requires an active request transaction".to_string(),
        )
    })?;

    let json = scalar_value_to_json(&payload.value).map_err(|error| {
        KalamDbError::ExecutionError(format!("failed to encode topic payload: {error}"))
    })?;
    let encoded = kalamdb_serialization::encode_object(&json.0).map_err(|error| {
        KalamDbError::ExecutionError(format!("failed to encode topic payload: {error}"))
    })?;
    let topic_id = TopicId::new(topic.to_ascii_lowercase());
    if !app.topic_publisher().topic_exists(&topic_id) {
        return Err(KalamDbError::NotFound(format!("topic {topic} not found")));
    }
    app.function_runtime().stage(
        transaction_id,
        StagedTopicPublish {
            topic_id,
            payload: encoded.into_bytes(),
            user_id: Some(exec_ctx.user_id().clone()),
        },
    );
    Ok(())
}

pub(crate) fn flush_staged_publishes(
    app: &AppContext,
    transaction_id: &TransactionId,
) -> Result<(), KalamDbError> {
    let staged = app.function_runtime().take(transaction_id);
    for publish in staged {
        app.topic_publisher()
            .publish_typed(&publish.topic_id, publish.payload, publish.user_id.as_ref())
            .map_err(|error| {
                KalamDbError::ExecutionError(format!(
                    "failed to flush typed topic publish: {error}"
                ))
            })?;
    }
    Ok(())
}

pub(crate) fn drop_staged_publishes(app: &AppContext, transaction_id: &TransactionId) {
    let _ = app.function_runtime().take(transaction_id);
}

fn routine_value_to_execution_result(
    value: &RoutineValue,
) -> Result<ExecutionResult, KalamDbError> {
    let array = value
        .value
        .to_array()
        .map_err(|error| KalamDbError::ExecutionError(error.to_string()))?;
    let field = Field::new("result", array.data_type().clone(), true);
    let schema = Arc::new(Schema::new(vec![field]));
    let batch = RecordBatch::try_new(schema, vec![array])
        .map_err(|error| KalamDbError::ExecutionError(error.to_string()))?;
    Ok(ExecutionResult::Rows {
        row_count: batch.num_rows(),
        batches:   vec![batch],
        schema:    None,
    })
}

fn map_functions(error: FunctionsError) -> KalamDbError {
    error.into()
}

fn attach_transfer(args: &[RoutineValue], contract_hash: &str) -> Vec<RoutineValue> {
    args.iter()
        .map(|arg| {
            if arg.transfer.is_some() {
                return arg.clone();
            }
            let Ok(json) = scalar_value_to_js_json(&arg.value) else {
                return arg.clone();
            };
            match kalamdb_serialization::encode_function_value(contract_hash, &json.0) {
                Ok(bytes) => arg.clone().with_transfer(bytes::Bytes::from(bytes), contract_hash),
                Err(_) => arg.clone(),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use kalamdb_commons::{
        models::{NamespaceId, RoutineId, RoutineSecurityMode, UserId},
        ArtifactId,
    };
    use kalamdb_functions::FunctionErrorCode;
    use kalamdb_system::CatalogRoutine;

    use super::*;

    fn catalog_routine(language: &str, artifact: bool) -> CatalogRoutine {
        let ns = NamespaceId::new("api");
        CatalogRoutine {
            routine_id:         RoutineId::from_parts(Some(&ns), "health"),
            namespace_id:       ns,
            name:               "health".to_string(),
            owner:              UserId::new("root"),
            security:           RoutineSecurityMode::Invoker,
            language:           Some(language.to_string()),
            body:               Some("return 1;".to_string()),
            return_type_id:     None,
            return_type_name:   None,
            return_is_array:    false,
            return_not_null:    false,
            comment:            None,
            return_data_type:   None,
            inline_source_hash: Some("hash".to_string()),
            inline_artifact_id: artifact.then(|| ArtifactId::new("js")),
        }
    }

    #[test]
    fn inline_typescript_without_artifact_is_refused() {
        assert!(is_uncompiled_inline_typescript(&catalog_routine("TYPESCRIPT", false)));
        assert!(!is_uncompiled_inline_typescript(&catalog_routine("JAVASCRIPT", true)));
        assert!(!is_uncompiled_inline_typescript(&catalog_routine("TYPESCRIPT", true)));
        let err = KalamDbError::from(FunctionsError::NotImplemented("api.health".into()));
        assert_eq!(err.function_error_code(), Some(FunctionErrorCode::ProcedureNotImplemented));
    }

    #[test]
    fn nested_failure_rolls_back_owned_root_transaction() {
        // FunctionService::invoke begins a request transaction when none is
        // active. Nested CALL/SQL share that request_id, so a nested error
        // returns Err from invoke_root and the owned txn is rolled back.
        assert!(matches!(FunctionCallOrigin::Sql, FunctionCallOrigin::Sql));
    }
}
