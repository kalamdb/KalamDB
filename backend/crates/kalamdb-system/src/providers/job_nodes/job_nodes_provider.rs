//! System.job_nodes table provider

use std::sync::Arc;

use chrono::Utc;
use kalamdb_commons::{
    models::{rows::SystemTableRow, JobNodeId},
    next_storage_key_bytes, JobId, NodeId, StorageKey, SystemTable,
};
use kalamdb_store::{entity_store::EntityStore, IndexedEntityStore, StorageBackend};

use crate::{
    error::{SystemError, SystemResultExt},
    providers::{job_nodes::models::JobNode, keyed_provider::KeyedTableProvider},
    system_row_mapper::{model_to_system_row, system_row_to_model},
    JobStatus,
};

pub type JobNodesStore = IndexedEntityStore<JobNodeId, SystemTableRow>;

#[derive(Clone, Debug)]
pub struct JobNodesTableProvider {
    sql: KeyedTableProvider<JobNodeId, SystemTableRow>,
}

impl JobNodesTableProvider {
    pub fn new(backend: Arc<dyn StorageBackend>) -> Self {
        let store = IndexedEntityStore::new(
            backend,
            SystemTable::JobNodes
                .column_family_name()
                .expect("JobNodes is a table, not a view"),
            Vec::new(),
        );
        let sql = KeyedTableProvider::from_stored_rows(
            store,
            JobNode::definition(),
            SystemTable::JobNodes.table_id(),
            SystemTable::JobNodes.table_name(),
            "id",
            |value| JobNodeId::from_string(value).ok(),
        );
        Self { sql }
    }

    pub fn create_job_node(&self, job_node: JobNode) -> Result<String, SystemError> {
        let key = job_node.id();
        let row = Self::encode_job_node_row(&job_node)?;
        self.sql.store().insert(&key, &row).into_system_error("insert job_node error")?;
        Ok(format!("Job node {} created", key))
    }

    pub fn get_job_node(
        &self,
        job_id: &kalamdb_commons::JobId,
        node_id: &NodeId,
    ) -> Result<Option<JobNode>, SystemError> {
        let key = JobNodeId::new(job_id, node_id);
        let row = self.sql.store().get(&key)?;
        row.map(|value| Self::decode_job_node_row(&value)).transpose()
    }

    pub async fn get_job_node_async(
        &self,
        job_id: &kalamdb_commons::JobId,
        node_id: &NodeId,
    ) -> Result<Option<JobNode>, SystemError> {
        let key = JobNodeId::new(job_id, node_id);
        let row = self
            .sql
            .store()
            .get_async(key)
            .await
            .into_system_error("get_async job_node error")?;
        row.map(|value| Self::decode_job_node_row(&value)).transpose()
    }

    pub async fn update_job_node_async(&self, job_node: JobNode) -> Result<(), SystemError> {
        let key = job_node.id();
        let row = Self::encode_job_node_row(&job_node)?;
        self.sql
            .store()
            .insert_async(key, row)
            .await
            .into_system_error("update_async job_node error")
    }

    pub async fn list_for_node_with_statuses_async(
        &self,
        node_id: &NodeId,
        statuses: &[JobStatus],
        limit: usize,
    ) -> Result<Vec<JobNode>, SystemError> {
        const PAGE_SIZE: usize = 256;

        let prefix = JobNodeId::prefix_for_node(node_id);
        let mut start_key: Option<Vec<u8>> = None;
        let mut filtered = Vec::new();

        loop {
            let rows = {
                let store = self.sql.store().clone();
                let prefix = prefix.clone();
                let start_key = start_key.clone();
                tokio::task::spawn_blocking(move || {
                    store.scan_with_raw_prefix(&prefix, start_key.as_deref(), PAGE_SIZE)
                })
                .await
                .into_system_error("scan_async job_nodes join error")?
                .into_system_error("scan_async job_nodes error")?
            };

            if rows.is_empty() {
                break;
            }

            let next_start_key =
                rows.last().map(|(key, _)| next_storage_key_bytes(&key.storage_key()));

            for (_, row) in rows {
                let node = Self::decode_job_node_row(&row)?;
                if statuses.contains(&node.status) {
                    filtered.push(node);
                    if limit > 0 && filtered.len() >= limit {
                        return Ok(filtered);
                    }
                }
            }

            if next_start_key.is_none() {
                break;
            }
            start_key = next_start_key;
        }

        Ok(filtered)
    }

