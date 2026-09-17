//! Invocation-owned rejection tracking, including detached Promise chains.

use super::adapter::format_js_exception;
use crate::{FunctionsError, Result};

#[derive(Default)]
pub(crate) struct Rejections {
    promises: Vec<v8::Global<v8::Promise>>,
    overflow: bool,
}

impl Rejections {
    pub(crate) fn check(&self, scope: &v8::PinScope) -> Result<()> {
        if self.overflow {
            return Err(FunctionsError::Javascript("unhandled Promise rejection overflow".into()));
        }
        for promise in &self.promises {
            let local = v8::Local::new(scope, promise);
            if local.has_handler() {
                continue;
            }
            let reason = if local.state() == v8::PromiseState::Rejected {
                format_js_exception(scope, local.result(scope))
            } else {
                "unhandled Promise rejection".to_string()
            };
            return Err(FunctionsError::Javascript(format!(
                "unhandled Promise rejection: {reason}"
            )));
        }
        Ok(())
    }
}

pub(crate) extern "C" fn rejected(message: v8::PromiseRejectMessage) {
    // SAFETY: this scope is entered only by V8's rejection callback.
    v8::callback_scope!(unsafe scope, &message);
    let promise = v8::Global::new(scope, message.get_promise());
    let Some(state) = scope.get_slot_mut::<Rejections>() else {
        return;
    };
    match message.get_event() {
        v8::PromiseRejectEvent::PromiseRejectWithNoHandler => {
            if state.promises.len() < 1024 {
                state.promises.push(promise);
            } else {
                state.overflow = true;
            }
        },
        v8::PromiseRejectEvent::PromiseHandlerAddedAfterReject => {
            state.promises.retain(|other| other != &promise);
        },
        _ => {},
    }
}
