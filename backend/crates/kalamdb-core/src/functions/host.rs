//! V8 host callbacks implemented by core (nested SQL, CALL, topics, HTTP).

use std::sync::Arc;

use kalamdb_commons::{models::RoutineId, NamespaceId, Role, RoutineSecurityMode, UserId};
use kalamdb_functions::{
    FunctionCallOrigin, FunctionExecutionRoot, FunctionHost, FunctionsError, HostFuture,
    InvocationMetadata, InvocationScope, PrincipalKey, ProcedureFrame, ProcedureFrameStack,
    RoutineValue,
};
use parking_lot::Mutex;
use smallvec::SmallVec;
use tokio::runtime::Handle;

use super::executor;
use crate::{app_context::AppContext, sql::context::ExecutionContext};

const MAX_PROCEDURE_DEPTH: usize = 16;

pub(super) type PrincipalContextCache = SmallVec<[(PrincipalKey, ExecutionContext); 2]>;

#[derive(Clone)]
pub(super) struct CoreFunctionHost {
    pub app:      Arc<AppContext>,
    pub handle:   Handle,
    pub session:  Arc<HostSession>,
    pub origin:   FunctionCallOrigin,
    pub scope:    InvocationScope,
    pub sql_gate: Arc<tokio::sync::Mutex<()>>,
}

#[derive(Clone)]
pub(super) struct HostSession {
    pub exec_ctx:      ExecutionContext,
    pub stack:         ProcedureFrameStack,
    pub root:          FunctionExecutionRoot,
    pub principal_ctx: Arc<Mutex<PrincipalContextCache>>,
}

impl CoreFunctionHost {
    pub(super) fn current_exec_ctx(&self) -> ExecutionContext {
        let session = self.session.as_ref();
        let Some(frame) = session.stack.last() else {
            return session.exec_ctx.clone();
        };
        let key = PrincipalKey {
            user:      frame.principal_user.clone(),
            role:      frame.principal_role,
            namespace: if frame.security == RoutineSecurityMode::Definer {
                frame.namespace_id.clone()
            } else {
                session.exec_ctx.default_namespace()
            },
        };
        {
            let cache = session.principal_ctx.lock();
            if let Some((_, ctx)) = cache.iter().find(|(cached, _)| cached == &key) {
                return ctx.clone();
            }
        }
        let ctx = session.exec_ctx.with_effective_identity(key.user.clone(), key.role);
        let ctx = if frame.security == RoutineSecurityMode::Definer {
            ctx.with_namespace_id(frame.namespace_id.clone())
        } else {
            ctx
        };
        session.principal_ctx.lock().push((key, ctx.clone()));
        ctx
    }

    pub(super) fn stack_label(&self) -> String {
        let session = self.session.as_ref();
        session
            .stack
            .iter()
            .map(ProcedureFrame::stack_label)
            .collect::<Vec<_>>()
            .join(" -> ")
    }
    async fn run_sql(
        &self,
        sql: String,
        params: Vec<RoutineValue>,
        rows: bool,
    ) -> kalamdb_functions::Result<RoutineValue> {
        self.scope.check()?;
        let engine = self.app.function_runtime().engine()?;
        if sql.len() > engine.config().max_sql_text_bytes {
            return Err(FunctionsError::ResourceLimit("sql text".into()));
        }
        let _gate = self.sql_gate.lock().await;
        self.scope.check()?;
        let ctx = self.current_exec_ctx();
        let executor = self.app.sql_executor();
        let metadata = executor
            .prepare_statement_metadata(&sql, &ctx)
            .map_err(|e| FunctionsError::Invalid(e.to_string()))?;
        // Procedure transactions are owned by their root invocation.
        if metadata.classified_statement.as_ref().is_some_and(|statement| {
            matches!(
                statement.kind(),
                kalamdb_sql::SqlStatementKind::BeginTransaction
                    | kalamdb_sql::SqlStatementKind::CommitTransaction
                    | kalamdb_sql::SqlStatementKind::RollbackTransaction
            )
        }) {
            return Err(FunctionsError::Invalid(
                "transaction control is not allowed inside a procedure".into(),
            ));
        }
        let result = executor.execute_with_metadata(
            &metadata,
            &ctx,
            params.into_iter().map(|v| v.value).collect(),
        );
        let result = tokio::select! {
            biased;
            _ = self.scope.cancel.cancelled() => return Err(FunctionsError::Cancelled),
            _ = tokio::time::sleep_until(self.scope.deadline.into()) => return Err(FunctionsError::Timeout),
            result = result => result.map_err(map_core)?,
        };
        if rows {
            super::convert::execution_result_to_rows(result).map_err(map_core)
        } else {
            super::convert::execution_result_to_routine(result).map_err(map_core)
        }
    }
}

impl FunctionHost for CoreFunctionHost {
    fn sql(&self, sql: &str, params: &[RoutineValue]) -> kalamdb_functions::Result<RoutineValue> {
        self.handle.block_on(self.run_sql(sql.to_string(), params.to_vec(), false))
    }

