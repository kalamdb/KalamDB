//! System.namespaces table provider
//!
//! This module provides a DataFusion TableProvider implementation for the system.namespaces table.
//! Uses the new EntityStore architecture with SchemaId keys.

use std::sync::Arc;

use kalamdb_commons::{models::rows::SystemTableRow, SchemaId, SystemTable};
use kalamdb_store::{entity_store::EntityStore, IndexedEntityStore, StorageBackend};

use crate::{
    error::{SystemError, SystemResultExt},
    providers::{base::SimpleSystemTableScan, keyed_provider::KeyedTableProvider, schemas::models::Namespace},
    system_row_mapper::{model_to_system_row, system_row_to_model},
};

/// System.namespaces table provider using EntityStore architecture
#[derive(Clone, Debug)]
pub struct NamespacesTableProvider {
    sql: KeyedTableProvider<SchemaId, SystemTableRow>,
}

impl NamespacesTableProvider {
    /// Create a new namespaces table provider
    ///
    /// # Arguments
    /// * `backend` - Storage backend (RocksDB or mock)
    ///
    /// # Returns
    /// A new NamespacesTableProvider instance
    pub fn new(backend: Arc<dyn StorageBackend>) -> Self {
        let store = IndexedEntityStore::new(
            backend,
            crate::SystemTable::Namespaces
                .column_family_name()
                .expect("Namespaces is a table"),
            Vec::new(),
        );
        let sql = KeyedTableProvider::from_stored_rows(
            store,
            Namespace::definition(),
            SystemTable::Namespaces.table_id(),
            SystemTable::Namespaces.table_name(),
            "namespace_id",
            |value| Some(SchemaId::new(value)),
        );
        Self { sql }
    }

    /// Create a new namespace entry
    pub fn create_namespace(&self, namespace: Namespace) -> Result<(), SystemError> {
        let row = Self::encode_namespace_row(&namespace)?;
        self.sql.store().put(&namespace.namespace_id, &row)?;
        Ok(())
    }

    /// Get a namespace by ID
    pub fn get_namespace(&self, namespace_id: &SchemaId) -> Result<Option<Namespace>, SystemError> {
        let row = self.sql.store().get(namespace_id)?;
        row.map(|value| Self::decode_namespace_row(&value)).transpose()
    }

    /// Async version of `get_namespace()` - offloads to blocking thread pool.
    ///
    /// Use this in async contexts to avoid blocking the Tokio runtime.
    pub async fn get_namespace_async(
        &self,
        namespace_id: &SchemaId,
    ) -> Result<Option<Namespace>, SystemError> {
        let row = self
            .sql
            .store()
            .get_async(namespace_id.clone())
            .await
            .into_system_error("get_async error")?;
        row.map(|value| Self::decode_namespace_row(&value)).transpose()
    }

    /// Update an existing namespace entry
    pub fn update_namespace(&self, namespace: Namespace) -> Result<(), SystemError> {
        // Check if namespace exists
        if self.sql.store().get(&namespace.namespace_id)?.is_none() {
            return Err(SystemError::NotFound(format!(
                "Namespace not found: {}",
                namespace.namespace_id
            )));
        }

        let row = Self::encode_namespace_row(&namespace)?;
        self.sql.store().put(&namespace.namespace_id, &row)?;
        Ok(())
    }

    /// Delete a namespace entry
    pub fn delete_namespace(&self, namespace_id: &SchemaId) -> Result<(), SystemError> {
        self.sql.store().delete(namespace_id)?;
        Ok(())
    }

    /// List all namespaces
    pub fn list_namespaces(&self) -> Result<Vec<Namespace>, SystemError> {
        let iter = self.sql.store().scan_iterator(None, None)?;
        let mut namespaces = Vec::new();
        for item in iter {
            let (_, row) = item?;
            namespaces.push(Self::decode_namespace_row(&row)?);
        }
        Ok(namespaces)
    }

    /// Scan all namespaces and return as RecordBatch
    pub fn scan_all_namespaces(&self) -> Result<datafusion::arrow::array::RecordBatch, SystemError> {
        self.sql.scan_all_to_batch()
    }

    fn encode_namespace_row(namespace: &Namespace) -> Result<SystemTableRow, SystemError> {
        model_to_system_row(namespace, &Namespace::definition())
    }

    fn decode_namespace_row(row: &SystemTableRow) -> Result<Namespace, SystemError> {
        system_row_to_model(row, &Namespace::definition())
    }
}

crate::impl_keyed_table_provider!(NamespacesTableProvider);

#[cfg(test)]
mod tests {
    use datafusion::datasource::TableProvider;
    use kalamdb_store::test_utils::InMemoryBackend;

