use kalamdb_commons::{models::ReadContext, NamespaceId, Role, UserId};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(super) struct PointReadSessionCacheKey {
    user_id: UserId,
    role: Role,
    namespace_id: NamespaceId,
    read_context: ReadContext,
}

impl PointReadSessionCacheKey {
    pub(super) fn new(
        user_id: UserId,
        role: Role,
        namespace_id: NamespaceId,
        read_context: ReadContext,
    ) -> Self {
        Self {
            user_id,
            role,
            namespace_id,
            read_context,
        }
    }
}
