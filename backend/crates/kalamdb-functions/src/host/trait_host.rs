//! Host callbacks injected into a V8 isolate. Implemented by `kalamdb-core`.

use std::{
    future::Future,
    pin::Pin,
    time::Duration,
};

use datafusion_common::ScalarValue;

use crate::{error::Result, value::RoutineValue, InvocationMetadata};

pub type HostFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T>> + Send + 'a>>;

/// Host-created invocation metadata. Callers cannot forge this.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InvocationSource {
    Call,
    Topic {
        topic_name: String,
        event_id:   String,
        partition:  u32,
        offset:     u64,
        attempt:    u32,
    },
}

/// Synchronous host surface used by `ctx.db` / `ctx.functions` / `ctx.topics` / `ctx.http`.
///
/// Nested work must not re-enter the calling isolate. Core runs host methods
/// via `Handle::block_on` from the V8 worker thread.
pub trait FunctionHost: Send + Sync {
    fn sql(&self, sql: &str, params: &[RoutineValue]) -> Result<RoutineValue>;
    fn call(&self, procedure: &str, args: &[RoutineValue]) -> Result<RoutineValue>;
    fn publish(&self, topic: &str, payload: &RoutineValue) -> Result<()>;
    fn http_request_header(&self, name: &str) -> Result<Option<String>>;
    fn http_set_status(&self, status: i32) -> Result<()>;
    fn http_set_header(&self, name: &str, value: &str) -> Result<()>;
    fn is_http_root(&self) -> bool;
    fn query(&self, sql: String, params: Vec<RoutineValue>) -> HostFuture<'_, RoutineValue> {
        Box::pin(async move { self.sql(&sql, &params) })
    }
    fn execute(&self, sql: String, params: Vec<RoutineValue>) -> HostFuture<'_, RoutineValue> {
        self.query(sql, params)
    }
    fn call_async(
        &self,
        procedure: String,
        args: Vec<RoutineValue>,
    ) -> HostFuture<'_, RoutineValue> {
        Box::pin(async move { self.call(&procedure, &args) })
    }
    fn publish_async(&self, topic: String, payload: RoutineValue) -> HostFuture<'_, ()> {
        Box::pin(async move { self.publish(&topic, &payload) })
    }
    fn sleep(&self, ms: f64) -> HostFuture<'_, RoutineValue> {
        Box::pin(async move {
            let millis = ms.max(0.0).min(60_000.0) as u64;
            if millis > 0 {
                tokio::time::sleep(Duration::from_millis(millis)).await;
            }
            Ok(RoutineValue::new(ScalarValue::Null))
        })
    }
    fn metadata(&self) -> Option<InvocationMetadata> {
        None
    }
    fn log(&self, record: crate::HostLogRecord) -> Result<()> {
        crate::host::emit_function_log(self, record)
    }
    fn max_log_bytes(&self) -> usize {
        64 * 1024
    }
    fn procedure_stack(&self) -> String {
        String::new()
    }
    fn routine_js_map(&self) -> String {
        "{}".to_string()
    }
    fn invocation_source(&self) -> InvocationSource {
        InvocationSource::Call
    }
    fn parent_procedure(&self) -> Option<String> {
        None
    }
    fn http_enabled(&self) -> bool {
        false
    }
    fn http_method(&self) -> String {
        String::new()
    }
    fn http_path(&self) -> String {
        String::new()
    }
    fn http_query(&self, _name: &str) -> Result<Option<String>> {
        Ok(None)
    }
    /// Same-isolate nested CALL. `None` falls back to [`Self::call_async`].
    fn prepare_nested_call(
        &self,
        _procedure: String,
        _args: Vec<RoutineValue>,
    ) -> Result<Option<(crate::Invocation, std::sync::Arc<dyn FunctionHost>)>> {
        Ok(None)
    }
}
