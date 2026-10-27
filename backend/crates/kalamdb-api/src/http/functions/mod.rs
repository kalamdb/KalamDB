//! HTTP routine calls and the current-epoch discovery endpoints.

mod capabilities;
mod contracts;
mod error;
mod idempotency;
mod invoke;
mod openapi;

pub use capabilities::capabilities_v1;
pub use contracts::contracts_manifest_v1;
pub use invoke::{invoke_function_v1, CONTENT_TYPE_KALAM_DOMAIN};
pub use openapi::openapi_v1;
