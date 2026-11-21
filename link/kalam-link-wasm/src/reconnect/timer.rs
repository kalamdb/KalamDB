use std::{future::Future, pin::pin, time::Duration};

use futures_util::future::{select, Either};
use js_sys::Function;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;

use crate::helpers::create_promise;

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_name = setTimeout)]
    fn set_timeout(callback: &Function, delay: i32) -> JsValue;
    #[wasm_bindgen(js_name = clearTimeout)]
    fn clear_timeout(handle: &JsValue);
    #[wasm_bindgen(js_name = setInterval)]
    fn set_interval(callback: &Function, delay: i32) -> JsValue;
    #[wasm_bindgen(js_name = clearInterval)]
    fn clear_interval(handle: &JsValue);
}

pub(super) struct Timer {
    handle:    JsValue,
    repeating: bool,
}

impl Timer {
    pub(super) fn interval(callback: &Function, delay: Duration) -> Self {
        Self {
            handle:    set_interval(callback, timer_ms(delay)),
            repeating: true,
        }
    }

    fn timeout(callback: &Function, delay: Duration) -> Self {
        Self {
            handle:    set_timeout(callback, timer_ms(delay)),
            repeating: false,
        }
    }

    pub(super) async fn sleep(delay: Duration) {
        let (value, resolve, _) = create_promise();
        let _timer = Self::timeout(&resolve, delay);
        let _ = JsFuture::from(value).await;
    }
}

impl Drop for Timer {
    fn drop(&mut self) {
        if self.repeating {
            clear_interval(&self.handle);
        } else {
            clear_timeout(&self.handle);
        }
    }
}

fn timer_ms(duration: Duration) -> i32 {
    duration.as_millis().min(i32::MAX as u128) as i32
}

pub(super) async fn within<T>(
    operation: impl Future<Output = Result<T, JsValue>>,
    timeout: Option<(Duration, &str)>,
) -> Result<T, JsValue> {
    let Some((duration, message)) = timeout else {
        return operation.await;
    };
    let (deadline, resolve, _) = create_promise();
    let _timer = Timer::timeout(&resolve, duration);
    match select(pin!(operation), JsFuture::from(deadline)).await {
        Either::Left((result, _)) => result,
        Either::Right(_) => Err(JsValue::from_str(message)),
    }
}