    pub async fn list_for_job_id_async(&self, job_id: &JobId) -> Result<Vec<JobNode>, SystemError> {
        let rows: Vec<(Vec<u8>, SystemTableRow)> = self
            .sql
            .store()
            .scan_all_async(None, None, None)
            .await
            .into_system_error("scan_async job_nodes error")?;
        let mut filtered = Vec::new();
        for (_, row) in rows {
            let node = Self::decode_job_node_row(&row)?;
            if &node.job_id == job_id {
                filtered.push(node);
            }
        }

        Ok(filtered)
    }

    /// Delete job_nodes older than retention period (in days).
    ///
    /// Only deletes terminal statuses: Completed, Failed, Cancelled.
    /// Uses finished_at/started_at/updated_at as reference time.
    pub fn cleanup_old_job_nodes(&self, retention_days: i64) -> Result<usize, SystemError> {
        let now = Utc::now().timestamp_millis();
        let retention_ms = retention_days * 24 * 60 * 60 * 1000;
        let cutoff_time = now - retention_ms;

        let rows = self
            .sql
            .store()
            .scan_all_typed(None, None, None)
            .into_system_error("scan job_nodes error")?;

        let mut deleted = 0;

        for (_key, row) in rows {
            let node = Self::decode_job_node_row(&row)?;
            if !matches!(
                node.status,
                JobStatus::Completed
                    | JobStatus::Failed
                    | JobStatus::Cancelled
                    | JobStatus::Skipped
            ) {
                continue;
            }

            let reference_time = node.finished_at.or(node.started_at).unwrap_or(node.updated_at);

            if reference_time < cutoff_time {
                self.sql.store().delete(&node.id()).into_system_error("delete job_node error")?;
                deleted += 1;
            }
        }

        Ok(deleted)
    }

    fn encode_job_node_row(job_node: &JobNode) -> Result<SystemTableRow, SystemError> {
        model_to_system_row(job_node, &JobNode::definition())
    }

    fn decode_job_node_row(row: &SystemTableRow) -> Result<JobNode, SystemError> {
        system_row_to_model(row, &JobNode::definition())
    }
}

crate::impl_keyed_table_provider!(JobNodesTableProvider);

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use kalamdb_store::{test_utils::InMemoryBackend, StorageBackend};

    use super::*;

    fn provider() -> JobNodesTableProvider {
        let backend: Arc<dyn StorageBackend> = Arc::new(InMemoryBackend::new());
        JobNodesTableProvider::new(backend)
    }

    fn job_node(job_id: &str, node_id: NodeId, status: JobStatus, now: i64) -> JobNode {
        JobNode {
            job_id: JobId::new(job_id),
            node_id,
            status,
            error_message: None,
            created_at: now,
            updated_at: now,
            started_at: None,
            finished_at: None,
        }
    }

    #[test]
    fn list_for_node_with_statuses_scans_past_terminal_entries() {
        let provider = provider();
        let node_id = NodeId::from(1u64);
        let now = Utc::now().timestamp_millis();

        for idx in 0..300 {
            provider
                .create_job_node(job_node(
                    &format!("AA-{idx:012}"),
                    node_id,
                    JobStatus::Completed,
                    now,
                ))
                .expect("create completed job_node");
        }

        provider
            .create_job_node(job_node("ZZ-queued", node_id, JobStatus::Queued, now))
            .expect("create queued job_node");

        let runtime = tokio::runtime::Runtime::new().expect("tokio runtime");
        let rows = runtime
            .block_on(provider.list_for_node_with_statuses_async(&node_id, &[JobStatus::Queued], 1))
            .expect("list queued job_nodes");

        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].job_id, JobId::new("ZZ-queued"));
    }
}
