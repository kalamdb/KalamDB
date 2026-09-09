//! Function runtime ABI, V8 adapter, and revision activation.

mod abi;
mod catalog;
mod engine;
mod host;
mod v8;

pub(crate) use abi::{convert, error, hash, invocation, value};
#[cfg(feature = "catalog")]
pub use catalog::activation::FunctionActivation;
#[cfg(feature = "catalog")]
pub use catalog::active_set::{
    ActiveFunctionSet, FunctionExecutionRoot, ImplementationRef, InlineArtifact, ProcedureSlot,
};
pub(crate) use catalog::revision;
pub use catalog::revision::ModuleRevision;
#[cfg(feature = "catalog")]
pub use catalog::runtime_state::{now_ms, FunctionRuntimeState, StagedTopicPublish};
pub use engine::{
    config::EngineConfig,
    engine::FunctionEngine,
    limits::{check_host_bytes, RuntimeLimits, ABI_VERSION},
    runtime::FunctionRuntime,
};
pub(crate) use engine::{deadline, limits, runtime};
pub use error::{FunctionErrorCode, FunctionsError, Result};
pub use hash::hash_artifact_bytes;
pub use host::{
    ActorMeta, FunctionCallOrigin, FunctionCallResult, FunctionHost, HostFuture, HostLogRecord,
    HttpResponseOverrides, InvocationMetadata, InvocationSource, LogChannel, PrincipalKey,
    ProcedureFrame, ProcedureFrameStack,
};
pub use invocation::{Invocation, InvocationScope};
pub use v8::inline::{lint_inline_javascript, prepare_inline_javascript};
pub(crate) use v8::{adapter as v8_adapter, async_ops as v8_async, wrap};
pub use v8_adapter::{V8Session, FIXTURE_SOURCE};
pub use value::RoutineValue;
pub use wrap::wrap_procedure_source;
