//! WebSocket auth, control, snapshot, and change frames on the shared domain codec.
//!
//! Binary sockets use these frames. JSON text remains an explicit external representation.
//! MessagePack bytes are rejected by the frame decoder before a message is applied.
//!
//! [`WS_CONTRACT_DESCRIPTOR_ID`] is the live message-layout revision. It is not a SQL catalog
//! hash. Bump it when the composite slot numbers in this file change, and reject every other id
//! before a handler can subscribe, acknowledge, or apply a change.
//!
//! Slot layout, kept in this file so callers do not invent a second codec:
//!
//! - client auth: credentials composite, protocol composite
//! - subscribe: id, sql, optional options
//! - unsubscribe: id
//! - next batch: id, optional version
//! - server auth: variant `1` success (user, role, protocol) or `2` error (message)
//! - ack: id, total rows, batch control, schema
//! - snapshot: id, rows, batch control
//! - change: id, change type, optional rows, optional old values
//! - error: id, code, message

use std::collections::{hash_map::Entry, HashMap};

use kalamdb_commons::ids::{VersionDomain, VersionId};
use kalamdb_commons::models::{KalamCellValue, KalamDataType, Role, SchemaField, SchemaId, UserId};
use kalamdb_commons::websocket_auth::WsAuthCredentials;
use kalamdb_commons::websocket_messages::{
    BatchControl, BatchStatus, ChangeTypeRaw, ClientMessage, ServerMessage, SubscriptionOptions,
    SubscriptionRequest,
};
use kalamdb_commons::websocket_protocol::{CompressionType, ProtocolOptions, SerializationType};
use kalamdb_commons::{FieldFlag, TableId, TableName};

use crate::decode_domain_frame_for_surface;
use crate::domain::{
    encode_domain_frame, encode_domain_frame_from_payload, write_domain_value, DomainDecodeBudget,
    DomainMessageKind, DomainValue, EndpointSurface, DTAG_COMPOSITE,
};
use crate::error::{Result, SerializationError};

/// Live WebSocket message-layout revision inside the current domain epoch.
///
/// SQL catalog descriptor pins are a separate check. This id only identifies the slot layout
/// above. Handlers must not run for any other value.
pub const WS_CONTRACT_DESCRIPTOR_ID: u16 = 1;

/// Encode one client live-query message as a current-epoch WebSocket frame.
pub fn encode_client_message(message: &ClientMessage) -> Result<Vec<u8>> {
    let (kind, payload) = match message {
        ClientMessage::Authenticate {
            credentials,
            protocol,
        } => (
            DomainMessageKind::WsAuth,
            composite(vec![
                (1, credentials_to_domain(credentials)?),
                (2, protocol_to_domain(protocol)),
            ]),
        ),
        ClientMessage::Subscribe { subscription } => {
            (DomainMessageKind::WsSubscribe, subscription_to_domain(subscription)?)
        },
        ClientMessage::Unsubscribe { subscription_id } => {
            (DomainMessageKind::WsUnsubscribe, composite(vec![(1, utf8(subscription_id))]))
        },
        ClientMessage::NextBatch {
            subscription_id,
            last_seq_id,
        } => {
            let mut fields = vec![(1, utf8(subscription_id))];
            if let Some(version) = last_seq_id {
                fields.push((2, DomainValue::UInt64(version.as_u64())));
            }
            (DomainMessageKind::WsCursor, composite(fields))
        },
        ClientMessage::Ping => (DomainMessageKind::WsPing, DomainValue::Void),
    };
    encode_domain_frame(kind, WS_CONTRACT_DESCRIPTOR_ID, 0, &payload)
}

/// Decode one client live-query message from a current-epoch WebSocket frame.
pub fn decode_client_message(bytes: &[u8]) -> Result<ClientMessage> {
    let frame = decode_frame(bytes)?;
    require_current_descriptor(frame.descriptor_short_id)?;
    if frame.kind == DomainMessageKind::WsPing {
        if frame.payload != DomainValue::Void {
            return Err(decode("ping payload must be void"));
        }
        return Ok(ClientMessage::Ping);
    }
    let mut slots = Slots::parse(&frame.payload)?;
    let message = match frame.kind {
        DomainMessageKind::WsAuth => ClientMessage::Authenticate {
            credentials: credentials_from_domain(slots.take(1)?)?,
            protocol: protocol_from_domain(slots.take(2)?)?,
        },
        DomainMessageKind::WsSubscribe => ClientMessage::Subscribe {
            subscription: subscription_from_domain(&mut slots)?,
        },
        DomainMessageKind::WsUnsubscribe => ClientMessage::Unsubscribe {
            subscription_id: owned_utf8(slots.take(1)?)?,
        },
        DomainMessageKind::WsCursor => ClientMessage::NextBatch {
            subscription_id: owned_utf8(slots.take(1)?)?,
            last_seq_id: match slots.optional(2) {
                Some(value) => Some(version_from_domain(value)?),
                None => None,
            },
        },
        other => {
            return Err(decode(format!("client frame kind {other:?} is not a client message")))
        },
    };
    slots.finish()?;
    Ok(message)
}

