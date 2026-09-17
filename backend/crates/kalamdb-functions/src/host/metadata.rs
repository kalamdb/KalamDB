//! Immutable, host-authenticated procedure identity.

use kalamdb_commons::{NamespaceId, Role, UserId};
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct ActorMeta {
    pub id:   UserId,
    pub role: Role,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InvocationMetadata {
    pub actor:      ActorMeta,
    pub principal:  ActorMeta,
    pub namespace:  NamespaceId,
    pub request_id: String,
}
