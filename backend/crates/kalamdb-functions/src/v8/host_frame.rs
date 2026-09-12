/// Context-local capability identity. Generation prevents stale contexts resolving a new caller.
pub(crate) struct HostFrame {
    pub index:      usize,
    pub generation: u64,
}