/// Encode one server live-query message as a current-epoch WebSocket frame.
pub fn encode_server_message(message: &ServerMessage) -> Result<Vec<u8>> {
    let (kind, payload) = server_payload(message)?;
    encode_domain_frame(kind, WS_CONTRACT_DESCRIPTOR_ID, 0, &payload)
}

/// Decode one server live-query message from a current-epoch WebSocket frame.
pub fn decode_server_message(bytes: &[u8]) -> Result<ServerMessage> {
    let frame = decode_frame(bytes)?;
    require_current_descriptor(frame.descriptor_short_id)?;
    decode_server_frame(frame.kind, &frame.payload)
}

/// Encode a shared change payload once. The returned bytes are one domain array value.
pub fn encode_named_rows(rows: &[HashMap<String, KalamCellValue>]) -> Result<Vec<u8>> {
    let mut buf = Vec::new();
    write_domain_value(&mut buf, &rows_to_domain(rows)?)?;
    Ok(buf)
}

/// Build a per-subscriber change frame from row bytes produced by [`encode_named_rows`].
pub fn encode_change_frame(
    subscription_id: &str,
    change_type: &str,
    rows: Option<&[u8]>,
    old_values: Option<&[u8]>,
) -> Result<Vec<u8>> {
    let mut payload = Vec::with_capacity(
        32 + subscription_id.len()
            + rows.map_or(0, |bytes| bytes.len())
            + old_values.map_or(0, |bytes| bytes.len()),
    );
    let count = 2 + u16::from(rows.is_some()) + u16::from(old_values.is_some());
    payload.push(DTAG_COMPOSITE);
    payload.extend_from_slice(&count.to_le_bytes());
    push_value(&mut payload, 1, &utf8(subscription_id))?;
    push_value(&mut payload, 2, &utf8(change_type))?;
    if let Some(rows) = rows {
        push_raw(&mut payload, 3, rows);
    }
    if let Some(old_values) = old_values {
        push_raw(&mut payload, 4, old_values);
    }
    encode_domain_frame_from_payload(
        DomainMessageKind::WsChanges,
        WS_CONTRACT_DESCRIPTOR_ID,
        0,
        &payload,
    )
}

fn require_current_descriptor(descriptor_short_id: u16) -> Result<()> {
    if descriptor_short_id == WS_CONTRACT_DESCRIPTOR_ID {
        return Ok(());
    }
    Err(SerializationError::Decode(format!(
        "stale WebSocket descriptor {descriptor_short_id}; current contract is {WS_CONTRACT_DESCRIPTOR_ID}"
    )))
}

fn server_payload(message: &ServerMessage) -> Result<(DomainMessageKind, DomainValue)> {
    Ok(match message {
        ServerMessage::AuthSuccess {
            user,
            role,
            protocol,
        } => (
            DomainMessageKind::WsAuth,
            composite(vec![
                (1, DomainValue::UInt8(1)),
                (2, utf8(user.as_str())),
                (3, utf8(role.as_str())),
                (4, protocol_to_domain(protocol)),
            ]),
        ),
        ServerMessage::AuthError { message } => (
            DomainMessageKind::WsAuth,
            composite(vec![(1, DomainValue::UInt8(2)), (2, utf8(message))]),
        ),
        ServerMessage::SubscriptionAck {
            subscription_id,
            total_rows,
            batch_control,
            schema,
        } => (
            DomainMessageKind::WsAck,
            composite(vec![
                (1, utf8(subscription_id)),
                (2, DomainValue::UInt32(*total_rows)),
                (3, batch_control_to_domain(batch_control)?),
                (4, schema_to_domain(schema)?),
            ]),
        ),
        ServerMessage::InitialDataBatch {
            subscription_id,
            rows,
            batch_control,
        } => (
            snapshot_kind(batch_control),
            composite(vec![
                (1, utf8(subscription_id)),
                (2, rows_to_domain(rows)?),
                (3, batch_control_to_domain(batch_control)?),
            ]),
        ),
        ServerMessage::Change {
            subscription_id,
            change_type,
            rows,
            old_values,
        } => {
            let mut fields = vec![
                (1, utf8(subscription_id)),
                (2, utf8(change_type_name(change_type.clone()))),
            ];
            if let Some(rows) = rows {
                fields.push((3, rows_to_domain(rows)?));
            }
            if let Some(old_values) = old_values {
                fields.push((4, rows_to_domain(old_values)?));
            }
            (DomainMessageKind::WsChanges, composite(fields))
        },
        ServerMessage::Error {
            subscription_id,
            code,
            message,
        } => (
            DomainMessageKind::WsError,
            composite(vec![
                (1, utf8(subscription_id)),
                (2, utf8(code)),
                (3, utf8(message)),
            ]),
        ),
    })
}