    fn query(&self, sql: String, params: Vec<RoutineValue>) -> HostFuture<'_, RoutineValue> {
        Box::pin(self.run_sql(sql, params, true))
    }

    fn execute(&self, sql: String, params: Vec<RoutineValue>) -> HostFuture<'_, RoutineValue> {
        Box::pin(self.run_sql(sql, params, false))
    }

    fn call_async(
        &self,
        procedure: String,
        args: Vec<RoutineValue>,
    ) -> HostFuture<'_, RoutineValue> {
        Box::pin(async move {
            let id = resolve_routine_id(&procedure, &self.session.exec_ctx.default_namespace());
            executor::invoke_nested(self, id, &args).await.map_err(map_core)
        })
    }

    fn prepare_nested_call(
        &self,
        procedure: String,
        args: Vec<RoutineValue>,
    ) -> kalamdb_functions::Result<Option<(kalamdb_functions::Invocation, Arc<dyn FunctionHost>)>>
    {
        let max_depth = self.app.function_runtime().engine()?.config().max_depth;
        let mut child = self.clone();
        child.scope = self.scope.child(max_depth)?;
        let id = resolve_routine_id(&procedure, &child.session.exec_ctx.default_namespace());
        let (invocation, host) = executor::prepare_call(&child, id, &args).map_err(map_core)?;
        Ok(Some((invocation, host)))
    }

    fn metadata(&self) -> Option<InvocationMetadata> {
        let ctx = self.current_exec_ctx();
        Some(InvocationMetadata {
            actor:      kalamdb_functions::ActorMeta {
                id:   self.session.exec_ctx.user_id().clone(),
                role: self.session.exec_ctx.user_role(),
            },
            principal:  kalamdb_functions::ActorMeta {
                id:   ctx.user_id().clone(),
                role: ctx.user_role(),
            },
            namespace:  ctx.default_namespace(),
            request_id: ctx.request_id().unwrap_or_default().to_string(),
        })
    }

    fn max_log_bytes(&self) -> usize {
        self.app
            .function_runtime()
            .engine()
            .map(|engine| engine.config().max_log_bytes)
            .unwrap_or(64 * 1024)
    }

    fn procedure_stack(&self) -> String {
        self.stack_label()
    }

    fn routine_js_map(&self) -> String {
        let Ok(routines) = self.app.system_tables().catalog_stores().list_routines() else {
            return "{}".to_string();
        };
        kalamdb_functions_host::build_routine_js_map(routines.iter().map(|routine| {
            (
                routine.namespace_id.as_str(),
                routine.name.as_str(),
                routine.routine_id.as_str(),
            )
        }))
    }

    fn call(
        &self,
        procedure: &str,
        args: &[RoutineValue],
    ) -> kalamdb_functions::Result<RoutineValue> {
        let default_ns = {
            let session = self.session.as_ref();
            session.exec_ctx.default_namespace()
        };
        let routine_id = resolve_routine_id(procedure, &default_ns);
        match self.handle.block_on(executor::invoke_nested(self, routine_id, args)) {
            Ok(value) => Ok(value),
            Err(error) => Err(annotate(self.stack_label(), error.to_string())),
        }
    }

    fn publish(&self, topic: &str, payload: &RoutineValue) -> kalamdb_functions::Result<()> {
        let max = self
            .app
            .function_runtime()
            .engine()
            .map(|engine| engine.config().max_topic_bytes)
            .unwrap_or(1024 * 1024);
        if topic.len().saturating_add(payload.value.size()) > max {
            return Err(FunctionsError::ResourceLimit("topic payload".into()));
        }
        let exec_ctx = self.current_exec_ctx();
        executor::stage_topic_publish(&self.app, &exec_ctx, topic, payload).map_err(map_core)
    }

    fn http_request_header(&self, name: &str) -> kalamdb_functions::Result<Option<String>> {
        if is_blocked_request_header(name) {
            return Ok(None);
        }
        match &self.origin {
            FunctionCallOrigin::Http { headers, .. } => {
                Ok(header_lookup(headers.as_slice(), name).cloned())
            },
            FunctionCallOrigin::Sql | FunctionCallOrigin::Topic { .. } => Ok(None),
        }
    }

    fn http_set_status(&self, status: i32) -> kalamdb_functions::Result<()> {
        let FunctionCallOrigin::Http { response, .. } = &self.origin else {
            return Err(FunctionsError::Invalid(
                "ctx.http.status is only available on HTTP-root invocations".to_string(),
            ));
        };
        if !self.is_http_root() {
            return Err(FunctionsError::Invalid(
                "nested procedures cannot mutate ctx.http".to_string(),
            ));
        }
        if !(100..=599).contains(&status) {
            return Err(FunctionsError::Invalid(format!("invalid http status {status}")));
        }
        response.lock().status = Some(status as u16);
        Ok(())
    }

    fn http_set_header(&self, name: &str, value: &str) -> kalamdb_functions::Result<()> {
        let FunctionCallOrigin::Http { response, .. } = &self.origin else {
            return Err(FunctionsError::Invalid(
                "ctx.http.header is only available on HTTP-root invocations".to_string(),
            ));
        };
        if !self.is_http_root() {
            return Err(FunctionsError::Invalid(
                "nested procedures cannot mutate ctx.http".to_string(),
            ));
        }
        if is_blocked_response_header(name) {
            return Err(FunctionsError::Invalid(format!(
                "response header '{name}' is not allowed"
            )));
        }
        let max = self
            .app
            .function_runtime()
            .engine()
            .map(|engine| engine.config().max_header_bytes)
            .unwrap_or(16 * 1024);
        if name.len().saturating_add(value.len()) > max {
            return Err(FunctionsError::ResourceLimit("http header".into()));
        }
        response.lock().headers.insert(name.to_string(), value.to_string());
        Ok(())
    }

    fn http_enabled(&self) -> bool {
        matches!(self.origin, FunctionCallOrigin::Http { .. })
    }

    fn http_method(&self) -> String {
        match &self.origin {
            FunctionCallOrigin::Http { method, .. } => method.clone(),
            _ => String::new(),
        }
    }

    fn http_path(&self) -> String {
        match &self.origin {
            FunctionCallOrigin::Http { path, .. } => path.clone(),
            _ => String::new(),
        }
    }

    fn http_query(&self, name: &str) -> kalamdb_functions::Result<Option<String>> {
        match &self.origin {
            FunctionCallOrigin::Http { query, .. } => {
                Ok(header_lookup(query.as_slice(), name).cloned())
            },
            _ => Ok(None),
        }
    }

    fn is_http_root(&self) -> bool {
        matches!(self.origin, FunctionCallOrigin::Http { .. })
            && self.session.as_ref().stack.len() <= 1
    }

    fn invocation_source(&self) -> kalamdb_functions::InvocationSource {
        match &self.origin {
            FunctionCallOrigin::Topic {
                topic_name,
                event_id,
                partition,
                offset,
                attempt,
            } => kalamdb_functions::InvocationSource::Topic {
                topic_name: topic_name.clone(),
                event_id:   event_id.clone(),
                partition:  *partition,
                offset:     *offset,
                attempt:    *attempt,
            },
            FunctionCallOrigin::Sql | FunctionCallOrigin::Http { .. } => {
                kalamdb_functions::InvocationSource::Call
            },
        }
    }

    fn parent_procedure(&self) -> Option<String> {
        let session = self.session.as_ref();
        let len = session.stack.len();
        if len < 2 {
            return None;
        }
        session.stack.get(len - 2).map(ProcedureFrame::stack_label)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blocked_request_headers_are_hidden() {
        assert!(is_blocked_request_header("Authorization"));
        assert!(is_blocked_request_header("proxy-authorization"));
        assert!(is_blocked_request_header("Cookie"));
        assert!(!is_blocked_request_header("x-client-version"));
    }

    #[test]
    fn blocked_response_headers_are_rejected() {
        assert!(is_blocked_response_header("Connection"));
        assert!(is_blocked_response_header("Transfer-Encoding"));
        assert!(is_blocked_response_header("Content-Length"));
        assert!(is_blocked_response_header("Host"));
        assert!(!is_blocked_response_header("x-kalam-trace"));
    }
}

