//! System.migrations table provider.

use std::sync::{Arc, OnceLock};

use datafusion::arrow::{array::RecordBatch, datatypes::SchemaRef};
use kalamdb_commons::{
    models::{rows::SystemTableRow, MigrationId, NamespaceId},
    SystemTable,
};
use kalamdb_store::{entity_store::EntityStore, IndexedEntityStore, StorageBackend};

use super::models::Migration;
use crate::{
    error::{SystemError, SystemResultExt},
    providers::base::{system_rows_to_batch, IndexedProviderDefinition},
    system_row_mapper::{model_to_system_row, system_row_to_model},
};

pub type MigrationsStore = IndexedEntityStore<MigrationId, SystemTableRow>;

#[derive(Clone)]
pub struct MigrationsTableProvider {
    store: MigrationsStore,
}

impl MigrationsTableProvider {
    pub fn new(backend: Arc<dyn StorageBackend>) -> Self {
        let store = IndexedEntityStore::new(
            backend,
            SystemTable::Migrations
                .column_family_name()
                .expect("Migrations is a table, not a view"),
            Vec::new(),
        );
        Self { store }
    }

    pub async fn upsert_migration_async(&self, migration: Migration) -> Result<(), SystemError> {
        let migration_key = migration.migration_key.clone();
        let row = Self::encode_migration_row(&migration)?;
        self.store
            .insert_async(migration_key, row)
            .await
            .into_system_error("insert_async migration error")
    }

    pub async fn get_migration_async(
        &self,
        migration_key: &MigrationId,
    ) -> Result<Option<Migration>, SystemError> {
        let row = self
            .store
            .get_async(migration_key.clone())
            .await
            .into_system_error("get_async migration error")?;
        row.map(|value| Self::decode_migration_row(&value)).transpose()
    }

    pub fn list_migrations(&self) -> Result<Vec<Migration>, SystemError> {
        let rows = self.store.scan_all_typed(None, None, None)?;
        rows.into_iter().map(|(_, row)| Self::decode_migration_row(&row)).collect()
    }

    pub fn delete_migrations_for_namespace(
        &self,
        namespace_id: &NamespaceId,
    ) -> Result<usize, SystemError> {
        let rows = self.store.scan_all_typed(None, None, None)?;
        let mut keys = Vec::new();
        for (key, row) in rows {
            let migration = Self::decode_migration_row(&row)?;
            if migration.namespace == namespace_id.as_str() {
                keys.push(key);
            }
        }
        let deleted = keys.len();
        self.store
            .delete_batch(&keys)
            .into_system_error("delete namespace migrations error")?;
        Ok(deleted)
    }

    fn build_batch_from_pairs(
        &self,
        pairs: Vec<(MigrationId, SystemTableRow)>,
    ) -> Result<RecordBatch, SystemError> {
        let rows = pairs.into_iter().map(|(_, row)| row).collect();
        system_rows_to_batch(&Self::schema(), rows)
    }

    fn encode_migration_row(migration: &Migration) -> Result<SystemTableRow, SystemError> {
        model_to_system_row(migration, &Migration::definition())
    }

    fn decode_migration_row(row: &SystemTableRow) -> Result<Migration, SystemError> {
        system_row_to_model(row, &Migration::definition())
    }
}

crate::impl_system_table_provider_metadata!(
    indexed,
    provider = MigrationsTableProvider,
    key = MigrationId,
    table_name = SystemTable::Migrations.table_name(),
    primary_key_column = "migration_key",
    parse_key = |value| Some(MigrationId::new(value)),
    schema = Migration::definition()
        .to_arrow_schema()
        .expect("failed to build migrations schema")
);

crate::impl_indexed_system_table_provider!(
    provider = MigrationsTableProvider,
    key = MigrationId,
    value = SystemTableRow,
    store = store,
    definition = provider_definition,
    build_batch = build_batch_from_pairs
);

#[cfg(test)]
mod tests {
    use kalamdb_store::test_utils::InMemoryBackend;

    use super::*;

    #[tokio::test]
    async fn upsert_and_get_migration() {
        let provider = MigrationsTableProvider::new(Arc::new(InMemoryBackend::new()));
        let migration = Migration {
            migration_key: MigrationId::new("app:0001_init.sql"),
            migration_id:  "0001_init.sql".to_string(),
            namespace:     "app".to_string(),
            name:          "init".to_string(),
            checksum:      "abc".to_string(),
            status:        "applied".to_string(),
            started_at:    Some(1_700_000_000_000),
            finished_at:   Some(1_700_000_000_100),
            error_message: None,
            source:        Some("0001_init.sql".to_string()),
            kalam_version: Some("test".to_string()),
        };

        provider.upsert_migration_async(migration.clone()).await.unwrap();

        let found = provider
            .get_migration_async(&MigrationId::new("app:0001_init.sql"))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(found, migration);
    }

    #[tokio::test]
    async fn delete_migrations_for_namespace_only_removes_matching_namespace() {
        let provider = MigrationsTableProvider::new(Arc::new(InMemoryBackend::new()));
        let app_migration = Migration {
            migration_key: MigrationId::new("app:0001_init.sql"),
            migration_id:  "0001_init.sql".to_string(),
            namespace:     "app".to_string(),
            name:          "init".to_string(),
            checksum:      "abc".to_string(),
            status:        "applied".to_string(),
            started_at:    Some(1_700_000_000_000),
            finished_at:   Some(1_700_000_000_100),
            error_message: None,
            source:        Some("0001_init.sql".to_string()),
            kalam_version: Some("test".to_string()),
        };
        let prod_migration = Migration {
            migration_key: MigrationId::new("prod:0001_init.sql"),
            namespace: "prod".to_string(),
            ..app_migration.clone()
        };

        provider.upsert_migration_async(app_migration).await.unwrap();
        provider.upsert_migration_async(prod_migration).await.unwrap();

        let deleted = provider.delete_migrations_for_namespace(&NamespaceId::new("app")).unwrap();

        assert_eq!(deleted, 1);
        assert!(provider
            .get_migration_async(&MigrationId::new("app:0001_init.sql"))
            .await
            .unwrap()
            .is_none());
        assert!(provider
            .get_migration_async(&MigrationId::new("prod:0001_init.sql"))
            .await
            .unwrap()
            .is_some());
    }
}