fn decode_server_frame(kind: DomainMessageKind, payload: &DomainValue) -> Result<ServerMessage> {
    if kind == DomainMessageKind::WsPing {
        return Err(decode("ping is not a server message"));
    }
    let mut slots = Slots::parse(payload)?;
    let message = match kind {
        DomainMessageKind::WsAuth => match as_u8(slots.take(1)?)? {
            1 => ServerMessage::AuthSuccess {
                user: UserId::new(owned_utf8(slots.take(2)?)?),
                role: role_from_name(&owned_utf8(slots.take(3)?)?)?,
                protocol: protocol_from_domain(slots.take(4)?)?,
            },
            2 => ServerMessage::AuthError {
                message: owned_utf8(slots.take(2)?)?,
            },
            other => return Err(decode(format!("unknown auth variant {other}"))),
        },
        DomainMessageKind::WsAck => ServerMessage::SubscriptionAck {
            subscription_id: owned_utf8(slots.take(1)?)?,
            total_rows: as_u32(slots.take(2)?)?,
            batch_control: batch_control_from_domain(slots.take(3)?)?,
            schema: schema_from_domain(slots.take(4)?)?,
        },
        DomainMessageKind::WsSnapshotBegin
        | DomainMessageKind::WsSnapshotBatch
        | DomainMessageKind::WsSnapshotEnd => ServerMessage::InitialDataBatch {
            subscription_id: owned_utf8(slots.take(1)?)?,
            rows: rows_from_domain(slots.take(2)?)?,
            batch_control: batch_control_from_domain(slots.take(3)?)?,
        },
        DomainMessageKind::WsChanges => ServerMessage::Change {
            subscription_id: owned_utf8(slots.take(1)?)?,
            change_type: change_type_from_name(&owned_utf8(slots.take(2)?)?)?,
            rows: optional_rows(&mut slots, 3)?,
            old_values: optional_rows(&mut slots, 4)?,
        },
        DomainMessageKind::WsError => ServerMessage::Error {
            subscription_id: owned_utf8(slots.take(1)?)?,
            code: owned_utf8(slots.take(2)?)?,
            message: owned_utf8(slots.take(3)?)?,
        },
        other => {
            return Err(decode(format!("server frame kind {other:?} is not a server message")))
        },
    };
    slots.finish()?;
    Ok(message)
}

fn snapshot_kind(batch: &BatchControl) -> DomainMessageKind {
    if batch.batch_num == 0 && batch.has_more {
        DomainMessageKind::WsSnapshotBegin
    } else if batch.has_more {
        DomainMessageKind::WsSnapshotBatch
    } else {
        DomainMessageKind::WsSnapshotEnd
    }
}

fn decode_frame(bytes: &[u8]) -> Result<crate::DomainFrame> {
    decode_domain_frame_for_surface(
        bytes,
        EndpointSurface::WebSocket,
        DomainDecodeBudget::default(),
    )
}

fn subscription_to_domain(subscription: &SubscriptionRequest) -> Result<DomainValue> {
    let mut fields = vec![(1, utf8(&subscription.id)), (2, utf8(&subscription.sql))];
    if let Some(options) = &subscription.options {
        fields.push((3, options_to_domain(options)?));
    }
    Ok(composite(fields))
}

fn subscription_from_domain(slots: &mut Slots<'_>) -> Result<SubscriptionRequest> {
    let request = SubscriptionRequest {
        id: owned_utf8(slots.take(1)?)?,
        sql: owned_utf8(slots.take(2)?)?,
        options: match slots.optional(3) {
            Some(value) => Some(options_from_domain(value)?),
            None => None,
        },
    };
    Ok(request)
}

fn options_to_domain(options: &SubscriptionOptions) -> Result<DomainValue> {
    let mut fields = Vec::new();
    if let Some(domain) = &options.version_domain {
        fields.push((1, version_domain_to_domain(domain)));
    }
    if let Some(batch_size) = options.batch_size {
        let size = u32::try_from(batch_size)
            .map_err(|_| SerializationError::Encode("batch size exceeds u32".to_string()))?;
        fields.push((2, DomainValue::UInt32(size)));
    }
    if let Some(last_rows) = options.last_rows {
        fields.push((3, DomainValue::UInt32(last_rows)));
    }
    if let Some(from) = options.from {
        fields.push((4, DomainValue::UInt64(from.as_u64())));
    }
    if let Some(auto_fetch) = options.auto_fetch_batches {
        fields.push((5, DomainValue::Bool(auto_fetch)));
    }
    Ok(composite(fields))
}

fn options_from_domain(value: &DomainValue) -> Result<SubscriptionOptions> {
    let mut slots = Slots::parse(value)?;
    let options = SubscriptionOptions {
        version_domain: match slots.optional(1) {
            Some(value) => Some(version_domain_from_domain(value)?),
            None => None,
        },
        batch_size: match slots.optional(2) {
            Some(value) => Some(
                usize::try_from(as_u32(value)?).map_err(|_| decode("batch size exceeds usize"))?,
            ),
            None => None,
        },
        last_rows: match slots.optional(3) {
            Some(value) => Some(as_u32(value)?),
            None => None,
        },
        from: match slots.optional(4) {
            Some(value) => Some(version_from_domain(value)?),
            None => None,
        },
        auto_fetch_batches: match slots.optional(5) {
            Some(value) => Some(as_bool(value)?),
            None => None,
        },
    };
    slots.finish()?;
    Ok(options)
}

