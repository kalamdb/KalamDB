//! Shared Rust implementation used by `kalam-client`, `kalam-consumer-wasm`,
//! and `kalam-link-dart`.

#[cfg(any(feature = "tokio-runtime", feature = "wasm"))]
pub mod auth;
#[cfg(feature = "tokio-runtime")]
pub mod client;
#[cfg(any(feature = "tokio-runtime", feature = "wasm"))]
pub mod compression;
#[cfg(any(feature = "tokio-runtime", feature = "wasm"))]
pub(crate) mod connection;
#[cfg(feature = "consumer")]
pub mod consumer;
#[cfg(any(feature = "tokio-runtime", feature = "wasm"))]
pub mod credentials;
#[cfg(any(feature = "tokio-runtime", feature = "wasm"))]
pub mod error;
#[cfg(any(feature = "tokio-runtime", feature = "wasm"))]
pub mod event_handlers;
#[path = "models/mod.rs"]
pub mod models;
#[cfg(any(feature = "tokio-runtime", feature = "wasm"))]
pub mod query;
#[cfg(any(feature = "tokio-runtime", feature = "wasm"))]
pub mod seq_tracking;
#[cfg(any(feature = "tokio-runtime", feature = "wasm"))]
pub mod subscription;
#[cfg(any(feature = "tokio-runtime", feature = "wasm"))]
pub mod timeouts;
#[cfg(any(feature = "tokio-runtime", feature = "wasm"))]
pub mod timestamp;
#[cfg(feature = "wasm")]
#[path = "wasm/mod.rs"]
pub mod wasm;

#[cfg(feature = "tokio-runtime")]
pub use auth::{ArcDynAuthProvider, AuthProvider, DynamicAuthProvider, ResolvedAuth};
#[cfg(feature = "tokio-runtime")]
pub use client::KalamLinkClient;
#[cfg(all(feature = "tokio-runtime", feature = "consumer"))]
pub use consumer::ConsumerConfig;
#[cfg(feature = "consumer")]
pub use consumer::{
    AutoOffsetReset, CommitMode, CommitResult, ConsumerOffsets, ConsumerRecord, PayloadMode,
    TopicOp,
};
#[cfg(all(feature = "tokio-runtime", feature = "consumer"))]
pub use consumer::{ConsumerBuilder, TopicConsumer};
#[cfg(any(feature = "tokio-runtime", feature = "wasm"))]
pub use credentials::{CredentialStore, Credentials, MemoryCredentialStore};
#[cfg(any(feature = "tokio-runtime", feature = "wasm"))]
pub use error::{KalamLinkError, Result};
#[cfg(any(feature = "tokio-runtime", feature = "wasm"))]
pub use event_handlers::{ConnectionError, DisconnectReason, EventHandlers, MessageDirection};
#[cfg(any(feature = "tokio-runtime", feature = "wasm"))]
pub use kalamdb_commons::ids::SeqId;
#[cfg(any(feature = "tokio-runtime", feature = "wasm"))]
pub use kalamdb_commons::Role;
pub use kalamdb_commons::TableId;
pub use kalamdb_commons::UserId;
pub use models::{
    parse_i64, BoundFileRef, FieldFlag, FieldFlags, FileDownload, FileRef, FileRefContext,
    FileUpload, KalamCellValue, KalamDataType, RowData, SchemaField,
};
#[cfg(feature = "consumer")]
pub use models::{AckResponse, ConsumeMessage, ConsumeRequest, ConsumeResponse};
#[cfg(any(feature = "tokio-runtime", feature = "wasm"))]
pub use models::{
    ChangeEvent, ClusterHealthResponse, ClusterNodeHealth, ConnectionOptions, ErrorDetail,
    HealthCheckResponse, HttpVersion, LoginRequest, LoginResponse, LoginUserInfo, QueryRequest,
    QueryResponse, QueryResult, ServerSetupRequest, ServerSetupResponse, SetupStatusResponse,
    SetupUserInfo, SqlSubscriptionDescriptor, SqlSubscriptionRow, SqlSubscriptionStatus,
    SubscriptionConfig, SubscriptionInfo, SubscriptionOptions, UploadProgress,
};
#[cfg(any(feature = "tokio-runtime", feature = "wasm"))]
pub use query::models::QueryParam;
#[cfg(feature = "tokio-runtime")]
pub use query::AuthRefreshCallback;
#[cfg(feature = "tokio-runtime")]
pub use query::QueryExecutor;
#[cfg(feature = "tokio-runtime")]
pub use query::UploadProgressCallback;
#[cfg(feature = "tokio-runtime")]
pub use subscription::LiveRowsSubscription;
#[cfg(feature = "tokio-runtime")]
pub use subscription::SubscriptionManager;
#[cfg(any(feature = "tokio-runtime", feature = "wasm"))]
pub use subscription::{LiveRowsConfig, LiveRowsEvent, LiveRowsMaterializer};
#[cfg(any(feature = "tokio-runtime", feature = "wasm"))]
pub use timeouts::{KalamLinkTimeouts, KalamLinkTimeoutsBuilder};
#[cfg(any(feature = "tokio-runtime", feature = "wasm"))]
pub use timestamp::{now, parse_iso8601, TimestampFormat, TimestampFormatter};

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
