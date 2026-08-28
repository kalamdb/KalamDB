//! Shared Rust implementation used by `kalam-client`, `kalam-link-wasm`,
//! `kalam-consumer-wasm`, and `kalam-link-dart`.

#[cfg(feature = "client-core")]
pub mod auth;
#[cfg(feature = "tokio-runtime")]
pub mod client;
#[cfg(feature = "client-core")]
pub mod compression;
#[cfg(feature = "client-core")]
pub(crate) mod connection;
#[cfg(feature = "consumer")]
pub mod consumer;
#[cfg(feature = "client-core")]
pub mod credentials;
#[cfg(feature = "client-core")]
pub mod error;
#[cfg(feature = "client-core")]
pub mod event_handlers;
#[path = "models/mod.rs"]
pub mod models;
#[cfg(feature = "client-core")]
pub mod query;
#[cfg(feature = "client-core")]
pub mod seq_tracking;
#[cfg(feature = "client-core")]
pub mod subscription;
#[cfg(feature = "client-core")]
pub mod timeouts;
#[cfg(feature = "client-core")]
pub mod timestamp;

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
#[cfg(feature = "client-core")]
pub use credentials::{CredentialStore, Credentials, MemoryCredentialStore};
#[cfg(feature = "client-core")]
pub use error::{KalamLinkError, Result};
#[cfg(feature = "client-core")]
pub use event_handlers::{ConnectionError, DisconnectReason, EventHandlers, MessageDirection};
#[cfg(feature = "client-core")]
pub use kalamdb_commons::ids::SeqId;
#[cfg(feature = "client-core")]
pub use kalamdb_commons::Role;
pub use kalamdb_commons::{TableId, UserId};
pub use models::{
    parse_i64, BoundFileRef, FieldFlag, FieldFlags, FileDownload, FileRef, FileRefContext,
    FileUpload, KalamCellValue, KalamDataType, RowData, SchemaField,
};
#[cfg(feature = "consumer")]
pub use models::{AckResponse, ConsumeMessage, ConsumeRequest, ConsumeResponse};
#[cfg(feature = "client-core")]
pub use models::{
    ChangeEvent, ClusterHealthResponse, ClusterNodeHealth, ConnectionOptions, ErrorDetail,
    HealthCheckResponse, HttpVersion, LoginRequest, LoginResponse, LoginUserInfo, QueryRequest,
    QueryResponse, QueryResult, ServerSetupRequest, ServerSetupResponse, SetupStatusResponse,
    SetupUserInfo, SqlSubscriptionDescriptor, SqlSubscriptionRow, SqlSubscriptionStatus,
    SubscriptionAckMode, SubscriptionConfig, SubscriptionInfo, SubscriptionOptions, UploadProgress,
};
#[cfg(feature = "client-core")]
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
#[cfg(feature = "client-core")]
pub use subscription::{LiveRowsConfig, LiveRowsEvent, LiveRowsMaterializer};
#[cfg(feature = "client-core")]
pub use timeouts::{KalamLinkTimeouts, KalamLinkTimeoutsBuilder};
#[cfg(feature = "client-core")]
pub use timestamp::{now, parse_iso8601, TimestampFormat, TimestampFormatter};

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
