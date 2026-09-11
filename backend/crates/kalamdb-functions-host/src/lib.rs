//! Host API spec and JS isolate bootstrap for KalamDB functions `ctx`.

mod emit_js;
mod idents;
mod routine_map;
mod spec;

pub use emit_js::emit_js_bootstrap;
pub use idents::{
    camel_case, method_ident, namespace_object_ident, pascal_case, sanitize_js_ident,
    DEFAULT_NAMESPACE,
};
pub use routine_map::build_routine_js_map;
pub use spec::{is_async_op, HostDispatch, HostMethod, ASYNC_OPS, HOST_METHODS, NATIVE_FNS};
