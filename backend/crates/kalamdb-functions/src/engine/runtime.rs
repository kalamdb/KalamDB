//! Runtime surface consumed by `kalamdb-core`. Core must not import `V8Session`.

use std::sync::Arc;

use crate::{host::HostFuture, FunctionHost, Invocation, ModuleRevision, RoutineValue};

/// Engine operations needed by the SQL CALL path without V8 types.
pub trait FunctionRuntime: Send + Sync {
    fn load_module(&self, revision: Arc<ModuleRevision>) -> HostFuture<'_, Arc<ModuleRevision>>;
    fn invoke_root(
        &self,
        invocation: Invocation,
        host: Arc<dyn FunctionHost>,
    ) -> HostFuture<'_, RoutineValue>;
    fn terminate(&self);
}
