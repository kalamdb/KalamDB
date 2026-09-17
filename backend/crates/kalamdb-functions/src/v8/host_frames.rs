//! Native capabilities resolve through their creation realm, not the currently driven procedure.
//!
//! Frame identity is stored as a private property on the realm's global object. That keeps
//! parent and nested CALL hosts distinct when V8 runs parent microtasks during a nested
//! checkpoint. `Context::set_slot` is not used: rust-v8's context annex holds a
//! `Weak<Context>` whose first-pass callback aborts the process (`Option::unwrap` on `None`)
//! when GC races isolate `exit`/`enter` during async host work.

use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc,
};

use super::host_frame::HostFrame;
use crate::FunctionHost;

const FRAME_KEY: &str = "kalamdb#HostFrame";

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
        write_frame(
            scope,
            HostFrame {
                index: id,
                generation,
            },
        );
    }

    pub(crate) fn current(scope: &v8::PinScope) -> Option<Arc<dyn FunctionHost>> {
        let frame = read_frame(scope)?;
        let frames = scope.get_slot::<Self>()?;
        if frames.generation != frame.generation {
            return None;
        }
        frames.hosts.get(frame.index).cloned()
    }

    pub(crate) fn has_expired_frame(scope: &v8::PinScope) -> bool {
        read_frame(scope).is_some_and(|frame| {
            scope
                .get_slot::<Self>()
                .is_none_or(|frames| frames.generation != frame.generation)
        })
    }
}

fn frame_key<'s>(scope: &v8::PinScope<'s, '_>) -> Option<v8::Local<'s, v8::Private>> {
    let name = v8::String::new(scope, FRAME_KEY)?;
    Some(v8::Private::for_api(scope, Some(name)))
}

fn write_frame(scope: &mut v8::PinScope, frame: HostFrame) {
    let Some(key) = frame_key(scope) else {
        return;
    };
    let payload = v8::Array::new(scope, 2);
    let index = v8::Number::new(scope, frame.index as f64);
    let generation = v8::Number::new(scope, frame.generation as f64);
    payload.set_index(scope, 0, index.into());
    payload.set_index(scope, 1, generation.into());
    let global = scope.get_current_context().global(scope);
    global.set_private(scope, key, payload.into());
}

fn read_frame(scope: &v8::PinScope) -> Option<HostFrame> {
    let key = frame_key(scope)?;
    let global = scope.get_current_context().global(scope);
    let value = global.get_private(scope, key)?;
    let payload = v8::Local::<v8::Array>::try_from(value).ok()?;
    let index = payload.get_index(scope, 0)?.number_value(scope)? as usize;
    let generation = payload.get_index(scope, 1)?.number_value(scope)? as u64;
    Some(HostFrame { index, generation })
}
