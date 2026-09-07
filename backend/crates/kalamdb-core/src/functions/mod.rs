//! SQL procedure CALL runtime (Wave 3 F7–F9).

mod acl;
mod convert;
mod dispatcher;
mod executor;
mod host;

pub use convert::{bind_call_arguments, json_to_routine_value};
pub use dispatcher::{dispatch_once, start_trigger_dispatcher, TriggerDispatcherRuntime};
pub use executor::{
    activate_module_artifact, function_storage, rebuild_active_function_set,
    rollback_module_revision, FunctionService,
};
pub(crate) use executor::{drop_staged_publishes, flush_staged_publishes};
pub use kalamdb_functions::{
    now_ms, ActiveFunctionSet, FunctionCallOrigin, FunctionCallResult, FunctionExecutionRoot,
    FunctionRuntimeState, HttpResponseOverrides, ImplementationRef, InlineArtifact, ProcedureSlot,
    RoutineValue, StagedTopicPublish,
};
