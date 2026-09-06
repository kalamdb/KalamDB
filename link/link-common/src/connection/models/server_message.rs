use std::collections::HashMap;

use kalamdb_commons::{ChangeTypeRaw, Role, UserId};
use serde::{Deserialize, Serialize};

use super::ProtocolOptions;
use crate::{
    models::{KalamCellValue, SchemaField},
    subscription::models::BatchControl,
};

/// WebSocket message types sent from server to client.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerMessage {
    AuthSuccess {
        user:     UserId,
        role:     Role,
        protocol: ProtocolOptions,
    },
    AuthError {
        message: String,
    },
    SubscriptionAck {
        subscription_id: String,
        total_rows:      u32,
        batch_control:   BatchControl,
        schema:          Vec<SchemaField>,
    },
    InitialDataBatch {
        subscription_id: String,
        rows:            Vec<HashMap<String, KalamCellValue>>,
        batch_control:   BatchControl,
    },
    Change {
        subscription_id: String,
        change_type:     ChangeTypeRaw,
        #[serde(skip_serializing_if = "Option::is_none")]
        rows:            Option<Vec<HashMap<String, KalamCellValue>>>,
        #[serde(skip_serializing_if = "Option::is_none")]
        old_values:      Option<Vec<HashMap<String, KalamCellValue>>>,
    },
    Error {
        subscription_id: String,
        code:            String,
        message:         String,
    },
}
