use crate::{models::RowData, seq_id::SeqId};

/// High-level event emitted by a materialized live-query subscription.
#[derive(Debug, Clone)]
pub enum LiveRowsEvent {
    /// The current materialized row set.
    Rows {
        subscription_id: String,
        rows: Vec<RowData>,
        last_seq_id: Option<SeqId>,
    },
    /// A server-side subscription error.
    Error {
        subscription_id: String,
        code: String,
        message: String,
    },
}
