use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc,
};

/// Marks a membership table as unstable while a logical mutation is visible.
pub struct AuthorizationMutationGuard {
    generation:       Arc<AtomicU64>,
    active_mutations: Arc<AtomicU64>,
}

impl AuthorizationMutationGuard {
    pub fn begin(generation: Arc<AtomicU64>, active_mutations: Arc<AtomicU64>) -> Self {
        active_mutations.fetch_add(1, Ordering::SeqCst);
        generation.fetch_add(1, Ordering::SeqCst);
        Self {
            generation,
            active_mutations,
        }
    }
}

impl Drop for AuthorizationMutationGuard {
    fn drop(&mut self) {
        self.generation.fetch_add(1, Ordering::SeqCst);
        self.active_mutations.fetch_sub(1, Ordering::SeqCst);
    }
}
