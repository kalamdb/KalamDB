//! Context-independent scripts, owned by one isolate and reused in fresh realms.

pub(crate) struct CompiledScripts {
    pub sandbox:   v8::Global<v8::UnboundScript>,
    pub bootstrap: v8::Global<v8::UnboundScript>,
    pub module:    v8::Global<v8::UnboundScript>,
}
