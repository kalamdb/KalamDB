//! System.migrations table provider.

use std::sync::Arc;

use kalamdb_commons::{
    models::{rows::SystemTableRow, MigrationId, SchemaId},
    SystemTable,
};
use kalamdb_store::{entity_store::EntityStore, IndexedEntityStore, StorageBackend};

use super::models::Migration;
use crate::{
    error::{SystemError, SystemResultExt},
    providers::keyed_provider::KeyedTableProvider,
    system_row_mapper::{model_to_system_row, system_row_to_model},
};

pub type MigrationsStore = IndexedEntityStore<MigrationId, SystemTableRow>;

#[derive(Clone, Debug)]
pub struct MigrationsTableProvider {
    sql: KeyedTableProvider<MigrationId, SystemTableRow>,
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
        let sql = KeyedTableProvider::from_stored_rows(
            store,
            Migration::definition(),
            SystemTable::Migrations.table_id(),
            SystemTable::Migrations.table_name(),
            "migration_key",
            |value| Some(MigrationId::new(value)),
        );
        Self { sql }
    }

    pub async fn upsert_migration_async(&self, migration: Migration) -> Result<(), SystemError> {
        let migration_key = migration.migration_key.clone();
        let row = Self::encode_migration_row(&migration)?;
        self.sql.store()
            .insert_async(migration_key, row)
            .await
            .into_system_error("insert_async migration error")
    }

    pub async fn get_migration_async(
        &self,
        migration_key: &MigrationId,
    ) -> Result<Option<Migration>, SystemError> {
        let row = self
            .sql
            .store()
            .get_async(migration_key.clone())
            .await
            .into_system_error("get_async migration error")?;
        row.map(|value| Self::decode_migration_row(&value)).transpose()
    }

    pub fn delete_migrations_for_namespace(
        &self,
        namespace_id: &SchemaId,
    ) -> Result<usize, SystemError> {
        let rows = self.sql.store().scan_all_typed(None, None, None)?;
        let mut keys = Vec::new();
        for (key, row) in rows {
            let migration = Self::decode_migration_row(&row)?;
            if migration.namespace == namespace_id.as_str() {
                keys.push(key);
            }
        }
        let deleted = keys.len();
        self.sql.store()
            .delete_batch(&keys)
            .into_system_error("delete namespace migrations error")?;
        Ok(deleted)
    }

    fn encode_migration_row(migration: &Migration) -> Result<SystemTableRow, SystemError> {
        model_to_system_row(migration, &Migration::definition())
    }

    fn decode_migration_row(row: &SystemTableRow) -> Result<Migration, SystemError> {
        system_row_to_model(row, &Migration::definition())
    }
}

crate::impl_keyed_table_provider!(MigrationsTableProvider);

#[cfg(test)]
mod tests {
    use kalamdb_store::test_utils::InMemoryBackend;

    use super::*;

    #[tokio::test]
    async fn upsert_and_get_migration() {
        let provider = MigrationsTableProvider::new(Arc::new(InMemoryBackend::new()));
        let migration = Migration {
            migration_key: MigrationId::new("app:0001_init.sql"),
            migration_id: "0001_init.sql".to_string(),
            namespace: "app".to_string(),
            name: "init".to_string(),
            checksum: "abc".to_string(),
            status: "applied".to_string(),
            started_at: Some(1_700_000_000_000),
            finished_at: Some(1_700_000_000_100),
            error_message: None,
            source: Some("0001_init.sql".to_string()),
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
            migration_id: "0001_init.sql".to_string(),
            namespace: "app".to_string(),
            name: "init".to_string(),
            checksum: "abc".to_string(),
            status: "applied".to_string(),
            started_at: Some(1_700_000_000_000),
            finished_at: Some(1_700_000_000_100),
            error_message: None,
            source: Some("0001_init.sql".to_string()),
            kalam_version: Some("test".to_string()),
        };
        let prod_migration = Migration {
            migration_key: MigrationId::new("prod:0001_init.sql"),
            namespace: "prod".to_string(),
            ..app_migration.clone()
        };

        provider.upsert_migration_async(app_migration).await.unwrap();
        provider.upsert_migration_async(prod_migration).await.unwrap();

        let deleted = provider.delete_migrations_for_namespace(&SchemaId::new("app")).unwrap();

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
