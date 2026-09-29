//! WebSocket models

mod ws_error_code;
mod ws_notification;

pub use ws_error_code::WsErrorCode;
pub use ws_notification::{ChangeType, Notification};
