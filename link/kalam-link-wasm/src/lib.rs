//! Browser WASM bindings for `@kalamdb/client`.
//!
//! Protocol types, subscription materialization, compression, and auth models
//! live in [`link_common`]. This crate owns only the browser transport layer
//! (`wasm-bindgen`, `fetch`, and `WebSocket`).

use wasm_bindgen::prelude::*;

mod wasm_auth;
mod client;
mod file_ref;
mod helpers;
mod reconnect;
mod state;
mod wasm_timestamp;
mod validation;

pub use client::KalamClient;
pub use file_ref::{
    file_ref_download_url, file_ref_relative_path, file_ref_relative_url, file_ref_stored_name,
};
pub use wasm_timestamp::{parse_iso8601, timestamp_now, WasmTimestampFormatter};

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = console)]
    fn log(s: &str);
}

macro_rules! wasm_debug_log {
    (&format!($fmt:literal $(, $args:expr)* $(,)?)) => {{
        #[cfg(all(target_arch = "wasm32", debug_assertions))]
        $crate::log(&format!($fmt $(, $args)*));
    }};
    ($message:expr $(,)?) => {{
        #[cfg(all(target_arch = "wasm32", debug_assertions))]
        $crate::log($message);
    }};
}

pub(crate) use wasm_debug_log;

#[allow(dead_code)]
#[inline(always)]
pub(crate) fn console_log(message: &str) {
    #[cfg(all(target_arch = "wasm32", debug_assertions))]
    log(message);

    #[cfg(any(not(target_arch = "wasm32"), not(debug_assertions)))]
    let _ = message;
}

pub use link_common::*;