    use super::*;

    fn create_test_provider() -> NamespacesTableProvider {
        let backend: Arc<dyn StorageBackend> = Arc::new(InMemoryBackend::new());
        NamespacesTableProvider::new(backend)
    }

    fn create_test_namespace(namespace_id: &SchemaId, name: &str) -> Namespace {
        Namespace {
            namespace_id: namespace_id.clone(),
            name: name.to_string(),
            created_at: 1000,
            options: Some(serde_json::json!({})),
            table_count: 0,
        }
    }

    #[test]
    fn get_namespace_reads_the_entity_store_without_a_query_plan() {
        let backend = Arc::new(InMemoryBackend::new());
        let provider = NamespacesTableProvider::new(backend.clone());
        provider
            .create_namespace(create_test_namespace(&SchemaId::new("app"), "app"))
            .unwrap();
        backend.clear_recorded_ops();

        let namespace = provider.get_namespace(&SchemaId::new("app")).unwrap().unwrap();

        assert_eq!(namespace.name, "app");
        let partition = SystemTable::Namespaces
            .column_family_name()
            .expect("namespaces is a table")
            .to_string();
        let ops = backend.recorded_ops();
        assert!(!ops.is_empty());
        assert!(
            ops.iter().all(|(kind, name)| *kind == "get" && name == &partition),
            "get_namespace must call IndexedEntityStore::get, recorded {ops:?}"
        );
    }

    #[test]
    fn test_create_and_get_namespace() {
        let provider = create_test_provider();
        let namespace = create_test_namespace(&SchemaId::new("app"), "app");

        provider.create_namespace(namespace.clone()).unwrap();

        let namespace_id = SchemaId::new("app");
        let retrieved = provider.get_namespace(&namespace_id).unwrap();
        assert!(retrieved.is_some());
        let retrieved = retrieved.unwrap();
        assert_eq!(retrieved.namespace_id, namespace_id);
        assert_eq!(retrieved.name, "app");
    }

    #[test]
    fn test_update_namespace() {
        let provider = create_test_provider();
        let mut namespace = create_test_namespace(&SchemaId::new("app"), "app");
        provider.create_namespace(namespace.clone()).unwrap();

        // Update
        namespace.table_count = 5;
        provider.update_namespace(namespace.clone()).unwrap();

        // Verify
        let namespace_id = SchemaId::new("app");
        let retrieved = provider.get_namespace(&namespace_id).unwrap().unwrap();
        assert_eq!(retrieved.table_count, 5);
    }

    #[test]
    fn test_delete_namespace() {
        let provider = create_test_provider();
        let namespace = create_test_namespace(&SchemaId::new("app"), "app");

        provider.create_namespace(namespace).unwrap();

        let namespace_id = SchemaId::new("app");
        provider.delete_namespace(&namespace_id).unwrap();

        let retrieved = provider.get_namespace(&namespace_id).unwrap();
        assert!(retrieved.is_none());
    }

    #[test]
    fn test_list_namespaces() {
        let provider = create_test_provider();

        // Insert multiple namespaces
        for i in 1..=3 {
            let namespace = create_test_namespace(
                &SchemaId::new(&format!("ns{}", i)),
                &format!("namespace{}", i),
            );
            provider.create_namespace(namespace).unwrap();
        }

        // List
        let namespaces = provider.list_namespaces().unwrap();
        assert_eq!(namespaces.len(), 3);
    }

    #[test]
    fn test_scan_all_namespaces() {
        let provider = create_test_provider();

        // Insert test data
        let namespace = create_test_namespace(&SchemaId::new("app"), "app");
        provider.create_namespace(namespace).unwrap();

        // Scan
        let batch = provider.scan_all_namespaces().unwrap();
        assert_eq!(batch.num_rows(), 1);
        assert_eq!(batch.num_columns(), 5);
    }

    #[tokio::test]
    async fn test_datafusion_scan() {
        let provider = create_test_provider();

        // Insert test data
        let namespace = create_test_namespace(&SchemaId::new("app"), "app");
        provider.create_namespace(namespace).unwrap();

        // Create DataFusion session
        let ctx = datafusion::execution::context::SessionContext::new();
        let mut state = ctx.state();
        state.config_mut().options_mut().extensions.insert(
            kalamdb_session_datafusion::SessionUserContext::new(
                kalamdb_commons::models::UserId::new("system"),
                kalamdb_commons::models::Role::System,
                kalamdb_commons::models::ReadContext::Internal,
            ),
        );

        let plan = provider.scan(&state, None, &[], None).await.unwrap();
        assert!(!plan.schema().fields().is_empty());
    }
}