fn version_domain_to_domain(domain: &VersionDomain) -> DomainValue {
    let mut fields = vec![
        (1, utf8(&domain.history_incarnation)),
        (2, utf8(domain.table_id.namespace_id().as_str())),
        (3, utf8(domain.table_id.table_name().as_str())),
        (4, DomainValue::UInt64(domain.scope_id)),
    ];
    if let Some(partition) = domain.partition_id {
        fields.push((5, DomainValue::UInt64(partition)));
    }
    composite(fields)
}

fn version_domain_from_domain(value: &DomainValue) -> Result<VersionDomain> {
    let mut slots = Slots::parse(value)?;
    let mut domain = VersionDomain::new(
        owned_utf8(slots.take(1)?)?,
        TableId::new(
            SchemaId::new(owned_utf8(slots.take(2)?)?),
            TableName::new(owned_utf8(slots.take(3)?)?),
        ),
        as_u64(slots.take(4)?)?,
    );
    domain.partition_id = match slots.optional(5) {
        Some(value) => Some(as_u64(value)?),
        None => None,
    };
    slots.finish()?;
    Ok(domain)
}

fn batch_control_to_domain(batch: &BatchControl) -> Result<DomainValue> {
    let mut fields = Vec::new();
    if let Some(domain) = &batch.version_domain {
        fields.push((1, version_domain_to_domain(domain)));
    }
    fields.push((2, DomainValue::UInt32(batch.batch_num)));
    fields.push((3, DomainValue::Bool(batch.has_more)));
    fields.push((4, utf8(batch_status_name(batch.status))));
    if let Some(version) = batch.last_seq_id {
        fields.push((5, DomainValue::UInt64(version.as_u64())));
    }
    Ok(composite(fields))
}

fn batch_control_from_domain(value: &DomainValue) -> Result<BatchControl> {
    let mut slots = Slots::parse(value)?;
    let control = BatchControl {
        version_domain: match slots.optional(1) {
            Some(value) => Some(version_domain_from_domain(value)?),
            None => None,
        },
        batch_num: as_u32(slots.take(2)?)?,
        has_more: as_bool(slots.take(3)?)?,
        status: batch_status_from_name(&owned_utf8(slots.take(4)?)?)?,
        last_seq_id: match slots.optional(5) {
            Some(value) => Some(version_from_domain(value)?),
            None => None,
        },
    };
    slots.finish()?;
    Ok(control)
}

fn schema_to_domain(schema: &[SchemaField]) -> Result<DomainValue> {
    let mut fields = Vec::with_capacity(schema.len());
    for field in schema {
        let mut slots = vec![
            (1, utf8(&field.name)),
            (2, data_type_to_domain(field.data_type)),
            (
                3,
                DomainValue::UInt32(u32::try_from(field.index).map_err(|_| {
                    SerializationError::Encode("schema index exceeds u32".to_string())
                })?),
            ),
        ];
        if let Some(flags) = &field.flags {
            let names = flags.iter().map(|flag| utf8(field_flag_name(flag))).collect::<Vec<_>>();
            slots.push((4, DomainValue::Array(names)));
        }
        fields.push(composite(slots));
    }
    Ok(DomainValue::Array(fields))
}

fn schema_from_domain(value: &DomainValue) -> Result<Vec<SchemaField>> {
    let DomainValue::Array(items) = value else {
        return Err(decode("schema must be an array"));
    };
    items
        .iter()
        .map(|item| {
            let mut slots = Slots::parse(item)?;
            let field = SchemaField {
                name: owned_utf8(slots.take(1)?)?,
                data_type: data_type_from_domain(slots.take(2)?)?,
                index: usize::try_from(as_u32(slots.take(3)?)?)
                    .map_err(|_| decode("schema index exceeds usize"))?,
                flags: match slots.optional(4) {
                    Some(value) => Some(flags_from_domain(value)?),
                    None => None,
                },
            };
            slots.finish()?;
            Ok(field)
        })
        .collect()
}

fn data_type_to_domain(data_type: KalamDataType) -> DomainValue {
    match data_type {
        KalamDataType::Embedding(dimension) => composite(vec![
            (1, DomainValue::UInt8(data_type.tag())),
            (2, DomainValue::UInt16(dimension)),
        ]),
        KalamDataType::Decimal { precision, scale } => composite(vec![
            (1, DomainValue::UInt8(data_type.tag())),
            (2, DomainValue::UInt8(precision)),
            (3, DomainValue::UInt8(scale)),
        ]),
        other => DomainValue::UInt8(other.tag()),
    }
}

