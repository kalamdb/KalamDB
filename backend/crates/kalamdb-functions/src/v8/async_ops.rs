//! Promise-driven execution on an isolate's owning worker.

use std::sync::Arc;

use datafusion_common::ScalarValue;
use futures_util::{stream::FuturesUnordered, StreamExt};

use crate::{
    convert::{infer_v8_value, routine_to_v8, v8_to_routine},
    deadline::DeadlineGuard,
    v8_adapter::{
        arg_string, bind_ctx, bind_native, current_host, js_exception, throw_host_error, ActiveHost,
    },
    FunctionHost, FunctionsError, HostFuture, Invocation, Result, RoutineValue, V8Session,
};

type Completion = (v8::Global<v8::PromiseResolver>, Result<RoutineValue>);
type Pending = std::pin::Pin<Box<dyn std::future::Future<Output = Completion>>>;

struct NestedWork {
    invocation: Invocation,
    host:       Arc<dyn FunctionHost>,
    resolver:   v8::Global<v8::PromiseResolver>,
}

#[derive(Default)]
struct Operations {
    pending: Vec<Pending>,
    nested:  Vec<NestedWork>,
    count:   usize,
}

pub(crate) fn install(scope: &mut v8::PinScope) -> Result<()> {
    scope.set_slot(Operations::default());
    bind_native(scope, "kalamAsyncOp", host_operation)?;
    bind_native(scope, "kalamHostMetadata", metadata)?;
    bind_native(scope, "kalamHostLog", log)?;
    Ok(())
}

fn metadata(scope: &mut v8::PinScope, _: v8::FunctionCallbackArguments, mut rv: v8::ReturnValue) {
    let text = current_host(scope)
        .ok()
        .and_then(|host| host.metadata())
        .and_then(|metadata| serde_json::to_string(&metadata).ok())
        .filter(|text| text != "null")
        .unwrap_or_else(|| "{}".to_string());
    if let Some(text) = v8::String::new(scope, &text) {
        rv.set(text.into());
    }
}

fn log(scope: &mut v8::PinScope, args: v8::FunctionCallbackArguments, _: v8::ReturnValue) {
    let level = arg_string(scope, &args, 0);
    let payload = arg_string(scope, &args, 1);
    let channel = arg_string(scope, &args, 2);
    let record = crate::host::parse_log_payload(&level, &payload, &channel);
    let result = current_host(scope).and_then(|host| {
        let total = record.message.len()
            + record.error.as_ref().map(String::len).unwrap_or(0)
            + record.args_json.as_ref().map(String::len).unwrap_or(0);
        if total > host.max_log_bytes() {
            Err(FunctionsError::ResourceLimit("log message".into()))
        } else {
            host.log(record)
        }
    });
    if let Err(error) = result {
        throw_host_error(scope, error);
    }
}

