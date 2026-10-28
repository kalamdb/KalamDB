//! system.cluster_groups virtual view
//!
//! **Type**: Virtual View (not backed by persistent storage)
//!
//! Provides per-Raft-group OpenRaft metrics directly from RaftMetrics.
//! Each row represents one Raft group's current state on this node.

use std::sync::Arc;

use datafusion::arrow::{
    array::{ArrayRef, Int64Array, StringArray},
    datatypes::SchemaRef,
    record_batch::RecordBatch,
};
use kalamdb_commons::schemas::TableDefinition;
use kalamdb_macros::table;
use kalamdb_raft::{CommandExecutor, GroupId};
use kalamdb_system::SystemTable;

use super::common::{view_provider_with, Json, SystemViewProvider};
use crate::{error::RegistryError, view_base::VirtualView};

crate::memoized_view_schema!(cluster_groups_schema, ClusterGroupsView);

#[table(
    name = "cluster_groups",
    view = true,
    comment = "Per-Raft-group membership and replication status (read-only view)"
)]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClusterGroupRow {
    /// Raft group numeric ID
    pub group_id: i64,
    /// Cluster identifier
    pub cluster_id: String,
    /// Raft node ID (this node's ID for the group)
    pub node_id: i64,
    /// Group type: meta, user_data, shared_data
    pub group_type: String,
    /// Current Raft term
    pub current_term: Option<i64>,
    /// Last accepted vote (JSON format)
    pub vote: Option<Json>,
    /// Last log index appended to this node's log
    pub last_log_index: Option<i64>,
    /// Last log index applied to state machine
    pub last_applied: Option<i64>,
    /// Last log index included in snapshot
    pub snapshot: Option<i64>,
    /// Last log index purged from storage (inclusive)
    pub purged: Option<i64>,
    /// Last log index committed by the cluster
    pub committed: Option<i64>,
    /// Server state: Leader, Follower, Candidate, Learner, Shutdown
    pub state: Option<String>,
    /// Current cluster leader node ID
    pub current_leader: Option<i64>,
    /// Milliseconds since most recent quorum acknowledgment (leader only)
    pub millis_since_quorum_ack: Option<i64>,
}

#[derive(Debug)]
pub struct ClusterGroupsView {
    executor: Arc<dyn CommandExecutor>,
}

impl ClusterGroupsView {
    pub fn definition() -> TableDefinition {
        ClusterGroupRow::definition()
    }

    pub fn new(executor: Arc<dyn CommandExecutor>) -> Self {
        Self { executor }
    }
}

impl VirtualView for ClusterGroupsView {
    fn system_table(&self) -> SystemTable {
        SystemTable::ClusterGroups
    }

    fn schema(&self) -> SchemaRef {
        cluster_groups_schema()
    }

