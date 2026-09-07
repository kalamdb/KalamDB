//! Host API spec and TypeScript / JS emitters for KalamDB functions `ctx`.
//!
//! This crate has no V8 dependency so the CLI can generate `runtime.d.ts`
//! from the same table the isolate bootstrap uses.

mod emit_js;
mod emit_ts;
mod idents;
mod routine_map;
mod spec;

pub use emit_js::emit_js_bootstrap;
pub use emit_ts::{emit_typed_functions_host, emit_typescript, TypedRoutine};
pub use idents::{
    camel_case, method_ident, pascal_case, sanitize_js_ident, schema_object_ident, DEFAULT_SCHEMA,
};
pub use routine_map::build_routine_js_map;
pub use spec::{is_async_op, HostDispatch, HostMethod, ASYNC_OPS, HOST_METHODS, NATIVE_FNS};