fn data_type_from_domain(value: &DomainValue) -> Result<KalamDataType> {
    match value {
        DomainValue::UInt8(tag) => KalamDataType::from_tag(*tag).map_err(|message| decode(message)),
        DomainValue::Composite(_) => {
            let mut slots = Slots::parse(value)?;
            let tag = as_u8(slots.take(1)?)?;
            let data_type = match tag {
                0x0D => {
                    let dimension = as_u16(slots.take(2)?)?;
                    KalamDataType::validate_embedding_dimension(dimension).map_err(decode)?;
                    KalamDataType::Embedding(dimension)
                },
                0x0F => {
                    let precision = as_u8(slots.take(2)?)?;
                    let scale = as_u8(slots.take(3)?)?;
                    KalamDataType::validate_decimal_params(precision, scale).map_err(decode)?;
                    KalamDataType::Decimal { precision, scale }
                },
                other => {
                    return Err(decode(format!("parameterized type tag 0x{other:02x} is unknown")))
                },
            };
            slots.finish()?;
            Ok(data_type)
        },
        _ => Err(decode("data type must be a tag or parameterized composite")),
    }
}

fn flags_from_domain(value: &DomainValue) -> Result<kalamdb_commons::FieldFlags> {
    let DomainValue::Array(items) = value else {
        return Err(decode("field flags must be an array"));
    };
    let mut flags = kalamdb_commons::FieldFlags::new();
    for item in items {
        flags.insert(field_flag_from_name(&owned_utf8(item)?)?);
    }
    Ok(flags)
}

fn rows_to_domain(rows: &[HashMap<String, KalamCellValue>]) -> Result<DomainValue> {
    let mut encoded = Vec::with_capacity(rows.len());
    for row in rows {
        let mut names = row.keys().cloned().collect::<Vec<_>>();
        names.sort();
        let mut fields = Vec::with_capacity(names.len());
        for name in names {
            let cell = row.get(&name).expect("sorted key is present");
            fields.push(composite(vec![(1, utf8(name)), (2, cell_to_domain(cell)?)]));
        }
        encoded.push(DomainValue::Array(fields));
    }
    Ok(DomainValue::Array(encoded))
}

fn rows_from_domain(value: &DomainValue) -> Result<Vec<HashMap<String, KalamCellValue>>> {
    let DomainValue::Array(rows) = value else {
        return Err(decode("rows must be an array"));
    };
    rows.iter()
        .map(|row| {
            let DomainValue::Array(fields) = row else {
                return Err(decode("row must be an array of named cells"));
            };
            let mut decoded = HashMap::with_capacity(fields.len());
            for field in fields {
                let mut slots = Slots::parse(field)?;
                let name = owned_utf8(slots.take(1)?)?;
                let cell = cell_from_domain(slots.take(2)?)?;
                slots.finish()?;
                match decoded.entry(name) {
                    Entry::Vacant(entry) => {
                        entry.insert(cell);
                    },
                    Entry::Occupied(entry) => {
                        return Err(decode(format!("duplicate column {}", entry.key())));
                    },
                }
            }
            Ok(decoded)
        })
        .collect()
}

fn optional_rows(
    slots: &mut Slots<'_>,
    slot: u16,
) -> Result<Option<Vec<HashMap<String, KalamCellValue>>>> {
    match slots.optional(slot) {
        Some(value) => Ok(Some(rows_from_domain(value)?)),
        None => Ok(None),
    }
}

fn cell_to_domain(cell: &KalamCellValue) -> Result<DomainValue> {
    json_to_domain(cell.inner())
}

fn json_to_domain(value: &serde_json::Value) -> Result<DomainValue> {
    match value {
        serde_json::Value::Null => Ok(DomainValue::SqlNull),
        serde_json::Value::Bool(flag) => Ok(DomainValue::Bool(*flag)),
        serde_json::Value::Number(number) => {
            if let Some(signed) = number.as_i64() {
                Ok(DomainValue::Int64(signed))
            } else if let Some(unsigned) = number.as_u64() {
                Ok(DomainValue::UInt64(unsigned))
            } else if let Some(float) = number.as_f64() {
                Ok(DomainValue::Float64(float))
            } else {
                Err(SerializationError::Encode("json number is not a finite domain number".into()))
            }
        },
        serde_json::Value::String(text) => Ok(utf8(text)),
        serde_json::Value::Array(items) => {
            let mut encoded = Vec::with_capacity(items.len());
            for item in items {
                encoded.push(json_to_domain(item)?);
            }
            Ok(DomainValue::Array(encoded))
        },
        serde_json::Value::Object(map) => {
            let mut names = map.keys().cloned().collect::<Vec<_>>();
            names.sort();
            let mut pairs = Vec::with_capacity(names.len());
            for name in names {
                let child = map.get(&name).expect("sorted key is present");
                pairs.push(composite(vec![(1, utf8(name)), (2, json_to_domain(child)?)]));
            }
            Ok(composite(vec![(1, utf8("object")), (2, DomainValue::Array(pairs))]))
        },
    }
}

fn cell_from_domain(value: &DomainValue) -> Result<KalamCellValue> {
    Ok(KalamCellValue::from_json(domain_to_json(value)?))
}

