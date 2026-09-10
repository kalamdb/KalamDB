//! Native capabilities resolve through their creation context, not the currently driven procedure.

use std::{
    rc::Rc,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
};

use super::host_frame::HostFrame;
use crate::FunctionHost;

static NEXT_GENERATION: AtomicU64 = AtomicU64::new(1);
pub(crate) struct HostFrames {
    hosts:      Vec<Arc<dyn FunctionHost>>,
    generation: u64,
}
impl Default for HostFrames {
    fn default() -> Self {
        Self {
            hosts:      Vec::new(),
            generation: NEXT_GENERATION.fetch_add(1, Ordering::Relaxed),
        }
    }
}

impl HostFrames {
    pub(crate) fn bind(scope: &mut v8::PinScope, host: Arc<dyn FunctionHost>) {
        if scope.get_slot::<Self>().is_none() {
            scope.set_slot(Self::default());
        }
        let frames = scope.get_slot_mut::<Self>().expect("host frames initialized");
        let id = frames.hosts.len();
        frames.hosts.push(host);
        let generation = frames.generation;
        scope.get_current_context().set_slot(Rc::new(HostFrame {
            index: id,
            generation,
        }));
    }

    pub(crate) fn current(scope: &v8::PinScope) -> Option<Arc<dyn FunctionHost>> {
        let frame = scope.get_current_context().get_slot::<HostFrame>()?;
        let frames = scope.get_slot::<Self>()?;
        if frames.generation != frame.generation {
            return None;
        }
        frames.hosts.get(frame.index).cloned()
    }
}
