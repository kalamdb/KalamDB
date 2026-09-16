/// Realm-local capability identity. Generation prevents stale realms resolving a new caller.
pub(crate) struct HostFrame {
    pub index:      usize,
    pub generation: u64,
}
