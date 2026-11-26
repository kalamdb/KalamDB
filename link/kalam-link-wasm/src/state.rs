use link_common::{
    models::{ChangeEvent, ServerMessage, SubscriptionOptions, SubscriptionRequest},
    realtime::delivered_version,
    seq_tracking,
    subscription::{LiveRowsConfig, LiveRowsMaterializer},
    VersionId,
};

#[derive(Clone)]
pub(crate) enum SubscriptionCallbackMode {
    RawEvents,
    LiveRows { materializer: LiveRowsMaterializer },
}

impl SubscriptionCallbackMode {
    pub(crate) fn raw() -> Self {
        Self::RawEvents
    }

    pub(crate) fn live_rows(config: LiveRowsConfig) -> Self {
        Self::LiveRows {
            materializer: LiveRowsMaterializer::new(config),
        }
    }

    pub(crate) fn reset_snapshot(&mut self) {
        if let Self::LiveRows { materializer } = self {
            materializer.reset_snapshot();
        }
    }
}

/// Stored subscription info for reconnection
pub(crate) struct SubscriptionState {
    /// The SQL query for this subscription
    pub(crate) sql: String,
    /// Original subscription options
    pub(crate) options: SubscriptionOptions,
    /// JavaScript callback function
    pub(crate) callback: js_sys::Function,
    /// Last received seq_id for resumption
    pub(crate) last_seq_id: Option<VersionId>,
    pub(crate) batch_seq_id: Option<VersionId>,
    pub(crate) resume_pending: bool,
    /// Promise resolver for an in-flight subscribe request waiting for ack.
    pub(crate) pending_subscribe_resolve: Option<js_sys::Function>,
    /// Promise rejector for an in-flight subscribe request waiting for ack.
    pub(crate) pending_subscribe_reject: Option<js_sys::Function>,
    /// True until the server responds with subscription_ack or error.
    pub(crate) awaiting_initial_response: bool,
    /// Callback behavior for this subscription.
    pub(crate) callback_mode: SubscriptionCallbackMode,
}

impl SubscriptionState {
    pub(crate) fn request(&self, id: &str) -> SubscriptionRequest {
        SubscriptionRequest {
            id:      id.to_owned(),
            sql:     self.sql.clone(),
            options: Some(self.options.clone()),
        }
    }
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(crate) enum WasmLiveRowsEvent {
    Rows {
        subscription_id: String,
        rows:            Vec<link_common::models::RowData>,
        last_seq_id:     Option<VersionId>,
    },
    Error {
        subscription_id: String,
        code:            String,
        message:         String,
    },
}

#[derive(Debug, Clone, Default, serde::Deserialize)]
pub(crate) struct WasmLiveRowsOptions {
    pub(crate) limit:                Option<usize>,
    pub(crate) key_columns:          Option<Vec<String>>,
    pub(crate) subscription_options: Option<SubscriptionOptions>,
}

#[inline]
pub(crate) fn track_subscription_checkpoint(
    last_seq_id: &mut Option<VersionId>,
    event: &ChangeEvent,
) {
    if let Some(version) = delivered_version(event) {
        seq_tracking::advance_seq(last_seq_id, version);
    }
}

#[inline]
pub(crate) fn filter_subscription_event(
    options: &SubscriptionOptions,
    event: &ServerMessage,
) -> Option<ChangeEvent> {
    let change_event = ChangeEvent::from_server_message(event.clone())?;
    link_common::subscription::filter_replayed_event(change_event, options.from)
}

#[inline]
pub(crate) fn callback_payload(
    mode: &mut SubscriptionCallbackMode,
    event: &ChangeEvent,
) -> Option<String> {
    match mode {
        SubscriptionCallbackMode::RawEvents => {
            serde_json::to_string(&event.to_server_message()).ok()
        },
        SubscriptionCallbackMode::LiveRows { materializer } => {
            let update = materializer.apply(event.clone())?;
            let wasm_event = match update {
                link_common::subscription::LiveRowsEvent::Rows {
                    subscription_id,
                    rows,
                    last_seq_id,
                } => WasmLiveRowsEvent::Rows {
                    subscription_id,
                    rows,
                    last_seq_id,
                },
                link_common::subscription::LiveRowsEvent::Error {
                    subscription_id,
                    code,
                    message,
                } => WasmLiveRowsEvent::Error {
                    subscription_id,
                    code,
                    message,
                },
            };
            serde_json::to_string(&wasm_event).ok()
        },
    }
}