fn host_operation(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue,
) {
    let result = (|| {
        let host = current_host(scope)?;
        let kind = arg_string(scope, &args, 0);
        let name = arg_string(scope, &args, 1);
        let values = v8::Local::<v8::Array>::try_from(args.get(2))
            .map_err(|_| FunctionsError::Invalid("host arguments must be an array".into()))?;
        if values.length() > 1024 {
            return Err(FunctionsError::ResourceLimit("host arguments".into()));
        }
        let mut params = Vec::with_capacity(values.length() as usize);
        for index in 0..values.length() {
            let value = values
                .get_index(scope, index)
                .ok_or_else(|| FunctionsError::Invalid("missing argument".into()))?;
            params.push(RoutineValue::new(infer_v8_value(scope, value)?));
        }
        let resolver = v8::PromiseResolver::new(scope)
            .ok_or_else(|| FunctionsError::ResourceLimit("promise".into()))?;
        let promise = resolver.get_promise(scope);
        let resolver = v8::Global::new(scope, resolver);
        let operations = scope
            .get_slot_mut::<Operations>()
            .ok_or_else(|| FunctionsError::Invalid("host operation state missing".into()))?;
        if operations.count >= 1024 {
            return Err(FunctionsError::ResourceLimit("host operations".into()));
        }
        operations.count += 1;
        if kind == "call" {
            if let Some((invocation, child)) =
                host.prepare_nested_call(name.clone(), params.clone())?
            {
                operations.nested.push(NestedWork {
                    invocation,
                    host: child,
                    resolver,
                });
                return Ok(promise);
            }
        }
        let future: HostFuture<'static, RoutineValue> = Box::pin(async move {
            if !kalamdb_functions_host::is_async_op(&kind) {
                return Err(FunctionsError::Invalid("unknown host operation".into()));
            }
            match kind.as_str() {
                "query" => host.query(name, params).await,
                "execute" => host.execute(name, params).await,
                "call" => host.call_async(name, params).await,
                "publish" => {
                    let payload = params.into_iter().next().ok_or_else(|| {
                        FunctionsError::Invalid("publish payload required".into())
                    })?;
                    host.publish_async(name, payload).await?;
                    Ok(RoutineValue::new(ScalarValue::Null))
                },
                "sleep" => {
                    let ms = match params.first().map(|value| &value.value) {
                        Some(ScalarValue::Float64(Some(value))) => *value,
                        Some(ScalarValue::Float32(Some(value))) => f64::from(*value),
                        Some(ScalarValue::Int64(Some(value))) => *value as f64,
                        Some(ScalarValue::Int32(Some(value))) => f64::from(*value),
                        Some(ScalarValue::UInt64(Some(value))) => *value as f64,
                        Some(ScalarValue::UInt32(Some(value))) => f64::from(*value),
                        _ => 0.0,
                    };
                    host.sleep(ms).await
                },
                other => {
                    Err(FunctionsError::Invalid(format!("host operation '{other}' is not wired")))
                },
            }
        });
        operations.pending.push(Box::pin(async move { (resolver, future.await) }));
        Ok(promise)
    })();
    match result {
        Ok(promise) => rv.set(promise.into()),
        Err(error) => throw_host_error(scope, error),
    }
}

impl V8Session {
    pub(crate) async fn invoke_async(
        &mut self,
        invocation: &Invocation,
        host: Arc<dyn FunctionHost>,
    ) -> Result<RoutineValue> {
        self.attach();
        self.isolate.set_slot(ActiveHost { host: Some(host) });
        let guard = DeadlineGuard::new(
            self.isolate.thread_safe_handle(),
            invocation.scope.deadline,
            invocation.scope.cancel.clone(),
        );
        let result = self.run_async(invocation).await;
        self.attach();
        self.isolate.remove_slot::<Operations>();
        self.isolate.set_slot(ActiveHost { host: None });
        guard.check()?;
        if self.heap_limit_hit() {
            return Err(FunctionsError::MemoryLimit);
        }
        result
    }

    async fn run_async(&mut self, invocation: &Invocation) -> Result<RoutineValue> {
        invocation.scope.check()?;
        if invocation.scope.depth == 0 {
            self.reset_context()?;
        }
        self.drive(invocation).await
    }

    async fn invoke_nested_local(
        &mut self,
        invocation: Invocation,
        host: Arc<dyn FunctionHost>,
    ) -> Result<RoutineValue> {
        let previous = self
            .isolate
            .get_slot_mut::<ActiveHost>()
            .and_then(|slot| slot.host.replace(host));
        let result = self.drive(&invocation).await;
        if let Some(slot) = self.isolate.get_slot_mut::<ActiveHost>() {
            slot.host = previous;
        }
        result
    }