pub(super) fn resolve_routine_id(name: &str, default_ns: &NamespaceId) -> RoutineId {
    let trimmed = name.trim();
    if let Some((schema, rest)) = trimmed.split_once('.') {
        RoutineId::from_parts(Some(&NamespaceId::new(schema)), rest)
    } else {
        RoutineId::from_parts(Some(default_ns), trimmed)
    }
}

fn header_lookup<'a>(headers: &'a [(String, String)], name: &str) -> Option<&'a String> {
    headers
        .iter()
        .find(|(key, _)| key.eq_ignore_ascii_case(name))
        .map(|(_, value)| value)
}

fn is_blocked_request_header(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        "authorization" | "proxy-authorization" | "cookie"
    )
}

fn is_blocked_response_header(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        "connection" | "transfer-encoding" | "content-length" | "host"
    )
}

fn map_core(error: crate::error::KalamDbError) -> FunctionsError {
    FunctionsError::Invalid(error.to_string())
}

fn annotate(stack: String, message: String) -> FunctionsError {
    if stack.is_empty() {
        FunctionsError::Invalid(message)
    } else {
        FunctionsError::Invalid(format!("{stack}: {message}"))
    }
}

pub(super) fn push_frame(
    session: &HostSession,
    frame: ProcedureFrame,
) -> Result<HostSession, crate::error::KalamDbError> {
    if session.stack.len() >= MAX_PROCEDURE_DEPTH {
        return Err(crate::error::KalamDbError::from(FunctionsError::ResourceLimit(
            "procedure call depth".into(),
        )));
    }
    let mut next = session.clone();
    next.stack.push(frame);
    Ok(next)
}

pub(super) fn frame_principal(
    security: RoutineSecurityMode,
    caller_user: UserId,
    caller_role: Role,
    owner: UserId,
    owner_role: Role,
) -> (UserId, Role) {
    match security {
        RoutineSecurityMode::Invoker => (caller_user, caller_role),
        RoutineSecurityMode::Definer => (owner, owner_role),
    }
}
