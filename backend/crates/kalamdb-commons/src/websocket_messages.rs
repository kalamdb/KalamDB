use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::{
    ids::{VersionDomain, VersionId},
    models::{KalamCellValue, Role, SchemaField, UserId},
    websocket_protocol::ProtocolOptions,
};

/// Type of change that occurred in the database.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ChangeTypeRaw {
    /// New row(s) inserted.
    Insert,
    /// Existing row(s) updated.
    Update,
    /// Row(s) deleted.
    Delete,
}

/// Batch loading status for a live subscription.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BatchStatus {
    /// Initial batch being loaded.
    Loading,
    /// Subsequent batches are being loaded.
    LoadingBatch,
    /// Initial loading is finished and live updates are active.
    Ready,
}

/// Batch control metadata for paginated initial data loading.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BatchControl {
    /// Preserve this envelope alongside SHARED row versions for safe resumption.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version_domain: Option<VersionDomain>,
    /// Current batch number (0-indexed).
    pub batch_num:      u32,
    /// Whether more batches are available to fetch.
    pub has_more:       bool,
    /// Loading status for the subscription.
    pub status:         BatchStatus,
    /// The version of the last row in this batch (used for subsequent requests).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_seq_id:    Option<VersionId>,
}

impl BatchControl {
    /// Create a batch control for the first batch (batch_num=0).
    pub fn first(has_more: bool) -> Self {
        Self {
            version_domain: None,
            batch_num: 0,
            has_more,
            status: if has_more {
                BatchStatus::Loading
            } else {
                BatchStatus::Ready
            },
            last_seq_id: None,
        }
    }

    /// Create a batch control for a subsequent batch (batch_num > 0).
    pub fn subsequent(batch_num: u32, has_more: bool) -> Self {
        Self {
            version_domain: None,
            batch_num,
            has_more,
            status: if has_more {
                BatchStatus::LoadingBatch
            } else {
                BatchStatus::Ready
            },
            last_seq_id: None,
        }
    }

    /// Create batch control with all fields specified.
    pub fn new(batch_num: u32, has_more: bool, last_seq_id: Option<VersionId>) -> Self {
        let status = if batch_num == 0 {
            if has_more {
                BatchStatus::Loading
            } else {
                BatchStatus::Ready
            }
        } else if has_more {
            BatchStatus::LoadingBatch
        } else {
            BatchStatus::Ready
        };

        Self {
            version_domain: None,
            batch_num,
            has_more,
            status,
            last_seq_id,
        }
    }
}

/// Options for live query subscriptions.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct SubscriptionOptions {
    /// Required with `from` for SHARED tables; copied from the subscription ack.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version_domain:     Option<VersionDomain>,
    /// Hint for server-side batch sizing during initial data load.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub batch_size:         Option<usize>,
    /// Number of last (newest) rows to fetch for initial data.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_rows:          Option<u32>,
    /// Resume subscription from a specific row version.
    #[serde(skip_serializing_if = "Option::is_none", alias = "from_seq_id")]
    pub from:               Option<VersionId>,
    /// Client-side control for automatically requesting subsequent initial data batches.
    #[serde(default, skip_serializing, alias = "autoFetchBatches")]
    pub auto_fetch_batches: Option<bool>,
}

impl SubscriptionOptions {
    /// Create new subscription options with defaults.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the batch size for initial data loading.
    pub fn with_batch_size(mut self, size: usize) -> Self {
        self.batch_size = Some(size);
        self
    }

    /// Set the number of last rows to fetch.
    pub fn with_last_rows(mut self, count: u32) -> Self {
        self.last_rows = Some(count);
        self
    }

    /// Resume from a specific row version.
    pub fn with_from(mut self, version: VersionId) -> Self {
        self.from = Some(version);
        self
    }

    /// Attach the version domain that owns `from`. Shared tables reject a cursor without it.
    pub fn with_version_domain(mut self, domain: VersionDomain) -> Self {
        self.version_domain = Some(domain);
        self
    }

    /// Resume from a specific row version via deprecated alias.
    pub fn with_from_seq_id(self, version: VersionId) -> Self {
        self.with_from(version)
    }

    /// Control whether the client should automatically request subsequent initial data batches.
    pub fn with_auto_fetch_batches(mut self, enabled: bool) -> Self {
        self.auto_fetch_batches = Some(enabled);
        self
    }

    /// Check if this contains a resume version.
    pub fn has_resume_seq_id(&self) -> bool {
        self.from.is_some()
    }
}

/// Subscription request details.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubscriptionRequest {
    /// Unique subscription identifier (client-generated).
    pub id:      String,
    /// SQL query for live updates (must be a SELECT statement).
    pub sql:     String,
    /// Optional subscription options.
    #[serde(default)]
    pub options: Option<SubscriptionOptions>,
}

/// WebSocket message types sent from server to client for the SDK transport.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerMessage {
    /// Authentication successful response (browser clients only).
    AuthSuccess {
        user:     UserId,
        role:     Role,
        protocol: ProtocolOptions,
    },
    /// Authentication failed response (browser clients only).
    AuthError { message: String },
    /// Acknowledgement of successful subscription registration.
    SubscriptionAck {
        subscription_id: String,
        total_rows:      u32,
        batch_control:   BatchControl,
        schema:          Vec<SchemaField>,
    },
    /// Initial data batch sent after subscription or on client request.
    InitialDataBatch {
        subscription_id: String,
        rows:            Vec<HashMap<String, KalamCellValue>>,
        batch_control:   BatchControl,
    },
    /// Change notification for INSERT/UPDATE/DELETE operations.
    Change {
        subscription_id: String,
        change_type:     ChangeTypeRaw,
        #[serde(skip_serializing_if = "Option::is_none")]
        rows:            Option<Vec<HashMap<String, KalamCellValue>>>,
        #[serde(skip_serializing_if = "Option::is_none")]
        old_values:      Option<Vec<HashMap<String, KalamCellValue>>>,
    },
    /// Error notification.
    Error {
        subscription_id: String,
        code:            String,
        message:         String,
    },
}

/// Client-to-server live query request messages.
#[cfg(feature = "websocket-auth")]
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientMessage {
    /// Authenticate WebSocket connection.
    Authenticate {
        #[serde(flatten)]
        credentials: crate::websocket_auth::WsAuthCredentials,
        protocol:    ProtocolOptions,
    },
    /// Subscribe to live query updates.
    Subscribe { subscription: SubscriptionRequest },
    /// Request next batch of initial data.
    NextBatch {
        subscription_id: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        last_seq_id:     Option<VersionId>,
    },
    /// Unsubscribe from live query.
    Unsubscribe { subscription_id: String },
    /// Application-level keepalive ping.
    Ping,
}

#[cfg(feature = "websocket-auth")]
impl ClientMessage {
    /// Create a subscribe message for a single subscription.
    pub fn subscribe(subscription: SubscriptionRequest) -> Self {
        Self::Subscribe { subscription }
    }

    /// Create a next batch request message.
    pub fn next_batch(subscription_id: String, last_seq_id: Option<VersionId>) -> Self {
        Self::NextBatch {
            subscription_id,
            last_seq_id,
        }
    }

    /// Create an unsubscribe message.
    pub fn unsubscribe(subscription_id: String) -> Self {
        Self::Unsubscribe { subscription_id }
    }
}