    fn compute_batch(&self) -> Result<RecordBatch, RegistryError> {
        let info = self.executor.get_cluster_info();

        // Try to downcast to RaftExecutor so we can read per-group OpenRaft metrics.
        let raft_exec = self.executor.as_any().downcast_ref::<kalamdb_raft::RaftExecutor>();

        // Build ALL expected group IDs from configuration (source of truth)
        let mut group_ids = Vec::with_capacity(info.total_groups as usize);
        group_ids.push(GroupId::Meta);
        for shard in 0..info.user_shards {
            group_ids.push(GroupId::DataUserShard(shard));
        }
        for shard in 0..info.shared_shards {
            group_ids.push(GroupId::DataSharedShard(shard));
        }

        // log::info!(
        //     "cluster_groups: Building view with {} user_shards, {} shared_shards = {} total
        // groups",     info.user_shards, info.shared_shards, group_ids.len()
        // );

        // Pre-allocate vectors for each column
        let estimated_rows = group_ids.len();
        let mut cluster_ids: Vec<&str> = Vec::with_capacity(estimated_rows);
        let mut node_ids: Vec<i64> = Vec::with_capacity(estimated_rows);
        let mut group_id_nums: Vec<i64> = Vec::with_capacity(estimated_rows);
        let mut group_types: Vec<&str> = Vec::with_capacity(estimated_rows);
        let mut current_terms: Vec<Option<i64>> = Vec::with_capacity(estimated_rows);
        let mut votes: Vec<Option<String>> = Vec::with_capacity(estimated_rows);
        let mut last_log_indexes: Vec<Option<i64>> = Vec::with_capacity(estimated_rows);
        let mut last_applieds: Vec<Option<i64>> = Vec::with_capacity(estimated_rows);
        let mut snapshots: Vec<Option<i64>> = Vec::with_capacity(estimated_rows);
        let mut purgeds: Vec<Option<i64>> = Vec::with_capacity(estimated_rows);
        let mut committeds: Vec<Option<i64>> = Vec::with_capacity(estimated_rows);
        let mut states: Vec<Option<String>> = Vec::with_capacity(estimated_rows);
        let mut current_leaders: Vec<Option<i64>> = Vec::with_capacity(estimated_rows);
        let mut millis_since_quorum_acks: Vec<Option<i64>> = Vec::with_capacity(estimated_rows);

        for gid in &group_ids {
            // New order: group_id, cluster_id, node_id, group_type, ...
            group_id_nums.push(gid.as_u64() as i64);
            cluster_ids.push(info.cluster_id.as_str());

            // Determine group type
            let group_type_str = match gid {
                GroupId::Meta => "meta",
                GroupId::DataUserShard(_) => "user_data",
                GroupId::DataSharedShard(_) => "shared_data",
            };
            group_types.push(group_type_str);

            // Try to get metrics for this group
            let metrics_opt = raft_exec.and_then(|re| re.manager().group_metrics(*gid));

            if let Some(metrics) = metrics_opt {
                // Metrics available - populate from RaftMetrics
                node_ids.push(metrics.id as i64);
                current_terms.push(Some(metrics.current_term as i64));
                votes.push(Some(format!("{:?}", metrics.vote)));
                last_log_indexes.push(metrics.last_log_index.map(|idx| idx as i64));
                last_applieds.push(metrics.last_applied.map(|log_id| log_id.index as i64));
                snapshots.push(metrics.snapshot.map(|log_id| log_id.index as i64));
                purgeds.push(metrics.purged.map(|log_id| log_id.index as i64));
                // Note: RaftMetrics doesn't directly expose committed; we'd need to get it from
                // storage/log For now, use None
                committeds.push(None);
                states.push(Some(format!("{:?}", metrics.state)));
                current_leaders.push(metrics.current_leader.map(|id| id as i64));
                millis_since_quorum_acks.push(metrics.millis_since_quorum_ack.map(|ms| ms as i64));
            } else {
                // No metrics available - use defaults/NULLs
                node_ids.push(info.current_node_id.as_u64() as i64);
                current_terms.push(None);
                votes.push(None);
                last_log_indexes.push(None);
                last_applieds.push(None);
                snapshots.push(None);
                purgeds.push(None);
                committeds.push(None);
                states.push(None);
                current_leaders.push(None);
                millis_since_quorum_acks.push(None);
            }
        }

        // log::info!(
        //     "cluster_groups: Created {} rows from {} groups",
        //     cluster_ids.len(), group_ids.len()
        // );

        RecordBatch::try_new(
            self.schema(),
            vec![
                Arc::new(Int64Array::from(group_id_nums)) as ArrayRef,
                Arc::new(StringArray::from(cluster_ids)) as ArrayRef,
                Arc::new(Int64Array::from(node_ids)) as ArrayRef,
                Arc::new(StringArray::from(group_types)) as ArrayRef,
                Arc::new(Int64Array::from(current_terms)) as ArrayRef,
                {
                    let refs: Vec<Option<&str>> = votes.iter().map(|s| s.as_deref()).collect();
                    Arc::new(StringArray::from(refs)) as ArrayRef
                },
                Arc::new(Int64Array::from(last_log_indexes)) as ArrayRef,
                Arc::new(Int64Array::from(last_applieds)) as ArrayRef,
                Arc::new(Int64Array::from(snapshots)) as ArrayRef,
                Arc::new(Int64Array::from(purgeds)) as ArrayRef,
                Arc::new(Int64Array::from(committeds)) as ArrayRef,
                {
                    let refs: Vec<Option<&str>> = states.iter().map(|s| s.as_deref()).collect();
                    Arc::new(StringArray::from(refs)) as ArrayRef
                },
                Arc::new(Int64Array::from(current_leaders)) as ArrayRef,
                Arc::new(Int64Array::from(millis_since_quorum_acks)) as ArrayRef,
            ],
        )
        .map_err(|e| RegistryError::Other(format!("Failed to build cluster_groups batch: {}", e)))
    }
}

pub type ClusterGroupsTableProvider = SystemViewProvider<ClusterGroupsView>;

pub fn create_cluster_groups_provider(
    executor: Arc<dyn CommandExecutor>,
) -> ClusterGroupsTableProvider {
    view_provider_with(executor, ClusterGroupsView::new)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_schema() {
        let schema = cluster_groups_schema();
        assert_eq!(schema.fields().len(), 14);
        assert_eq!(schema.field(0).name(), "group_id");
        assert_eq!(schema.field(1).name(), "cluster_id");
        assert_eq!(schema.field(2).name(), "node_id");
        assert_eq!(schema.field(3).name(), "group_type");
        assert_eq!(schema.field(4).name(), "current_term");
        assert_eq!(schema.field(5).name(), "vote");
        assert_eq!(schema.field(6).name(), "last_log_index");
        assert_eq!(schema.field(7).name(), "last_applied");
        assert_eq!(schema.field(8).name(), "snapshot");
        assert_eq!(schema.field(9).name(), "purged");
        assert_eq!(schema.field(10).name(), "committed");
        assert_eq!(schema.field(11).name(), "state");
        assert_eq!(schema.field(12).name(), "current_leader");
        assert_eq!(schema.field(13).name(), "millis_since_quorum_ack");
    }
}
