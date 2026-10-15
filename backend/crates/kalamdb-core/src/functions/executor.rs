//! Root and nested procedure invocation.

use std::sync::Arc;

use arrow::{
    array::RecordBatch,
    datatypes::{Field, Schema},
};
use datafusion::scalar::ScalarValue;
use kalamdb_commons::{
    conversions::arrow_json_conversion::scalar_value_to_json,
    models::{
        RoutineCall, RoutineId, RoutineKind, RoutineNullSafety, RoutineParameterMode, TopicId,
        TransactionId,
    },
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
use kalamdb_views::active_procedure_runs::ActiveProcedureRunSnapshot;
use parking_lot::Mutex;
use smallvec::SmallVec;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use super::{
    acl, bind,
    convert::routine_value_as_json,
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
        let stores = app.system_tables().catalog_stores();
        if let Some(routine) = stores.get_routine(&statement.call.routine_id).map_err(|error| {
            KalamDbError::ExecutionError(format!(
                "failed to load procedure {}: {error}",
                statement.call.routine_id
            ))
        })? {
            if routine.kind == RoutineKind::Function {
                return Err(KalamDbError::InvalidSql(format!(
                    "{} is a function, not a procedure; invoke it with SELECT",
                    statement.call.routine_id
                )));
            }
        }
        let routine_params =
            stores.list_parameters(&statement.call.routine_id).map_err(|error| {
                KalamDbError::ExecutionError(format!(
                    "failed to load parameters for procedure {}: {error}",
                    statement.call.routine_id
                ))
            })?;
        let out_params: Vec<_> = routine_params
            .into_iter()
            .filter(|param| {
                matches!(param.mode, RoutineParameterMode::Out | RoutineParameterMode::InOut)
            })
            .collect();
        let result = Self::invoke_sql_call(app, exec_ctx, &statement.call, params).await?;
        if !out_params.is_empty() {
            return procedure_out_value_to_execution_result(&result.value, &out_params);
        }
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
        let stores = app.system_tables().catalog_stores();
        if let Some(routine) = stores.get_routine(&call.routine_id).map_err(|error| {
            KalamDbError::ExecutionError(format!(
                "failed to load routine {}: {error}",
                call.routine_id
            ))
        })? {
            if routine.kind == RoutineKind::Procedure {
                return Err(KalamDbError::InvalidSql(format!(
                    "{} is a procedure, not a function; procedures cannot be used in scalar or DEFAULT expressions",
                    call.routine_id
                )));
            }
        }
        let result = Self::invoke_sql_call(app, exec_ctx, call, &[]).await?;
        Ok(result.value.value)
    }

    pub async fn execute_scalar_function_call(
        app: Arc<AppContext>,
        exec_ctx: &ExecutionContext,
        call: &RoutineCall,
        params: &[ScalarValue],
    ) -> Result<RoutineValue, KalamDbError> {
        let stores = app.system_tables().catalog_stores();
        let Some(routine) = stores.get_routine(&call.routine_id).map_err(|error| {
            KalamDbError::ExecutionError(format!(
                "failed to load routine {}: {error}",
                call.routine_id
            ))
        })?
        else {
            return Err(FunctionsError::UnknownProcedure(call.routine_id.to_string()).into());
        };
        if routine.kind == RoutineKind::Procedure {
            return Err(KalamDbError::InvalidSql(format!(
                "{} is a procedure, not a function; invoke it with CALL",
                call.routine_id
            )));
        }
        let result = Self::invoke_sql_call(app, exec_ctx, call, params).await?;
        Ok(result.value)
    }

    async fn invoke_sql_call(
        app: Arc<AppContext>,
        exec_ctx: &ExecutionContext,
        call: &RoutineCall,
        params: &[ScalarValue],
    ) -> Result<FunctionCallResult, KalamDbError> {
        let stores = app.system_tables().catalog_stores();
        let args = bind::bind_sql_routine_call(&stores, call, params)?;
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
        let exec_ctx =
            exec_ctx.clone().with_request_id(request_id.clone()).with_stream_autocommit();

        let coordinator = AppContextRequestTransactionCoordinator::new(app.as_ref());
        let mut request_state = RequestTransactionState::from_request_id(Some(request_id.as_str()))
            .expect("request id is present");
        request_state.sync(&coordinator);
        let owned_tx = !request_state.is_active();
        let mut began_owned_tx = false;
        // Armed once this call begins its own transaction. Drop rolls that
        // transaction back if the caller disconnects before commit.
        let owned_tx_guard = OwnedFunctionTx {
            app: Arc::clone(&app),
            id:  Mutex::new(None),
        };

        let actor = exec_ctx.user_id().clone();
        let origin_kind = origin.kind();
        let schedule_id = origin.schedule_id().map(str::to_owned);
        kalamdb_observability::begin_function_run();
        let started = std::time::Instant::now();
        let invoke_result = invoke_root(
            Arc::clone(&app),
            exec_ctx,
            origin,
            routine_id.clone(),
            args,
            &mut request_state,
            &coordinator,
            owned_tx,
            &mut began_owned_tx,
            &owned_tx_guard,
        )
        .await;
        if let Err(error) = &invoke_result {
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
        }
        let duration_ms = i64::try_from(started.elapsed().as_millis()).unwrap_or(i64::MAX);
        let (module_id, revision_id) = match app.function_runtime().active_set().lookup(&routine_id)
        {
            ImplementationRef::Module { revision, .. } => (
                Some(revision.module_id.as_str().to_string()),
                Some(revision.revision_id.as_str().to_string()),
            ),
            _ => (None, None),
        };
        let (outcome, error_code, message) = match &invoke_result {
            Ok(_) => ("ok", None, None),
            Err(error) => (
                "error",
                Some(
                    error
                        .function_error_code()
                        .map(|code| code.as_str().to_string())
                        .unwrap_or_else(|| "INTERNAL_RUNTIME_ERROR".into()),
                ),
                Some(crate::procedure_log_logger::sanitize_procedure_log_message(
                    error.to_string(),
                )),
            ),
        };
        app.procedure_log_logger()
            .record(crate::procedure_log_logger::ProcedureLogRecord {
                execution_id: request_id.clone(),
                request_id: request_id.clone(),
                procedure_id: routine_id.to_string(),
                module_id,
                revision_id,
                actor: actor.to_string().into(),
                origin: origin_kind.into(),
                outcome: outcome.into(),
                channel: "invocation".into(),
                level: if outcome == "error" {
                    "error".into()
                } else {
                    "info".into()
                },
                error_code,
                message,
                duration_ms,
                timestamp: now_ms(),
                node_id: app.node_id().as_ref().to_string(),
                schedule_id,
            });
        kalamdb_observability::finish_function_run(started.elapsed(), invoke_result.is_err());
        log::debug!(
            target: "kalamdb::functions",
            "routine={routine_id} origin={origin_kind} duration_ms={} ok={}",
            started.elapsed().as_millis(),
            invoke_result.is_ok()
        );

        match invoke_result {
            Ok(result) => {
                if began_owned_tx {
                    // Disarm before commit so a drop during commit cannot roll
                    // back a transaction that is already committing.
                    owned_tx_guard.disarm();
                    request_state
                        .commit(&coordinator)
                        .await
                        .map_err(map_request_transaction_error)?;
                }
                Ok(result)
            },
            Err(error) => {
                if began_owned_tx {
                    let _ = request_state.rollback(&coordinator);
                    owned_tx_guard.disarm();
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
    request_state: &mut RequestTransactionState<'_>,
    coordinator: &AppContextRequestTransactionCoordinator<'_>,
    owned_tx: bool,
    began_owned_tx: &mut bool,
    owned_tx_guard: &OwnedFunctionTx,
) -> Result<FunctionCallResult, KalamDbError> {
    if app.function_runtime().active_set().generation == 0 {
        rebuild_active_function_set(&app).await?;
    }
    let deployment = app.function_runtime().active_set();
    let execution_id = exec_ctx
        .request_id()
        .map(|id| id.to_string())
        .unwrap_or_else(|| Uuid::now_v7().to_string());
    let (module_id, revision_id) = match deployment.lookup(&routine_id) {
        ImplementationRef::Module { revision, .. } => (
            Some(revision.module_id.as_str().to_string()),
            Some(revision.revision_id.to_string()),
        ),
        _ => (None, None),
    };
    let runtime = app.function_runtime();
    runtime.begin_run(ActiveProcedureRunSnapshot {
        execution_id: execution_id.clone(),
        request_id: execution_id.clone(),
        procedure_id: routine_id.to_string(),
        module_id,
        revision_id,
        actor: exec_ctx.user_id().to_string(),
        principal: exec_ctx.user_id().to_string(),
        origin: origin.kind().into(),
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
            // Cancelled when the HTTP statement future is dropped, and again
            // on timeout inside the engine. The socket watch lives on the
            // connection, not here.
            cancel:   CancellationToken::new(),
            depth:    0,
        },
        sql_gate: Arc::new(tokio::sync::Mutex::new(())),
    };
    let value = {
        let prepared = prepare_call_target(&host, routine_id.clone(), &args)?;
        match prepared {
            PreparedCall::StrictNull { routine } => {
                let value = RoutineValue::new(ScalarValue::Null);
                let stores = host.app.system_tables().catalog_stores();
                bind::validate_call_return(
                    host.app.schema_registry().type_registry(),
                    &stores,
                    &routine,
                    &value,
                )?;
                value
            },
            PreparedCall::Sql {
                routine,
                body,
                input_args,
                child,
            } => {
                if owned_tx {
                    let transaction_id =
                        request_state.begin(coordinator).map_err(map_request_transaction_error)?;
                    *began_owned_tx = true;
                    owned_tx_guard.arm(transaction_id);
                }
                let value = Box::pin(invoke_sql_routine_body(&child, &routine, &body, &input_args))
                    .await?;
                let stores = host.app.system_tables().catalog_stores();
                bind::validate_call_return(
                    host.app.schema_registry().type_registry(),
                    &stores,
                    &routine,
                    &value,
                )?;
                value
            },
            PreparedCall::V8 {
                routine,
                invocation,
                child,
            } => {
                let engine = host.app.function_runtime().engine().map_err(map_functions)?;
                let admission = engine.admit(&invocation).await.map_err(map_functions)?;
                if owned_tx {
                    let transaction_id =
                        request_state.begin(coordinator).map_err(map_request_transaction_error)?;
                    *began_owned_tx = true;
                    owned_tx_guard.arm(transaction_id);
                }
                let value = engine
                    .invoke_admitted(invocation, child, admission)
                    .await
                    .map_err(map_functions)?;
                let stores = host.app.system_tables().catalog_stores();
                bind::validate_call_return(
                    host.app.schema_registry().type_registry(),
                    &stores,
                    &routine,
                    &value,
                )?;
                value
            },
        }
    };

    let (http_status, http_headers) = match origin {
        FunctionCallOrigin::Http { response, .. } => {
            let overrides = response.lock().clone();
            (overrides.status, overrides.headers)
        },
        FunctionCallOrigin::Sql
        | FunctionCallOrigin::Topic { .. }
        | FunctionCallOrigin::Schedule { .. } => (None, std::collections::HashMap::new()),
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

enum PreparedCall {
    StrictNull {
        routine: CatalogRoutine,
    },
    Sql {
        routine:    CatalogRoutine,
        body:       String,
        input_args: Vec<(kalamdb_system::CatalogRoutineParameter, RoutineValue)>,
        child:      CoreFunctionHost,
    },
    V8 {
        routine:    CatalogRoutine,
        invocation: Invocation,
        child:      Arc<dyn kalamdb_functions::FunctionHost>,
    },
}

async fn invoke_on_host(
    host: &CoreFunctionHost,
    routine_id: RoutineId,
    args: &[RoutineValue],
) -> Result<RoutineValue, KalamDbError> {
    let prepared = prepare_call_target(host, routine_id.clone(), args)?;
    let stores = host.app.system_tables().catalog_stores();
    match prepared {
        PreparedCall::StrictNull { routine } => {
            let value = RoutineValue::new(ScalarValue::Null);
            bind::validate_call_return(
                host.app.schema_registry().type_registry(),
                &stores,
                &routine,
                &value,
            )?;
            Ok(value)
        },
        PreparedCall::Sql {
            routine,
            body,
            input_args,
            child,
        } => {
            let value = Box::pin(invoke_sql_routine_body(&child, &routine, &body, &input_args))
                .await?;
            bind::validate_call_return(
                host.app.schema_registry().type_registry(),
                &stores,
                &routine,
                &value,
            )?;
            Ok(value)
        },
        PreparedCall::V8 {
            routine,
            invocation,
            child,
        } => {
            let engine = host.app.function_runtime().engine().map_err(map_functions)?;
            let value = engine.invoke(invocation, child).await.map_err(map_functions)?;
            bind::validate_call_return(
                host.app.schema_registry().type_registry(),
                &stores,
                &routine,
                &value,
            )?;
            Ok(value)
        },
    }
}

pub(super) fn prepare_nested_v8_call(
    host: &CoreFunctionHost,
    routine_id: RoutineId,
    args: &[RoutineValue],
) -> Result<Option<(Invocation, Arc<dyn kalamdb_functions::FunctionHost>)>, KalamDbError> {
    match prepare_call_target(host, routine_id, args)? {
        PreparedCall::V8 {
            invocation, child, ..
        } => Ok(Some((invocation, child))),
        PreparedCall::StrictNull { .. } | PreparedCall::Sql { .. } => Ok(None),
    }
}

fn prepare_call_target(
    host: &CoreFunctionHost,
    routine_id: RoutineId,
    args: &[RoutineValue],
) -> Result<PreparedCall, KalamDbError> {
    let stores = host.app.system_tables().catalog_stores();
    let routine = stores.get_routine(&routine_id).map_err(|error| {
        KalamDbError::ExecutionError(format!("failed to load procedure {routine_id}: {error}"))
    })?;
    let Some(routine) = routine else {
        return Err(FunctionsError::UnknownProcedure(routine_id.to_string()).into());
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

    bind::validate_call_arguments(
        host.app.schema_registry().type_registry(),
        &stores,
        &routine.routine_id,
        args,
    )?;
    let input_args = bind::resolve_input_arguments(
        host.app.schema_registry().type_registry(),
        &stores,
        &routine.routine_id,
        args,
    )?;
    if routine.null_safety == RoutineNullSafety::ReturnsNullOnNullInput
        && input_args.iter().any(|(_, val)| is_null_routine_value(val))
    {
        return Ok(PreparedCall::StrictNull { routine });
    }

    let language = routine.language.as_deref().unwrap_or("");
    if is_sql_language(language) {
        let body = routine.body.clone().ok_or_else(|| {
            KalamDbError::from(FunctionsError::Invalid(format!(
                "LANGUAGE SQL routine {} has no body",
                routine.routine_id
            )))
        })?;
        let frame = ProcedureFrame {
            routine_id: routine.routine_id.clone(),
            revision_id: FunctionRevisionId::new("sql:inline"),
            namespace_id: routine.namespace_id.clone(),
            principal_user,
            principal_role,
            security: routine.security,
            volatility: routine.volatility,
        };
        let mut child = host.clone();
        child.session = Arc::new(push_frame(host.session.as_ref(), frame)?);
        return Ok(PreparedCall::Sql {
            routine,
            body,
            input_args,
            child,
        });
    }

    let revision = revision_for_routine(host, &routine)?;
    let args = pack_named_call_input_from_resolved(&input_args);
    let frame = ProcedureFrame {
        routine_id: routine.routine_id.clone(),
        revision_id: revision.revision_id.clone(),
        namespace_id: routine.namespace_id.clone(),
        principal_user,
        principal_role,
        security: routine.security,
        volatility: routine.volatility,
    };

    let mut child = host.clone();
    child.session = Arc::new(push_frame(host.session.as_ref(), frame)?);
    Ok(PreparedCall::V8 {
        routine,
        invocation: Invocation {
            routine_id,
            revision,
            args,
            scope: host.scope.clone(),
            return_template: None,
        },
        child: Arc::new(child),
    })
}

fn is_null_routine_value(val: &RoutineValue) -> bool {
    matches!(
        &val.value,
        ScalarValue::Null | ScalarValue::Utf8(None) | ScalarValue::LargeUtf8(None)
    ) || val.value.is_null()
}

async fn invoke_sql_routine_body(
    host: &CoreFunctionHost,
    routine: &CatalogRoutine,
    body: &str,
    input_args: &[(kalamdb_system::CatalogRoutineParameter, RoutineValue)],
) -> Result<RoutineValue, KalamDbError> {
    host.scope.check().map_err(map_functions)?;
    let _gate = host.sql_gate.lock().await;
    host.scope.check().map_err(map_functions)?;

    let mut ctx = host.current_exec_ctx().with_stream_autocommit();
    if let Some(config_json) = &routine.config_params {
        if let Ok(config) =
            serde_json::from_str::<std::collections::BTreeMap<String, String>>(config_json)
        {
            if let Some(sp) = config.get("search_path").or_else(|| config.get("schema")) {
                let ns = sp
                    .split(',')
                    .next()
                    .unwrap_or(sp)
                    .trim()
                    .trim_matches('"')
                    .trim_matches('\'');
                if !ns.is_empty() {
                    ctx = ctx.with_namespace_id(kalamdb_commons::SchemaId::new(ns));
                }
            }
        }
    }

    let all_params = host
        .app
        .system_tables()
        .catalog_stores()
        .list_parameters(&routine.routine_id)
        .map_err(|error| {
            KalamDbError::CatalogError(format!(
                "failed to load parameters for routine {}: {error}",
                routine.routine_id
            ))
        })?;
    let out_params: Vec<_> = all_params
        .iter()
        .filter(|param| matches!(param.mode, RoutineParameterMode::Out | RoutineParameterMode::InOut))
        .cloned()
        .collect();
    let has_out_params = !out_params.is_empty();

    let mut env_args = input_args.to_vec();
    for out_param in &out_params {
        if !env_args.iter().any(|(p, _)| p.name.eq_ignore_ascii_case(&out_param.name)) {
            env_args.push((out_param.clone(), RoutineValue::new(ScalarValue::Null)));
        }
    }

    let mut working_body = body.trim();
    let mut declare_decls: Vec<(String, Option<String>)> = Vec::new();
    if routine.language.as_deref().is_some_and(|l| l.eq_ignore_ascii_case("PLPGSQL")) {
        let (decls, after_begin) = extract_plpgsql_declare_and_body(working_body);
        declare_decls = decls;
        if let Some(after) = after_begin {
            working_body = after;
        }
    }

    let executor = host.app.sql_executor();
    for (var_name, init_expr) in declare_decls {
        let init_val = if let Some(expr_sql) = init_expr {
            eval_plpgsql_expr(&executor, &ctx, &expr_sql, &env_args).await?
        } else {
            RoutineValue::new(ScalarValue::Null)
        };
        let ord = i32::try_from(env_args.len() + 1).unwrap_or(i32::MAX);
        let pseudo_param = kalamdb_system::CatalogRoutineParameter {
            parameter_id: kalamdb_commons::models::RoutineParameterId::new(
                &routine.routine_id,
                ord,
            )
            .map_err(KalamDbError::InvalidSql)?,
            routine_id: routine.routine_id.clone(),
            name: var_name,
            ordinal: ord,
            type_id: None,
            type_name: "TEXT".to_string(),
            is_array: false,
            not_null: false,
            nonempty: false,
            data_type: Some(kalamdb_commons::KalamDataType::Text),
            mode: RoutineParameterMode::In,
            default_expr: None,
        };
        env_args.push((pseudo_param, init_val));
    }

    let (returned, last_result) = execute_plpgsql_block(
        &executor,
        &ctx,
        routine,
        working_body,
        &mut env_args,
    )
    .await?;
    if let Some(ret_val) = returned {
        return Ok(ret_val);
    }

    if has_out_params
        && env_args.iter().any(|(p, v)| {
            matches!(p.mode, RoutineParameterMode::Out | RoutineParameterMode::InOut)
                && !is_null_routine_value(v)
        })
    {
        if out_params.len() == 1 {
            if let Some((_, val)) = env_args
                .iter()
                .find(|(p, _)| p.name.eq_ignore_ascii_case(&out_params[0].name))
            {
                if !is_null_routine_value(val) {
                    return Ok(val.clone());
                }
            }
        } else {
            let mut map = serde_json::Map::with_capacity(out_params.len());
            for out_param in &out_params {
                let json_val = env_args
                    .iter()
                    .find(|(p, _)| p.name.eq_ignore_ascii_case(&out_param.name))
                    .and_then(|(_, v)| routine_value_as_json(v))
                    .unwrap_or(serde_json::Value::Null);
                map.insert(out_param.name.clone(), json_val);
            }
            return super::convert::json_to_routine_value(&serde_json::Value::Object(map), None);
        }
    }

    match last_result {
        Some(result) => {
            if routine.kind == RoutineKind::Procedure && !has_out_params {
                if matches!(result, ExecutionResult::Rows { .. } | ExecutionResult::ScalarRows { .. })
                {
                    super::convert::execution_result_to_routine(result)
                } else {
                    Ok(RoutineValue::new(ScalarValue::Null))
                }
            } else {
                super::convert::execution_result_to_routine(result)
            }
        },
        None => Ok(RoutineValue::new(ScalarValue::Null)),
    }
}

fn extract_plpgsql_declare_and_body(body: &str) -> (Vec<(String, Option<String>)>, Option<&str>) {
    let trimmed = body.trim().trim_end_matches(';').trim();
    let upper = trimmed.to_ascii_uppercase();
    let Some(begin_pos) = find_top_level_keyword(&upper, "BEGIN") else {
        return (Vec::new(), None);
    };
    let mut decls = Vec::new();
    let before_begin = trimmed[..begin_pos].trim();
    if let Some(after_declare) = strip_keyword_prefix_ci(before_begin, "DECLARE") {
        for raw_decl in split_plpgsql_statements(after_declare) {
            let decl = raw_decl.trim();
            if decl.is_empty() {
                continue;
            }
            if let Some((lhs, rhs)) = decl.split_once(":=").or_else(|| decl.split_once('=')) {
                let var_name = lhs
                    .split_whitespace()
                    .next()
                    .unwrap_or("")
                    .trim()
                    .to_string();
                if !var_name.is_empty() {
                    decls.push((var_name, Some(rhs.trim().to_string())));
                }
            } else {
                let var_name = decl
                    .split_whitespace()
                    .next()
                    .unwrap_or("")
                    .trim()
                    .to_string();
                if !var_name.is_empty() {
                    decls.push((var_name, None));
                }
            }
        }
    }
    let after_begin = &trimmed[begin_pos + "BEGIN".len()..];
    if !after_begin.starts_with(|c: char| c.is_whitespace() || c == ';') {
        return (decls, None);
    }
    let after_begin = after_begin.trim();
    let after_upper = after_begin.to_ascii_uppercase();
    let stripped = if after_upper.ends_with("END") {
        &after_begin[..after_begin.len() - 3]
    } else {
        after_begin
    };
    (decls, Some(stripped.trim()))
}

fn find_top_level_keyword(upper: &str, keyword: &str) -> Option<usize> {
    let bytes = upper.as_bytes();
    let kw = keyword.as_bytes();
    if bytes.len() < kw.len() {
        return None;
    }
    for i in 0..=(bytes.len() - kw.len()) {
        if &bytes[i..i + kw.len()] == kw {
            let before_ok = i == 0 || !bytes[i - 1].is_ascii_alphanumeric() && bytes[i - 1] != b'_';
            let after_idx = i + kw.len();
            let after_ok = after_idx == bytes.len()
                || (!bytes[after_idx].is_ascii_alphanumeric() && bytes[after_idx] != b'_');
            if before_ok && after_ok {
                return Some(i);
            }
        }
    }
    None
}

fn split_plpgsql_statements(body: &str) -> Vec<String> {
    let mut out = Vec::new();
    let bytes = body.as_bytes();
    let mut i = 0usize;
    let mut start = 0usize;
    let mut paren_depth = 0i32;
    let mut block_depth = 0i32;

    while i < bytes.len() {
        let ch = bytes[i];
        if ch == b'\'' || ch == b'"' {
            let quote = ch;
            i += 1;
            while i < bytes.len() {
                if bytes[i] == quote {
                    i += 1;
                    if i < bytes.len() && bytes[i] == quote {
                        i += 1;
                        continue;
                    }
                    break;
                }
                i += 1;
            }
            continue;
        }
        if ch == b'(' {
            paren_depth += 1;
            i += 1;
            continue;
        }
        if ch == b')' {
            paren_depth = (paren_depth - 1).max(0);
            i += 1;
            continue;
        }
        if paren_depth == 0 && (ch.is_ascii_alphabetic() || ch == b'_') {
            let word_start = i;
            i += 1;
            while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                i += 1;
            }
            let word = &body[word_start..i];
            let preceded_by_end = body[start..word_start]
                .split_whitespace()
                .last()
                .is_some_and(|prev| prev.eq_ignore_ascii_case("END"));
            if (word.eq_ignore_ascii_case("IF") || word.eq_ignore_ascii_case("LOOP"))
                && !preceded_by_end
            {
                block_depth += 1;
            } else if (word.eq_ignore_ascii_case("IF") || word.eq_ignore_ascii_case("LOOP"))
                && preceded_by_end
            {
                block_depth = (block_depth - 1).max(0);
            }
            continue;
        }
        if ch == b';' && paren_depth == 0 && block_depth == 0 {
            let piece = body[start..i].trim();
            if !piece.is_empty() {
                out.push(piece.to_string());
            }
            i += 1;
            start = i;
            continue;
        }
        i += 1;
    }
    let tail = body[start..].trim();
    if !tail.is_empty() {
        out.push(tail.to_string());
    }
    out
}

async fn eval_plpgsql_expr(
    executor: &crate::sql::executor::SqlExecutor,
    ctx: &ExecutionContext,
    expr_sql: &str,
    env_args: &[(kalamdb_system::CatalogRoutineParameter, RoutineValue)],
) -> Result<RoutineValue, KalamDbError> {
    let select_sql = format!("SELECT {expr_sql}");
    let sql = substitute_sql_routine_parameters(&select_sql, env_args);
    let params: Vec<ScalarValue> = env_args.iter().map(|(_, val)| val.value.clone()).collect();
    let metadata = executor
        .prepare_statement_metadata(&sql, ctx)
        .map_err(|err| KalamDbError::InvalidSql(err.to_string()))?;
    let result = Box::pin(executor.execute_with_metadata(&metadata, ctx, params)).await?;
    super::convert::execution_result_to_routine(result)
}

async fn eval_plpgsql_bool(
    executor: &crate::sql::executor::SqlExecutor,
    ctx: &ExecutionContext,
    cond_sql: &str,
    env_args: &[(kalamdb_system::CatalogRoutineParameter, RoutineValue)],
) -> Result<bool, KalamDbError> {
    let val = eval_plpgsql_expr(executor, ctx, cond_sql, env_args).await?;
    Ok(matches!(val.value, ScalarValue::Boolean(Some(true))))
}

fn execute_plpgsql_block<'a>(
    executor: &'a crate::sql::executor::SqlExecutor,
    ctx: &'a ExecutionContext,
    routine: &'a CatalogRoutine,
    block_sql: &'a str,
    env_args: &'a mut Vec<(kalamdb_system::CatalogRoutineParameter, RoutineValue)>,
) -> std::pin::Pin<
    Box<
        dyn std::future::Future<
                Output = Result<(Option<RoutineValue>, Option<ExecutionResult>), KalamDbError>,
            > + Send
            + 'a,
    >,
> {
    Box::pin(async move {
        let statements = split_plpgsql_statements(block_sql);
        let mut last_result = None;
        for raw_sql in statements {
            let trimmed_stmt = raw_sql.trim().trim_end_matches(';').trim();
            if trimmed_stmt.is_empty()
                || trimmed_stmt.eq_ignore_ascii_case("BEGIN")
                || trimmed_stmt.eq_ignore_ascii_case("END")
                || trimmed_stmt.eq_ignore_ascii_case("NULL")
            {
                continue;
            }
            if let Some(rest) = strip_keyword_prefix_ci(trimmed_stmt, "RETURN") {
                let expr_sql = rest.trim();
                if expr_sql.is_empty() {
                    return Ok((Some(RoutineValue::new(ScalarValue::Null)), last_result));
                }
                let val = eval_plpgsql_expr(executor, ctx, expr_sql, env_args).await?;
                return Ok((Some(val), last_result));
            }
            if let Some((cond_sql, loop_body)) = parse_while_loop_stmt(trimmed_stmt) {
                let mut iterations = 0usize;
                while eval_plpgsql_bool(executor, ctx, cond_sql, env_args).await? {
                    iterations += 1;
                    if iterations > 10_000 {
                        return Err(KalamDbError::from(FunctionsError::ResourceLimit(
                            "PL/pgSQL loop iteration limit".into(),
                        )));
                    }
                    let (ret, res) =
                        execute_plpgsql_block(executor, ctx, routine, loop_body, env_args).await?;
                    if res.is_some() {
                        last_result = res;
                    }
                    if ret.is_some() {
                        return Ok((ret, last_result));
                    }
                }
                continue;
            }
            if let Some((cond_sql, then_body, else_body)) = parse_if_stmt(trimmed_stmt) {
                if eval_plpgsql_bool(executor, ctx, cond_sql, env_args).await? {
                    let (ret, res) =
                        execute_plpgsql_block(executor, ctx, routine, then_body, env_args).await?;
                    if res.is_some() {
                        last_result = res;
                    }
                    if ret.is_some() {
                        return Ok((ret, last_result));
                    }
                } else if let Some(else_sql) = else_body {
                    let (ret, res) =
                        execute_plpgsql_block(executor, ctx, routine, else_sql, env_args).await?;
                    if res.is_some() {
                        last_result = res;
                    }
                    if ret.is_some() {
                        return Ok((ret, last_result));
                    }
                }
                continue;
            }
            if let Some((lhs, rhs)) = split_plpgsql_assignment(trimmed_stmt) {
                let assigned_val = eval_plpgsql_expr(executor, ctx, rhs, env_args).await?;
                if let Some((_, slot)) = env_args
                    .iter_mut()
                    .find(|(p, _)| p.name.eq_ignore_ascii_case(lhs))
                {
                    *slot = assigned_val;
                }
                continue;
            }
            let sql = substitute_sql_routine_parameters(trimmed_stmt, env_args);
            let params: Vec<ScalarValue> =
                env_args.iter().map(|(_, val)| val.value.clone()).collect();
            let metadata = executor
                .prepare_statement_metadata(&sql, ctx)
                .map_err(|err| KalamDbError::InvalidSql(err.to_string()))?;
            if metadata.classified_statement.as_ref().is_some_and(|statement| {
                matches!(
                    statement.kind(),
                    kalamdb_sql::SqlStatementKind::BeginTransaction
                        | kalamdb_sql::SqlStatementKind::CommitTransaction
                        | kalamdb_sql::SqlStatementKind::RollbackTransaction
                )
            }) {
                return Err(KalamDbError::from(FunctionsError::Invalid(
                    "transaction control is not allowed inside a routine".into(),
                )));
            }
            if matches!(
                routine.volatility,
                kalamdb_commons::models::RoutineVolatility::Immutable
                    | kalamdb_commons::models::RoutineVolatility::Stable
            ) && metadata.classified_statement.as_ref().is_some_and(|statement| {
                !matches!(
                    statement.kind(),
                    kalamdb_sql::SqlStatementKind::Select
                        | kalamdb_sql::SqlStatementKind::DataFusionMetaCommand
                        | kalamdb_sql::SqlStatementKind::SetSearchPath(_)
                )
            }) {
                return Err(KalamDbError::from(FunctionsError::Invalid(format!(
                    "{} routine {} cannot execute read-write or DDL statements",
                    routine.volatility.as_str(),
                    routine.routine_id
                ))));
            }
            let result = Box::pin(executor.execute_with_metadata(&metadata, ctx, params)).await?;
            last_result = Some(result);
        }
        Ok((None, last_result))
    })
}

fn parse_while_loop_stmt(stmt: &str) -> Option<(&str, &str)> {
    let rest = strip_keyword_prefix_ci(stmt.trim(), "WHILE")?.trim();
    let upper = rest.to_ascii_uppercase();
    if !upper.ends_with("END LOOP") {
        return None;
    }
    let without_end = rest[..rest.len() - "END LOOP".len()].trim();
    let upper_inner = without_end.to_ascii_uppercase();
    let loop_pos = find_top_level_keyword(&upper_inner, "LOOP")?;
    let cond = without_end[..loop_pos].trim();
    let body = without_end[loop_pos + "LOOP".len()..].trim();
    Some((cond, body))
}

fn parse_if_stmt(stmt: &str) -> Option<(&str, &str, Option<&str>)> {
    let rest = strip_keyword_prefix_ci(stmt.trim(), "IF")?.trim();
    let upper = rest.to_ascii_uppercase();
    if !upper.ends_with("END IF") {
        return None;
    }
    let without_end = rest[..rest.len() - "END IF".len()].trim();
    let upper_inner = without_end.to_ascii_uppercase();
    let then_pos = find_top_level_keyword(&upper_inner, "THEN")?;
    let cond = without_end[..then_pos].trim();
    let after_then = without_end[then_pos + "THEN".len()..].trim();
    let upper_after_then = after_then.to_ascii_uppercase();
    if let Some(else_pos) = find_top_level_keyword(&upper_after_then, "ELSE") {
        let then_body = after_then[..else_pos].trim();
        let else_body = after_then[else_pos + "ELSE".len()..].trim();
        Some((cond, then_body, Some(else_body)))
    } else {
        Some((cond, after_then, None))
    }
}

fn strip_keyword_prefix_ci<'a>(input: &'a str, keyword: &str) -> Option<&'a str> {
    if input.len() < keyword.len() {
        return None;
    }
    let (prefix, rest) = input.split_at(keyword.len());
    if !prefix.eq_ignore_ascii_case(keyword) {
        return None;
    }
    if rest.is_empty() || rest.starts_with(char::is_whitespace) {
        Some(rest)
    } else {
        None
    }
}

fn split_plpgsql_assignment(stmt: &str) -> Option<(&str, &str)> {
    let (lhs, rhs) = stmt.split_once(":=")?;
    let lhs = lhs.trim();
    let rhs = rhs.trim();
    if lhs.is_empty()
        || rhs.is_empty()
        || !lhs.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
    {
        return None;
    }
    Some((lhs, rhs))
}

fn substitute_sql_routine_parameters(
    sql: &str,
    input_args: &[(kalamdb_system::CatalogRoutineParameter, RoutineValue)],
) -> String {
    if input_args.is_empty() {
        return sql.to_string();
    }
    let mut named: Vec<(&str, usize)> = input_args
        .iter()
        .enumerate()
        .filter_map(|(idx, (param, _))| {
            let trimmed = param.name.trim();
            if trimmed.is_empty() || trimmed.starts_with('$') {
                None
            } else {
                Some((trimmed, idx + 1))
            }
        })
        .collect();
    if named.is_empty() {
        return sql.to_string();
    }
    named.sort_by(|a, b| b.0.len().cmp(&a.0.len()));

    let mut out = String::with_capacity(sql.len() + 16);
    let bytes = sql.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let ch = bytes[i];
        if ch == b'\'' || ch == b'"' {
            let quote = ch;
            out.push(quote as char);
            i += 1;
            while i < bytes.len() {
                let c = bytes[i];
                out.push(c as char);
                i += 1;
                if c == quote {
                    if i < bytes.len() && bytes[i] == quote {
                        out.push(bytes[i] as char);
                        i += 1;
                    } else {
                        break;
                    }
                }
            }
            continue;
        }
        if ch.is_ascii_alphabetic() || ch == b'_' {
            let start = i;
            i += 1;
            while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                i += 1;
            }
            let ident = &sql[start..i];
            let preceded_by_dot = start > 0 && bytes[start - 1] == b'.';
            let followed_by_paren = sql[i..].trim_start().starts_with('(');
            if !preceded_by_dot && !followed_by_paren {
                if let Some((_, pos)) = named
                    .iter()
                    .find(|(name, _)| name.eq_ignore_ascii_case(ident))
                {
                    out.push('$');
                    out.push_str(&pos.to_string());
                    continue;
                }
            }
            out.push_str(ident);
            continue;
        }
        out.push(ch as char);
        i += 1;
    }
    out
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
    // Concurrent CREATE PROCEDURE calls each rebuild from a catalog snapshot.
    // Without this lock, a snapshot taken earlier can publish last and drop a
    // procedure that a later snapshot already installed.
    let runtime = app.function_runtime();
    let _rebuild = runtime.rebuild_lock().lock().await;
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

    let module_exports: std::collections::HashSet<String> = match module_revision.as_ref() {
        Some(revision) => stores
            .get_function_revision(&revision.revision_id)
            .map_err(|error| KalamDbError::ExecutionError(error.to_string()))?
            .map(|stored| stored.exported_procedure_ids.into_iter().collect())
            .unwrap_or_default(),
        None => std::collections::HashSet::new(),
    };
    let previous = app.function_runtime().active_set();
    let set = ActiveFunctionSet::resolve(
        previous.generation.saturating_add(1).max(1),
        module_revision
            .as_ref()
            .map(|revision| revision.contract_hash.clone())
            .unwrap_or_default(),
        module_revision,
        &module_exports,
        rows,
    );
    let generation = set.generation;
    let procedure_count = set.procedures.len();
    let (module_id, revision_id, log_procedure_id) = match set.module_revision.as_ref() {
        Some(revision) => {
            let module = revision.module_id.as_str().to_string();
            let revision_id = revision.revision_id.as_str().to_string();
            (Some(module.clone()), Some(revision_id), module)
        },
        None => (None, None, "inline".to_string()),
    };
    app.function_runtime().publish_active_set(set);
    super::lifecycle::record_deployment(
        app.procedure_log_logger().as_ref(),
        app.node_id().as_ref().to_string(),
        &log_procedure_id,
        module_id,
        revision_id,
        generation,
        procedure_count,
    );
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
    let persisted_exports = {
        let mut ids: Vec<String> = requested.into_iter().collect();
        ids.sort();
        ids
    };
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
        .activate(
            module_id,
            artifact,
            contract_hash.to_string(),
            expected.as_ref(),
            persisted_exports,
        )
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

struct OwnedFunctionTx {
    app: Arc<AppContext>,
    id:  Mutex<Option<TransactionId>>,
}

impl OwnedFunctionTx {
    fn arm(&self, transaction_id: TransactionId) {
        *self.id.lock() = Some(transaction_id);
    }

    fn disarm(&self) {
        self.id.lock().take();
    }
}

impl Drop for OwnedFunctionTx {
    fn drop(&mut self) {
        if let Some(transaction_id) = self.id.lock().take() {
            let _ = self.app.transaction_coordinator().rollback(&transaction_id);
        }
    }
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

fn is_uncompiled_inline_typescript(routine: &CatalogRoutine) -> bool {
    let language = routine.language.as_deref().unwrap_or("");
    matches!(language.to_ascii_uppercase().as_str(), "TYPESCRIPT" | "TS")
        && routine.inline_artifact_id.is_none()
}

pub fn function_storage(app: &AppContext) -> Result<Arc<StorageCached>, KalamDbError> {
    Ok(app.functions_storage())
}

fn owner_role(app: &AppContext, owner: &UserId) -> Result<Role, KalamDbError> {
    let user = app.system_tables().users().get_user_by_id(owner).map_err(|error| {
        KalamDbError::ExecutionError(format!("failed to load procedure owner: {error}"))
    })?;
    Ok(user.map(|user| user.role).unwrap_or(Role::User))
}

fn is_sql_language(language: &str) -> bool {
    language.eq_ignore_ascii_case("SQL") || language.eq_ignore_ascii_case("PLPGSQL")
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

fn procedure_out_value_to_execution_result(
    value: &RoutineValue,
    out_params: &[kalamdb_system::CatalogRoutineParameter],
) -> Result<ExecutionResult, KalamDbError> {
    let json = routine_value_as_json(value).unwrap_or(serde_json::Value::Null);
    let parsed_json = match &json {
        serde_json::Value::String(s) => serde_json::from_str::<serde_json::Value>(s).unwrap_or(json),
        _ => json,
    };
    let mut fields = Vec::with_capacity(out_params.len());
    let mut arrays = Vec::with_capacity(out_params.len());
    for param in out_params {
        let cell_json = match &parsed_json {
            serde_json::Value::Object(map) => {
                map.get(&param.name).cloned().unwrap_or_else(|| {
                    if out_params.len() == 1 {
                        parsed_json.clone()
                    } else {
                        serde_json::Value::Null
                    }
                })
            },
            other if out_params.len() == 1 => other.clone(),
            _ => serde_json::Value::Null,
        };
        let rv = super::convert::json_to_routine_value(&cell_json, param.builtin_data_type().as_ref())?;
        let array = rv
            .value
            .to_array()
            .map_err(|error| KalamDbError::ExecutionError(error.to_string()))?;
        fields.push(Field::new(param.name.clone(), array.data_type().clone(), !param.not_null));
        arrays.push(array);
    }
    let schema = Arc::new(Schema::new(fields));
    let batch = RecordBatch::try_new(schema, arrays)
        .map_err(|error| KalamDbError::ExecutionError(error.to_string()))?;
    Ok(ExecutionResult::Rows {
        row_count: batch.num_rows(),
        batches:   vec![batch],
        schema:    None,
    })
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

fn pack_named_call_input_from_resolved(
    input_args: &[(kalamdb_system::CatalogRoutineParameter, RoutineValue)],
) -> Vec<RoutineValue> {
    if input_args.is_empty() {
        return Vec::new();
    }
    let fields: Vec<(String, RoutineValue)> = input_args
        .iter()
        .map(|(param, arg)| (param.name.clone(), arg.clone()))
        .collect();
    vec![RoutineValue::named_object(fields)]
}

#[cfg(test)]
mod tests {
    use kalamdb_commons::{
        models::{RoutineId, RoutineSecurityMode, SchemaId, UserId},
        ArtifactId,
    };
    use kalamdb_functions::FunctionErrorCode;
    use kalamdb_system::CatalogRoutine;

    use super::*;

    fn catalog_routine(language: &str, artifact: bool) -> CatalogRoutine {
        let ns = SchemaId::new("api");
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
            kind:               kalamdb_commons::models::RoutineKind::default(),
            volatility:         kalamdb_commons::models::RoutineVolatility::default(),
            null_safety:        kalamdb_commons::models::RoutineNullSafety::default(),
            parallel_safety:    kalamdb_commons::models::RoutineParallelSafety::default(),
            cost:               None,
            rows:               None,
            config_params:      None,
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

    #[test]
    fn named_object_carries_typed_fields_without_json_packing() {
        let named = RoutineValue::named_object(vec![(
            "room_id".to_string(),
            RoutineValue::new(ScalarValue::Utf8(Some("main".to_string()))),
        )]);
        assert!(!named.json_sql);
        let fields = named.named_fields.as_ref().expect("named fields");
        assert_eq!(fields.len(), 1);
        assert_eq!(fields[0].0, "room_id");
        assert_eq!(fields[0].1.value, ScalarValue::Utf8(Some("main".to_string())));
    }

    #[test]
    fn named_object_with_nested_json_sql_preserves_json_flag() {
        let payload = RoutineValue::json(ScalarValue::Utf8(Some(
            r#"{"role":"user","content":"hi","_table":"chat_demo:messages"}"#.into(),
        )));
        let packed = RoutineValue::named_object(vec![("payload".to_string(), payload)]);
        let fields = packed.named_fields.as_ref().expect("named fields");
        assert_eq!(fields[0].0, "payload");
        assert!(fields[0].1.json_sql);
        let decoded = crate::functions::convert::routine_value_as_json(&packed)
            .expect("routine_value_as_json");
        assert!(decoded["payload"].is_object());
        assert_eq!(decoded["payload"]["role"], "user");
        assert_eq!(decoded["payload"]["content"], "hi");
    }
}