    async fn drive(&mut self, invocation: &Invocation) -> Result<RoutineValue> {
        let returned = self.start(invocation)?;
        let mut pending = FuturesUnordered::<Pending>::new();
        loop {
            invocation.scope.check()?;
            self.isolate.perform_microtask_checkpoint();
            let mut nested = Vec::new();
            if let Some(operations) = self.isolate.get_slot_mut::<Operations>() {
                pending.extend(operations.pending.drain(..));
                nested = std::mem::take(&mut operations.nested);
            }
            let settled = self.poll_return(&returned, invocation)?;
            if pending.is_empty() && nested.is_empty() {
                return settled.ok_or_else(|| {
                    FunctionsError::Javascript(
                        "procedure Promise cannot settle: no pending host operations".into(),
                    )
                });
            }
            for call in nested {
                let result = Box::pin(self.invoke_nested_local(call.invocation, call.host)).await;
                self.complete(call.resolver, result)?;
            }
            if pending.is_empty() {
                continue;
            }
            self.detach();
            let completion = tokio::select! {
                biased;
                _ = invocation.scope.cancel.cancelled() => Err(FunctionsError::Cancelled),
                _ = tokio::time::sleep_until(invocation.scope.deadline.into()) => Err(FunctionsError::Timeout),
                next = pending.next() => next.ok_or_else(|| FunctionsError::Invalid("host completion missing".into())),
            };
            self.attach();
            let (resolver, result) = completion?;
            // Remaining host futures drop here before the caller can roll back.
            self.complete(resolver, result)?;
        }
    }

    fn start(&mut self, invocation: &Invocation) -> Result<v8::Global<v8::Value>> {
        v8::scope!(let scope, &mut self.isolate);
        let context = v8::Local::new(scope, &self.context);
        let mut scope = v8::ContextScope::new(scope, context);
        bind_ctx(&mut scope)?;
        v8::tc_scope!(let scope, &mut scope);
        let key = v8::String::new(scope, "kalamInvoke").unwrap();
        let function = context
            .global(scope)
            .get(scope, key.into())
            .and_then(|v| v8::Local::<v8::Function>::try_from(v).ok())
            .ok_or_else(|| FunctionsError::Invalid("artifact must export kalamInvoke".into()))?;
        let name = v8::String::new(scope, invocation.routine_id.as_str())
            .ok_or_else(|| FunctionsError::Invalid("procedure name".into()))?;
        let args = v8::Array::new(scope, invocation.args.len() as i32);
        for (index, value) in invocation.args.iter().enumerate() {
            let value = routine_to_v8(scope, value)?;
            args.set_index(scope, index as u32, value)
                .ok_or_else(|| FunctionsError::Invalid("procedure argument".into()))?;
        }
        let recv = v8::undefined(scope).into();
        let value = function
            .call(scope, recv, &[name.into(), args.into()])
            .ok_or_else(|| js_exception(scope))?;
        Ok(v8::Global::new(scope, value))
    }

    fn poll_return(
        &mut self,
        returned: &v8::Global<v8::Value>,
        invocation: &Invocation,
    ) -> Result<Option<RoutineValue>> {
        v8::scope!(let scope, &mut self.isolate);
        let context = v8::Local::new(scope, &self.context);
        let scope = &mut v8::ContextScope::new(scope, context);
        let mut value = v8::Local::new(scope, returned);
        if let Ok(promise) = v8::Local::<v8::Promise>::try_from(value) {
            match promise.state() {
                v8::PromiseState::Pending => return Ok(None),
                v8::PromiseState::Rejected => {
                    return Err(FunctionsError::Javascript(
                        promise.result(scope).to_rust_string_lossy(scope),
                    ))
                },
                v8::PromiseState::Fulfilled => value = promise.result(scope),
            }
        }
        match &invocation.return_template {
            Some(template) => v8_to_routine(scope, value, template).map(Some),
            None => infer_v8_value(scope, value).map(|value| Some(RoutineValue::new(value))),
        }
    }

    fn complete(
        &mut self,
        resolver: v8::Global<v8::PromiseResolver>,
        result: Result<RoutineValue>,
    ) -> Result<()> {
        v8::scope!(let scope, &mut self.isolate);
        let context = v8::Local::new(scope, &self.context);
        let scope = &mut v8::ContextScope::new(scope, context);
        let resolver = v8::Local::new(scope, resolver);
        match result {
            Ok(value) => {
                let value = routine_to_v8(scope, &value)?;
                resolver.resolve(scope, value);
            },
            Err(error) => {
                let message = v8::String::new(scope, &error.to_string())
                    .ok_or_else(|| FunctionsError::ResourceLimit("host error".into()))?;
                let error = v8::Exception::error(scope, message);
                resolver.reject(scope, error);
            },
        }
        Ok(())
    }
}