fn domain_to_json(value: &DomainValue) -> Result<serde_json::Value> {
    match value {
        DomainValue::SqlNull | DomainValue::JsonNull => Ok(serde_json::Value::Null),
        DomainValue::Bool(flag) => Ok(serde_json::Value::Bool(*flag)),
        DomainValue::Int64(number) => Ok(serde_json::Value::Number((*number).into())),
        DomainValue::UInt64(number) => Ok(serde_json::Value::Number((*number).into())),
        DomainValue::Float64(number) => serde_json::Number::from_f64(*number)
            .map(serde_json::Value::Number)
            .ok_or_else(|| decode("non-finite float")),
        DomainValue::Utf8(text) => Ok(serde_json::Value::String(text.clone())),
        DomainValue::Array(items) => {
            let mut decoded = Vec::with_capacity(items.len());
            for item in items {
                decoded.push(domain_to_json(item)?);
            }
            Ok(serde_json::Value::Array(decoded))
        },
        DomainValue::Composite(_) => {
            let mut slots = Slots::parse(value)?;
            let marker = slots.optional(1);
            if matches!(marker, Some(DomainValue::Utf8(text)) if text == "object") {
                let pairs = match slots.take(2)? {
                    DomainValue::Array(pairs) => pairs,
                    _ => return Err(decode("object cells store an array of pairs")),
                };
                slots.finish()?;
                let mut map = serde_json::Map::new();
                for pair in pairs {
                    let mut pair_slots = Slots::parse(pair)?;
                    let name = owned_utf8(pair_slots.take(1)?)?;
                    let child = domain_to_json(pair_slots.take(2)?)?;
                    pair_slots.finish()?;
                    match map.entry(name) {
                        serde_json::map::Entry::Vacant(entry) => {
                            entry.insert(child);
                        },
                        serde_json::map::Entry::Occupied(entry) => {
                            return Err(decode(format!("duplicate object key {}", entry.key())));
                        },
                    }
                }
                return Ok(serde_json::Value::Object(map));
            }
            Err(decode("unsupported cell composite"))
        },
        other => Err(decode(format!("unsupported cell value {other:?}"))),
    }
}

fn credentials_to_domain(credentials: &WsAuthCredentials) -> Result<DomainValue> {
    match credentials {
        WsAuthCredentials::Jwt { token } => Ok(composite(vec![(1, utf8("jwt")), (2, utf8(token))])),
    }
}

fn credentials_from_domain(value: &DomainValue) -> Result<WsAuthCredentials> {
    let mut slots = Slots::parse(value)?;
    let method = owned_utf8(slots.take(1)?)?;
    let credentials = match method.as_str() {
        "jwt" => WsAuthCredentials::Jwt {
            token: owned_utf8(slots.take(2)?)?,
        },
        other => return Err(decode(format!("unknown auth method {other}"))),
    };
    slots.finish()?;
    Ok(credentials)
}

fn protocol_to_domain(protocol: &ProtocolOptions) -> DomainValue {
    composite(vec![
        (1, utf8(serialization_name(protocol.serialization))),
        (2, utf8(compression_name(protocol.compression))),
    ])
}

fn protocol_from_domain(value: &DomainValue) -> Result<ProtocolOptions> {
    let mut slots = Slots::parse(value)?;
    let protocol = ProtocolOptions {
        serialization: serialization_from_name(&owned_utf8(slots.take(1)?)?)?,
        compression: compression_from_name(&owned_utf8(slots.take(2)?)?)?,
    };
    slots.finish()?;
    Ok(protocol)
}

fn serialization_name(serialization: SerializationType) -> &'static str {
    match serialization {
        SerializationType::Json => "json",
        SerializationType::Domain => "domain",
    }
}

fn serialization_from_name(name: &str) -> Result<SerializationType> {
    match name {
        "json" => Ok(SerializationType::Json),
        "domain" => Ok(SerializationType::Domain),
        "msgpack" => Err(SerializationError::UnsupportedFormat(
            "legacy MessagePack WebSocket serialization is not supported".to_string(),
        )),
        other => Err(decode(format!("unknown serialization {other}"))),
    }
}

fn compression_name(compression: CompressionType) -> &'static str {
    match compression {
        CompressionType::None => "none",
        CompressionType::Gzip => "gzip",
    }
}

fn compression_from_name(name: &str) -> Result<CompressionType> {
    match name {
        "none" => Ok(CompressionType::None),
        "gzip" => Ok(CompressionType::Gzip),
        other => Err(decode(format!("unknown compression {other}"))),
    }
}

fn change_type_name(change_type: ChangeTypeRaw) -> &'static str {
    match change_type {
        ChangeTypeRaw::Insert => "insert",
        ChangeTypeRaw::Update => "update",
        ChangeTypeRaw::Delete => "delete",
    }
}

fn change_type_from_name(name: &str) -> Result<ChangeTypeRaw> {
    match name {
        "insert" => Ok(ChangeTypeRaw::Insert),
        "update" => Ok(ChangeTypeRaw::Update),
        "delete" => Ok(ChangeTypeRaw::Delete),
        other => Err(decode(format!("unknown change type {other}"))),
    }
}

fn batch_status_name(status: BatchStatus) -> &'static str {
    match status {
        BatchStatus::Loading => "loading",
        BatchStatus::LoadingBatch => "loading_batch",
        BatchStatus::Ready => "ready",
    }
}

