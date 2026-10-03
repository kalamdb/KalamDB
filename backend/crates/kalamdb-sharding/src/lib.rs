mod group_id;

use std::hash::{Hash, Hasher};

// Re-export GroupId and related types
pub use group_id::{GroupId, DEFAULT_SHARED_SHARDS, DEFAULT_USER_SHARDS};
use kalamdb_commons::{
    models::{TableId, UserId},
    schemas::{TableDefinition, TableOptions, TableType},
};
// Re-export cluster config types for shared consumption
pub use kalamdb_configs::{ClusterConfig, PeerConfig};
#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

/// Shard kind used across stream and data shards.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum ShardKind {
    User,
    Shared,
    Stream,
}

/// Shard identifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct Shard {
    kind: ShardKind,
    id:   u32,
}

impl Shard {
    pub fn new(kind: ShardKind, id: u32) -> Self {
        Self { kind, id }
    }

    pub fn kind(&self) -> ShardKind {
        self.kind
    }

    pub fn id(&self) -> u32 {
        self.id
    }

    /// Folder name used in storage paths.
    pub fn folder_name(&self) -> String {
        format!("shard_{}", self.id)
    }
}

/// Routes operations to the correct shard based on user_id or table_id.
#[derive(Debug, Clone)]
pub struct ShardRouter {
    num_user_shards:   u32,
    num_shared_shards: u32,
}

impl ShardRouter {
    pub fn new(num_user_shards: u32, num_shared_shards: u32) -> Self {
        Self {
            num_user_shards,
            num_shared_shards,
        }
    }

    pub fn from_cluster_config(config: &ClusterConfig) -> Self {
        Self::new(config.user_shards, config.shared_shards)
    }

    pub fn from_optional_cluster_config(config: Option<&ClusterConfig>) -> Self {
        config.map(Self::from_cluster_config).unwrap_or_else(Self::default_config)
    }

    pub fn default_config() -> Self {
        Self::new(32, 1)
    }

    pub fn route_user(&self, user_id: &UserId) -> Shard {
        let shard = self.hash_to_shard(user_id.as_str(), self.num_user_shards.max(1));
        Shard::new(ShardKind::User, shard)
    }

    pub fn user_shard_id(&self, user_id: &UserId) -> u32 {
        self.route_user(user_id).id()
    }

    pub fn user_group_id(&self, user_id: &UserId) -> GroupId {
        GroupId::DataUserShard(self.user_shard_id(user_id))
    }

    pub fn route_stream_user(&self, user_id: &UserId) -> Shard {
        let shard = self.hash_to_shard(user_id.as_str(), self.num_user_shards.max(1));
        Shard::new(ShardKind::Stream, shard)
    }

    pub fn route_table(&self, table_id: &TableId) -> Shard {
        let shard = self.hash_table_to_shard(table_id, self.num_user_shards.max(1));
        Shard::new(ShardKind::User, shard)
    }

    pub fn table_shard_id(&self, table_id: &TableId) -> u32 {
        self.route_table(table_id).id()
    }

    pub fn table_group_id(&self, table_id: &TableId) -> GroupId {
        GroupId::DataUserShard(self.table_shard_id(table_id))
    }

    /// Placement policy for NEW tables only. Persist the result before proposing CREATE.
    pub fn place_shared_table(&self, table_id: &TableId) -> u32 {
        self.hash_table_to_shard(table_id, self.num_shared_shards.max(1))
    }

    /// Resolve existing metadata without applying the placement policy again.
    pub fn shared_group_id(&self, table: &TableDefinition) -> Result<GroupId, String> {
        let TableOptions::Shared(options) = &table.table_options else {
            return Err(format!("'{}' is not a shared table", table.table_id()));
        };
        if table.table_type != TableType::Shared
            || options.shared_shard_id >= self.num_shared_shards
        {
            return Err(format!(
                "Invalid or unavailable shared owner {} for '{}'",
                options.shared_shard_id,
                table.table_id()
            ));
        }
        Ok(GroupId::DataSharedShard(options.shared_shard_id))
    }

    pub fn num_user_shards(&self) -> u32 {
        self.num_user_shards
    }

    pub fn num_shared_shards(&self) -> u32 {
        self.num_shared_shards
    }

    fn hash_to_shard(&self, key: &str, num_shards: u32) -> u32 {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        key.hash(&mut hasher);
        let hash = hasher.finish();
        (hash % num_shards as u64) as u32
    }

    fn hash_table_to_shard(&self, table_id: &TableId, num_shards: u32) -> u32 {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        table_id.namespace_id().as_str().hash(&mut hasher);
        table_id.table_name().as_str().hash(&mut hasher);
        let hash = hasher.finish();
        (hash % num_shards as u64) as u32
    }
}

#[cfg(test)]
mod ownership_tests {
    use std::sync::Arc;

    use super::*;

    fn table(name: &str) -> TableDefinition {
        TableDefinition::new_with_defaults(
            "app".into(),
            name.into(),
            TableType::Shared,
            vec![],
            None,
        )
        .unwrap()
    }

    #[test]
    fn persisted_owners_survive_growth_and_concurrent_resolution() {
        let original = ShardRouter::new(8, 4);
        let expanded = ShardRouter::new(8, 16);
        let mut distribution = [0usize; 4];
        let tables: Vec<_> = (0..2048)
            .map(|i| {
                let mut table = table(&format!("t_{i}"));
                let owner = original.place_shared_table(&table.table_id());
                if let TableOptions::Shared(options) = &mut table.table_options {
                    options.shared_shard_id = owner;
                }
                distribution[owner as usize] += 1;
                (table, GroupId::DataSharedShard(owner))
            })
            .collect();
        assert!(distribution.iter().all(|count| *count > 1));
        let tables = Arc::new(tables);
        std::thread::scope(|scope| {
            for _ in 0..8 {
                let tables = Arc::clone(&tables);
                let router = expanded.clone();
                scope.spawn(move || {
                    for (table, owner) in tables.iter() {
                        assert_eq!(router.shared_group_id(table).unwrap(), *owner);
                    }
                });
            }
        });
    }

    #[test]
    fn legacy_ownership_and_invalid_metadata_fail_closed() {
        let router = ShardRouter::new(8, 4);
        let mut table = table("legacy");
        assert_eq!(router.shared_group_id(&table).unwrap(), GroupId::DataSharedShard(0));
        if let TableOptions::Shared(options) = &mut table.table_options {
            options.shared_shard_id = 4;
        }
        assert!(router.shared_group_id(&table).is_err());
        table.table_type = TableType::User;
        assert!(router.shared_group_id(&table).is_err());
    }
}
