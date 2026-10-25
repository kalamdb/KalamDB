//! Link-side adapter from the shared domain WebSocket codec onto the SDK message type.

use std::collections::HashMap;

use kalamdb_serialization::{
    decode_server_message as decode_commons_server_message, SerializationError,
};

use crate::error::{KalamLinkError, Result};
use crate::models::{KalamCellValue, ServerMessage};

/// Decode a current-epoch domain frame into the Link [`ServerMessage`].
pub fn decode_server_message(bytes: &[u8]) -> Result<ServerMessage> {
    let message = decode_commons_server_message(bytes).map_err(link_error)?;
    Ok(match message {
        kalamdb_commons::websocket_messages::ServerMessage::AuthSuccess {
            user,
            role,
            protocol,
        } => ServerMessage::AuthSuccess {
            user,
            role,
            protocol,
        },
        kalamdb_commons::websocket_messages::ServerMessage::AuthError { message } => {
            ServerMessage::AuthError { message }
        },
        kalamdb_commons::websocket_messages::ServerMessage::SubscriptionAck {
            subscription_id,
            total_rows,
            batch_control,
            schema,
        } => ServerMessage::SubscriptionAck {
            subscription_id,
            total_rows,
            batch_control,
            schema,
        },
        kalamdb_commons::websocket_messages::ServerMessage::InitialDataBatch {
            subscription_id,
            rows,
            batch_control,
        } => ServerMessage::InitialDataBatch {
            subscription_id,
            rows: map_rows(rows),
            batch_control,
        },
        kalamdb_commons::websocket_messages::ServerMessage::Change {
            subscription_id,
            change_type,
            rows,
            old_values,
        } => ServerMessage::Change {
            subscription_id,
            change_type,
            rows: rows.map(map_rows),
            old_values: old_values.map(map_rows),
        },
        kalamdb_commons::websocket_messages::ServerMessage::Error {
            subscription_id,
            code,
            message,
        } => ServerMessage::Error {
            subscription_id,
            code,
            message,
        },
    })
}

fn map_rows(
    rows: Vec<HashMap<String, kalamdb_commons::models::KalamCellValue>>,
) -> Vec<HashMap<String, KalamCellValue>> {
    rows.into_iter()
        .map(|row| {
            row.into_iter()
                .map(|(name, cell)| (name, KalamCellValue::from_json(cell.into_inner())))
                .collect()
        })
        .collect()
}

fn link_error(error: SerializationError) -> KalamLinkError {
    KalamLinkError::SerializationError(error.to_string())
}