fn batch_status_from_name(name: &str) -> Result<BatchStatus> {
    match name {
        "loading" => Ok(BatchStatus::Loading),
        "loading_batch" => Ok(BatchStatus::LoadingBatch),
        "ready" => Ok(BatchStatus::Ready),
        other => Err(decode(format!("unknown batch status {other}"))),
    }
}

fn field_flag_name(flag: &FieldFlag) -> &'static str {
    match flag {
        FieldFlag::PrimaryKey => "pk",
        FieldFlag::NonNull => "nn",
        FieldFlag::Unique => "uq",
    }
}

fn field_flag_from_name(name: &str) -> Result<FieldFlag> {
    match name {
        "pk" => Ok(FieldFlag::PrimaryKey),
        "nn" => Ok(FieldFlag::NonNull),
        "uq" => Ok(FieldFlag::Unique),
        other => Err(decode(format!("unknown field flag {other}"))),
    }
}

fn role_from_name(name: &str) -> Result<Role> {
    Role::from_str_opt(name).ok_or_else(|| decode(format!("unknown role {name}")))
}

fn version_from_domain(value: &DomainValue) -> Result<VersionId> {
    VersionId::try_from_raw(as_u64(value)?).map_err(|error| decode(error.to_string()))
}

fn utf8(text: impl Into<String>) -> DomainValue {
    DomainValue::Utf8(text.into())
}

fn composite(fields: Vec<(u16, DomainValue)>) -> DomainValue {
    DomainValue::Composite(fields)
}

fn push_value(buf: &mut Vec<u8>, slot: u16, value: &DomainValue) -> Result<()> {
    buf.extend_from_slice(&slot.to_le_bytes());
    write_domain_value(buf, value)
}

fn push_raw(buf: &mut Vec<u8>, slot: u16, raw: &[u8]) {
    buf.extend_from_slice(&slot.to_le_bytes());
    buf.extend_from_slice(raw);
}

fn owned_utf8(value: &DomainValue) -> Result<String> {
    match value {
        DomainValue::Utf8(text) => Ok(text.clone()),
        _ => Err(decode("expected utf8")),
    }
}

fn as_bool(value: &DomainValue) -> Result<bool> {
    match value {
        DomainValue::Bool(flag) => Ok(*flag),
        _ => Err(decode("expected bool")),
    }
}

fn as_u8(value: &DomainValue) -> Result<u8> {
    match value {
        DomainValue::UInt8(number) => Ok(*number),
        _ => Err(decode("expected u8")),
    }
}

fn as_u16(value: &DomainValue) -> Result<u16> {
    match value {
        DomainValue::UInt16(number) => Ok(*number),
        _ => Err(decode("expected u16")),
    }
}

fn as_u32(value: &DomainValue) -> Result<u32> {
    match value {
        DomainValue::UInt32(number) => Ok(*number),
        _ => Err(decode("expected u32")),
    }
}

fn as_u64(value: &DomainValue) -> Result<u64> {
    match value {
        DomainValue::UInt64(number) => Ok(*number),
        _ => Err(decode("expected u64")),
    }
}

fn decode(message: impl Into<String>) -> SerializationError {
    SerializationError::Decode(message.into())
}

struct Slots<'a> {
    remaining: &'a [(u16, DomainValue)],
}

impl<'a> Slots<'a> {
    fn parse(value: &'a DomainValue) -> Result<Self> {
        let DomainValue::Composite(fields) = value else {
            return Err(decode("expected composite"));
        };
        // The domain decoder validates strictly increasing, unique slots. Every
        // message reader below consumes them in order, so no lookup map is needed.
        Ok(Self { remaining: fields })
    }

