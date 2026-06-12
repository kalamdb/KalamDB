//! Data models shared by the KalamDB SDK.
//!
//! Core shared types (cell values, schema, file references) live here.
//! Domain-specific models are organized in their respective modules:
//! - Auth:         [`crate::auth::models`]
//! - Connection:   [`crate::connection::models`]
//! - Consumer:     [`crate::consumer::models`]
//! - Query:        [`crate::query::models`]
//! - Subscription: [`crate::subscription::models`]

// ── Core shared types (defined here) ────────────────────────────────────────
pub mod file_download;
pub mod file_ref;
pub mod file_upload;
pub mod kalam_cell_value;
pub mod utils;

#[cfg(test)]
mod tests;

pub use file_download::FileDownload;
pub use file_ref::{BoundFileRef, FileRef, FileRefContext};
pub use file_upload::FileUpload;
pub use kalam_cell_value::{KalamCellValue, RowData};
pub use kalamdb_commons::{FieldFlag, FieldFlags, KalamDataType, SchemaField, UserId};
pub use utils::parse_i64;

// ── Auth models ──────────────────────────────────────────────────────────────
#[cfg(any(feature = "tokio-runtime", feature = "wasm"))]
pub use crate::auth::models::{
    LoginRequest, LoginResponse, LoginUserInfo, ServerSetupRequest, ServerSetupResponse,
    SetupStatusResponse, SetupUserInfo, WsAuthCredentials,
};
// ── Connection models ────────────────────────────────────────────────────────
#[cfg(any(feature = "tokio-runtime", feature = "wasm"))]
pub use crate::connection::models::{
    ClientMessage, ClusterHealthResponse, ClusterNodeHealth, CompressionType, ConnectionOptions,
    HealthCheckResponse, HttpVersion, ProtocolOptions, SerializationType, ServerMessage,
};
// ── Consumer models ──────────────────────────────────────────────────────────
#[cfg(feature = "consumer")]
pub use crate::consumer::models::{AckResponse, ConsumeMessage, ConsumeRequest, ConsumeResponse};
// ── Query models ─────────────────────────────────────────────────────────────
#[cfg(any(feature = "tokio-runtime", feature = "wasm"))]
pub use crate::query::models::{
    ErrorDetail, QueryRequest, QueryResponse, QueryResult, ResponseStatus,
    SqlSubscriptionDescriptor, SqlSubscriptionRow, SqlSubscriptionStatus, UploadProgress,
};
// ── Subscription models ──────────────────────────────────────────────────────
#[cfg(any(feature = "tokio-runtime", feature = "wasm"))]
pub use crate::subscription::models::{
    BatchControl, BatchStatus, ChangeEvent, ChangeTypeRaw, SubscriptionConfig, SubscriptionInfo,
    SubscriptionOptions, SubscriptionRequest,
};