    fn take(&mut self, slot: u16) -> Result<&'a DomainValue> {
        self.optional(slot).ok_or_else(|| decode(format!("missing slot {slot}")))
    }

    fn optional(&mut self, slot: u16) -> Option<&'a DomainValue> {
        let ((next_slot, value), rest) = self.remaining.split_first()?;
        if *next_slot != slot {
            return None;
        }
        self.remaining = rest;
        Some(value)
    }

    fn finish(self) -> Result<()> {
        if let Some((slot, _)) = self.remaining.first() {
            return Err(decode(format!("unknown slot {slot}")));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::encode_domain_frame;

    fn version() -> VersionId {
        VersionId::try_from_raw(42).expect("version")
    }

    #[test]
    fn client_messages_roundtrip_and_reject_messagepack() {
        let messages = vec![
            ClientMessage::Ping,
            ClientMessage::Unsubscribe {
                subscription_id: "sub-1".to_string(),
            },
            ClientMessage::NextBatch {
                subscription_id: "sub-1".to_string(),
                last_seq_id: Some(version()),
            },
            ClientMessage::Authenticate {
                credentials: WsAuthCredentials::Jwt {
                    token: "token-1".to_string(),
                },
                protocol: ProtocolOptions {
                    serialization: SerializationType::Domain,
                    compression: CompressionType::None,
                },
            },
        ];
        for message in messages {
            let bytes = encode_client_message(&message).expect("encode");
            assert_eq!(decode_client_message(&bytes).expect("decode"), message);
        }
        let error = decode_client_message(&[0x82, 0xa4]).expect_err("msgpack");
        assert!(matches!(error, SerializationError::UnsupportedFormat(_)));
    }

    #[test]
    fn server_change_preserves_empty_array_null_and_object_cells() {
        let mut row = HashMap::new();
        row.insert("id".to_string(), KalamCellValue::int(7));
        row.insert("tags".to_string(), KalamCellValue::from_json(serde_json::json!([])));
        row.insert("note".to_string(), KalamCellValue::null());
        row.insert(
            "file".to_string(),
            KalamCellValue::from_json(serde_json::json!({"id": "f1", "n": 1})),
        );
        let message = ServerMessage::Change {
            subscription_id: "sub-1".to_string(),
            change_type: ChangeTypeRaw::Insert,
            rows: Some(vec![row]),
            old_values: None,
        };
        let bytes = encode_server_message(&message).expect("encode");
        assert_eq!(decode_server_message(&bytes).expect("decode"), message);

        let rows = message_rows(&message);
        let cached = encode_named_rows(rows).expect("rows");
        let spliced = encode_change_frame("sub-1", "insert", Some(&cached), None).expect("splice");
        assert_eq!(decode_server_message(&spliced).expect("spliced"), message);
    }

    #[test]
    fn unknown_change_slot_is_rejected() {
        let payload = DomainValue::Composite(vec![
            (1, DomainValue::Utf8("sub-1".to_string())),
            (2, DomainValue::Utf8("insert".to_string())),
            (9, DomainValue::Bool(true)),
        ]);
        let bytes = encode_domain_frame(
            DomainMessageKind::WsChanges,
            WS_CONTRACT_DESCRIPTOR_ID,
            0,
            &payload,
        )
        .expect("frame");
        assert!(decode_server_message(&bytes).is_err());
    }

    #[test]
    fn stale_descriptor_is_rejected_before_a_message_is_returned() {
        let ping =
            encode_domain_frame(DomainMessageKind::WsPing, 0, 0, &DomainValue::Void).expect("ping");
        let ping_error = decode_client_message(&ping).expect_err("stale client descriptor");
        assert!(ping_error.to_string().contains("stale WebSocket descriptor"));

        let change = encode_domain_frame(
            DomainMessageKind::WsChanges,
            99,
            0,
            &DomainValue::Composite(vec![
                (1, DomainValue::Utf8("sub-1".to_string())),
                (2, DomainValue::Utf8("insert".to_string())),
            ]),
        )
        .expect("change");
        let change_error = decode_server_message(&change).expect_err("stale server descriptor");
        assert!(change_error.to_string().contains("current contract is 1"));
    }

    #[test]
    fn change_roundtrips_with_optional_slot_gaps() {
        for (rows, old_values) in [
            (None, None),
            (Some(vec![]), None),
            (None, Some(vec![])),
            (Some(vec![]), Some(vec![])),
        ] {
            let message = ServerMessage::Change {
                subscription_id: "sub-1".to_string(),
                change_type: ChangeTypeRaw::Update,
                rows,
                old_values,
            };
            let bytes = encode_server_message(&message).unwrap();
            assert_eq!(decode_server_message(&bytes).unwrap(), message);
        }
    }

    #[test]
    fn change_rejects_unknown_slots_before_between_and_after_known_slots() {
        for unknown in [0, 5, u16::MAX] {
            let mut fields = vec![
                (1, utf8("sub-1")),
                (2, utf8("insert")),
                (unknown, DomainValue::SqlNull),
            ];
            fields.sort_by_key(|(slot, _)| *slot);
            let bytes = encode_domain_frame(
                DomainMessageKind::WsChanges,
                WS_CONTRACT_DESCRIPTOR_ID,
                0,
                &DomainValue::Composite(fields),
            )
            .unwrap();
            assert!(decode_server_message(&bytes).is_err());
        }
    }

    #[test]
    fn object_cell_rejects_duplicate_keys() {
        let pair = || composite(vec![(1, utf8("same")), (2, DomainValue::Int64(1))]);
        let object = composite(vec![
            (1, utf8("object")),
            (2, DomainValue::Array(vec![pair(), pair()])),
        ]);
        let payload = composite(vec![
            (1, utf8("sub-1")),
            (2, utf8("insert")),
            (
                3,
                DomainValue::Array(vec![DomainValue::Array(vec![composite(vec![
                    (1, utf8("metadata")),
                    (2, object),
                ])])]),
            ),
        ]);
        let bytes = encode_domain_frame(
            DomainMessageKind::WsChanges,
            WS_CONTRACT_DESCRIPTOR_ID,
            0,
            &payload,
        )
        .unwrap();
        assert!(decode_server_message(&bytes)
            .unwrap_err()
            .to_string()
            .contains("duplicate object key"));
    }

    fn message_rows(message: &ServerMessage) -> &[HashMap<String, KalamCellValue>] {
        match message {
            ServerMessage::Change {
                rows: Some(rows), ..
            } => rows,
            _ => unreachable!("fixture is a change with rows"),
        }
    }
}
